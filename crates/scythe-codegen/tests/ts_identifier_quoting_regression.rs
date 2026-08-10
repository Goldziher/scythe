//! End-to-end regression tests for #215: a column name that is not a valid
//! TypeScript identifier (`with-dash`, `my col`, `1st`, `2fa`, `it's`) must
//! never reach a generated file in a position that requires one.
//!
//! There are three such positions, and the original fix only covered the
//! first:
//!
//! 1. a declared property key -- `interface R { my col: string }` is
//!    `TS1131`; `"my col": string` is fine;
//! 2. a property *read* off a driver row or a batch item -- `row.my col`
//!    and `item.2fa` do not parse either, and `row['it's']` closes its own
//!    string literal;
//! 3. a function parameter binding -- `function f(my col: string)`. This one
//!    has no quoted form and is **not** fixed here; see
//!    `a_scalar_parameter_named_after_a_non_identifier_column_is_still_broken`
//!    at the bottom of this file, which pins the gap so it cannot be
//!    mistaken for coverage.
//!
//! Exercised through the real parse -> analyze -> codegen pipeline. String
//! assertions are unconditional; the external checkers (`poly` via
//! `validate_with_tools`, and `tsc --strict` when it is on `PATH`) are
//! additive on top -- a machine without them still fails on a regression.

use std::collections::HashMap;
use std::process::Command;

use scythe_codegen::validation::{strict_mode_enabled, validate_with_tools};
use scythe_codegen::{CodegenBackend, GeneratedCode, generate_with_backend, get_backend, provenance};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// Every TypeScript backend, with an engine it supports and the dialect
/// fixtures below are written in.
const TS_BACKENDS: [(&str, &str, SqlDialect); 11] = [
    ("typescript-pg", "postgresql", SqlDialect::PostgreSQL),
    ("typescript-postgres", "postgresql", SqlDialect::PostgreSQL),
    ("typescript-kysely", "postgresql", SqlDialect::PostgreSQL),
    ("typescript-mysql2", "mysql", SqlDialect::MySQL),
    ("typescript-better-sqlite3", "sqlite", SqlDialect::SQLite),
    ("typescript-node-sqlite", "sqlite", SqlDialect::SQLite),
    ("typescript-wasm-sqlite", "sqlite", SqlDialect::SQLite),
    ("typescript-mssql", "mssql", SqlDialect::MsSql),
    ("typescript-oracledb", "oracle", SqlDialect::Oracle),
    ("typescript-snowflake", "snowflake", SqlDialect::Snowflake),
    // DuckDB has no `SqlDialect` variant of its own; it speaks PostgreSQL's
    // identifier and placeholder syntax, which is what the engine resolves to.
    ("typescript-duckdb", "duckdb", SqlDialect::PostgreSQL),
];

/// The identifier-quoting character(s) each dialect's parser accepts, and a
/// schema/query pair written with them.
fn fixture(dialect: &SqlDialect) -> (&'static str, [&'static str; 3]) {
    match dialect {
        SqlDialect::MySQL => (
            "CREATE TABLE weird (\
                id INT AUTO_INCREMENT PRIMARY KEY, \
                `with-dash` VARCHAR(50) NOT NULL, \
                `my col` VARCHAR(50) NOT NULL, \
                `2fa` VARCHAR(50) NOT NULL\
            );",
            [
                "-- @name FindWeird\n-- @returns :one\n\
                 SELECT id, `with-dash`, `my col`, `2fa` FROM weird WHERE id = ?;",
                "-- @name ListWeird\n-- @returns :many\n\
                 SELECT id, `with-dash`, `my col`, `2fa` FROM weird;",
                "-- @name AddWeird\n-- @returns :batch\n\
                 INSERT INTO weird (`with-dash`, `my col`, `2fa`) VALUES (?, ?, ?);",
            ],
        ),
        _ => (
            "CREATE TABLE weird (\
                id INT PRIMARY KEY, \
                \"with-dash\" VARCHAR(50) NOT NULL, \
                \"my col\" VARCHAR(50) NOT NULL, \
                \"2fa\" VARCHAR(50) NOT NULL\
            );",
            [
                "-- @name FindWeird\n-- @returns :one\n\
                 SELECT id, \"with-dash\", \"my col\", \"2fa\" FROM weird WHERE id = $1;",
                "-- @name ListWeird\n-- @returns :many\n\
                 SELECT id, \"with-dash\", \"my col\", \"2fa\" FROM weird;",
                "-- @name AddWeird\n-- @returns :batch\n\
                 INSERT INTO weird (\"with-dash\", \"my col\", \"2fa\") VALUES ($1, $2, $3);",
            ],
        ),
    }
}

/// The three keys every fixture projects, spelled as they must appear in the
/// generated file: quoted.
const QUOTED_KEYS: [&str; 3] = ["\"with-dash\"", "\"my col\"", "\"2fa\""];

/// The same three, spelled as a bare identifier -- what #215 emitted. Each
/// is checked with a trailing `:` so the search cannot match the quoted form
/// or an occurrence inside the SQL text.
const BARE_KEYS: [&str; 3] = ["with-dash:", "my col:", "2fa:"];

