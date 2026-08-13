//! Regression tests for board #192 and board #193: every python backend's
//! `:one` contract and `:execrows` return type.
//!
//! Before this fix, every python backend (`python-psycopg3`, `python-asyncpg`,
//! `python-aiomysql`, `python-aiosqlite`, `python-duckdb`, `python-pyodbc`,
//! `python-oracledb`, `python-snowflake`) matched `QueryCommand::One |
//! QueryCommand::Opt` in one shared arm and emitted the `:opt` shape
//! (`-> {Struct} | None:` / `if row is None: return None`) for both commands.
//! `:opt`'s own output was correct, but `:one` -- "exactly one row, error if
//! absent" -- silently inherited `:opt`'s permissiveness and returned `None`
//! instead of raising on a missing row. That is a silent wrong answer in the
//! caller's happy path: `crates/scythe-codegen/tests/opt_command_regression.rs`
//! documents every python backend as `KNOWN_UNDIFFERENTIATED_BACKENDS` for
//! exactly this reason, and its own census only checks that `:one` and `:opt`
//! render *different* code, not which behaviour each renders. This file
//! checks the behaviour.
//!
//! `:one` now returns the bare, non-optional struct type and raises
//! `ScytheNoRowsError` (defined once per generated module by
//! `python_common::no_rows_exception_def`) when the row is missing; `:opt`
//! keeps its pre-existing shape unchanged.
//!
//! Separately, board #193: `python-snowflake`'s `:execrows` path declared
//! `-> int` but returned `cur.rowcount` directly, which
//! `snowflake-connector-python` types `int | None` (every other python
//! backend's driver types `rowcount`/the equivalent plain `int`) -- a real
//! pyrefly-caught type mismatch, fixed here by narrowing with `or 0` rather
//! than widening the other seven backends' `int` contract.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery};
use scythe_core::parser::QueryCommand;

/// A minimal single-column, param-free, non-`RETURNING` query -- the same
/// shape `opt_command_regression.rs::one_column_query` uses, duplicated here
/// rather than imported: each file under `tests/` is its own compiled crate,
/// so there is no shared module to import a private helper from without
/// introducing one (the same reasoning `opt_command_regression.rs` gives for
/// duplicating `ALL_BACKEND_NAMES` instead of reaching into `lib.rs`).
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

/// The exact text every python backend's `:one` arm raises for
/// [`one_column_query`]'s `"GetItem"` query name -- shared verbatim across
/// backends because `python_common::write_missing_row_guard` (`pub(crate)`,
/// not reachable from this external test crate) is the single place that
/// formats it.
const EXPECTED_RAISE: &str = "raise ScytheNoRowsError(\"GetItem: no rows returned\")";

/// One entry per python backend this crate registers under `get_backend`:
/// the backend name, the one engine it needs to resolve a manifest for
/// [`one_column_query`], and the exact `:execrows` return line its
/// `QueryCommand::ExecResult | QueryCommand::ExecRows` arm emits (see the
/// file-level doc comment for board #193 -- every backend but
/// `python-snowflake` already declared and returned a plain `int`, so only
/// `python-snowflake`'s line carries the `or 0` narrowing this fix added).
struct PythonBackend {
    name: &'static str,
    engine: &'static str,
    execrows_return_line: &'static str,
}

const PYTHON_BACKENDS: &[PythonBackend] = &[
    PythonBackend {
        name: "python-psycopg3",
        engine: "postgresql",
        execrows_return_line: "    return cur.rowcount",
    },
    PythonBackend {
        name: "python-asyncpg",
        engine: "postgresql",
        execrows_return_line: "    return int(result.split()[-1])",
    },
    PythonBackend {
        name: "python-aiomysql",
        engine: "mysql",
        execrows_return_line: "        return cur.rowcount",
    },
    PythonBackend {
        name: "python-aiosqlite",
        engine: "sqlite",
        execrows_return_line: "    return cursor.rowcount",
    },
    PythonBackend {
        name: "python-duckdb",
        engine: "duckdb",
        execrows_return_line: "    return row[0] if row else 0",
    },
    PythonBackend {
        name: "python-pyodbc",
        engine: "mssql",
        execrows_return_line: "    return cursor.rowcount",
    },
    PythonBackend {
        name: "python-oracledb",
        engine: "oracle",
        execrows_return_line: "        return cur.rowcount",
    },
    PythonBackend {
        name: "python-snowflake",
        engine: "snowflake",
        execrows_return_line: "    return cur.rowcount or 0",
    },
];

