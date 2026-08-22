use ahash::AHashMap;

use super::audit;
use super::rule::LintRule;
use super::rules;
use super::types::{LintConfig, RuleCategory, Severity};
use scythe_core::dialect::SqlDialect;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

pub struct RuleRegistry {
    rules: Vec<Box<dyn LintRule>>,
    severity_overrides: AHashMap<String, Severity>,
    category_overrides: AHashMap<RuleCategory, Severity>,
}

impl Default for RuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleRegistry {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            severity_overrides: AHashMap::new(),
            category_overrides: AHashMap::new(),
        }
    }

    /// Register a new rule.
    pub fn register(&mut self, rule: Box<dyn LintRule>) {
        self.rules.push(rule);
    }

    /// Apply a lint configuration (category and per-rule overrides).
    pub fn apply_config(&mut self, config: &LintConfig) {
        for (&cat, &sev) in &config.categories {
            self.category_overrides.insert(cat, sev);
        }
        for (id, &sev) in &config.rules {
            self.severity_overrides.insert(id.clone(), sev);
        }
    }

    /// Return the effective severity for a given rule.
    pub fn effective_severity(&self, rule: &dyn LintRule) -> Severity {
        if let Some(&sev) = self.severity_overrides.get(rule.id()) {
            return sev;
        }
        if let Some(&sev) = self.category_overrides.get(&rule.category()) {
            return sev;
        }
        rule.default_severity()
    }

    /// Return references to all rules whose effective severity is not Off.
    pub fn active_rules(&self) -> Vec<(&dyn LintRule, Severity)> {
        self.rules
            .iter()
            .filter_map(|r| {
                let sev = self.effective_severity(r.as_ref());
                if sev == Severity::Off {
                    None
                } else {
                    Some((r.as_ref(), sev))
                }
            })
            .collect()
    }

    /// Ids of rules that would have run but cannot, because they do not
    /// apply to `dialect`.
    ///
    /// This is the count `scythe audit` needs to stop reporting a clean bill
    /// of health it did not earn (#167): 28 of the 35 canonical audit rules
    /// declare `dialects = ["postgres"]`, so an audit of a MySQL project runs
    /// 7 rules, finds nothing, and prints "No findings" — indistinguishable,
    /// to the reader, from 35 rules running and finding nothing.
    ///
    /// Derived from [`LintRule::is_applicable_to`], the same predicate the
    /// rules gate themselves on, so what is counted here and what is skipped
    /// during the run cannot disagree. Rules whose effective severity is
    /// `Off` are excluded: those did not run for a different reason, and
    /// attributing them to the engine would misreport why.
    pub fn rules_inapplicable_to(&self, dialect: SqlDialect) -> Vec<&'static str> {
        self.active_rules()
            .iter()
            .filter(|(rule, _)| !rule.is_applicable_to(dialect))
            .map(|(rule, _)| rule.id())
            .collect()
    }

    /// Return every registered rule together with its effective severity,
    /// **including** rules whose effective severity is `Off`.
    ///
    /// [`active_rules`](Self::active_rules) answers "what will actually run"
    /// and drops `Off` rules — correct for `scythe lint` / `scythe audit`'s
    /// execution path, wrong for discovery commands. `SC-A02`
    /// (`ImplicitTypeCoercion`) and `SC-C01` (`MissingReturnsAnnotation`) are
    /// `Off` by default but are still real, registered rules a user can turn
    /// on; `scythe audit --list-rules` and `--explain` used to build on
    /// `active_rules` and silently omitted both, undercounting the
    /// documented "23 lint rules" figure by two. Use this method wherever the
    /// full catalog — not the active subset — is what's being reported.
    pub fn all_rules(&self) -> Vec<(&dyn LintRule, Severity)> {
        self.rules
            .iter()
            .map(|r| (r.as_ref(), self.effective_severity(r.as_ref())))
            .collect()
    }
}

