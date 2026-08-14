//! Regression tests for GH #149: `java-jdbc`, `kotlin-jdbc`,
//! `kotlin-exposed`, and `php-amphp` used to call
//! `rewrite_pg_placeholders(sql, |_| "?")`, discarding the placeholder
//! number `$N` and then binding in *declaration* order
//! (`params.iter().enumerate()`) instead of the order placeholders actually
//! appear in the SQL text. Three independent ways that mismatch showed up:
//!
//! (a) `$2` before `$1` in the SQL text bound the wrong argument to the wrong
//!     slot -- a silent wrong answer, not an error.
//! (b) a repeated `$1` produced two `?` in the SQL but only one bind
//!     statement, so the driver threw a parameter-count mismatch at runtime.
//! (c) `@optional` rewrites `col = $1` to `($1 IS NULL OR col = $1)` -- two
//!     occurrences of the same declared parameter -- which hit the same
//!     one-bind-per-declared-parameter bug as (b).
//!
//! The fix (see `crates/scythe-codegen/src/backends/mod.rs`'s
//! `rewrite_placeholders_indexed` and `resolved_param_for_position`) binds
//! per SQL-text occurrence instead: each backend now emits one bind
//! statement per placeholder *occurrence*, resolved back to the correct
//! declared parameter by its actual `$N`, not by loop position.
//!
//! Every assertion below is an exact substring of the generated code, and
//! each test's doc comment states what the pre-fix code printed for the same
//! input.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedParam, AnalyzedQuery};
use scythe_core::parser::QueryCommand;

/// Two non-nullable params: `a` (int32, position 1) and `b` (string,
/// position 2) -- matching what the analyzer would produce for a query
/// declaring `$1`/`$2` in that order, regardless of where each is used in
/// the SQL text.
fn two_param_query(sql: &str, optional_params: Vec<String>) -> AnalyzedQuery {
    AnalyzedQuery::build(|q| {
        q.name = "TouchRow".to_string();
        q.command = QueryCommand::Exec;
        q.sql = sql.to_string();
        q.columns = vec![];
        q.params = vec![
            AnalyzedParam {
                name: "a".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
            },
            AnalyzedParam {
                name: "b".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                position: 2,
            },
        ];
        q.optional_params = optional_params;
    })
}

/// One non-nullable string param: `name` (position 1).
fn one_param_query(sql: &str, optional_params: Vec<String>) -> AnalyzedQuery {
    AnalyzedQuery::build(|q| {
        q.name = "TouchRow".to_string();
        q.command = QueryCommand::Exec;
        q.sql = sql.to_string();
        q.columns = vec![];
        q.params = vec![AnalyzedParam {
            name: "name".to_string(),
            neutral_type: "string".to_string(),
            nullable: false,
            position: 1,
        }];
        q.optional_params = optional_params;
    })
}

fn query_fn(backend_name: &str, query: &AnalyzedQuery) -> String {
    let backend = get_backend(backend_name, "postgresql")
        .unwrap_or_else(|error| panic!("{backend_name}: backend must support postgresql: {error}"));
    let generated = generate_with_backend(query, &*backend)
        .unwrap_or_else(|error| panic!("{backend_name}: codegen failed: {error}"));
    generated
        .query_fn
        .unwrap_or_else(|| panic!("{backend_name}: produced no query fn"))
}

fn assert_contains(backend: &str, code: &str, needle: &str) {
    assert!(
        code.contains(needle),
        "{backend}: expected `{needle}`;\ngenerated:\n{code}"
    );
}

// ---------------------------------------------------------------------
// (a) `$2 ... $1`: binds must follow SQL-text order, not declaration order.
// ---------------------------------------------------------------------