/// `:one` must declare a non-optional return (no `| None` anywhere in the
/// signature) and must raise [`EXPECTED_RAISE`] on a missing row, for every
/// python backend -- not just the pre-fix reference case,
/// `python-psycopg3`.
#[test]
fn python_one_declares_non_optional_return_and_raises_on_missing_row() {
    let mut failures = Vec::new();
    for backend in PYTHON_BACKENDS {
        let one_fn = query_fn_for(backend.name, backend.engine, QueryCommand::One);
        if one_fn.contains("| None") {
            failures.push(format!(
                "{}: :one return annotation still contains `| None`; got:\n{one_fn}",
                backend.name
            ));
        }
        if !one_fn.contains(EXPECTED_RAISE) {
            failures.push(format!(
                "{}: :one does not raise `{EXPECTED_RAISE}` on a missing row; got:\n{one_fn}",
                backend.name
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} python backend(s) failed the :one contract:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

/// `:opt` must keep its pre-existing optional shape (`| None` in the
/// signature, `return None` on a missing row) and must never raise, for
/// every python backend.
#[test]
fn python_opt_keeps_optional_return_and_never_raises() {
    let mut failures = Vec::new();
    for backend in PYTHON_BACKENDS {
        let opt_fn = query_fn_for(backend.name, backend.engine, QueryCommand::Opt);
        if !opt_fn.contains("| None:") {
            failures.push(format!(
                "{}: :opt return annotation lost its `| None:` optional shape; got:\n{opt_fn}",
                backend.name
            ));
        }
        if !opt_fn.contains("return None") {
            failures.push(format!(
                "{}: :opt no longer returns None on a missing row; got:\n{opt_fn}",
                backend.name
            ));
        }
        if opt_fn.contains("ScytheNoRowsError") {
            failures.push(format!(
                "{}: :opt must never raise ScytheNoRowsError -- that is :one's contract; got:\n{opt_fn}",
                backend.name
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} python backend(s) failed the :opt contract:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}

/// `:one` and `:opt` must render different code for every python backend --
/// the direct regression check for the fold GH #197 and board #192 describe
/// (`opt_command_regression.rs` lists all eight as
/// `KNOWN_UNDIFFERENTIATED_BACKENDS`; this fix is what makes every one of
/// those entries stale).
#[test]
fn python_one_and_opt_render_different_code() {
    let mut failures = Vec::new();
    for backend in PYTHON_BACKENDS {
        let one_fn = query_fn_for(backend.name, backend.engine, QueryCommand::One);
        let opt_fn = query_fn_for(backend.name, backend.engine, QueryCommand::Opt);
        if one_fn == opt_fn {
            failures.push(backend.name);
        }
    }
    assert!(
        failures.is_empty(),
        "these python backends still render identical code for :one and :opt: {failures:?}"
    );
}

/// Board #193: every python backend's `:execrows` arm must return the exact
/// line [`PythonBackend::execrows_return_line`] names -- `int`-typed on
/// every backend, `python-snowflake` narrowed with `or 0` because
/// `snowflake-connector-python` is the one driver among these eight whose
/// `Cursor.rowcount` is typed `int | None`.
#[test]
fn python_execrows_returns_the_expected_line() {
    let mut failures = Vec::new();
    for backend in PYTHON_BACKENDS {
        let exec_fn = query_fn_for(backend.name, backend.engine, QueryCommand::ExecRows);
        if !exec_fn.contains(backend.execrows_return_line) {
            failures.push(format!(
                "{}: expected :execrows to contain `{}`; got:\n{exec_fn}",
                backend.name, backend.execrows_return_line
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} python backend(s) failed the :execrows return-line check:\n{}",
        failures.len(),
        failures.join("\n---\n")
    );
}
