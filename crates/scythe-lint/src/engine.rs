use std::borrow::Cow;

use ahash::AHashMap;

use super::registry::RuleRegistry;
use super::rule::LintRule;
use super::rules::codegen::DuplicateQueryNames;
use super::types::{LintContext, Severity, Violation};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;

// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct QueryViolation {
    pub query_name: String,
    pub rule_id: Cow<'static, str>,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug)]
pub struct LintReport {
    pub violations: Vec<QueryViolation>,
    pub queries_checked: usize,
    pub rules_active: usize,
}

impl LintReport {
    pub fn has_errors(&self) -> bool {
        self.violations.iter().any(|v| matches!(v.severity, Severity::Error))
    }

    pub fn has_warnings(&self) -> bool {
        self.violations.iter().any(|v| matches!(v.severity, Severity::Warn))
    }
}

pub struct LintEngine {
    registry: RuleRegistry,
}

impl LintEngine {
    pub fn new(registry: RuleRegistry) -> Self {
        Self { registry }
    }

    /// Lint a single query, returning violations.
    pub fn check_query(&self, ctx: &LintContext<'_>) -> Vec<(Violation, Severity)> {
        let mut results = Vec::new();
        for (rule, sev) in self.registry.active_rules() {
            for v in rule.check_query(ctx) {
                results.push((v, sev));
            }
        }
        results
    }

    /// Lint the catalog (table naming, etc.), returning violations.
    pub fn check_catalog(&self, catalog: &Catalog) -> Vec<(Violation, Severity)> {
        let mut results = Vec::new();
        for (rule, sev) in self.registry.active_rules() {
            for v in rule.check_catalog(catalog) {
                results.push((v, sev));
            }
        }
        results
    }

