use super::types::{LintContext, RuleCategory, Severity, Violation};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;

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

    /// CWE identifiers this rule maps to (e.g. `["CWE-78"]`), if any.
    ///
    /// Defaults to empty: most `LintRule` implementors are plain SQL-shape
    /// checks with no CWE mapping. [`crate::audit::MatcherRule`] overrides
    /// this to expose its [`crate::audit::RuleSpec::cwe`] — falling back to
    /// scanning [`description`](Self::description) for a `CWE-\d+` pattern
    /// only when the spec declared none — so a caller reporting security
    /// findings (e.g. as SARIF) never has to know whether it is looking at a
    /// canonical or a user-supplied rule to get the right CWE list.
    fn cwe(&self) -> Vec<String> {
        Vec::new()
    }

    /// Whether this rule applies to `dialect`.
    ///
    /// Defaults to `true` for every dialect: most `LintRule` implementors
    /// are dialect-agnostic SQL-shape checks. [`crate::audit::MatcherRule`]
    /// overrides this to honour its spec's `dialects` restriction (an empty
    /// list there also means "every dialect"), matching the gate its own
    /// `check_query` already applies internally. Exposing the same decision
    /// here lets a caller compute "N rule(s) skipped: not applicable to
    /// engine '<e>'" instead of that gate staying invisible inside a
    /// `check_query` call that simply returns nothing (#167) -- a caller
    /// silently getting zero findings from a dialect-gated rule must be able
    /// to tell that apart from a rule that ran and found nothing.
    fn is_applicable_to(&self, _dialect: SqlDialect) -> bool {
        true
    }

    /// Check a single parsed + analyzed query.  Returns violations found.
    fn check_query(&self, _ctx: &LintContext<'_>) -> Vec<Violation> {
        Vec::new()
    }

    /// Check the full catalog (useful for cross-table / naming rules).
    fn check_catalog(&self, _catalog: &Catalog) -> Vec<Violation> {
        Vec::new()
    }
}
