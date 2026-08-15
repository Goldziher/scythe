//! Regression tests for board #192: every TypeScript/JavaScript backend's
//! `QueryCommand::One | QueryCommand::Opt` match arm rendered byte-identical
//! code -- the permissive `:opt` shape (`{Struct} | null` return type, `null`
//! result on a missing row). `:opt` is correct under that shape; `:one` means
//! "exactly one row, error if absent" and silently returned `null` instead,
//! exactly the shape `opt_command_regression.rs`'s `KNOWN_UNDIFFERENTIATED_BACKENDS`
//! lists every `typescript-*`/`javascript-*` backend under.
//!
//! The fix (`crates/scythe-codegen/src/backends/typescript_common.rs`,
//! `crates/scythe-codegen/src/backends/typescript_*.rs`) splits every such arm
//! into `QueryCommand::One` (non-nullable return, throws a plain `Error` naming
//! the query on a missing row) and `QueryCommand::Opt` (unchanged: nullable
//! return, `null` on a missing row). This file asserts that split held for
//! every backend in the family, with exact expected substrings rather than
//! truthiness -- see `crates/scythe-codegen/src/backends/typescript_common.rs`'s
//! `ts_row_not_found_throw` for the exact throw text every backend shares.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery};
use scythe_core::parser::QueryCommand;

/// A minimal single-column, param-free, non-`RETURNING` query -- same shape
/// as `opt_command_regression.rs`'s `one_column_query`, so both files exercise
/// exactly the branch under test without any RETURNING-clause, enum, or
/// composite special case needing a per-backend fixture. The column is
/// `NOT NULL` so " | null" cannot appear anywhere in the generated body
/// except in `:opt`'s own nullable annotation -- what makes the " | null"
/// substring check below exact rather than incidental.
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

/// Every `typescript-*`/`javascript-*` backend name registered in
/// `get_backend`, paired with an engine it supports -- the same fifteen names
/// `opt_command_regression.rs`'s `KNOWN_UNDIFFERENTIATED_BACKENDS` lists under
/// the shared-arm reason this fix removes. Kept as an independent copy for
/// the same reason that file gives for its own `ALL_BACKEND_NAMES`: the
/// authoritative list lives in another agent's owned `lib.rs`.
const TS_JS_BACKENDS: &[(&str, &str)] = &[
    ("typescript-postgres", "postgresql"),
    ("javascript-postgres", "postgresql"),
    ("typescript-pg", "postgresql"),
    ("javascript-pg", "postgresql"),
    ("typescript-mysql2", "mysql"),
    ("javascript-mysql2", "mysql"),
    ("typescript-better-sqlite3", "sqlite"),
    ("javascript-better-sqlite3", "sqlite"),
    ("typescript-duckdb", "duckdb"),
    ("typescript-node-sqlite", "sqlite"),
    ("javascript-node-sqlite", "sqlite"),
    ("typescript-wasm-sqlite", "sqlite"),
    ("javascript-wasm-sqlite", "sqlite"),
    ("typescript-kysely", "postgresql"),
    ("typescript-mssql", "mssql"),
    ("typescript-oracledb", "oracle"),
    ("typescript-snowflake", "snowflake"),
    ("javascript-snowflake", "snowflake"),
];

/// The exact text `ts_row_not_found_throw("GetItem")` renders -- pinned here
/// independently of that function so a change to either drifts loudly rather
/// than the test quietly re-deriving whatever the source currently does.
const EXPECTED_THROW: &str = "throw new Error(\"no row found for query: GetItem\");";

/// #192, split across the family: `:one` never renders " | null" and always
/// renders the exact throw text; `:opt` always renders " | null" and never
/// renders the throw text (or any fragment of it). Both are exact substring
/// checks, not truthiness -- `" | null"` cannot appear in this fixture's
/// output except in a nullable return-type annotation, since the query's one
/// column is `NOT NULL` and no other type in the generated body is optional.
#[test]
fn one_never_renders_nullable_and_always_throws_on_a_missing_row() {
    let mut failures = Vec::new();

    for &(name, engine) in TS_JS_BACKENDS {
        let one_fn = query_fn_for(name, engine, QueryCommand::One);

        if one_fn.contains(" | null") {
            failures.push(format!(
                "{name}: `:one` must not render a nullable return type; got:\n{one_fn}"
            ));
        }
        if !one_fn.contains(EXPECTED_THROW) {
            failures.push(format!(
                "{name}: `:one` must throw exactly `{EXPECTED_THROW}` on a missing row; got:\n{one_fn}"
            ));
        }
        if one_fn.contains("?? null") || one_fn.contains("return null;") {
            failures.push(format!(
                "{name}: `:one` must not silently return null on a missing row; got:\n{one_fn}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} backends violate the `:one` contract (#192):\n{}",
        failures.len(),
        TS_JS_BACKENDS.len(),
        failures.join("\n\n")
    );
}

/// The `:opt` half of the same split: unchanged from before #192 -- still
/// nullable, still returns `null` rather than throwing, on every backend.
#[test]
fn opt_still_renders_nullable_and_never_throws_on_a_missing_row() {
    let mut failures = Vec::new();

    for &(name, engine) in TS_JS_BACKENDS {
        let opt_fn = query_fn_for(name, engine, QueryCommand::Opt);

        if !opt_fn.contains(" | null") {
            failures.push(format!(
                "{name}: `:opt` must keep rendering a nullable return type; got:\n{opt_fn}"
            ));
        }
        if opt_fn.contains("no row found for query") {
            failures.push(format!(
                "{name}: `:opt` must not throw on a missing row; got:\n{opt_fn}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} backends violate the `:opt` contract (#192 must not have touched it):\n{}",
        failures.len(),
        TS_JS_BACKENDS.len(),
        failures.join("\n\n")
    );
}

/// The regression itself, phrased the way #192 (and #197's census before it)
/// named it: before this fix every one of these backends rendered
/// byte-identical code for `:one` and `:opt`.
#[test]
fn one_and_opt_render_different_code_for_every_backend() {
    let mut failures = Vec::new();

    for &(name, engine) in TS_JS_BACKENDS {
        let one_fn = query_fn_for(name, engine, QueryCommand::One);
        let opt_fn = query_fn_for(name, engine, QueryCommand::Opt);
        if one_fn == opt_fn {
            failures.push(name.to_string());
        }
    }

    assert!(
        failures.is_empty(),
        "{} backends still render identical code for :one and :opt: {:?}",
        failures.len(),
        failures
    );
}