    /// Rules registered here that cannot run against `dialect`, by id.
    ///
    /// A dialect-gated rule contributes nothing to a run and says nothing
    /// about why. `scythe audit` on a MySQL project reports "No findings"
    /// while 28 of its 35 canonical rules are gated to `postgres` and never
    /// executed (#167) — a caller that wants to say "N rule(s) skipped: not
    /// applicable to engine '<e>'" needs the gate to be answerable *before*
    /// the run, which is what this is for.
    ///
    /// Only rules that would otherwise have run are listed: a rule switched
    /// off through `[lint.rules]` is not "skipped for the engine", it is off,
    /// and reporting it here would double-count the two reasons a rule
    /// produced nothing.
    pub fn rules_inapplicable_to(&self, dialect: SqlDialect) -> Vec<&'static str> {
        self.registry.rules_inapplicable_to(dialect)
    }

    /// The cross-query duplicate-`@name` check (`SC-C03`).
    ///
    /// Separate from [`check_query`](Self::check_query) because it is not a
    /// per-query question: `DuplicateQueryNames` implements no `check_query`
    /// and never can, since a `LintContext` describes one query and cannot
    /// see the others. That left `SC-C03` advertised at `error` severity by
    /// `scythe audit --list-rules` while being unable to produce a finding
    /// from any command (#137).
    ///
    /// This is the one implementation of the check; [`build_report`] calls
    /// it, and a caller that drives `check_query` itself must call it too, or
    /// duplicate `@name`s go unreported and `scythe generate` emits two
    /// functions with the same name.
    ///
    /// Returns nothing when `DuplicateQueryNames` is not registered or is
    /// configured `off` — its severity comes from the registry, so
    /// `[lint.rules] "SC-C03" = "warn"` is honoured rather than overridden by
    /// a hardcoded `Severity::Error`. Each duplicated name is reported once,
    /// however many times it occurs, with the occurrence count in the
    /// message.
    pub fn check_duplicate_query_names<'n>(&self, names: impl IntoIterator<Item = &'n str>) -> Vec<QueryViolation> {
        // The id comes from the rule itself rather than a `"SC-C03"` literal:
        // the rule struct and the finding it produces must not be two
        // derivations of one id that can drift apart.
        let wanted_id = DuplicateQueryNames.id();
        let Some(severity) = self
            .registry
            .active_rules()
            .iter()
            .find(|(rule, _)| rule.id() == wanted_id)
            .map(|(_, sev)| *sev)
        else {
            return Vec::new();
        };

        let mut counts: AHashMap<&str, usize> = AHashMap::new();
        let mut first_seen: Vec<&str> = Vec::new();
        for name in names {
            let count = counts.entry(name).or_insert(0);
            *count += 1;
            if *count == 1 {
                first_seen.push(name);
            }
        }

        first_seen
            .into_iter()
            .filter_map(|name| {
                let count = counts[name];
                (count > 1).then(|| QueryViolation {
                    query_name: name.to_string(),
                    rule_id: Cow::Borrowed(wanted_id),
                    severity,
                    message: format!("duplicate query name: \"{}\" ({} occurrences)", name, count),
                })
            })
            .collect()
    }

    /// Run all checks over a set of queries and produce a report.
    ///
    /// `queries` is an iterator of `LintContext` for each query.
    /// The engine also performs cross-query checks (e.g. duplicate names).
    pub fn build_report<'a>(&self, queries: impl Iterator<Item = LintContext<'a>>, catalog: &Catalog) -> LintReport {
        let active = self.registry.active_rules();
        let rules_active = active.len();
        let mut violations = Vec::new();
        let mut queries_checked: usize = 0;
        let mut names: Vec<String> = Vec::new();

        for ctx in queries {
            queries_checked += 1;
            names.push(ctx.analyzed.name.clone());

            for (rule, sev) in &active {
                for v in rule.check_query(&ctx) {
                    violations.push(QueryViolation {
                        query_name: ctx.analyzed.name.clone(),
                        rule_id: v.rule_id,
                        severity: *sev,
                        message: v.message,
                    });
                }
            }
        }

        violations.extend(self.check_duplicate_query_names(names.iter().map(String::as_str)));

        for (rule, sev) in &active {
            for v in rule.check_catalog(catalog) {
                violations.push(QueryViolation {
                    query_name: String::new(),
                    rule_id: v.rule_id,
                    severity: *sev,
                    message: v.message,
                });
            }
        }

        LintReport {
            violations,
            queries_checked,
            rules_active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::RuleRegistry;
    use crate::rule::LintRule;
    use crate::types::{LintConfig, LintContext, RuleCategory, Violation};
    use scythe_core::analyzer::AnalyzedQuery;
    use scythe_core::catalog::Catalog;
    use scythe_core::parser::{Annotations, QueryCommand};
    use sqlparser::ast::Statement;
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;
    use std::borrow::Cow;

    /// A test rule that always emits one query-level violation.
    struct AlwaysWarnRule;

    impl LintRule for AlwaysWarnRule {
        fn id(&self) -> &'static str {
            "TEST-01"
        }
        fn name(&self) -> &'static str {
            "always-warn"
        }
        fn category(&self) -> RuleCategory {
            RuleCategory::Safety
        }
        fn default_severity(&self) -> Severity {
            Severity::Warn
        }
        fn description(&self) -> &'static str {
            "always fires"
        }
        fn check_query(&self, _ctx: &LintContext<'_>) -> Vec<Violation> {
            vec![Violation {
                rule_id: Cow::Borrowed("TEST-01"),
                message: "always fires".to_string(),
                fix: None,
            }]
        }
    }

    /// A test rule that always emits one catalog-level violation.
    struct CatalogRule;

    impl LintRule for CatalogRule {
        fn id(&self) -> &'static str {
            "TEST-CAT"
        }
        fn name(&self) -> &'static str {
            "catalog-rule"
        }
        fn category(&self) -> RuleCategory {
            RuleCategory::Naming
        }
        fn default_severity(&self) -> Severity {
            Severity::Error
        }
        fn description(&self) -> &'static str {
            "catalog level check"
        }
        fn check_catalog(&self, _catalog: &Catalog) -> Vec<Violation> {
            vec![Violation {
                rule_id: Cow::Borrowed("TEST-CAT"),
                message: "catalog issue".to_string(),
                fix: None,
            }]
        }
    }

    /// A silent rule that never fires.
    struct SilentRule;

    impl LintRule for SilentRule {
        fn id(&self) -> &'static str {
            "TEST-SILENT"
        }
        fn name(&self) -> &'static str {
            "silent-rule"
        }
        fn category(&self) -> RuleCategory {
            RuleCategory::Style
        }
        fn default_severity(&self) -> Severity {
            Severity::Warn
        }
        fn description(&self) -> &'static str {
            "never fires"
        }
    }

    fn parse_stmt(sql: &str) -> Statement {
        let dialect = PostgreSqlDialect {};
        Parser::parse_sql(&dialect, sql).unwrap().remove(0)
    }

    fn empty_catalog() -> Catalog {
        Catalog::from_ddl(&[]).unwrap()
    }

    fn dummy_analyzed(name: &str) -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = name.to_string();
            aq.command = QueryCommand::Many;
            aq.sql = "SELECT 1".to_string();
            aq.columns = vec![];
            aq.params = vec![];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = None;
            aq.custom = vec![];
        })
    }

    fn dummy_annotations(name: &str) -> Annotations {
        Annotations {
            name: name.to_string(),
            command: QueryCommand::Many,
            param_docs: vec![],
            nullable_overrides: vec![],
            nonnull_overrides: vec![],
            json_mappings: vec![],
            deprecated: None,
            optional_params: vec![],
            group_by: None,
            positional_param_docs: vec![],
            custom: vec![],
        }
    }

    fn make_ctx<'a>(
        sql: &'a str,
        stmt: &'a Statement,
        analyzed: &'a AnalyzedQuery,
        catalog: &'a Catalog,
        annotations: &'a Annotations,
    ) -> LintContext<'a> {
        LintContext {
            sql,
            stmt,
            analyzed,
            catalog,
            annotations,
            dialect: scythe_core::dialect::SqlDialect::PostgreSQL,
        }
    }

    #[test]
    fn lint_engine_new_creates_engine() {
        let reg = RuleRegistry::new();
        let engine = LintEngine::new(reg);
        let catalog = empty_catalog();
        let report = engine.build_report(std::iter::empty(), &catalog);
        assert_eq!(report.rules_active, 0);
        assert_eq!(report.queries_checked, 0);
        assert!(report.violations.is_empty());
    }

    #[test]
    fn check_query_returns_violations_from_active_rules() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(AlwaysWarnRule));
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();
        let analyzed = dummy_analyzed("test_query");
        let annotations = dummy_annotations("test_query");
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);

        let results = engine.check_query(&ctx);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.rule_id, "TEST-01");
        assert_eq!(results[0].1, Severity::Warn);
    }

    #[test]
    fn check_query_respects_severity_overrides_off() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(AlwaysWarnRule));
        let mut config = LintConfig::default();
        config.rules.insert("TEST-01".to_string(), Severity::Off);
        reg.apply_config(&config);
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();
        let analyzed = dummy_analyzed("test_query");
        let annotations = dummy_annotations("test_query");
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);

        let results = engine.check_query(&ctx);
        assert!(results.is_empty(), "Off rule should not fire");
    }

    #[test]
    fn check_catalog_returns_catalog_level_violations() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(CatalogRule));
        let engine = LintEngine::new(reg);

        let catalog = empty_catalog();
        let results = engine.check_catalog(&catalog);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.rule_id, "TEST-CAT");
        assert_eq!(results[0].1, Severity::Error);
    }

    #[test]
    fn build_report_counts_errors_vs_warnings() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(AlwaysWarnRule));
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();
        let analyzed = dummy_analyzed("q1");
        let annotations = dummy_annotations("q1");
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);

        let report = engine.build_report(std::iter::once(ctx), &catalog);
        assert_eq!(report.queries_checked, 1);
        assert_eq!(report.rules_active, 1);
        assert!(report.has_warnings());
        assert!(!report.has_errors());
    }

    #[test]
    fn build_report_with_mixed_severities() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(AlwaysWarnRule));
        reg.register(Box::new(CatalogRule));
        reg.register(Box::new(SilentRule));
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();
        let analyzed = dummy_analyzed("q1");
        let annotations = dummy_annotations("q1");
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);

        let report = engine.build_report(std::iter::once(ctx), &catalog);
        assert_eq!(report.queries_checked, 1);
        assert_eq!(report.rules_active, 3);
        assert!(report.has_warnings());
        assert!(report.has_errors());
        assert_eq!(report.violations.len(), 2);
    }

    #[test]
    fn build_report_duplicate_query_names() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(SilentRule));
        reg.register(Box::new(crate::rules::codegen::DuplicateQueryNames));
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();

        let analyzed1 = dummy_analyzed("dup_name");
        let annotations1 = dummy_annotations("dup_name");
        let analyzed2 = dummy_analyzed("dup_name");
        let annotations2 = dummy_annotations("dup_name");
        let analyzed3 = dummy_analyzed("unique_name");
        let annotations3 = dummy_annotations("unique_name");

        let queries = vec![
            make_ctx(sql, &stmt, &analyzed1, &catalog, &annotations1),
            make_ctx(sql, &stmt, &analyzed2, &catalog, &annotations2),
            make_ctx(sql, &stmt, &analyzed3, &catalog, &annotations3),
        ];

        let report = engine.build_report(queries.into_iter(), &catalog);
        assert_eq!(report.queries_checked, 3);

        let dup_violations: Vec<_> = report.violations.iter().filter(|v| v.rule_id == "SC-C03").collect();
        assert_eq!(dup_violations.len(), 1);
        assert_eq!(dup_violations[0].query_name, "dup_name");
        assert_eq!(dup_violations[0].severity, Severity::Error);
        assert!(dup_violations[0].message.contains("duplicate query name"));
    }

    /// Regression for #137: `engine.rs` used to hardcode `Severity::Error`
    /// for SC-C03 regardless of `[lint.rules]`, so
    /// `"SC-C03" = "warn"` had no effect. The emitted violation's severity
    /// must now track the rule's effective (configured) severity.
    #[test]
    fn build_report_duplicate_query_names_honours_severity_override() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(crate::rules::codegen::DuplicateQueryNames));
        let mut config = LintConfig::default();
        config.rules.insert("SC-C03".to_string(), Severity::Warn);
        reg.apply_config(&config);
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();

        let analyzed1 = dummy_analyzed("dup_name");
        let annotations1 = dummy_annotations("dup_name");
        let analyzed2 = dummy_analyzed("dup_name");
        let annotations2 = dummy_annotations("dup_name");

        let queries = vec![
            make_ctx(sql, &stmt, &analyzed1, &catalog, &annotations1),
            make_ctx(sql, &stmt, &analyzed2, &catalog, &annotations2),
        ];

        let report = engine.build_report(queries.into_iter(), &catalog);
        let dup_violations: Vec<_> = report.violations.iter().filter(|v| v.rule_id == "SC-C03").collect();
        assert_eq!(dup_violations.len(), 1);
        assert_eq!(
            dup_violations[0].severity,
            Severity::Warn,
            "SC-C03 severity override must be honoured, not hardcoded to Error"
        );
    }

    /// Regression for #137: when `DuplicateQueryNames` is not registered at
    /// all (nothing has configured a severity for SC-C03), build_report must
    /// not synthesize an SC-C03 violation out of thin air.
    #[test]
    fn build_report_no_duplicate_violation_when_rule_not_registered() {
        let reg = RuleRegistry::new();
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();

        let analyzed1 = dummy_analyzed("dup_name");
        let annotations1 = dummy_annotations("dup_name");
        let analyzed2 = dummy_analyzed("dup_name");
        let annotations2 = dummy_annotations("dup_name");

        let queries = vec![
            make_ctx(sql, &stmt, &analyzed1, &catalog, &annotations1),
            make_ctx(sql, &stmt, &analyzed2, &catalog, &annotations2),
        ];

        let report = engine.build_report(queries.into_iter(), &catalog);
        let dup_violations: Vec<_> = report.violations.iter().filter(|v| v.rule_id == "SC-C03").collect();
        assert!(dup_violations.is_empty());
    }

    /// Regression for #137: `[lint.rules] "SC-C03" = "off"` must silence the
    /// duplicate-name check entirely, same as any other rule.
    #[test]
    fn build_report_duplicate_query_names_off_produces_no_violation() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(crate::rules::codegen::DuplicateQueryNames));
        let mut config = LintConfig::default();
        config.rules.insert("SC-C03".to_string(), Severity::Off);
        reg.apply_config(&config);
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();

        let analyzed1 = dummy_analyzed("dup_name");
        let annotations1 = dummy_annotations("dup_name");
        let analyzed2 = dummy_analyzed("dup_name");
        let annotations2 = dummy_annotations("dup_name");

        let queries = vec![
            make_ctx(sql, &stmt, &analyzed1, &catalog, &annotations1),
            make_ctx(sql, &stmt, &analyzed2, &catalog, &annotations2),
        ];

        let report = engine.build_report(queries.into_iter(), &catalog);
        let dup_violations: Vec<_> = report.violations.iter().filter(|v| v.rule_id == "SC-C03").collect();
        assert!(dup_violations.is_empty());
    }

    #[test]
    fn build_report_no_duplicates_when_names_unique() {
        let reg = RuleRegistry::new();
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();

        let analyzed1 = dummy_analyzed("alpha");
        let annotations1 = dummy_annotations("alpha");
        let analyzed2 = dummy_analyzed("beta");
        let annotations2 = dummy_annotations("beta");

        let queries = vec![
            make_ctx(sql, &stmt, &analyzed1, &catalog, &annotations1),
            make_ctx(sql, &stmt, &analyzed2, &catalog, &annotations2),
        ];

        let report = engine.build_report(queries.into_iter(), &catalog);
        assert_eq!(report.queries_checked, 2);
        let dup_violations: Vec<_> = report.violations.iter().filter(|v| v.rule_id == "SC-C03").collect();
        assert!(dup_violations.is_empty());
    }

    #[test]
    fn lint_report_has_errors_and_has_warnings() {
        let report = LintReport {
            violations: vec![
                QueryViolation {
                    query_name: "q1".to_string(),
                    rule_id: Cow::Borrowed("R1"),
                    severity: Severity::Warn,
                    message: "warning".to_string(),
                },
                QueryViolation {
                    query_name: "q2".to_string(),
                    rule_id: Cow::Borrowed("R2"),
                    severity: Severity::Error,
                    message: "error".to_string(),
                },
            ],
            queries_checked: 2,
            rules_active: 1,
        };
        assert!(report.has_errors());
        assert!(report.has_warnings());
    }

    #[test]
    fn lint_report_empty_has_no_errors_or_warnings() {
        let report = LintReport {
            violations: vec![],
            queries_checked: 0,
            rules_active: 0,
        };
        assert!(!report.has_errors());
        assert!(!report.has_warnings());
    }

    /// #137: the duplicate-name check must be reachable *without* going
    /// through `build_report`. `scythe lint` drives `check_query` /
    /// `check_catalog` directly and never calls `build_report`, which is
    /// exactly why SC-C03 could not fire from any command. A caller in that
    /// shape needs one call it can make with the names it already has.
    #[test]
    fn check_duplicate_query_names_reports_duplicates_without_build_report() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(crate::rules::codegen::DuplicateQueryNames));
        let engine = LintEngine::new(reg);

        let violations = engine.check_duplicate_query_names(["GetUser", "ListUsers", "GetUser"]);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "SC-C03");
        assert_eq!(violations[0].query_name, "GetUser");
        assert_eq!(violations[0].severity, Severity::Error);
        assert_eq!(
            violations[0].message,
            "duplicate query name: \"GetUser\" (2 occurrences)"
        );
    }

    /// #137: a name repeated three times is one problem, not two. The old
    /// code pushed a violation per *extra* occurrence, so three `GetUser`
    /// queries produced two identical findings.
    #[test]
    fn check_duplicate_query_names_reports_each_name_once_with_its_count() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(crate::rules::codegen::DuplicateQueryNames));
        let engine = LintEngine::new(reg);

        let violations = engine.check_duplicate_query_names(["GetUser", "GetUser", "GetUser", "Other"]);

        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].message,
            "duplicate query name: \"GetUser\" (3 occurrences)"
        );
    }

    /// #137: unique names must produce nothing at all — the check must not
    /// be able to fire vacuously on a healthy project.
    #[test]
    fn check_duplicate_query_names_is_silent_when_every_name_is_unique() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(crate::rules::codegen::DuplicateQueryNames));
        let engine = LintEngine::new(reg);

        assert!(engine.check_duplicate_query_names(["A", "B", "C"]).is_empty());
        assert!(engine.check_duplicate_query_names([]).is_empty());
    }

    /// #137: the id on the finding must come from the rule, so the rule the
    /// registry advertises and the finding a run emits cannot drift apart.
    #[test]
    fn check_duplicate_query_names_uses_the_rules_own_id() {
        use crate::rule::LintRule;

        let mut reg = RuleRegistry::new();
        reg.register(Box::new(crate::rules::codegen::DuplicateQueryNames));
        let engine = LintEngine::new(reg);

        let violations = engine.check_duplicate_query_names(["dup", "dup"]);
        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].rule_id,
            crate::rules::codegen::DuplicateQueryNames.id(),
            "the finding's id must be the registered rule's id"
        );
    }

    /// #137: `build_report` and the standalone check must be the same
    /// derivation — a caller must not get a different verdict depending on
    /// which entry point it used.
    #[test]
    fn build_report_and_check_duplicate_query_names_agree() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(crate::rules::codegen::DuplicateQueryNames));
        let engine = LintEngine::new(reg);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();

        let names = ["dup", "dup", "dup", "solo"];
        let analyzed: Vec<_> = names.iter().map(|n| dummy_analyzed(n)).collect();
        let annotations: Vec<_> = names.iter().map(|n| dummy_annotations(n)).collect();
        let queries: Vec<_> = analyzed
            .iter()
            .zip(annotations.iter())
            .map(|(a, ann)| make_ctx(sql, &stmt, a, &catalog, ann))
            .collect();

        let report = engine.build_report(queries.into_iter(), &catalog);
        let from_report: Vec<(&str, &str)> = report
            .violations
            .iter()
            .filter(|v| v.rule_id == "SC-C03")
            .map(|v| (v.query_name.as_str(), v.message.as_str()))
            .collect();

        let standalone = engine.check_duplicate_query_names(names);
        let from_standalone: Vec<(&str, &str)> = standalone
            .iter()
            .map(|v| (v.query_name.as_str(), v.message.as_str()))
            .collect();

        assert_eq!(from_report, from_standalone);
        assert_eq!(from_report.len(), 1);
    }

    /// #167: a dialect-gated rule that cannot run must be countable up
    /// front, not merely invisible. The engine's answer must match the
    /// registry's, since a caller reaches it through either.
    #[test]
    fn rules_inapplicable_to_reports_dialect_gated_rules() {
        use crate::audit::{MatcherRegistry, MatcherRule, canonical_specs};

        let mut reg = RuleRegistry::new();
        let matchers = MatcherRegistry::canonical();
        for spec in canonical_specs() {
            let matcher_fn = matchers.get(&spec.matcher).expect("canonical matcher must exist");
            reg.register(Box::new(MatcherRule::new(spec, matcher_fn)));
        }
        let engine = LintEngine::new(reg);

        let skipped_for_mysql = engine.rules_inapplicable_to(SqlDialect::MySQL);
        assert_eq!(
            skipped_for_mysql.len(),
            28,
            "28 canonical audit rules declare dialects = [\"postgres\"]; \
             if this number moves, `scythe audit`'s skipped-rule summary moves with it"
        );

        assert!(
            engine.rules_inapplicable_to(SqlDialect::PostgreSQL).is_empty(),
            "no canonical rule is gated away from PostgreSQL"
        );
    }
}
