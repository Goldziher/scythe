use super::types::{LintContext, RuleCategory, Severity, Violation};
use scythe_core::catalog::Catalog;

/// A single lint rule that can inspect queries and/or the catalog.
pub trait LintRule: Send + Sync {
    /// Unique identifier, e.g. "SC-S01".
    fn id(&self) -> &'static str;

    /// Human-readable short name.
    fn name(&self) -> &'static str;

    /// Category this rule belongs to.
    fn category(&self) -> RuleCategory;

    /// Default severity when no config overrides it.
    fn default_severity(&self) -> Severity;

    /// One-line description of what the rule checks.
    fn description(&self) -> &'static str;

    /// Check a single parsed + analyzed query.  Returns violations found.
    fn check_query(&self, _ctx: &LintContext<'_>) -> Vec<Violation> {
        Vec::new()
    }

    /// Check the full catalog (useful for cross-table / naming rules).
    fn check_catalog(&self, _catalog: &Catalog) -> Vec<Violation> {
        Vec::new()
    }
}
