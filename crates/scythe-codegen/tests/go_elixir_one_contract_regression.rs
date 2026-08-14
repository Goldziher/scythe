//! Regression tests for board #192, go-* and elixir-* families: `:one` must
//! error on a missing row; `:opt` must not.
//!
//! Before this fix every go-*.rs and elixir-*.rs backend folded
//! `QueryCommand::One` and `QueryCommand::Opt` into a single
//! `QueryCommand::One | QueryCommand::Opt` match arm, and which contract won
//! depended on the backend:
//!
//! - `go-database-sql`, `go-pgx`, `go-gosnowflake`: the shared arm used
//!   `db.QueryRowContext(...).Scan(...)` and returned `(r, err)` unmodified.
//!   That already gives `:one` its "error if absent" contract for free (the
//!   driver's `sql.ErrNoRows`/`pgx.ErrNoRows` propagates through `err`), but
//!   `:opt` inherited the same propagation and returned an error on the exact
//!   case it exists to hand back as an absent value instead.
//! - `go-godror`: the shared arm's non-`RETURNING` path did the opposite --
//!   it special-cased `err == sql.ErrNoRows` into `return nil, nil` for
//!   *both* commands, so `:opt`'s own output was correct but `:one` never
//!   errored on a missing row.
//! - All six `elixir-*.rs` backends: the shared arm already rendered
//!   `{:ok, %Struct{}}` on a row and `{:error, :not_found}` on an empty
//!   result -- correct for `:one`, but `:opt` inherited the same
//!   `{:error, :not_found}` instead of the permissive `{:ok, nil}` its own
//!   contract calls for.
//!
//! Every backend below is fixed to split that arm; each assertion here pins
//! the exact fragment the split produces, not mere presence/absence of an
//! error.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery};
use scythe_core::parser::QueryCommand;

/// A minimal single-column, param-free, non-`RETURNING` query -- exercises
/// only the `QueryCommand::One`/`QueryCommand::Opt` branch every backend's
/// `generate_query_fn` has, without touching RETURNING-clause or composite
/// special cases that would need per-backend fixtures.
fn one_column_query(command: QueryCommand) -> AnalyzedQuery {
    AnalyzedQuery::build(|query| {
        query.name = "GetItem".to_string();
        query.command = command;
        query.sql = "SELECT value FROM t WHERE id = 1".to_string();
        query.columns = vec![AnalyzedColumn {
            name: "value".to_string(),
            neutral_type: "string".to_string(),
            nullable: false,
            sql_type: "string".to_string(),
            ..Default::default()
        }];
    })
}

fn query_fn_for(backend_name: &str, engine: &str, command: QueryCommand) -> String {
    let backend = get_backend(backend_name, engine)
        .unwrap_or_else(|error| panic!("{backend_name}/{engine}: backend must support engine: {error}"));
    let query = one_column_query(command.clone());
    let generated = generate_with_backend(&query, &*backend)
        .unwrap_or_else(|error| panic!("{backend_name}/{engine}: codegen failed for {command:?}: {error}"));
    generated
        .query_fn
        .unwrap_or_else(|| panic!("{backend_name}/{engine}: {command:?} produced no query fn"))
}

// ---------------------------------------------------------------------
// go-database-sql / go-gosnowflake: identical shape, both driven by
// database/sql's sql.ErrNoRows.
// ---------------------------------------------------------------------

fn assert_go_database_sql_shape_one_errors_opt_does_not(backend_name: &str, engine: &str) {
    let one_fn = query_fn_for(backend_name, engine, QueryCommand::One);
    assert!(
        one_fn.contains("func GetItem(ctx context.Context, db *sql.DB) (GetItemRow, error) {"),
        "{backend_name}: :one must return the bare row type, not a pointer; got:\n{one_fn}"
    );
    assert!(
        one_fn.contains("err := row.Scan(&r.Value)") && one_fn.contains("return r, err"),
        "{backend_name}: :one must propagate Scan's err (sql.ErrNoRows on a missing row) \
         unmodified; got:\n{one_fn}"
    );
    assert!(
        !one_fn.contains("ErrNoRows"),
        "{backend_name}: :one must not swallow sql.ErrNoRows -- a missing row must surface as \
         an error, not (zero value, nil); got:\n{one_fn}"
    );

    let opt_fn = query_fn_for(backend_name, engine, QueryCommand::Opt);
    assert!(
        opt_fn.contains("func GetItem(ctx context.Context, db *sql.DB) (*GetItemRow, error) {"),
        "{backend_name}: :opt must return a pointer row type so absence is representable as \
         nil; got:\n{opt_fn}"
    );
    assert!(
        opt_fn.contains("if err == sql.ErrNoRows {") && opt_fn.contains("return nil, nil"),
        "{backend_name}: :opt must swallow sql.ErrNoRows into (nil, nil); got:\n{opt_fn}"
    );
    assert!(
        opt_fn.contains("return &r, nil"),
        "{backend_name}: :opt must return a pointer to the populated row on success; got:\n{opt_fn}"
    );
}

