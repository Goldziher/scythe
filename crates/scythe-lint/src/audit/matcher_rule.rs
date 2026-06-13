//! `MatcherRule` — a single `LintRule` impl that delegates to a named matcher.
//!
//! Leaked strings: `LintRule::id`, `name`, and `description` must return
//! `&'static str`.  Since `RuleSpec` fields are runtime `String`s we
//! `Box::leak` them once at construction time.  This is intentional and
//! bounded: we register rules once at startup and the count is fixed.

use std::borrow::Cow;

use crate::rule::LintRule;
use crate::types::{LintContext, RuleCategory, Severity, Violation};

use super::registry::{MatcherFn, MatcherHit};
use super::spec::RuleSpec;

// ---------------------------------------------------------------------------
// render_template
// ---------------------------------------------------------------------------

/// Single-pass `{ident}` substitution.
///
/// Scans `template` left-to-right using byte offsets (safe because `{` and `}`
/// are single-byte ASCII characters that cannot appear as continuation bytes in
/// valid UTF-8).  Each `{ident}` token whose key is present in `bindings` is
/// replaced with the bound value.  Unknown keys are left as the literal
/// `{ident}` text.  Substituted content is never re-scanned.
pub fn render_template(template: &str, bindings: &ahash::AHashMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'{' {
            // Find the matching closing brace
            if let Some(close_offset) = bytes[i + 1..].iter().position(|&b| b == b'}') {
                let close = i + 1 + close_offset;
                let key = &template[i + 1..close];
                // Only substitute if the key is a valid identifier-ish token
                if !key.is_empty()
                    && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    && let Some(value) = bindings.get(key)
                {
                    out.push_str(value);
                    i = close + 1;
                    continue;
                }
                // Not a known binding — emit the literal `{key}`
                out.push_str(&template[i..=close]);
                i = close + 1;
                continue;
            }
        }
        // Not a `{` byte.  Advance by one UTF-8 character to handle multi-byte
        // sequences correctly.  `{` (0x7B) and `}` (0x7D) are ASCII and cannot
        // appear as continuation bytes (0x80–0xBF), so the byte scan above is
        // always on a character boundary when it sees `{`.
        let ch_len = utf8_char_len(bytes[i]);
        out.push_str(&template[i..i + ch_len]);
        i += ch_len;
    }

    out
}

