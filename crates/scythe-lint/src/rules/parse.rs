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
//! move the documented "59 built-in rules" figure.
//!
//! Unlike the provenance and drift rules, these two are not scoped to a
//! single check-time command: `scythe check`, `scythe lint`, and
//! `scythe audit` each hit the same failure (a query that will not parse, or
//! parses but fails semantic analysis) and each used to hardcode its own
//! `Severity::Error` at the point the finding was constructed. Being a
//! registry rather than three independent hardcoded severities is what lets
//! one `[lint.rules] "SC-PARSE01" = "warn"` reach all three commands at once.
//!
//! `SC-PARSE03` ([`MisspelledAnnotation`]) below does **not** share that
//! structural gap — it exists precisely because the query *did* parse and
//! analyze successfully, so a real [`crate::types::LintContext`] is
//! available for it to inspect. It therefore lives in
//! [`crate::registry::default_registry`] (see `SC-PARSE03`'s own doc for
//! why), not [`crate::registry::parse_registry`], even though its id keeps
//! the file's `SC-PARSE` prefix and its struct lives in this module — both
//! follow scythe-lint's file-to-id-prefix convention (every rule in
//! `parse.rs` is `SC-PARSE*`), which is a filing convention, not a claim
//! that it shares SC-PARSE01/02's structural limitation.

use std::borrow::Cow;

use crate::rule::LintRule;
use crate::types::{LintContext, LintFix, RuleCategory, Severity, Violation};

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

/// `SC-PARSE03` — an unrecognised `-- @<name>` annotation is within edit
/// distance 2 of a known annotation keyword (see
/// `scythe_core::parser::KNOWN_ANNOTATION_KEYWORDS`), so it is very likely a
/// typo of that keyword rather than a genuine consumer-defined annotation.
///
/// `scythe_core::parser` deliberately never rejects an unrecognised
/// `-- @<name>` outright — that escape hatch is how consumers layer their own
/// vocabulary (`@http`, `@http_auth`, ...) on top of scythe without coupling
/// scythe to their domain — so it captures every one as an opaque
/// [`scythe_core::parser::CustomAnnotation`] and, since #152, additionally
/// populates [`scythe_core::parser::CustomAnnotation::suggested_keyword`]
/// when a known keyword is close enough to plausibly be what the user meant.
/// Nothing consumed that signal until this rule: a typo'd `@nullible` parsed
/// clean, analyzed clean, and `scythe generate`/`check`/`lint` all reported
/// success while the nullability override it named silently never took
/// effect.
///
/// `Warn`, not `Error`: the same escape hatch this rule rides on is a
/// deliberate, documented extension point with legitimate shipping usage
/// (`@http`, `@http_auth`). An annotation two edits from a known keyword is
/// worth flagging, but `suggested_keyword` is a heuristic, not proof the
/// annotation is wrong — `@nolabel` is two edits from `nullable` and could
/// just as easily be a real, if unfortunately-named, consumer annotation.
/// Erroring here would mean an SC-PARSE03 false positive fails `scythe
/// check` for someone who did nothing wrong; warning surfaces the same
/// signal without that risk, and `[lint.rules] "SC-PARSE03" = "error"`
/// remains available for a user who wants the stricter behavior.
///
/// Lives in [`crate::registry::default_registry`] rather than
/// [`crate::registry::parse_registry`]: unlike `SC-PARSE01`/`SC-PARSE02`,
/// this rule has a real, already-parsed-and-analyzed
/// [`crate::types::LintContext`] to examine, so it reaches `scythe lint` and
/// `scythe audit --list-rules` the same way every other `check_query` rule
/// does. `scythe check` (`crates/scythe-cli/src/commands/generate.rs`,
/// `run_check`) already builds a `LintContext` — including `annotations` —
/// for every successfully analyzed query and runs it through
/// `default_registry`'s `LintEngine::check_query`, so this rule needs no
/// additional check-command wiring beyond registration.
pub struct MisspelledAnnotation;

