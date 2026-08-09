//! End-to-end regression test for #215: a column name that is not a valid
//! TypeScript identifier (`with-dash`, `my col`, `1st`) must be emitted as a
//! quoted property key, not spliced bare into an interface member.
//!
//! Exercised through the real parse -> analyze -> codegen pipeline, and
//! through real `tsc --strict` when it is on `PATH`.

use std::process::Command;

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TABLE weird (\
    id SERIAL PRIMARY KEY, \
    \"with-dash\" TEXT NOT NULL, \
    \"my col\" TEXT NOT NULL, \
    \"1st\" TEXT NOT NULL\
);";

const QUERY: &str = "-- @name FindWeird\n-- @returns :one\n\
    SELECT id, \"with-dash\", \"my col\", \"1st\" FROM weird WHERE id = $1;";

#[test]
fn typescript_pg_quotes_non_identifier_column_names() {
    let backend = get_backend("typescript-pg", "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    let row_struct = code.row_struct.expect("expected a row struct");

    assert!(
        !row_struct.contains("with-dash: string"),
        "a bare non-identifier key is not valid TS (#215):\n{row_struct}"
    );
    assert!(row_struct.contains("\"with-dash\": string"), "got:\n{row_struct}");
    assert!(row_struct.contains("\"my col\": string"), "got:\n{row_struct}");
    assert!(row_struct.contains("\"1st\": string"), "got:\n{row_struct}");

    if Command::new("tsc").arg("--version").output().is_err() {
        eprintln!("skipping tsc check: not on PATH");
        return;
    }
    let source = format!(
        "{row_struct}\nconst _x: FindWeirdRow = {{ id: 1, \"with-dash\": \"a\", \"my col\": \"b\", \"1st\": \"c\" }};\n"
    );
    let path = std::env::temp_dir().join("scythe_ts_identifier_quoting_check.ts");
    std::fs::write(&path, &source).expect("failed to write temp file");
    let output = Command::new("tsc")
        .args(["--strict", "--noEmit", "--target", "es2022", path.to_str().unwrap()])
        .output()
        .expect("tsc could not be executed");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "tsc rejected the quoted interface:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
