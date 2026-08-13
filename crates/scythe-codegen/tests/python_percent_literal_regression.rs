//! Regression test for GH #201: a literal `%` in SQL text breaks `execute()`
//! on Python's `%`-paramstyle DB-API drivers. Both psycopg3 (`%(name)s`) and
//! aiomysql (`%s`) hand the *entire* SQL string to the driver's own
//! `%`-style formatting at execute time -- not just the placeholder tokens
//! the generator emits -- so an unescaped `%` in a `LIKE '100%'` pattern is
//! read as the start of a conversion specifier and raises (or, on aiomysql,
//! can silently mis-bind) instead of reaching the database as literal text.
//!
//! Also covers the aiomysql half of #153 item 2, closed without being fixed:
//! a trailing `.replace('?', "%s")` used to run *after* the span-aware
//! `rewrite_pg_placeholders` pass and blindly rewrote every remaining `?`,
//! including one sitting inside a SQL string literal (`'really?'` became
//! `'really%s'`).
//!
//! Every case drives the real parse -> analyze -> codegen pipeline (never
//! `AnalyzedQuery::build`), matching the pattern in
//! `python_named_placeholder_regression.rs`, so each test exercises exactly
//! what a user's `scythe.toml` run produces.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const PG_SCHEMA: &str = "CREATE TABLE items (id SERIAL PRIMARY KEY, label TEXT NOT NULL, note TEXT NOT NULL);";
const MYSQL_SCHEMA: &str =
    "CREATE TABLE items (id INT PRIMARY KEY, label VARCHAR(255) NOT NULL, note VARCHAR(255) NOT NULL);";

/// Parse, analyze, and generate the query fn for `backend_name`/`engine`
/// against `schema`/`query`, driven through the real pipeline end to end.
fn generate_query_fn(backend_name: &str, engine: &str, dialect: &SqlDialect, schema: &str, query: &str) -> String {
    let backend = get_backend(backend_name, engine).expect("backend must support engine");
    let catalog = Catalog::from_ddl_with_dialect(&[schema], dialect).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, dialect).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.query_fn.expect("expected a query fn")
}

// --- psycopg3 (%(name)s paramstyle) -----------------------------------

#[test]
fn psycopg3_doubles_a_literal_percent_in_a_like_pattern() {
    const QUERY: &str = "-- @name FindItem\n-- @returns :many\n\
        SELECT id, label, note FROM items WHERE label = $1 AND note LIKE '100%';";
    let query_fn = generate_query_fn(
        "python-psycopg3",
        "postgresql",
        &SqlDialect::PostgreSQL,
        PG_SCHEMA,
        QUERY,
    );

    assert!(
        query_fn.contains("note LIKE '100%%'"),
        "a literal % must be doubled for psycopg's %-style execute-time formatting, got:\n{query_fn}"
    );
}