#[test]
fn go_database_sql_one_errors_on_missing_row_opt_returns_nil() {
    assert_go_database_sql_shape_one_errors_opt_does_not("go-database-sql", "mysql");
}

#[test]
fn go_gosnowflake_one_errors_on_missing_row_opt_returns_nil() {
    assert_go_database_sql_shape_one_errors_opt_does_not("go-gosnowflake", "snowflake");
}

// ---------------------------------------------------------------------
// go-pgx: same shape, driven by pgx.ErrNoRows instead of sql.ErrNoRows.
// ---------------------------------------------------------------------

#[test]
fn go_pgx_one_errors_on_missing_row_opt_returns_nil() {
    let one_fn = query_fn_for("go-pgx", "postgresql", QueryCommand::One);
    assert!(
        one_fn.contains("func GetItem(ctx context.Context, db *pgxpool.Pool) (GetItemRow, error) {"),
        "go-pgx: :one must return the bare row type, not a pointer; got:\n{one_fn}"
    );
    assert!(
        one_fn.contains("err := row.Scan(&r.Value)") && one_fn.contains("return r, err"),
        "go-pgx: :one must propagate Scan's err (pgx.ErrNoRows on a missing row) unmodified; \
         got:\n{one_fn}"
    );
    assert!(
        !one_fn.contains("ErrNoRows"),
        "go-pgx: :one must not swallow pgx.ErrNoRows -- a missing row must surface as an \
         error; got:\n{one_fn}"
    );

    let opt_fn = query_fn_for("go-pgx", "postgresql", QueryCommand::Opt);
    assert!(
        opt_fn.contains("func GetItem(ctx context.Context, db *pgxpool.Pool) (*GetItemRow, error) {"),
        "go-pgx: :opt must return a pointer row type so absence is representable as nil; \
         got:\n{opt_fn}"
    );
    assert!(
        opt_fn.contains("if err == pgx.ErrNoRows {") && opt_fn.contains("return nil, nil"),
        "go-pgx: :opt must swallow pgx.ErrNoRows into (nil, nil); got:\n{opt_fn}"
    );
    assert!(
        opt_fn.contains("return &r, nil"),
        "go-pgx: :opt must return a pointer to the populated row on success; got:\n{opt_fn}"
    );
}

/// A file with a `:opt` query needs the bare `pgx` package import (for
/// `pgx.ErrNoRows`) alongside `pgxpool` -- omitting it makes the generated
/// file fail to compile with "undefined: pgx" (the #198 failure mode, for a
/// different import).
#[test]
fn go_pgx_file_header_adds_pgx_import_only_when_opt_query_present() {
    let backend = get_backend("go-pgx", "postgresql").unwrap();

    let opt_query = one_column_query(QueryCommand::Opt);
    let opt_code = generate_with_backend(&opt_query, &*backend).unwrap();
    let header_with_opt = backend.file_header_for_results(&[opt_code]);
    assert!(
        header_with_opt.contains("\"github.com/jackc/pgx/v5\""),
        "a :opt query's file header must import the bare pgx package for pgx.ErrNoRows; \
         got:\n{header_with_opt}"
    );

    let one_query = one_column_query(QueryCommand::One);
    let one_code = generate_with_backend(&one_query, &*backend).unwrap();
    let header_without_opt = backend.file_header_for_results(&[one_code]);
    assert!(
        !header_without_opt.contains("\"github.com/jackc/pgx/v5\""),
        "a file with no :opt query must not import the bare pgx package, or `go build` fails \
         with \"imported and not used\"; got:\n{header_without_opt}"
    );
}