/// Old code printed `ps.setInt(1, a);` then `ps.setString(2, b);` -- binding
/// `a`'s value into the first `?` (which is actually `$2`/`b`'s slot in the
/// rewritten SQL `"...WHERE b = ? AND a = ?"`) and vice versa.
#[test]
fn java_jdbc_binds_out_of_order_placeholders_in_sql_text_order() {
    let query = two_param_query("UPDATE t SET touched = true WHERE b = $2 AND a = $1", vec![]);
    let code = query_fn("java-jdbc", &query);
    assert_contains(
        "java-jdbc",
        &code,
        "conn.prepareStatement(\"UPDATE t SET touched = true WHERE b = ? AND a = ?\")",
    );
    assert_contains("java-jdbc", &code, "ps.setString(1, b);\n        ps.setInt(2, a);");
}

/// Old code printed `ps.setInt(1, a)` then `ps.setString(2, b)` (no
/// trailing `;` in Kotlin) -- same swap as the Java case.
#[test]
fn kotlin_jdbc_binds_out_of_order_placeholders_in_sql_text_order() {
    let query = two_param_query("UPDATE t SET touched = true WHERE b = $2 AND a = $1", vec![]);
    let code = query_fn("kotlin-jdbc", &query);
    assert_contains(
        "kotlin-jdbc",
        &code,
        "prepareStatement(\"UPDATE t SET touched = true WHERE b = ? AND a = ?\")",
    );
    assert_contains("kotlin-jdbc", &code, "ps.setString(1, b)\n        ps.setInt(2, a)");
}

/// Old code printed `listOf(IntegerColumnType() to a, TextColumnType() to
/// b)`. Exposed's `exec(sql, args)` binds `args` positionally to the `?`s in
/// `sql`, so that order bound `a`'s value to the first `?` (`b`'s slot).
#[test]
fn kotlin_exposed_binds_out_of_order_placeholders_in_sql_text_order() {
    let query = two_param_query("UPDATE t SET touched = true WHERE b = $2 AND a = $1", vec![]);
    let code = query_fn("kotlin-exposed", &query);
    assert_contains(
        "kotlin-exposed",
        &code,
        "exec(\"UPDATE t SET touched = true WHERE b = ? AND a = ?\", listOf(TextColumnType() to b, IntegerColumnType() to a))",
    );
}

/// Old code printed `->execute([$a, $b]);` -- `$a` bound to the first `?`
/// (`b`'s slot) and `$b` to the second (`a`'s slot).
#[test]
fn php_amphp_binds_out_of_order_placeholders_in_sql_text_order() {
    let query = two_param_query("UPDATE t SET touched = true WHERE b = $2 AND a = $1", vec![]);
    let code = query_fn("php-amphp", &query);
    assert_contains(
        "php-amphp",
        &code,
        "$pool->prepare('UPDATE t SET touched = true WHERE b = ? AND a = ?')->execute([$b, $a]);",
    );
}

// ---------------------------------------------------------------------
// (b) a repeated `$1` must bind once per occurrence, not once per declared
// parameter.
// ---------------------------------------------------------------------

/// Old code printed a single `ps.setInt(1, a);` for a SQL text carrying two
/// `?`s -- the second placeholder was never bound, so the generated code
/// compiled but `PreparedStatement.executeUpdate()` threw at runtime
/// ("parameter index out of range" / "not all parameters ... bound",
/// depending on driver).
#[test]
fn java_jdbc_binds_a_repeated_placeholder_once_per_occurrence() {
    let query = two_param_query("UPDATE t SET touched = true WHERE a = $1 OR parent_id = $1", vec![]);
    let code = query_fn("java-jdbc", &query);
    assert_contains("java-jdbc", &code, "ps.setInt(1, a);\n        ps.setInt(2, a);");
    assert_eq!(
        code.matches("ps.set").count(),
        2,
        "exactly two binds must be emitted for two placeholder occurrences; got:\n{code}"
    );
}

#[test]
fn kotlin_jdbc_binds_a_repeated_placeholder_once_per_occurrence() {
    let query = two_param_query("UPDATE t SET touched = true WHERE a = $1 OR parent_id = $1", vec![]);
    let code = query_fn("kotlin-jdbc", &query);
    assert_contains("kotlin-jdbc", &code, "ps.setInt(1, a)\n        ps.setInt(2, a)");
}