#[test]
fn psycopg3_emitted_named_placeholder_is_not_doubled() {
    // Deliberately does not assert on a specific placeholder spelling like
    // `%(label)s` -- this test is about doubling order, not param-name
    // inference, so it only needs to know *a* named placeholder was emitted
    // and that it was not doubled.
    const QUERY: &str = "-- @name FindItem\n-- @returns :many\n\
        SELECT id, label, note FROM items WHERE label = $1 AND note LIKE '100%';";
    let query_fn = generate_query_fn(
        "python-psycopg3",
        "postgresql",
        &SqlDialect::PostgreSQL,
        PG_SCHEMA,
        QUERY,
    );

    assert!(
        query_fn.contains("%("),
        "expected at least one emitted named placeholder, got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("%%("),
        "a placeholder the generator itself just emitted must never be doubled -- doubling must run \
         before placeholder rewriting, not after; got:\n{query_fn}"
    );
}

// --- aiomysql (%s paramstyle) -------------------------------------------

#[test]
fn aiomysql_doubles_a_literal_percent_in_a_like_pattern() {
    const QUERY: &str = "-- @name FindItem\n-- @returns :many\n\
        SELECT id, label, note FROM items WHERE label = ? AND note LIKE '100%';";
    let query_fn = generate_query_fn("python-aiomysql", "mysql", &SqlDialect::MySQL, MYSQL_SCHEMA, QUERY);

    assert!(
        query_fn.contains("note LIKE '100%%'"),
        "a literal % must be doubled for aiomysql's %-style execute-time formatting, got:\n{query_fn}"
    );
}

#[test]
fn aiomysql_emitted_positional_placeholder_is_not_doubled() {
    const QUERY: &str = "-- @name FindItem\n-- @returns :many\n\
        SELECT id, label, note FROM items WHERE label = ? AND note LIKE '100%';";
    let query_fn = generate_query_fn("python-aiomysql", "mysql", &SqlDialect::MySQL, MYSQL_SCHEMA, QUERY);

    assert!(
        query_fn.contains("cur.execute("),
        "sanity: expected cur.execute to wrap the SQL, got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("%%s"),
        "the emitted %s placeholder (from label = ?) must never be doubled, got:\n{query_fn}"
    );
}

/// Direct regression test for the aiomysql half of #153 item 2.
#[test]
fn aiomysql_preserves_a_question_mark_inside_a_string_literal() {
    const QUERY: &str = "-- @name FindReally\n-- @returns :many\n\
        SELECT id, label, note FROM items WHERE label = ? AND note = 'really?';";
    let query_fn = generate_query_fn("python-aiomysql", "mysql", &SqlDialect::MySQL, MYSQL_SCHEMA, QUERY);

    assert!(
        query_fn.contains("'really?'"),
        "a ? inside a string literal must survive unchanged, not be rewritten as a placeholder, got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("'really%s'"),
        "the literal ? must never be blindly rewritten into a %s placeholder, got:\n{query_fn}"
    );
}

// --- The parameterless case, which the first cut of the #201 fix got wrong ---

/// Both drivers run their `%`-formatting pass only from `execute(query, params)`.
/// psycopg3 documents `%` as not special when no parameters are passed, and PyMySQL
/// never reaches `query % escaped_args` with `args=None`. The generated code emits a
/// bare `execute(query)` for a query that binds nothing, so doubling there is not a
/// harmless no-op: the doubled text reaches the server verbatim and `LIKE '100%%'`
/// matches a literal percent sign instead of acting as a wildcard.
///
/// That is the same class of silent wrong answer #201 is about, merely inverted, so
/// it gets its own guard rather than riding on the doubling tests above.
#[test]
fn psycopg3_leaves_a_literal_percent_alone_when_the_query_binds_nothing() {
    const QUERY: &str = "-- @name AllHundreds\n-- @returns :many\n\
        SELECT id, label, note FROM items WHERE note LIKE '100%';";
    let query_fn = generate_query_fn(
        "python-psycopg3",
        "postgresql",
        &SqlDialect::PostgreSQL,
        PG_SCHEMA,
        QUERY,
    );

    assert!(
        query_fn.contains("note LIKE '100%'"),
        "a parameterless query is executed as `execute(query)`, where psycopg does no \
         %-formatting -- doubling would send a literal %% to the server, got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("'100%%'"),
        "the % must not be doubled for a query that binds no parameters, got:\n{query_fn}"
    );
}

#[test]
fn aiomysql_leaves_a_literal_percent_alone_when_the_query_binds_nothing() {
    const QUERY: &str = "-- @name AllHundreds\n-- @returns :many\n\
        SELECT id, label, note FROM items WHERE note LIKE '100%';";
    let query_fn = generate_query_fn("python-aiomysql", "mysql", &SqlDialect::MySQL, MYSQL_SCHEMA, QUERY);

    assert!(
        query_fn.contains("note LIKE '100%'"),
        "PyMySQL only formats from `execute(query, args)`; the parameterless branch emits \
         `execute(query)`, so doubling would reach the server verbatim, got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("'100%%'"),
        "the % must not be doubled for a query that binds no parameters, got:\n{query_fn}"
    );
}
