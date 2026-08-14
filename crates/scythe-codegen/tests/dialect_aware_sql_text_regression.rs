//! End-to-end regression tests for board #148: `java-jdbc` (which supports
//! `postgresql`, `mysql`, and `mssql` engines from one backend) is used here
//! to exercise the dialect-aware SQL-text pipeline
//! (`clean_sql_oneline_with_optional_dialect` /
//! `rewrite_placeholders_indexed` in `crates/scythe-codegen/src/backends/
//! mod.rs`) through the real `generate_with_backend` path, not just the unit
//! tests in `mod.rs` itself.
//!
//! Every SQL fixture here is constructed directly as `AnalyzedQuery.sql`
//! (bypassing the parser), so it does not need to be valid, parseable SQL --
//! only realistic MySQL/MSSQL source text, exactly as the old dialect-blind
//! `clean_sql`/`clean_sql_oneline`/`rewrite_pg_placeholders` would have seen
//! it too.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::AnalyzedQuery;
use scythe_core::parser::QueryCommand;

fn zero_param_exec_query(sql: &str) -> AnalyzedQuery {
    AnalyzedQuery::build(|q| {
        q.name = "TouchRow".to_string();
        q.command = QueryCommand::Exec;
        q.sql = sql.to_string();
        q.columns = vec![];
        q.params = vec![];
    })
}

fn query_fn(engine: &str, query: &AnalyzedQuery) -> String {
    let backend =
        get_backend("java-jdbc", engine).unwrap_or_else(|error| panic!("java-jdbc must support {engine}: {error}"));
    let generated = generate_with_backend(query, &*backend).unwrap_or_else(|error| panic!("codegen failed: {error}"));
    generated.query_fn.unwrap_or_else(|| panic!("produced no query fn"))
}

/// board #148 item 2: a MySQL backtick-quoted identifier containing `-- ` must
/// survive intact -- the old dialect-blind tokenizer had no concept of
/// backtick quoting, so it read the `--` inside as opening a line comment
/// and truncated the query at that point. Under the old code, the SQL
/// literal in the generated Java would have been just
/// `"SELECT \`a"` (see `mod.rs`'s
/// `test_clean_sql_dialect_mysql_backtick_preserves_double_dash_inside`,
/// which pins that exact corruption against the dialect-blind `clean_sql`).
#[test]
fn java_jdbc_mysql_preserves_a_backtick_identifier_containing_double_dash() {
    let query = zero_param_exec_query("SELECT `a -- b` FROM t");
    let code = query_fn("mysql", &query);
    assert!(
        code.contains("prepareStatement(\"SELECT `a -- b` FROM t\")"),
        "backtick identifier must survive intact; got:\n{code}"
    );
}

/// board #148 item 4: same as above for an MSSQL `[bracketed]` identifier.
#[test]
fn java_jdbc_mssql_preserves_a_bracket_identifier_containing_double_dash() {
    let query = zero_param_exec_query("SELECT [a -- b] FROM t");
    let code = query_fn("mssql", &query);
    assert!(
        code.contains("prepareStatement(\"SELECT [a -- b] FROM t\")"),
        "bracket identifier must survive intact; got:\n{code}"
    );
}

/// board #148 item 3: a MySQL `#` line comment must be stripped -- the old
/// dialect-blind tokenizer deliberately never treated `#` as a comment
/// starter (it collides with PostgreSQL's `#>`/`#>>` JSON operators), so it
/// left the comment text in the generated SQL literal verbatim.
#[test]
fn java_jdbc_mysql_strips_a_hash_comment() {
    let query = zero_param_exec_query("SELECT 1 # do not leak this comment\nFROM t");
    let code = query_fn("mysql", &query);
    assert!(
        !code.contains("do not leak this comment"),
        "# comment must be stripped under MySQL; got:\n{code}"
    );
    assert!(code.contains("SELECT 1"), "got:\n{code}");
    assert!(code.contains("FROM t"), "got:\n{code}");
}

/// board #148 item 1, at the `java-jdbc` integration level: a zero-placeholder
/// PostgreSQL query using the JSONB `?` key-existence operator must not
/// synthesize a phantom bind. Every occurrence in `mod.rs`'s formatter for
/// this backend is the literal character `?`, so unlike the `$N`-native
/// backends the board's headline example targets (`typescript-postgres`,
/// `csharp-npgsql`, `go-pgx`, none owned by this change), the SQL *text* this
/// backend emits cannot visibly distinguish "treated as JSONB operator" from
/// "misrewritten to a placeholder that happens to format as `?` too" -- and
/// the pre-fix bind loop derived its bind count from `params.len()` (always
/// correct, 0 here) rather than from the placeholder rewrite's own occurrence
/// count, so this specific defect shape was latent, not observable, through
/// this backend's *generated code* even before the fix. It is proven
/// directly (with a formatter that *does* make the difference visible) in
/// `mod.rs`'s
/// `test_bare_question_mark_is_never_a_placeholder_under_postgresql_even_with_zero_dollar_placeholders`.
/// This test instead guards the new occurrence-based bind design itself: it
/// would fail if a future change made the dialect check wrong in the
/// direction of *treating* the bare `?` as a placeholder, since that would
/// panic in `resolved_param_for_position` (no parameter registered for the
/// synthesized position) rather than silently emit zero binds.
#[test]
fn java_jdbc_postgresql_zero_param_jsonb_query_emits_no_binds() {
    let query = zero_param_exec_query("SELECT * FROM docs WHERE data ? 'active'");
    let code = query_fn("postgresql", &query);
    assert!(
        code.contains("prepareStatement(\"SELECT * FROM docs WHERE data ? 'active'\")"),
        "got:\n{code}"
    );
    assert_eq!(
        code.matches("ps.set").count(),
        0,
        "a zero-parameter query must emit no bind statements; got:\n{code}"
    );
}
