//! End-to-end regression tests for #217: every file `typescript-duckdb`
//! produced failed to compile, for two independent reasons.
//!
//! 1. `import type { Connection } from "@duckdb/node-api"` -- the package
//!    exports no `Connection`. The class is `DuckDBConnection`. `tsc`:
//!    `TS2614: Module '"@duckdb/node-api"' has no exported member
//!    'Connection'`.
//! 2. `await stmt.run(id)` -- `DuckDBPreparedStatement.run()` takes zero
//!    arguments; values are bound beforehand with `bind`. `tsc`:
//!    `TS2554: Expected 0 arguments, but got 1`, once per parameterised
//!    query.
//!
//! Both were verified against the published `@duckdb/node-api` `.d.ts`
//! (v1.5.5) with real `tsc --strict`, which is also how the fix was checked.
//! That package cannot be a test dependency of a Rust crate, so what is
//! asserted here is the *shape* the driver's declaration file requires:
//! the exported type name, and that no `stmt.run(` call is ever handed an
//! argument.

use scythe_codegen::validation::{strict_mode_enabled, validate_with_tools};
use scythe_codegen::{CodegenBackend, GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TABLE users (\
    id INTEGER PRIMARY KEY, \
    name VARCHAR NOT NULL, \
    weight DOUBLE\
);\
CREATE TABLE orders (\
    id INTEGER PRIMARY KEY, \
    user_id INTEGER NOT NULL, \
    total DOUBLE NOT NULL\
);";

/// One query per command shape that emits a `run` call.
const QUERIES: [&str; 6] = [
    "-- @name GetUser\n-- @returns :one\nSELECT id, name, weight FROM users WHERE id = $1;",
    "-- @name ListUsers\n-- @returns :many\nSELECT id, name, weight FROM users;",
    "-- @name DeleteUser\n-- @returns :exec\nDELETE FROM users WHERE id = $1;",
    "-- @name PurgeUsers\n-- @returns :exec_rows\nDELETE FROM users WHERE name = $1;",
    "-- @name AddUsers\n-- @returns :batch\nINSERT INTO users (id, name) VALUES ($1, $2);",
    "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\n\
     SELECT u.id, u.name, o.total FROM users u JOIN orders o ON o.user_id = u.id WHERE u.id = $1;",
];

fn generate_file() -> String {
    generate_file_for(&QUERIES)
}

fn generate_file_for(queries: &[&str]) -> String {
    let backend: Box<dyn CodegenBackend> =
        get_backend("typescript-duckdb", "duckdb").expect("typescript-duckdb must support duckdb");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let codes: Vec<GeneratedCode> = queries
        .iter()
        .map(|sql| {
            let parsed = parse_query_with_dialect(sql, &SqlDialect::PostgreSQL).expect("query must parse");
            let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
            generate_with_backend(&analyzed, &*backend).expect("codegen must succeed")
        })
        .collect();

    let mut file = backend.file_header_for_results(&codes);
    file.push('\n');
    for code in &codes {
        for text in [&code.row_struct, &code.query_fn].into_iter().flatten() {
            file.push_str(text);
            file.push_str("\n\n");
        }
    }
    file
}

/// This must fail before the fix: the import named a type the package does
/// not export, so every generated file was `TS2614` on line 2.
#[test]
fn the_driver_import_names_a_type_the_package_actually_exports() {
    let file = generate_file();

    assert!(
        file.contains("import type { DuckDBConnection, DuckDBValue } from \"@duckdb/node-api\";"),
        "expected the real exported names; got:\n{file}"
    );
    assert!(
        !file.contains("{ Connection }"),
        "`Connection` is not exported by @duckdb/node-api (#217); got:\n{file}"
    );
    assert!(
        !file.contains("conn: Connection"),
        "the connection parameter must be typed DuckDBConnection (#217); got:\n{file}"
    );
    assert!(
        file.contains("conn: DuckDBConnection"),
        "expected a DuckDBConnection parameter; got:\n{file}"
    );
}

/// This must fail before the fix: `stmt.run(id)`, `stmt.run(name)`,
/// `stmt.run(item.id, item.name)` -- five `TS2554`s across the six queries
/// above. `run()` is nullary; `bind` is what takes the values.
#[test]
fn prepared_statements_bind_their_arguments_instead_of_passing_them_to_run() {
    let file = generate_file();

    for (index, _) in file.match_indices("stmt.run(") {
        let rest = &file[index + "stmt.run(".len()..];
        assert!(
            rest.starts_with(')'),
            "DuckDBPreparedStatement.run() takes no arguments (#217); found `stmt.run({}...`:\n{file}",
            &rest[..rest.len().min(30)]
        );
    }

    // The values still have to reach the statement.
    assert!(
        file.contains("stmt.bind([id] as DuckDBValue[]);"),
        "a single parameter must be bound; got:\n{file}"
    );
    assert!(
        file.contains("stmt.bind([item.id, item.name] as DuckDBValue[]);"),
        "batch items must be bound per iteration; got:\n{file}"
    );
    // A zero-parameter query binds nothing at all.
    assert!(
        file.contains(
            "const stmt = await conn.prepare(`SELECT id, name, weight FROM users`);\n\tconst result = await stmt.run();"
        ),
        "a parameterless query must go straight to run(); got:\n{file}"
    );
}

/// The batch loop must re-bind on every iteration, not once outside it --
/// otherwise every item would insert the first item's values.
#[test]
fn the_batch_loop_rebinds_inside_the_iteration() {
    let file = generate_file();

    let loop_start = file
        .find("for (const item of items) {")
        .expect("the batch query must emit a loop");
    let after = &file[loop_start..];
    let bind = after.find("stmt.bind(").expect("the loop must bind");
    let run = after.find("await stmt.run();").expect("the loop must run");
    assert!(bind < run, "bind must precede run inside the loop:\n{file}");
}

/// `DuckDBValue` is only referenced by the bind assertion, so a file whose
/// every query is parameterless must not import it -- an unused `import
/// type` is a lint finding on output that is meant to be clean.
#[test]
fn the_value_type_import_is_dropped_when_nothing_binds() {
    let parameterless = generate_file_for(&[QUERIES[1]]);

    assert!(
        parameterless.contains("import type { DuckDBConnection } from \"@duckdb/node-api\";"),
        "a parameterless file must not import DuckDBValue; got:\n{parameterless}"
    );
    assert!(!parameterless.contains("DuckDBValue"), "got:\n{parameterless}");

    // ...and is still there the moment one query binds.
    let with_params = generate_file_for(&[QUERIES[0]]);
    assert!(with_params.contains("DuckDBValue"), "got:\n{with_params}");
}

/// Additive: the repository's own TypeScript checker over the whole file.
#[test]
fn the_generated_duckdb_file_passes_tool_validation() {
    let file = generate_file();
    let validation = validate_with_tools(&file, "typescript-duckdb");
    assert!(
        validation.errors().is_empty(),
        "{:#?}\n\nfile:\n{file}",
        validation.errors()
    );
    if strict_mode_enabled() {
        assert!(
            validation.fully_checked(),
            "strict mode requires every checker to have run, got {:?} run / {:?} missing",
            validation.tools_run(),
            validation.missing_tools()
        );
    }
}