pub fn default_registry() -> RuleRegistry {
    let mut reg = RuleRegistry::new();

    reg.register(Box::new(rules::safety::UpdateWithoutWhere));
    reg.register(Box::new(rules::safety::DeleteWithoutWhere));
    reg.register(Box::new(rules::safety::NoSelectStar));
    reg.register(Box::new(rules::safety::UnusedParams));
    reg.register(Box::new(rules::safety::MissingReturning));
    reg.register(Box::new(rules::safety::AmbiguousColumnInJoin));
    reg.register(Box::new(rules::safety::UnboundSqlParam));

    reg.register(Box::new(rules::codegen::MissingReturnsAnnotation));
    reg.register(Box::new(rules::codegen::ExecWithReturning));
    reg.register(Box::new(rules::codegen::DuplicateQueryNames));

    // ~keep `SC-PARSE03` lives in `rules::parse` (its id keeps that file's `SC-PARSE`
    // prefix), not `rules::codegen` — see its own doc comment for why it is
    // still registered here in `default_registry` rather than
    // `parse_registry`, unlike `SC-PARSE01`/`SC-PARSE02`.
    reg.register(Box::new(rules::parse::MisspelledAnnotation));

    reg.register(Box::new(rules::naming::PreferSnakeCaseColumns));
    reg.register(Box::new(rules::naming::PreferSnakeCaseTables));
    reg.register(Box::new(rules::naming::QueryNameConvention));
    reg.register(Box::new(rules::naming::ConsistentAliasCasing));

    reg.register(Box::new(rules::antipattern::NotEqualNull));
    reg.register(Box::new(rules::antipattern::ImplicitTypeCoercion));
    reg.register(Box::new(rules::antipattern::OrInJoinCondition));

    reg.register(Box::new(rules::performance::OrderWithoutLimit));
    reg.register(Box::new(rules::performance::LikeStartsWithWildcard));
    reg.register(Box::new(rules::performance::NotInSubquery));

    reg.register(Box::new(rules::style::PreferExplicitJoin));
    reg.register(Box::new(rules::style::PreferCoalesceOverCase));
    reg.register(Box::new(rules::style::PreferCountStar));

    let matcher_reg = audit::MatcherRegistry::canonical();
    for spec in audit::canonical_specs() {
        let matcher_fn = matcher_reg
            .get(&spec.matcher)
            .unwrap_or_else(|| panic!("canonical rule {} references unknown matcher {}", spec.id, spec.matcher));
        // Same standard as the unknown-matcher panic above: the canonical
        // rule TOML is a compiled-in build asset, so a rule that ships with
        // arguments its matcher cannot act on is a developer error, not a
        // runtime condition -- and shipping it silently would mean shipping a
        // rule that is advertised, counted as active, and inert (#165).
        if let Err(e) = audit::validate_matcher_args(&spec.matcher, &spec.matcher_args) {
            panic!("canonical rule {}: {}", spec.id, e);
        }
        reg.register(Box::new(audit::MatcherRule::new(spec, matcher_fn)));
    }

    reg
}

/// The eleven `SC-PRV*` provenance rules, in their own registry.
///
/// Deliberately **not** part of [`default_registry`]. Every consumer of that
/// registry evaluates rules through `LintRule::check_query` /
/// `check_catalog`, and these eleven implement neither — their findings come
/// from `scythe check`'s generated-artifact verification pass and its
/// `[[sql.gen]]`/query-file validation, neither of which has a `LintContext`
/// to offer. Putting them in the default registry would have
/// `scythe audit --list-rules` (via `load_registry_for_discovery`) and
/// `scythe lint` advertise eleven rules that neither command can ever emit,
/// and would move the documented "59 built-in rules" figure (58 before
/// `SC-PARSE03`, #152/#167) that appears across the README, the website, and
/// the skills bundle.
///
/// A registry rather than a bare list because that is what makes them
/// configurable: `scythe check` calls [`RuleRegistry::apply_config`] on this
/// registry with the same `[lint]` table it applies to the default one, then
/// resolves each severity through [`RuleRegistry::effective_severity`]. So
/// `[lint.rules] "SC-PRV01" = "off"` and `[lint.categories] provenance =
/// "off"` both work, and schema drift is not the one finding in scythe with
/// no way to opt out of failing CI. See `rules::provenance`'s module doc.
pub fn provenance_registry() -> RuleRegistry {
    let mut reg = RuleRegistry::new();

    reg.register(Box::new(rules::provenance::SchemaDrift));
    reg.register(Box::new(rules::provenance::ScytheVersionDrift));
    reg.register(Box::new(rules::provenance::BackendDrift));
    reg.register(Box::new(rules::provenance::EngineDrift));
    reg.register(Box::new(rules::provenance::MissingProvenanceHeader));
    reg.register(Box::new(rules::provenance::MalformedProvenanceHeader));
    reg.register(Box::new(rules::provenance::UnverifiableProvenance));
    reg.register(Box::new(rules::provenance::QueryDrift));
    reg.register(Box::new(rules::provenance::GenTargetInvalid));
    reg.register(Box::new(rules::provenance::EmptyQueryFile));
    reg.register(Box::new(rules::provenance::OptionsDrift));

    reg
}