/// Return the byte length of the UTF-8 character whose leading byte is `b`.
///
/// Leading byte ranges per the UTF-8 spec:
/// - `0x00–0x7F`: 1 byte (ASCII)
/// - `0x80–0xBF`: continuation byte — should not appear at a character
///   boundary; fall back to 1 so the scanner always makes progress.
/// - `0xC0–0xDF`: 2 bytes
/// - `0xE0–0xEF`: 3 bytes
/// - `0xF0–0xF7`: 4 bytes
fn utf8_char_len(b: u8) -> usize {
    match b {
        0x00..=0xBF => 1, // ASCII or (unexpected) continuation byte
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

// ---------------------------------------------------------------------------
// MatcherRule
// ---------------------------------------------------------------------------

/// A `LintRule` backed by a TOML `RuleSpec` and a `MatcherFn`.
pub struct MatcherRule {
    spec: RuleSpec,
    matcher_fn: MatcherFn,
    // Leaked because LintRule::id / name / description return &'static str.
    id_leaked: &'static str,
    name_leaked: &'static str,
    description_leaked: &'static str,
}

impl MatcherRule {
    /// Construct a `MatcherRule`.
    ///
    /// The `id`, `name`, and `description` strings from `spec` are leaked via
    /// `Box::leak` so they satisfy the `&'static str` contract of `LintRule`.
    /// This is acceptable because rules are registered once at startup and
    /// never freed.
    pub fn new(spec: RuleSpec, matcher_fn: MatcherFn) -> Self {
        let id_leaked = Box::leak(spec.id.clone().into_boxed_str());
        let name_leaked = Box::leak(spec.name.clone().into_boxed_str());
        let description_leaked = Box::leak(spec.description.clone().into_boxed_str());
        Self {
            spec,
            matcher_fn,
            id_leaked,
            name_leaked,
            description_leaked,
        }
    }
}

impl LintRule for MatcherRule {
    fn id(&self) -> &'static str {
        self.id_leaked
    }

    fn name(&self) -> &'static str {
        self.name_leaked
    }

    fn category(&self) -> RuleCategory {
        self.spec.category
    }

    fn default_severity(&self) -> Severity {
        self.spec.severity
    }

    fn description(&self) -> &'static str {
        self.description_leaked
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        // Dialect filtering: if the spec restricts dialects and the context
        // dialect is not in the list, skip this rule.
        if !self.spec.dialects.is_empty() && !self.spec.dialects.contains(&ctx.dialect) {
            return Vec::new();
        }

        (self.matcher_fn)(ctx, &self.spec.matcher_args)
            .into_iter()
            .map(|hit: MatcherHit| Violation {
                rule_id: Cow::Owned(self.spec.id.clone()),
                message: render_template(&self.spec.message, &hit.bindings),
                fix: None,
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;

    // -- render_template tests --------------------------------------------------

    #[test]
    fn render_literal_passthrough() {
        let bindings = AHashMap::new();
        assert_eq!(
            render_template("no placeholders", &bindings),
            "no placeholders"
        );
    }

    #[test]
    fn render_single_binding() {
        let mut b = AHashMap::new();
        b.insert("func".to_string(), "pg_read_file".to_string());
        assert_eq!(
            render_template("call to {func} — dangerous", &b),
            "call to pg_read_file — dangerous"
        );
    }

    #[test]
    fn render_missing_binding_left_as_literal() {
        let bindings = AHashMap::new();
        assert_eq!(
            render_template("call to {func}", &bindings),
            "call to {func}"
        );
    }

    #[test]
    fn render_brace_in_bound_value_not_rescanned() {
        let mut b = AHashMap::new();
        // If the value contains `{pattern}` it must NOT be expanded again.
        b.insert("func".to_string(), "{pattern}".to_string());
        b.insert("pattern".to_string(), "SHOULD_NOT_APPEAR".to_string());
        let result = render_template("call to {func}", &b);
        // The substituted value `{pattern}` is in the output but NOT further
        // expanded — the result must not contain "SHOULD_NOT_APPEAR".
        assert_eq!(result, "call to {pattern}");
        assert!(!result.contains("SHOULD_NOT_APPEAR"));
    }

    #[test]
    fn render_multiple_bindings() {
        let mut b = AHashMap::new();
        b.insert("func".to_string(), "xp_cmdshell".to_string());
        b.insert("reason".to_string(), "dangerous".to_string());
        assert_eq!(
            render_template("{func} is {reason}", &b),
            "xp_cmdshell is dangerous"
        );
    }

    // -- MatcherRule end-to-end test -------------------------------------------

    use scythe_core::analyzer::AnalyzedQuery;
    use scythe_core::catalog::Catalog;
    use scythe_core::dialect::SqlDialect;
    use scythe_core::parser::{Annotations, QueryCommand};
    use sqlparser::ast::Statement;
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;

    fn parse_stmt(sql: &str) -> Statement {
        Parser::parse_sql(&PostgreSqlDialect {}, sql)
            .unwrap()
            .remove(0)
    }

    fn empty_catalog() -> Catalog {
        Catalog::from_ddl(&[]).unwrap()
    }

    fn dummy_analyzed() -> AnalyzedQuery {
        AnalyzedQuery {
            name: "q".to_string(),
            command: QueryCommand::Many,
            sql: "SELECT 1".to_string(),
            columns: vec![],
            params: vec![],
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
            optional_params: vec![],
            group_by: None,
            custom: vec![],
        }
    }

    fn dummy_annotations() -> Annotations {
        Annotations {
            name: "q".to_string(),
            command: QueryCommand::Many,
            param_docs: vec![],
            nullable_overrides: vec![],
            nonnull_overrides: vec![],
            json_mappings: vec![],
            deprecated: None,
            optional_params: vec![],
            group_by: None,
            custom: vec![],
        }
    }

    fn always_fires(_ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
        vec![MatcherHit::with_binding("func", "dangerous_fn")]
    }

    fn never_fires(_ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
        vec![]
    }

    fn minimal_spec(id: &str, message: &str, severity: Severity) -> RuleSpec {
        use toml::Table;
        RuleSpec {
            id: id.to_string(),
            name: "test-rule".to_string(),
            category: RuleCategory::Security,
            severity,
            dialects: vec![],
            cwe: vec![],
            description: "test description (CWE-78)".to_string(),
            message: message.to_string(),
            matcher: "always_fires".to_string(),
            matcher_args: Table::new(),
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
            dialect: SqlDialect::PostgreSQL,
        }
    }

    #[test]
    fn matcher_rule_fires_and_renders_message() {
        let spec = minimal_spec("SC-TEST01", "call to {func} — bad", Severity::Error);
        let rule = MatcherRule::new(spec, always_fires);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();
        let analyzed = dummy_analyzed();
        let annotations = dummy_annotations();
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);

        let violations = rule.check_query(&ctx);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_id, "SC-TEST01");
        assert_eq!(violations[0].message, "call to dangerous_fn — bad");
    }

    #[test]
    fn matcher_rule_dialect_filter_skips_when_not_matching() {
        let mut spec = minimal_spec("SC-TEST02", "msg", Severity::Warn);
        spec.dialects = vec![SqlDialect::MySQL]; // only MySQL

        let rule = MatcherRule::new(spec, always_fires);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();
        let analyzed = dummy_analyzed();
        let annotations = dummy_annotations();
        // Context is PostgreSQL — rule should be skipped
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let violations = rule.check_query(&ctx);
        assert!(violations.is_empty());
    }

    #[test]
    fn matcher_rule_silent_when_matcher_returns_empty() {
        let spec = minimal_spec("SC-TEST03", "msg", Severity::Error);
        let rule = MatcherRule::new(spec, never_fires);

        let sql = "SELECT 1";
        let stmt = parse_stmt(sql);
        let catalog = empty_catalog();
        let analyzed = dummy_analyzed();
        let annotations = dummy_annotations();
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);

        let violations = rule.check_query(&ctx);
        assert!(violations.is_empty());
    }

    #[test]
    fn matcher_rule_static_str_methods() {
        let spec = minimal_spec("SC-TEST04", "msg", Severity::Warn);
        let rule = MatcherRule::new(spec, never_fires);
        assert_eq!(rule.id(), "SC-TEST04");
        assert_eq!(rule.name(), "test-rule");
        assert!(rule.description().contains("CWE-78"));
        assert_eq!(rule.default_severity(), Severity::Warn);
        assert_eq!(rule.category(), RuleCategory::Security);
    }
}
