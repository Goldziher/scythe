use std::borrow::Cow;

use ahash::AHashSet;

use super::registry::RuleRegistry;
use super::types::{LintContext, Severity, Violation};
use scythe_core::catalog::Catalog;

// ---------------------------------------------------------------------------
// Public types
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
        self.violations
            .iter()
            .any(|v| matches!(v.severity, Severity::Error))
    }

    pub fn has_warnings(&self) -> bool {
        self.violations
            .iter()
            .any(|v| matches!(v.severity, Severity::Warn))
    }
}

// ---------------------------------------------------------------------------
// LintEngine
// ---------------------------------------------------------------------------

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

    /// Run all checks over a set of queries and produce a report.
    ///
    /// `queries` is an iterator of `LintContext` for each query.
    /// The engine also performs cross-query checks (e.g. duplicate names).
    pub fn build_report<'a>(
        &self,
        queries: impl Iterator<Item = LintContext<'a>>,
        catalog: &Catalog,
    ) -> LintReport {
        let active = self.registry.active_rules();
        let rules_active = active.len();
        let mut violations = Vec::new();
        let mut queries_checked: usize = 0;
        let mut seen_names: AHashSet<String> = AHashSet::new();
        let mut duplicate_names: Vec<String> = Vec::new();

        for ctx in queries {
            queries_checked += 1;

            // Track duplicate query names
            let qname = ctx.analyzed.name.clone();
            if !seen_names.insert(qname.clone()) {
                duplicate_names.push(qname.clone());
            }

            // Run per-query rules
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

        // Emit duplicate-name violations (handled at engine level)
        for dup in &duplicate_names {
            violations.push(QueryViolation {
                query_name: dup.clone(),
                rule_id: Cow::Borrowed("SC-C03"),
                severity: Severity::Error,
                message: format!("duplicate query name: \"{}\"", dup),
            });
        }

        // Catalog-level checks
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

    // -- Helpers ---------------------------------------------------------------

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
        AnalyzedQuery {
            name: name.to_string(),
            command: QueryCommand::Many,
            sql: "SELECT 1".to_string(),
            columns: vec![],
            params: vec![],
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
        }
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
        }
    }

    // -- Tests -----------------------------------------------------------------

    #[test]
    fn lint_engine_new_creates_engine() {
        let reg = RuleRegistry::new();
        let engine = LintEngine::new(reg);
        // Engine is created successfully; no rules registered
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
        reg.register(Box::new(AlwaysWarnRule)); // default Warn
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
        reg.register(Box::new(AlwaysWarnRule)); // Warn
        reg.register(Box::new(CatalogRule)); // Error (catalog-level)
        reg.register(Box::new(SilentRule)); // Warn but never fires
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
        assert!(report.has_warnings()); // from AlwaysWarnRule
        assert!(report.has_errors()); // from CatalogRule
        // AlwaysWarnRule fires 1 query violation, CatalogRule fires 1 catalog violation
        assert_eq!(report.violations.len(), 2);
    }

    #[test]
    fn build_report_duplicate_query_names() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(SilentRule)); // no per-query violations
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

        // Should detect one duplicate for "dup_name"
        let dup_violations: Vec<_> = report
            .violations
            .iter()
            .filter(|v| v.rule_id == "SC-C03")
            .collect();
        assert_eq!(dup_violations.len(), 1);
        assert_eq!(dup_violations[0].query_name, "dup_name");
        assert_eq!(dup_violations[0].severity, Severity::Error);
        assert!(dup_violations[0].message.contains("duplicate query name"));
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
        let dup_violations: Vec<_> = report
            .violations
            .iter()
            .filter(|v| v.rule_id == "SC-C03")
            .collect();
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
}