#[test]
fn kotlin_exposed_binds_a_repeated_placeholder_once_per_occurrence() {
    let query = two_param_query("UPDATE t SET touched = true WHERE a = $1 OR parent_id = $1", vec![]);
    let code = query_fn("kotlin-exposed", &query);
    assert_contains(
        "kotlin-exposed",
        &code,
        "listOf(IntegerColumnType() to a, IntegerColumnType() to a)",
    );
}

#[test]
fn php_amphp_binds_a_repeated_placeholder_once_per_occurrence() {
    let query = two_param_query("UPDATE t SET touched = true WHERE a = $1 OR parent_id = $1", vec![]);
    let code = query_fn("php-amphp", &query);
    assert_contains("php-amphp", &code, "->execute([$a, $a]);");
}

// ---------------------------------------------------------------------
// (c) `@optional` must produce matching marker and bind counts.
// ---------------------------------------------------------------------

/// `-- @optional name` rewrites `name = $1` to `($1 IS NULL OR name = $1)`
/// before placeholder rewriting ever runs -- two occurrences of the same
/// declared parameter. Old code printed a single `ps.setString(1, name);`
/// for a SQL text carrying two `?`s, throwing at runtime exactly like the
/// repeated-placeholder case above (this is the same root cause reached
/// through `@optional` instead of a hand-written repeated `$1`).
#[test]
fn java_jdbc_optional_param_produces_matching_marker_and_bind_counts() {
    let query = one_param_query("UPDATE t SET touched = true WHERE name = $1", vec!["name".to_string()]);
    let code = query_fn("java-jdbc", &query);
    assert_contains(
        "java-jdbc",
        &code,
        "conn.prepareStatement(\"UPDATE t SET touched = true WHERE (? IS NULL OR name = ?)\")",
    );
    let marker_count = code.matches('?').count();
    let bind_count = code.matches("ps.set").count();
    assert_eq!(marker_count, 2, "expected two `?` markers; got:\n{code}");
    assert_eq!(
        bind_count, marker_count,
        "bind count must match marker count; got:\n{code}"
    );
    assert_contains(
        "java-jdbc",
        &code,
        "ps.setString(1, name);\n        ps.setString(2, name);",
    );
}

#[test]
fn kotlin_jdbc_optional_param_produces_matching_marker_and_bind_counts() {
    let query = one_param_query("UPDATE t SET touched = true WHERE name = $1", vec!["name".to_string()]);
    let code = query_fn("kotlin-jdbc", &query);
    assert_contains(
        "kotlin-jdbc",
        &code,
        "prepareStatement(\"UPDATE t SET touched = true WHERE (? IS NULL OR name = ?)\")",
    );
    assert_contains(
        "kotlin-jdbc",
        &code,
        "ps.setString(1, name)\n        ps.setString(2, name)",
    );
}

#[test]
fn kotlin_exposed_optional_param_produces_matching_marker_and_bind_counts() {
    let query = one_param_query("UPDATE t SET touched = true WHERE name = $1", vec!["name".to_string()]);
    let code = query_fn("kotlin-exposed", &query);
    assert_contains(
        "kotlin-exposed",
        &code,
        "exec(\"UPDATE t SET touched = true WHERE (? IS NULL OR name = ?)\", listOf(TextColumnType() to name, TextColumnType() to name))",
    );
}

#[test]
fn php_amphp_optional_param_produces_matching_marker_and_bind_counts() {
    let query = one_param_query("UPDATE t SET touched = true WHERE name = $1", vec!["name".to_string()]);
    let code = query_fn("php-amphp", &query);
    assert_contains(
        "php-amphp",
        &code,
        "$pool->prepare('UPDATE t SET touched = true WHERE (? IS NULL OR name = ?)')->execute([$name, $name]);",
    );
}