fn generate_all(backend: &dyn CodegenBackend, dialect: &SqlDialect) -> Vec<GeneratedCode> {
    let (schema, queries) = fixture(dialect);
    let catalog = Catalog::from_ddl_with_dialect(&[schema], dialect).expect("schema must parse");
    queries
        .iter()
        .map(|sql| {
            let parsed = parse_query_with_dialect(sql, dialect).expect("query must parse");
            let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
            generate_with_backend(&analyzed, backend).expect("codegen must succeed")
        })
        .collect()
}

/// Assemble the generated pieces into the file `scythe generate` would
/// write, so the checkers below see the same bytes.
fn assemble(backend: &dyn CodegenBackend, engine: &str, codes: &[GeneratedCode]) -> String {
    let mut body = backend.file_header_for_results(codes);
    body.push('\n');
    for code in codes {
        for text in [&code.row_struct, &code.query_fn].into_iter().flatten() {
            body.push_str(text);
            body.push_str("\n\n");
        }
    }
    provenance::assemble_file(
        &backend.file_preamble(),
        &provenance::header_line(
            backend,
            env!("CARGO_PKG_VERSION"),
            engine,
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ),
        &body,
    )
}

fn build(backend_name: &str, engine: &str, options: &HashMap<String, String>) -> Box<dyn CodegenBackend> {
    let mut backend =
        get_backend(backend_name, engine).unwrap_or_else(|e| panic!("{backend_name} must support {engine}: {e}"));
    backend.apply_options(options).expect("options must apply");
    backend
}

/// Run the repository's own TypeScript checker over `file`, and -- in strict
/// mode -- fail if it was not actually installed. Never a substitute for the
/// string assertions each caller makes first.
fn tool_check(backend_name: &str, file: &str) {
    let validation = validate_with_tools(file, backend_name);
    assert!(
        validation.errors().is_empty(),
        "{backend_name}: tool validation reported:\n{:#?}\n\nfile:\n{file}",
        validation.errors()
    );
    if strict_mode_enabled() {
        assert!(
            validation.fully_checked(),
            "{backend_name}: strict mode requires every checker to have run, got {:?} run / {:?} missing",
            validation.tools_run(),
            validation.missing_tools()
        );
    }
}

/// This must fail before the fix on every backend: a `:batch` params
/// interface spliced `p.field_name` in bare (`2fa: string;`), the per-item
/// bind read it back with `item.2fa`, and the pg/mysql2 row remaps read
/// driver rows with `row.my col`.
#[test]
fn every_typescript_backend_quotes_non_identifier_column_names() {
    for (backend_name, engine, dialect) in TS_BACKENDS {
        let backend = build(backend_name, engine, &HashMap::new());
        let codes = generate_all(&*backend, &dialect);
        let file = assemble(&*backend, engine, &codes);

        for quoted in QUOTED_KEYS {
            assert!(
                file.contains(&format!("{quoted}:")),
                "{backend_name}: expected the quoted key {quoted} in:\n{file}"
            );
        }
        for bare in BARE_KEYS {
            assert!(
                !file.contains(bare),
                "{backend_name}: `{bare}` is a bare non-identifier key (#215) in:\n{file}"
            );
        }
        // The read side: `item.2fa` / `row.my col` do not parse either.
        for bare in ["item.with-dash", "item.my col", "item.2fa", "row.my col", "row.2fa"] {
            assert!(
                !file.contains(bare),
                "{backend_name}: `{bare}` is not a valid property read (#215) in:\n{file}"
            );
        }

        tool_check(backend_name, &file);
    }
}

/// `field_case = "camelCase"` switches pg and mysql2 from a blind cast to a
/// field-by-field remap that reads the driver's *raw* keys -- the code path
/// that emitted `row.my col`. The declared field is renamed, but the raw key
/// it is read from is still the SQL column name.
#[test]
fn camel_case_remaps_read_non_identifier_columns_by_bracket_access() {
    let options = HashMap::from([("field_case".to_string(), "camelCase".to_string())]);
    for (backend_name, engine, dialect) in TS_BACKENDS {
        let backend = build(backend_name, engine, &options);
        let codes = generate_all(&*backend, &dialect);
        let file = assemble(&*backend, engine, &codes);

        for bare in ["row.with-dash", "row.my col", "row.2fa"] {
            assert!(
                !file.contains(bare),
                "{backend_name}: `{bare}` is not a valid property read (#215) in:\n{file}"
            );
        }
        assert!(
            file.contains("[\"my col\"]") || file.contains("['my col']"),
            "{backend_name}: the raw key must still be read, by bracket access:\n{file}"
        );

        tool_check(backend_name, &file);
    }
}

