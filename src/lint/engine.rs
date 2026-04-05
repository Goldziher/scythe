use ahash::AHashSet;

use super::registry::RuleRegistry;
use super::types::{LintContext, Severity, Violation};
use crate::catalog::Catalog;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct QueryViolation {
    pub query_name: String,
    pub rule_id: &'static str,
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
                rule_id: "SC-C03",
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