// ---------------------------------------------------------------------
// go-godror: non-RETURNING path already returns a pointer for both commands;
// only the ErrNoRows-swallow differs.
// ---------------------------------------------------------------------

#[test]
fn go_godror_one_errors_on_missing_row_opt_returns_nil() {
    let one_fn = query_fn_for("go-godror", "oracle", QueryCommand::One);
    assert!(
        one_fn.contains("func GetItem(ctx context.Context, db *sql.DB) (*GetItemRow, error) {"),
        "go-godror: :one keeps its established pointer return type; got:\n{one_fn}"
    );
    assert!(
        one_fn.contains("if err := row.Scan(&item.Value); err != nil {") && one_fn.contains("return nil, err"),
        "go-godror: :one must propagate Scan's err; got:\n{one_fn}"
    );
    assert!(
        !one_fn.contains("ErrNoRows"),
        "go-godror: :one must not swallow sql.ErrNoRows into (nil, nil) -- a missing row must \
         surface as an error; got:\n{one_fn}"
    );
    assert!(
        one_fn.contains("return &item, nil"),
        "go-godror: :one must return a pointer to the populated row on success; got:\n{one_fn}"
    );

    let opt_fn = query_fn_for("go-godror", "oracle", QueryCommand::Opt);
    assert!(
        opt_fn.contains("if err == sql.ErrNoRows {") && opt_fn.contains("return nil, nil"),
        "go-godror: :opt must keep swallowing sql.ErrNoRows into (nil, nil); got:\n{opt_fn}"
    );
}

// ---------------------------------------------------------------------
// elixir-postgrex / elixir-ecto / elixir-tds / elixir-jamdb: identical
// {:error, :not_found} vs {:ok, nil} shape, only the driver call differs.
// ---------------------------------------------------------------------

fn assert_elixir_tagged_tuple_shape_one_errors_opt_does_not(
    backend_name: &str,
    engine: &str,
    not_found_row_match: &str,
) {
    let one_fn = query_fn_for(backend_name, engine, QueryCommand::One);
    let one_not_found = format!("{not_found_row_match} -> {{:error, :not_found}}");
    assert!(
        one_fn.contains(&one_not_found),
        "{backend_name}: :one must error on a missing row via {{:error, :not_found}}; \
         got:\n{one_fn}"
    );
    assert!(
        one_fn.contains("{:error, :not_found} | {:error, term()}"),
        "{backend_name}: :one's @spec must declare the {{:error, :not_found}} it can return; \
         got:\n{one_fn}"
    );
    assert!(
        !one_fn.contains("{:ok, nil}"),
        "{backend_name}: :one must never return {{:ok, nil}} for an absent row; got:\n{one_fn}"
    );

    let opt_fn = query_fn_for(backend_name, engine, QueryCommand::Opt);
    let opt_not_found = format!("{not_found_row_match} -> {{:ok, nil}}");
    assert!(
        opt_fn.contains(&opt_not_found),
        "{backend_name}: :opt must return an absent row as {{:ok, nil}}, not an error; \
         got:\n{opt_fn}"
    );
    assert!(
        opt_fn.contains("{:ok, %GetItemRow{} | nil}"),
        "{backend_name}: :opt's @spec must declare the row as optional; got:\n{opt_fn}"
    );
    assert!(
        !opt_fn.contains(":not_found"),
        "{backend_name}: :opt must never mention :not_found -- absence is not an error for \
         :opt; got:\n{opt_fn}"
    );
}

#[test]
fn elixir_postgrex_one_errors_on_missing_row_opt_returns_ok_nil() {
    assert_elixir_tagged_tuple_shape_one_errors_opt_does_not("elixir-postgrex", "postgresql", "{:ok, %{rows: []}}");
}

#[test]
fn elixir_ecto_one_errors_on_missing_row_opt_returns_ok_nil() {
    assert_elixir_tagged_tuple_shape_one_errors_opt_does_not("elixir-ecto", "postgresql", "{:ok, %{rows: []}}");
}

#[test]
fn elixir_tds_one_errors_on_missing_row_opt_returns_ok_nil() {
    assert_elixir_tagged_tuple_shape_one_errors_opt_does_not("elixir-tds", "mssql", "{:ok, %{rows: []}}");
}