/// The two `SC-PARSE*` rules, in their own registry.
///
/// Deliberately **not** part of [`default_registry`], for the same
/// structural reason as [`provenance_registry`]: a query that fails to parse
/// or fails semantic analysis never becomes the `LintContext` any
/// `check_query` rule needs, so neither rule can run through the paths
/// `scythe lint` / `scythe audit --list-rules` advertise.
///
/// Unlike [`provenance_registry`] and [`drift_registry`], this registry is
/// consulted from three different commands -- `scythe check`, `scythe lint`,
/// and `scythe audit` all hit the same parse/analyze failure and used to
/// each hardcode their own `Severity::Error` at the point the finding was
/// constructed. A registry is what lets one `[lint.rules] "SC-PARSE01" =
/// "warn"` (or `[lint.categories] parse = "off"`) reach all three at once,
/// via [`RuleRegistry::effective_severity`] exactly as any other rule does.
pub fn parse_registry() -> RuleRegistry {
    let mut reg = RuleRegistry::new();

    reg.register(Box::new(rules::parse::UnparseableQuery));
    reg.register(Box::new(rules::parse::UnanalyzableQuery));

    reg
}

/// Registry holding only the schema-drift rules (`SC-DRF01`–`SC-DRF13`).
///
/// Deliberately separate from [`default_registry`]. Drift rules can only fire
/// when `scythe check` is given `--database-url`; folding them into the
/// default registry would list seven rules in `scythe lint` and
/// `scythe audit --list-rules` that those commands can never report, and would
/// silently restate scythe's documented built-in rule count as if drift
/// checking were seven more static lint rules.
///
/// Being a plain [`RuleRegistry`] is what keeps drift severities configurable:
/// callers apply the same `[lint]` `LintConfig` to it that they apply to
/// [`default_registry`], so `rules."SC-DRF02" = "error"` and
/// `categories.drift = "off"` both work exactly as they do for any other rule.
pub fn drift_registry() -> RuleRegistry {
    let mut reg = RuleRegistry::new();

    reg.register(Box::new(rules::drift::TableMissingFromDatabase));
    reg.register(Box::new(rules::drift::TableMissingFromDdl));
    reg.register(Box::new(rules::drift::ColumnMissingFromDatabase));
    reg.register(Box::new(rules::drift::ColumnMissingFromDdl));
    reg.register(Box::new(rules::drift::ColumnTypeMismatch));
    reg.register(Box::new(rules::drift::ColumnNullabilityMismatch));
    reg.register(Box::new(rules::drift::EnumValuesMismatch));
    reg.register(Box::new(rules::drift::CompositeMissingFromDatabase));
    reg.register(Box::new(rules::drift::CompositeMissingFromDdl));
    reg.register(Box::new(rules::drift::CompositeFieldMissingFromDatabase));
    reg.register(Box::new(rules::drift::CompositeFieldMissingFromDdl));
    reg.register(Box::new(rules::drift::CompositeFieldTypeMismatch));
    reg.register(Box::new(rules::drift::CompositeFieldNullabilityMismatch));

    reg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LintConfig;

    /// A trivial rule used only in tests.
    struct TestRule {
        id: &'static str,
        category: RuleCategory,
        severity: Severity,
    }

    impl TestRule {
        fn new(id: &'static str, category: RuleCategory, severity: Severity) -> Self {
            Self { id, category, severity }
        }
    }

    impl LintRule for TestRule {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            "test-rule"
        }
        fn category(&self) -> RuleCategory {
            self.category
        }
        fn default_severity(&self) -> Severity {
            self.severity
        }
        fn description(&self) -> &'static str {
            "a test rule"
        }
    }

    #[test]
    fn new_registry_is_empty() {
        let reg = RuleRegistry::new();
        assert!(reg.rules.is_empty());
        assert!(reg.severity_overrides.is_empty());
        assert!(reg.category_overrides.is_empty());
        assert!(reg.active_rules().is_empty());
    }

    #[test]
    fn register_adds_rule() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn)));
        assert_eq!(reg.rules.len(), 1);
        assert_eq!(reg.active_rules().len(), 1);
    }

    /// 24 SQL lint rules (23 plus `SC-PARSE03`, added for #152/#167) + 35
    /// canonical audit rules. This was the "58 built-in rules" figure quoted
    /// across the README, the website, and the skills bundle; it is now 59
    /// there too (out of this crate's territory to update). It moves only
    /// when a rule a user can actually trigger is added or removed.
    #[test]
    fn default_registry_has_59_rules() {
        let reg = default_registry();
        assert_eq!(reg.rules.len(), 59);
    }

    /// `all_rules` is the source `scythe audit --list-rules` and `--explain`
    /// build their catalog from. It must enumerate every rule `register`
    /// actually added — same count, same id set — with none dropped for
    /// defaulting to `Severity::Off`. This is the regression test for the
    /// bug where `--list-rules` reused `active_rules` (which filters `Off`
    /// out) and silently under-reported SC-A02 and SC-C01, two registered,
    /// off-by-default rules.
    #[test]
    fn all_rules_matches_every_registered_rule_exactly() {
        let reg = default_registry();

        let mut all_ids: Vec<&str> = reg.all_rules().iter().map(|(r, _)| r.id()).collect();
        let mut registered_ids: Vec<&str> = reg.rules.iter().map(|r| r.id()).collect();
        assert_eq!(
            all_ids.len(),
            registered_ids.len(),
            "all_rules must not filter any registered rule out"
        );
        all_ids.sort_unstable();
        registered_ids.sort_unstable();
        assert_eq!(
            all_ids, registered_ids,
            "all_rules and the registry's rule list must agree on the exact id set"
        );
        assert_eq!(
            all_ids.len(),
            59,
            "the default registry now holds 59 built-in rules (58 + SC-PARSE03, #152/#167)"
        );
    }

    /// SC-A02 (`ImplicitTypeCoercion`) and SC-C01 (`MissingReturnsAnnotation`)
    /// default to `Severity::Off` but are still registered, documented rules
    /// — a user can turn either on via `[lint.rules]`. The catalog must list
    /// them (as "off") rather than pretend they don't exist.
    #[test]
    fn all_rules_includes_off_by_default_rules() {
        let reg = default_registry();
        let ids: Vec<&str> = reg.all_rules().iter().map(|(r, _)| r.id()).collect();
        assert!(
            ids.contains(&"SC-A02"),
            "SC-A02 is off by default but must still appear in the catalog"
        );
        assert!(
            ids.contains(&"SC-C01"),
            "SC-C01 is off by default but must still appear in the catalog"
        );
    }

    /// The `SC-PRV*` rules live in [`provenance_registry`], not here. Every
    /// consumer of the default registry evaluates rules through
    /// `check_query` / `check_catalog`, which no provenance rule implements
    /// — so listing them via `scythe audit --list-rules` or running them
    /// through `scythe lint` would advertise rules that can never produce a
    /// finding from those commands.
    #[test]
    fn default_registry_excludes_provenance_rules() {
        let reg = default_registry();

        assert!(
            !reg.rules.iter().any(|r| r.category() == RuleCategory::Provenance),
            "no provenance-category rule may appear in the default registry"
        );
        for id in [
            "SC-PRV01", "SC-PRV02", "SC-PRV03", "SC-PRV04", "SC-PRV05", "SC-PRV06", "SC-PRV07", "SC-PRV08",
        ] {
            assert!(
                !reg.rules.iter().any(|r| r.id() == id),
                "{id} must not be registered in the default registry"
            );
        }
    }

    #[test]
    fn provenance_registry_has_every_registered_prv_rule() {
        let reg = provenance_registry();
        let ids: Vec<&str> = reg.rules.iter().map(|r| r.id()).collect();
        assert_eq!(
            ids,
            vec![
                "SC-PRV01", "SC-PRV02", "SC-PRV03", "SC-PRV04", "SC-PRV05", "SC-PRV06", "SC-PRV07", "SC-PRV08",
                "SC-PRV09", "SC-PRV10", "SC-PRV11"
            ]
        );
    }

    /// The provenance rules must be reachable by id through the same
    /// severity-resolution path every other rule uses — that reachability is
    /// the entire reason they get a registry rather than a bare list, and it
    /// is what lets `[lint.rules]` disable a schema-drift failure in CI.
    ///
    /// Covers `SC-PRV09`/`SC-PRV10` alongside an existing rule specifically:
    /// before these two joined `provenance_registry`, `scythe-cli` hardcoded
    /// `Severity::Error` at the point each finding was constructed, so no
    /// `[lint.rules]` entry for either id could have any effect at all.
    #[test]
    fn provenance_rules_honor_per_rule_config_overrides() {
        let mut reg = provenance_registry();

        let mut config = LintConfig::default();
        config.rules.insert("SC-PRV01".to_string(), Severity::Off);
        config.rules.insert("SC-PRV02".to_string(), Severity::Error);
        config.rules.insert("SC-PRV09".to_string(), Severity::Warn);
        config.rules.insert("SC-PRV10".to_string(), Severity::Off);
        reg.apply_config(&config);

        assert_eq!(reg.effective_severity(&rules::provenance::SchemaDrift), Severity::Off);
        assert_eq!(
            reg.effective_severity(&rules::provenance::ScytheVersionDrift),
            Severity::Error
        );
        assert_eq!(
            reg.effective_severity(&rules::provenance::GenTargetInvalid),
            Severity::Warn
        );
        assert_eq!(
            reg.effective_severity(&rules::provenance::EmptyQueryFile),
            Severity::Off
        );
        // Untouched by the config: still its own default.
        assert_eq!(
            reg.effective_severity(&rules::provenance::BackendDrift),
            Severity::Error
        );
    }

    /// A single `[lint.categories] provenance = "off"` switch must turn the
    /// whole provenance pass off — the coarse opt-out for projects that do
    /// not commit generated artifacts at all.
    #[test]
    fn provenance_category_override_disables_every_provenance_rule() {
        let mut reg = provenance_registry();

        let mut config = LintConfig::default();
        config.categories.insert(RuleCategory::Provenance, Severity::Off);
        reg.apply_config(&config);

        for id in [
            "SC-PRV01", "SC-PRV02", "SC-PRV03", "SC-PRV04", "SC-PRV05", "SC-PRV06", "SC-PRV07", "SC-PRV08", "SC-PRV09",
            "SC-PRV10", "SC-PRV11",
        ] {
            assert!(
                !reg.active_rules().iter().any(|(r, _)| r.id() == id),
                "{id} must be inactive when the provenance category is off"
            );
        }
    }

    /// Drift rules must stay out of the default registry: `scythe lint` and
    /// `scythe audit` cannot observe a live database, so listing them there
    /// would advertise rules those commands can never report.
    #[test]
    fn default_registry_excludes_drift_rules() {
        let reg = default_registry();
        assert!(
            !reg.rules.iter().any(|r| r.id().starts_with("SC-DRF")),
            "drift rules belong to drift_registry(), not default_registry()"
        );
    }

    /// The drift registry is a plain `RuleRegistry`, so `[lint]` severity
    /// overrides reach drift rules through exactly the path every other
    /// `SC-*` rule uses.
    #[test]
    fn drift_registry_honours_lint_config_overrides() {
        let mut reg = drift_registry();

        let mut config = LintConfig::default();
        config.rules.insert("SC-DRF02".to_string(), Severity::Error);
        config.rules.insert("SC-DRF07".to_string(), Severity::Off);
        reg.apply_config(&config);

        let active = reg.active_rules();
        let severity_of = |id: &str| active.iter().find(|(r, _)| r.id() == id).map(|(_, s)| *s);

        assert_eq!(severity_of("SC-DRF02"), Some(Severity::Error));
        assert_eq!(
            severity_of("SC-DRF07"),
            None,
            "an Off rule must drop out of active_rules"
        );
        assert_eq!(
            severity_of("SC-DRF06"),
            Some(Severity::Error),
            "untouched rules keep their default"
        );
    }

    /// One `categories.drift` switch must silence the whole drift check —
    /// the escape hatch for a user whose database legitimately differs.
    #[test]
    fn drift_category_override_disables_every_drift_rule() {
        let mut reg = drift_registry();

        let mut config = LintConfig::default();
        config.categories.insert(RuleCategory::Drift, Severity::Off);
        reg.apply_config(&config);

        assert!(reg.active_rules().is_empty());
    }

    /// `SC-PARSE01`/`SC-PARSE02` — the two structural rules with no working
    /// `check_query`/`check_catalog` — must stay out of the default
    /// registry, for the same reason the provenance and drift families do:
    /// no consumer of `default_registry` reaches a no-op `check_query`
    /// through anything. `SC-PARSE03` is deliberately excluded from this
    /// assertion: unlike the other two, it has a real `check_query` (see
    /// `rules::parse::MisspelledAnnotation`'s doc) and belongs in
    /// `default_registry` for exactly that reason — asserted by
    /// `default_registry_includes_sc_parse03` below.
    #[test]
    fn default_registry_excludes_parse_rules() {
        let reg = default_registry();
        for id in ["SC-PARSE01", "SC-PARSE02"] {
            assert!(
                !reg.rules.iter().any(|r| r.id() == id),
                "{id} belongs to parse_registry(), not default_registry()"
            );
        }
    }

    /// `SC-PARSE03` is the one `SC-PARSE`-prefixed rule with a real
    /// `check_query`, so — unlike `SC-PARSE01`/`SC-PARSE02` — it must be
    /// reachable the same way every other `default_registry` rule is: via
    /// `active_rules`, `[lint.rules]` overrides, and `scythe audit
    /// --list-rules`.
    #[test]
    fn default_registry_includes_sc_parse03() {
        let reg = default_registry();
        assert!(
            reg.rules.iter().any(|r| r.id() == "SC-PARSE03"),
            "SC-PARSE03 has a real check_query and must run through default_registry"
        );
        assert!(
            reg.active_rules()
                .iter()
                .any(|(r, sev)| r.id() == "SC-PARSE03" && *sev == Severity::Warn),
            "SC-PARSE03 must be active (Warn) by default"
        );
    }

    #[test]
    fn parse_registry_has_every_registered_rule() {
        let reg = parse_registry();
        let ids: Vec<&str> = reg.rules.iter().map(|r| r.id()).collect();
        assert_eq!(
            ids,
            vec!["SC-PARSE01", "SC-PARSE02"],
            "SC-PARSE03 must not be here — it runs through default_registry's check_query dispatch instead"
        );
    }

    /// The whole reason `SC-PARSE01`/`SC-PARSE02` moved into a registry:
    /// before this, `scythe check`, `scythe lint`, and `scythe audit` each
    /// hardcoded `Severity::Error` at the point their own ad hoc finding was
    /// constructed, so no `[lint.rules]` entry for either id could have any
    /// effect in any of the three commands.
    #[test]
    fn parse_rules_honor_per_rule_config_overrides() {
        let mut reg = parse_registry();

        let mut config = LintConfig::default();
        config.rules.insert("SC-PARSE01".to_string(), Severity::Warn);
        config.rules.insert("SC-PARSE02".to_string(), Severity::Off);
        reg.apply_config(&config);

        assert_eq!(reg.effective_severity(&rules::parse::UnparseableQuery), Severity::Warn);
        assert_eq!(reg.effective_severity(&rules::parse::UnanalyzableQuery), Severity::Off);
    }

    /// A single `[lint.categories] parse = "off"` switch must turn off both
    /// rules at once.
    #[test]
    fn parse_category_override_disables_every_parse_rule() {
        let mut reg = parse_registry();

        let mut config = LintConfig::default();
        config.categories.insert(RuleCategory::Parse, Severity::Off);
        reg.apply_config(&config);

        assert!(reg.active_rules().is_empty());
    }

    #[test]
    fn apply_config_rule_level_override() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn)));

        let mut config = LintConfig::default();
        config.rules.insert("TR-01".to_string(), Severity::Error);
        reg.apply_config(&config);

        let active = reg.active_rules();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].1, Severity::Error);
    }

    #[test]
    fn apply_config_category_level_override() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn)));

        let mut config = LintConfig::default();
        config.categories.insert(RuleCategory::Safety, Severity::Error);
        reg.apply_config(&config);

        let active = reg.active_rules();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].1, Severity::Error);
    }

    #[test]
    fn effective_severity_rule_override_beats_category_override() {
        let mut reg = RuleRegistry::new();
        let rule = TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn);
        reg.register(Box::new(TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn)));

        let mut config = LintConfig::default();
        config.categories.insert(RuleCategory::Safety, Severity::Off);
        config.rules.insert("TR-01".to_string(), Severity::Error);
        reg.apply_config(&config);

        assert_eq!(reg.effective_severity(&rule), Severity::Error);
    }

    #[test]
    fn effective_severity_category_override_beats_default() {
        let mut reg = RuleRegistry::new();
        let rule = TestRule::new("TR-01", RuleCategory::Naming, Severity::Warn);

        let mut config = LintConfig::default();
        config.categories.insert(RuleCategory::Naming, Severity::Error);
        reg.apply_config(&config);

        assert_eq!(reg.effective_severity(&rule), Severity::Error);
    }

    #[test]
    fn effective_severity_falls_back_to_default() {
        let reg = RuleRegistry::new();
        let rule = TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn);
        assert_eq!(reg.effective_severity(&rule), Severity::Warn);
    }

    #[test]
    fn active_rules_filters_out_off() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn)));
        reg.register(Box::new(TestRule::new("TR-02", RuleCategory::Safety, Severity::Off)));
        reg.register(Box::new(TestRule::new("TR-03", RuleCategory::Style, Severity::Error)));

        let active = reg.active_rules();
        assert_eq!(active.len(), 2);
        let ids: Vec<&str> = active.iter().map(|(r, _)| r.id()).collect();
        assert!(ids.contains(&"TR-01"));
        assert!(ids.contains(&"TR-03"));
        assert!(!ids.contains(&"TR-02"));
    }

    #[test]
    fn active_rules_returns_overridden_severity() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn)));

        let mut config = LintConfig::default();
        config.rules.insert("TR-01".to_string(), Severity::Error);
        reg.apply_config(&config);

        let active = reg.active_rules();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].0.id(), "TR-01");
        assert_eq!(active[0].1, Severity::Error);
    }

    /// #167: `scythe audit` on a non-PostgreSQL engine runs a small fraction
    /// of its rule set and reports "No findings" all the same. The count of
    /// rules that could not run has to be answerable from the registry, and
    /// it has to be derived from the same predicate the rules gate
    /// themselves on.
    #[test]
    fn rules_inapplicable_to_lists_dialect_gated_rules_by_id() {
        struct PostgresOnlyRule;

        impl LintRule for PostgresOnlyRule {
            fn id(&self) -> &'static str {
                "TR-PG"
            }
            fn name(&self) -> &'static str {
                "postgres-only"
            }
            fn category(&self) -> RuleCategory {
                RuleCategory::Security
            }
            fn default_severity(&self) -> Severity {
                Severity::Error
            }
            fn description(&self) -> &'static str {
                "only meaningful on PostgreSQL"
            }
            fn is_applicable_to(&self, dialect: SqlDialect) -> bool {
                dialect == SqlDialect::PostgreSQL
            }
        }

        let mut reg = RuleRegistry::new();
        reg.register(Box::new(PostgresOnlyRule));
        reg.register(Box::new(TestRule::new("TR-ANY", RuleCategory::Safety, Severity::Warn)));

        assert_eq!(reg.rules_inapplicable_to(SqlDialect::MySQL), vec!["TR-PG"]);
        assert!(reg.rules_inapplicable_to(SqlDialect::PostgreSQL).is_empty());
    }

    /// #167: "skipped because the engine does not support it" and "switched
    /// off in config" are different reasons a rule produced nothing.
    /// Counting an `Off` rule as engine-skipped would misreport why the run
    /// was quiet.
    #[test]
    fn rules_inapplicable_to_excludes_rules_that_are_switched_off() {
        struct PostgresOnlyRule;

        impl LintRule for PostgresOnlyRule {
            fn id(&self) -> &'static str {
                "TR-PG"
            }
            fn name(&self) -> &'static str {
                "postgres-only"
            }
            fn category(&self) -> RuleCategory {
                RuleCategory::Security
            }
            fn default_severity(&self) -> Severity {
                Severity::Error
            }
            fn description(&self) -> &'static str {
                "only meaningful on PostgreSQL"
            }
            fn is_applicable_to(&self, dialect: SqlDialect) -> bool {
                dialect == SqlDialect::PostgreSQL
            }
        }

        let mut reg = RuleRegistry::new();
        reg.register(Box::new(PostgresOnlyRule));

        let mut config = LintConfig::default();
        config.rules.insert("TR-PG".to_string(), Severity::Off);
        reg.apply_config(&config);

        assert!(
            reg.rules_inapplicable_to(SqlDialect::MySQL).is_empty(),
            "a rule that is off did not run because it is off, not because of the engine"
        );
    }

    /// #167, pinned: the canonical audit rule set is overwhelmingly
    /// PostgreSQL-gated, and the exact split is what any "N rule(s) skipped"
    /// summary reports. Both numbers move together or this fails — a rule
    /// gaining or losing a `dialects` restriction must be a deliberate,
    /// visible change.
    #[test]
    fn canonical_rules_gated_to_postgres_is_28_of_59() {
        let reg = default_registry();

        let skipped_on_mysql = reg.rules_inapplicable_to(SqlDialect::MySQL);
        assert_eq!(
            skipped_on_mysql.len(),
            28,
            "28 canonical audit rules declare dialects = [\"postgres\"]"
        );
        assert!(
            reg.rules_inapplicable_to(SqlDialect::PostgreSQL).is_empty(),
            "no rule is gated away from PostgreSQL"
        );

        // ~keep 59 registered, of which 57 are on by default (SC-A02 and SC-C01
        // default to `Off`; SC-PARSE03 defaults to `Warn`, so it counts as
        // on), of which 28 are PostgreSQL-gated.
        assert_eq!(reg.active_rules().len(), 57);
        let active_on_mysql = reg.active_rules().len() - skipped_on_mysql.len();
        assert_eq!(
            active_on_mysql, 29,
            "a MySQL audit runs 29 of the 57 on-by-default rules; that gap is what the summary must disclose"
        );
    }

    #[test]
    fn active_rules_config_can_turn_off_rule() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn)));

        let mut config = LintConfig::default();
        config.rules.insert("TR-01".to_string(), Severity::Off);
        reg.apply_config(&config);

        assert!(reg.active_rules().is_empty());
    }
}
