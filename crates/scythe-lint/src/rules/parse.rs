//! Parse-failure rules (`SC-PARSE01`, `SC-PARSE02`) — a query that never
//! became an [`scythe_core::analyzer::AnalyzedQuery`], so no other rule ever
//! saw it.
//!
//! Both rules leave [`LintRule::check_query`] and [`LintRule::check_catalog`]
//! at their no-op defaults, for the same structural reason the provenance and
//! drift rules do: `check_query` takes a [`crate::types::LintContext`], which
//! wraps an already-parsed-and-analyzed query, and these two findings exist
//! precisely for the case where that construction failed. There is nothing
//! for `check_query` to be called with.
//!
//! They live in [`crate::registry::parse_registry`], **not** in
//! [`crate::registry::default_registry`] — same reasoning as
//! [`crate::rules::provenance`]: putting them in the default registry would
//! have `scythe lint` and `scythe audit --list-rules` advertise two rules
//! neither can ever emit through `check_query`/`check_catalog`, and would
//! move the documented "58 built-in rules" figure.
//!
//! Unlike the provenance and drift rules, these two are not scoped to a
//! single check-time command: `scythe check`, `scythe lint`, and
//! `scythe audit` each hit the same failure (a query that will not parse, or
//! parses but fails semantic analysis) and each used to hardcode its own
//! `Severity::Error` at the point the finding was constructed. Being a
//! registry rather than three independent hardcoded severities is what lets
//! one `[lint.rules] "SC-PARSE01" = "warn"` reach all three commands at once.

use crate::rule::LintRule;
use crate::types::{RuleCategory, Severity};

/// `SC-PARSE01` — a query's SQL text could not be parsed at all.
///
/// `Error` by default: an unparseable query is not linted, not analyzed, and
/// not generated — everything downstream silently skipped it before this
/// became its own finding, which reads as a clean run over a query nobody
/// actually checked.
pub struct UnparseableQuery;

impl LintRule for UnparseableQuery {
    fn id(&self) -> &'static str {
        "SC-PARSE01"
    }
    fn name(&self) -> &'static str {
        "unparseable-query"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Parse
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &'static str {
        "Query SQL text could not be parsed, so no rule could examine it"
    }
}

/// `SC-PARSE02` — a query parsed but failed semantic analysis (e.g. an
/// unresolvable table or column reference).
///
/// `Error` by default, for the same reason as [`UnparseableQuery`]: a query
/// that never became an `AnalyzedQuery` is one nothing else in scythe ever
/// looked at.
pub struct UnanalyzableQuery;

impl LintRule for UnanalyzableQuery {
    fn id(&self) -> &'static str {
        "SC-PARSE02"
    }
    fn name(&self) -> &'static str {
        "unanalyzable-query"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Parse
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &'static str {
        "Query parsed but failed semantic analysis, so no rule could examine it"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_rules() -> Vec<&'static dyn LintRule> {
        vec![&UnparseableQuery as &dyn LintRule, &UnanalyzableQuery]
    }

    #[test]
    fn ids_are_unique_and_in_declared_order() {
        let ids: Vec<&str> = all_rules().iter().map(|r| r.id()).collect();
        assert_eq!(ids, vec!["SC-PARSE01", "SC-PARSE02"]);
    }

    #[test]
    fn every_rule_is_in_the_parse_category() {
        for rule in all_rules() {
            assert_eq!(rule.category(), RuleCategory::Parse, "rule: {}", rule.id());
        }
    }

    #[test]
    fn both_rules_default_to_error() {
        assert_eq!(UnparseableQuery.default_severity(), Severity::Error);
        assert_eq!(UnanalyzableQuery.default_severity(), Severity::Error);
    }

    /// If either rule ever grows a real `check_query`/`check_catalog`
    /// implementation, whichever command constructs its ad hoc finding today
    /// would start double-reporting it.
    #[test]
    fn no_parse_rule_participates_in_the_check_query_path() {
        let catalog = scythe_core::catalog::Catalog::from_ddl(&["CREATE TABLE t (id INTEGER);"]).unwrap();
        for rule in all_rules() {
            assert!(
                rule.check_catalog(&catalog).is_empty(),
                "rule {} produced catalog violations; parse findings must come from the parse/analyze call sites",
                rule.id()
            );
        }
    }
}