#[test]
fn elixir_jamdb_one_errors_on_missing_row_opt_returns_ok_nil() {
    // ~keep board #223: jamdb executes through `DBConnection.execute/3`, which returns
    // `{:ok, query, result}` -- not the two-element tuple `Jamdb.Oracle.query/3` returned.
    // The contract itself (`{:error, :not_found}` on a missing row) is unchanged; only the
    // shape it is matched out of moved.
    assert_elixir_tagged_tuple_shape_one_errors_opt_does_not("elixir-jamdb", "oracle", "{:ok, _query, %{rows: []}}");
}

// ---------------------------------------------------------------------
// elixir-myxql: same shape, but the driver's result struct is named
// MyXQL.Result, and (before this fix) the @spec never declared
// {:error, :not_found} even though the body already produced it for :one.
// ---------------------------------------------------------------------

#[test]
fn elixir_myxql_one_errors_on_missing_row_opt_returns_ok_nil() {
    assert_elixir_tagged_tuple_shape_one_errors_opt_does_not("elixir-myxql", "mysql", "{:ok, %MyXQL.Result{rows: []}}");
}

// ---------------------------------------------------------------------
// elixir-exqlite: same {:error, :not_found} vs {:ok, nil} split, but nested
// two lines deeper (`{:ok, []} ->` on its own line, the tagged result below
// it) because Exqlite.Sqlite3.fetch_all's empty-result shape is `{:ok, []}`,
// not a `%{rows: []}}` struct match.
// ---------------------------------------------------------------------

#[test]
fn elixir_exqlite_one_errors_on_missing_row_opt_returns_ok_nil() {
    let one_fn = query_fn_for("elixir-exqlite", "sqlite", QueryCommand::One);
    assert!(
        one_fn.contains("{:ok, []} ->") && one_fn.contains("{:error, :not_found}"),
        "elixir-exqlite: :one must error on a missing row via {{:error, :not_found}}; \
         got:\n{one_fn}"
    );
    assert!(
        one_fn.contains("{:error, :not_found} | {:error, term()}"),
        "elixir-exqlite: :one's @spec must declare the {{:error, :not_found}} it can return; \
         got:\n{one_fn}"
    );
    assert!(
        !one_fn.contains("{:ok, nil}"),
        "elixir-exqlite: :one must never return {{:ok, nil}} for an absent row; got:\n{one_fn}"
    );

    let opt_fn = query_fn_for("elixir-exqlite", "sqlite", QueryCommand::Opt);
    assert!(
        opt_fn.contains("{:ok, []} ->") && opt_fn.contains("{:ok, nil}"),
        "elixir-exqlite: :opt must return an absent row as {{:ok, nil}}, not an error; \
         got:\n{opt_fn}"
    );
    assert!(
        opt_fn.contains("{:ok, %GetItemRow{} | nil}"),
        "elixir-exqlite: :opt's @spec must declare the row as optional; got:\n{opt_fn}"
    );
    assert!(
        !opt_fn.contains(":not_found"),
        "elixir-exqlite: :opt must never mention :not_found -- absence is not an error for \
         :opt; got:\n{opt_fn}"
    );
}

// ---------------------------------------------------------------------
// Census: :one and :opt must now render different code for every go-* and
// elixir-* backend -- the exact fold #192 named.
// ---------------------------------------------------------------------

#[test]
fn one_and_opt_render_different_code_for_every_go_and_elixir_backend() {
    let backends: &[(&str, &str)] = &[
        ("go-database-sql", "mysql"),
        ("go-pgx", "postgresql"),
        ("go-godror", "oracle"),
        ("go-gosnowflake", "snowflake"),
        ("elixir-postgrex", "postgresql"),
        ("elixir-ecto", "postgresql"),
        ("elixir-myxql", "mysql"),
        ("elixir-exqlite", "sqlite"),
        ("elixir-tds", "mssql"),
        ("elixir-jamdb", "oracle"),
    ];

    let mut regressions = Vec::new();
    for &(name, engine) in backends {
        let one_fn = query_fn_for(name, engine, QueryCommand::One);
        let opt_fn = query_fn_for(name, engine, QueryCommand::Opt);
        if one_fn == opt_fn {
            regressions.push(name);
        }
    }

    assert!(
        regressions.is_empty(),
        "these go-*/elixir-* backends still render identical code for :one and :opt: {regressions:?}"
    );
}
