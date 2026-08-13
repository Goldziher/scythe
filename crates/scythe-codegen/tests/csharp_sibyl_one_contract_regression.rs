//! Regression tests for board #192: on the six `csharp-*` backends and on
//! `rust-sibyl`, `:one` ("exactly one row, error if absent") and `:opt`
//! ("zero or one row") were folded into a single `QueryCommand::One |
//! QueryCommand::Opt` match arm that always implemented `:opt`'s contract --
//! nullable/`Option`-wrapped return, `null`/`None` on a missing row. `:opt`'s
//! own output was correct, but `:one` silently inherited that permissiveness
//! and never errored on a missing row: a caller's `-- @returns :one` query
//! could return a null/`None` result in its happy path with no signal that
//! anything went wrong.
//!
//! This mirrors `opt_command_regression.rs`'s `query_fn_for`/`one_column_query`
//! helpers (kept as an independent copy in this file rather than shared, same
//! rationale as that file's own copy of `ALL_BACKEND_NAMES`: it is cheaper to
//! duplicate ~20 lines than to reach into another owner's test file).
//!
//! `KNOWN_UNDIFFERENTIATED_BACKENDS` in `opt_command_regression.rs` lists all
//! seven backends this file covers as still folding `:one` into `:opt`; those
//! entries are now stale and must be deleted by whoever owns that file.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery};
use scythe_core::parser::QueryCommand;

/// A minimal single-column, param-free, non-`RETURNING` query: it exercises
/// only the `QueryCommand::One`/`QueryCommand::Opt` branch every backend's
/// `generate_query_fn` has. `analyzed.name = "GetItem"` produces the row
/// struct `GetItemRow` (`row_struct_name` applies the same `Row` suffix
/// across every backend in this file -- confirmed directly against this
/// exact query name in `opt_command_regression.rs`'s own
/// `rust_tiberius_opt_returns_option_and_does_not_panic_on_missing_row`).
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

/// The six `csharp-*` backends this file covers, paired with an engine each
/// supports.
const CSHARP_BACKENDS: &[(&str, &str)] = &[
    ("csharp-npgsql", "postgresql"),
    ("csharp-mysqlconnector", "mysql"),
    ("csharp-microsoft-sqlite", "sqlite"),
    ("csharp-sqlclient", "mssql"),
    ("csharp-oracle", "oracle"),
    ("csharp-snowflake", "snowflake"),
];

/// `:one` must return the bare, non-nullable record type and throw on a
/// missing row; `:opt` must keep the nullable record type and return `null`.
#[test]
fn csharp_backends_distinguish_one_from_opt() {
    for &(backend_name, engine) in CSHARP_BACKENDS {
        let one_fn = query_fn_for(backend_name, engine, QueryCommand::One);
        let opt_fn = query_fn_for(backend_name, engine, QueryCommand::Opt);

        assert!(
            one_fn.contains("Task<GetItemRow>"),
            "{backend_name}: :one must return the non-nullable record type; got:\n{one_fn}"
        );
        assert!(
            !one_fn.contains("Task<GetItemRow?>"),
            "{backend_name}: :one must not carry a nullable return-type annotation; got:\n{one_fn}"
        );
        assert!(
            one_fn.contains("throw new InvalidOperationException("),
            "{backend_name}: :one must throw when the row is missing; got:\n{one_fn}"
        );
        assert!(
            one_fn.contains("expected exactly one row but found none"),
            "{backend_name}: :one's exception must explain the missing-row contract; got:\n{one_fn}"
        );
        assert!(
            !one_fn.contains("return null;"),
            "{backend_name}: :one must never silently return null; got:\n{one_fn}"
        );

        assert!(
            opt_fn.contains("Task<GetItemRow?>"),
            "{backend_name}: :opt must keep the nullable record type; got:\n{opt_fn}"
        );
        assert!(
            opt_fn.contains("return null;"),
            "{backend_name}: :opt must still return null on a missing row; got:\n{opt_fn}"
        );
        assert!(
            !opt_fn.contains("throw new InvalidOperationException("),
            "{backend_name}: :opt must not throw on a missing row -- that is :one's contract, \
             not :opt's; got:\n{opt_fn}"
        );

        assert_ne!(
            one_fn, opt_fn,
            "{backend_name}: :one and :opt must render different code"
        );
    }
}

/// `:one` must return `sibyl::Result<Struct>` and return an `Err` on a
/// missing row; `:opt` must keep `sibyl::Result<Option<Struct>>` and `Ok(None)`.
#[test]
fn rust_sibyl_distinguishes_one_from_opt() {
    let one_fn = query_fn_for("rust-sibyl", "oracle", QueryCommand::One);
    let opt_fn = query_fn_for("rust-sibyl", "oracle", QueryCommand::Opt);

    assert!(
        one_fn.contains("sibyl::Result<GetItemRow>"),
        "rust-sibyl: :one must return the bare row type, not Option-wrapped; got:\n{one_fn}"
    );
    assert!(
        !one_fn.contains("sibyl::Result<Option<GetItemRow>>"),
        "rust-sibyl: :one must not carry an Option-wrapped return type; got:\n{one_fn}"
    );
    assert!(
        one_fn.contains("Err(sibyl::Error::Interface("),
        "rust-sibyl: :one must return an Err on a missing row; got:\n{one_fn}"
    );
    assert!(
        one_fn.contains("expected exactly one row but found none"),
        "rust-sibyl: :one's error must explain the missing-row contract; got:\n{one_fn}"
    );
    assert!(
        !one_fn.contains("Ok(None)"),
        "rust-sibyl: :one must never silently return Ok(None); got:\n{one_fn}"
    );

    assert!(
        opt_fn.contains("sibyl::Result<Option<GetItemRow>>"),
        "rust-sibyl: :opt must keep the Option-wrapped return type; got:\n{opt_fn}"
    );
    assert!(
        opt_fn.contains("Ok(None)"),
        "rust-sibyl: :opt must still return Ok(None) on a missing row; got:\n{opt_fn}"
    );
    assert!(
        !opt_fn.contains("Err(sibyl::Error::Interface("),
        "rust-sibyl: :opt must not error on a missing row -- that is :one's contract, not \
         :opt's; got:\n{opt_fn}"
    );

    assert_ne!(one_fn, opt_fn, "rust-sibyl: :one and :opt must render different code");
}