impl LintRule for MisspelledAnnotation {
    fn id(&self) -> &'static str {
        "SC-PARSE03"
    }
    fn name(&self) -> &'static str {
        "misspelled-annotation"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Codegen
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "Unrecognised annotation is within edit distance 2 of a known annotation keyword"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        ctx.annotations
            .custom
            .iter()
            .filter_map(|annotation| {
                let suggested = annotation.suggested_keyword.as_deref()?;
                Some(Violation {
                    rule_id: Cow::Borrowed(self.id()),
                    message: format!(
                        "unrecognised annotation \"@{}\" — did you mean \"@{}\"?",
                        annotation.name, suggested
                    ),
                    fix: Some(LintFix {
                        description: format!("Rename \"@{}\" to \"@{}\"", annotation.name, suggested),
                        replacement: suggested.to_string(),
                    }),
                })
            })
            .collect()
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

    // ~keep SC-PARSE03 (`MisspelledAnnotation`) — unlike SC-PARSE01/02 above, this
    // rule has a real `LintContext` to inspect, so its tests build one the
    // same way `crate::rules::codegen`'s tests do.

    fn make_catalog() -> scythe_core::catalog::Catalog {
        scythe_core::catalog::Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL);",
        ])
        .unwrap()
    }

    fn make_ctx<'a>(
        query: &'a scythe_core::parser::Query,
        analyzed: &'a scythe_core::analyzer::AnalyzedQuery,
        catalog: &'a scythe_core::catalog::Catalog,
    ) -> LintContext<'a> {
        LintContext {
            sql: &query.sql,
            stmt: &query.stmt,
            analyzed,
            catalog,
            annotations: &query.annotations,
            dialect: scythe_core::dialect::SqlDialect::PostgreSQL,
        }
    }

    #[test]
    fn misspelled_annotation_metadata() {
        let rule = MisspelledAnnotation;
        assert_eq!(rule.id(), "SC-PARSE03");
        assert_eq!(rule.name(), "misspelled-annotation");
        assert_eq!(rule.category(), RuleCategory::Codegen);
        assert_eq!(rule.default_severity(), Severity::Warn);
    }

    /// The required assertion for #152's residual: a `-- @nullible` typo must
    /// be named, with its suggested keyword, in the violation message — not
    /// silently ignored the way it was before `suggested_keyword` existed.
    #[test]
    fn fires_naming_the_typo_and_the_suggested_keyword() {
        let catalog = make_catalog();
        let query = scythe_core::parser::parse_query(
            "-- @name GetUser\n-- @returns :one\n-- @nullible email\nSELECT id, name, email FROM users WHERE id = $1",
        )
        .unwrap();
        let analyzed = scythe_core::analyzer::analyze(&catalog, &query).unwrap();
        let ctx = make_ctx(&query, &analyzed, &catalog);

        let violations = MisspelledAnnotation.check_query(&ctx);

        assert_eq!(violations.len(), 1, "got: {violations:?}");
        assert_eq!(violations[0].rule_id, "SC-PARSE03");
        assert_eq!(
            violations[0].message,
            "unrecognised annotation \"@nullible\" — did you mean \"@nullable\"?"
        );
    }

    /// Required negative case: `suggested_keyword: None` must not fire.
    /// `@http` is the documented, legitimate consumer-annotation example
    /// (see `scythe_core::parser::CustomAnnotation`'s doc) and is far enough
    /// from every known keyword that `scythe_core`'s own
    /// `test_custom_annotation_far_from_any_keyword_has_no_suggestion`
    /// already pins `suggested_keyword` as `None` for it.
    #[test]
    fn does_not_fire_when_suggested_keyword_is_none() {
        let catalog = make_catalog();
        let query = scythe_core::parser::parse_query(
            "-- @name GetUser\n-- @returns :one\n-- @http GET /users/{id}\nSELECT id, name, email FROM users WHERE \
             id = $1",
        )
        .unwrap();
        let analyzed = scythe_core::analyzer::analyze(&catalog, &query).unwrap();
        let ctx = make_ctx(&query, &analyzed, &catalog);

        assert!(query.annotations.custom[0].suggested_keyword.is_none());
        assert!(MisspelledAnnotation.check_query(&ctx).is_empty());
    }

    #[test]
    fn does_not_fire_when_there_are_no_custom_annotations() {
        let catalog = make_catalog();
        let query =
            scythe_core::parser::parse_query("-- @name GetUser\n-- @returns :one\nSELECT id FROM users WHERE id = $1")
                .unwrap();
        let analyzed = scythe_core::analyzer::analyze(&catalog, &query).unwrap();
        let ctx = make_ctx(&query, &analyzed, &catalog);

        assert!(MisspelledAnnotation.check_query(&ctx).is_empty());
    }

    #[test]
    fn fires_once_per_misspelled_annotation() {
        let catalog = make_catalog();
        let query = scythe_core::parser::parse_query(
            "-- @name GetUser\n-- @returns :one\n-- @nullible email\n-- @nonull name\nSELECT id, name, email FROM \
             users WHERE id = $1",
        )
        .unwrap();
        let analyzed = scythe_core::analyzer::analyze(&catalog, &query).unwrap();
        let ctx = make_ctx(&query, &analyzed, &catalog);

        let violations = MisspelledAnnotation.check_query(&ctx);
        assert_eq!(violations.len(), 2, "got: {violations:?}");
        assert!(
            violations
                .iter()
                .any(|v| v.message.contains("@nullible") && v.message.contains("@nullable"))
        );
        assert!(
            violations
                .iter()
                .any(|v| v.message.contains("@nonull") && v.message.contains("@nonnull"))
        );
    }
}