/// A reserved word is a valid property key and a valid property read, so
/// #215 must not start quoting one -- that is #180/#151's problem (keyword-
/// ness), not this one (identifier shape), and they have different fixes.
#[test]
fn a_reserved_word_column_stays_an_unquoted_key() {
    const SCHEMA: &str = "CREATE TABLE items (id INT PRIMARY KEY, class TEXT NOT NULL);";
    const QUERY: &str = "-- @name FindItem\n-- @returns :one\nSELECT id, class FROM items WHERE id = $1;";

    let backend = get_backend("typescript-pg", "postgresql").unwrap();
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).unwrap();
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).unwrap();
    let analyzed = analyze(&catalog, &parsed).unwrap();
    let code = generate_with_backend(&analyzed, &*backend).unwrap();
    let row_struct = code.row_struct.expect("expected a row struct");

    assert!(row_struct.contains("\tclass: string;"), "got:\n{row_struct}");
    assert!(!row_struct.contains("\"class\""), "got:\n{row_struct}");
}

/// This must fail before the fix: the bracket-access read paths spliced the
/// column name into a single-quoted JS string without escaping, so a column
/// named `it's` produced `row['it's']` -- the literal closes after `it`.
#[test]
fn an_apostrophe_in_a_column_name_is_escaped_in_bracket_reads() {
    const SCHEMA: &str = "CREATE TABLE weird (id INT PRIMARY KEY, \"it's\" TEXT NOT NULL);";
    const QUERY: &str = "-- @name FindWeird\n-- @returns :many\nSELECT id, \"it's\" FROM weird;";

    let options = HashMap::from([("field_case".to_string(), "camelCase".to_string())]);
    for backend_name in ["typescript-postgres", "typescript-pg"] {
        let backend = build(backend_name, "postgresql", &options);
        let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).unwrap();
        let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).unwrap();
        let analyzed = analyze(&catalog, &parsed).unwrap();
        let code = generate_with_backend(&analyzed, &*backend).unwrap();
        let query_fn = code.query_fn.expect("expected a query fn");

        assert!(
            !query_fn.contains("['it's']"),
            "{backend_name}: an unescaped apostrophe closes the key literal:\n{query_fn}"
        );
        assert!(
            query_fn.contains("['it\\'s']") || query_fn.contains("[\"it's\"]"),
            "{backend_name}: expected an escaped or double-quoted key:\n{query_fn}"
        );
    }
}

/// Real `tsc --strict` over the declarations and object literals #215 is
/// about, assembled standalone so no driver package has to be installed.
/// Additive: the assertions above already fail on a regression without it.
#[test]
fn tsc_accepts_the_quoted_declarations_and_reads() {
    let source = "export interface WeirdRow {\n\
         \t\"with-dash\": string;\n\
         \t\"my col\": string;\n\
         \t\"2fa\": string;\n\
         }\n\
         const raw: Record<string, unknown> = {};\n\
         export const row: WeirdRow = {\n\
         \t\"with-dash\": raw['with-dash'] as string,\n\
         \t\"my col\": raw[\"my col\"] as string,\n\
         \t\"2fa\": raw['it\\'s'] as string,\n\
         };\n";

    if Command::new("tsc").arg("--version").output().is_err() {
        assert!(!strict_mode_enabled(), "strict mode requires tsc to be installed");
        eprintln!("skipping tsc check: not on PATH");
        return;
    }
    let path = std::env::temp_dir().join("scythe_ts_identifier_quoting_check.ts");
    std::fs::write(&path, source).expect("failed to write temp file");
    let output = Command::new("tsc")
        .args(["--strict", "--noEmit", "--target", "es2022", path.to_str().unwrap()])
        .output()
        .expect("tsc could not be executed");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "tsc rejected the quoted declarations:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// **Known gap, deliberately pinned rather than fixed.**
///
/// A scalar query parameter takes its name from the column it is compared
/// against, and that name lands in a *binding* position --
/// `function findByFirstName(client: PoolClient, my col: string)`. Quoting
/// is not available there; the only fix is to mangle the identifier, and
/// `field_name` (`scythe_backend::naming`) is the one place that could do it
/// -- shared by all ten target languages and by every committed generated
/// file. That is a cross-language naming decision, not a TypeScript bug fix,
/// so #215's quoting fix stops at the key and read positions.
///
/// This test asserts the *current* behaviour so the gap is visible and so
/// whoever closes it has to come here and say so.
#[test]
fn a_scalar_parameter_named_after_a_non_identifier_column_is_still_broken() {
    const SCHEMA: &str = "CREATE TABLE weird (id INT PRIMARY KEY, \"my col\" TEXT NOT NULL);";
    const QUERY: &str = "-- @name FindWeird\n-- @returns :one\nSELECT id FROM weird WHERE \"my col\" = $1;";

    let backend = get_backend("typescript-pg", "postgresql").unwrap();
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).unwrap();
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).unwrap();
    let analyzed = analyze(&catalog, &parsed).unwrap();
    let code = generate_with_backend(&analyzed, &*backend).unwrap();
    let query_fn = code.query_fn.expect("expected a query fn");

    assert!(
        query_fn.contains("my col: string"),
        "if this now emits a mangled identifier the gap is closed -- update the \
         doc comment above and delete this test:\n{query_fn}"
    );
}
