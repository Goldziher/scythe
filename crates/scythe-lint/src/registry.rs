use ahash::AHashMap;

use super::audit;
use super::rule::LintRule;
use super::rules;
use super::types::{LintConfig, RuleCategory, Severity};

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
        reg.register(Box::new(audit::MatcherRule::new(spec, matcher_fn)));
    }

    reg
}

/// The seven `SC-PRV*` provenance rules, in their own registry.
///
/// Deliberately **not** part of [`default_registry`]. Every consumer of that
/// registry evaluates rules through `LintRule::check_query` /
/// `check_catalog`, and these seven implement neither — their findings come
/// from `scythe check`'s generated-artifact verification pass, which has no
/// `LintContext` to offer. Putting them in the default registry would have
/// `scythe audit --list-rules` (via `load_registry_for_discovery`) and
/// `scythe lint` advertise seven rules that neither command can ever emit,
/// and would move the documented "58 built-in rules" figure that appears
/// across the README, the website, and the skills bundle.
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

    /// 23 SQL lint rules + 35 canonical audit rules. This is the "58
    /// built-in rules" figure quoted across the README, the website, and the
    /// skills bundle, so it moves only when a rule a user can actually
    /// trigger is added or removed.
    #[test]
    fn default_registry_has_58_rules() {
        let reg = default_registry();
        assert_eq!(reg.rules.len(), 58);
    }

    /// The `SC-PRV*` rules live in [`provenance_registry`], not here. Every
    /// consumer of the default registry evaluates rules through
    /// `check_query` / `check_catalog`, which no provenance rule implements
    /// — so listing them via `scythe audit --list-rules` or running them
    /// through `scythe lint` would advertise seven rules that can never
    /// produce a finding from those commands.
    #[test]
    fn default_registry_excludes_provenance_rules() {
        let reg = default_registry();

        assert!(
            !reg.rules.iter().any(|r| r.category() == RuleCategory::Provenance),
            "no provenance-category rule may appear in the default registry"
        );
        for id in [
            "SC-PRV01", "SC-PRV02", "SC-PRV03", "SC-PRV04", "SC-PRV05", "SC-PRV06", "SC-PRV07",
        ] {
            assert!(
                !reg.rules.iter().any(|r| r.id() == id),
                "{id} must not be registered in the default registry"
            );
        }
    }

    #[test]
    fn provenance_registry_has_the_seven_prv_rules() {
        let reg = provenance_registry();
        let ids: Vec<&str> = reg.rules.iter().map(|r| r.id()).collect();
        assert_eq!(
            ids,
            vec![
                "SC-PRV01", "SC-PRV02", "SC-PRV03", "SC-PRV04", "SC-PRV05", "SC-PRV06", "SC-PRV07"
            ]
        );
    }

    /// The provenance rules must be reachable by id through the same
    /// severity-resolution path every other rule uses — that reachability is
    /// the entire reason they get a registry rather than a bare list, and it
    /// is what lets `[lint.rules]` disable a schema-drift failure in CI.
    #[test]
    fn provenance_rules_honor_per_rule_config_overrides() {
        let mut reg = provenance_registry();

        let mut config = LintConfig::default();
        config.rules.insert("SC-PRV01".to_string(), Severity::Off);
        config.rules.insert("SC-PRV02".to_string(), Severity::Error);
        reg.apply_config(&config);

        assert_eq!(reg.effective_severity(&rules::provenance::SchemaDrift), Severity::Off);
        assert_eq!(
            reg.effective_severity(&rules::provenance::ScytheVersionDrift),
            Severity::Error
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
            "SC-PRV01", "SC-PRV02", "SC-PRV03", "SC-PRV04", "SC-PRV05", "SC-PRV06", "SC-PRV07",
        ] {
            assert!(
                !reg.active_rules().iter().any(|(r, _)| r.id() == id),
                "{id} must be inactive when the provenance category is off"
            );
        }
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
