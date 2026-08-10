//! End-to-end regression tests for the manifest-driven reserved-word
//! identifier mangling (`scythe_backend::naming::field_name`), exercised
//! through the real parse -> analyze -> codegen pipeline.
//!
//! Closes #180/#151: a column whose SQL name is a target-language keyword
//! (`type`, `class`, `fn`, ...) used to reach the generated file verbatim --
//! `pub type: String`, `String class`, `[id, fn] = row` -- none of which
//! parse. The fix is a per-manifest `[naming] reserved = [...]` array
//! (populated in every non-TypeScript manifest; see
//! `crates/scythe-codegen/manifests/*.toml`) consulted by `field_name`,
//! never a hardcoded cross-language table (#198).
//!
//! TypeScript declares `[naming] reserved_bindings` instead, consulted by
//! `param_name`, because it is the one target where a keyword is illegal in
//! a parameter binding and legal as a property key -- see
//! `typescript_pg_mangles_class_in_a_binding_but_leaves_the_row_key_alone`
//! at the bottom of this file for why the row key must not be mangled.

use std::process::Command;

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str =
    "CREATE TABLE items (id SERIAL PRIMARY KEY, type TEXT NOT NULL, class TEXT NOT NULL, fn TEXT NOT NULL);";

const QUERY: &str = "-- @name FindItem\n-- @returns :one\n\
    SELECT id, type, class, fn FROM items WHERE id = $1;";

/// Generate the row struct (and query fn, for backends whose parameters
/// happen to share the affected field) for `backend_name` against
/// [`SCHEMA`]/[`QUERY`].
fn generate_row_struct(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.row_struct.expect("expected a row struct")
}

#[test]
fn rust_sqlx_mangles_the_type_keyword() {
    let row_struct = generate_row_struct("rust-sqlx");
    assert!(
        !row_struct.contains("pub type:"),
        "`pub type: ...` is not valid Rust (type is a keyword):\n{row_struct}"
    );
    assert!(
        row_struct.contains("pub type_:"),
        "expected the reserved-word mangled field `type_`:\n{row_struct}"
    );
}

#[test]
fn python_psycopg3_mangles_the_class_keyword() {
    let row_struct = generate_row_struct("python-psycopg3");
    assert!(
        !row_struct.contains("    class:"),
        "`class: ...` is not valid Python (class is a keyword):\n{row_struct}"
    );
    assert!(
        row_struct.contains("    class_:"),
        "expected the reserved-word mangled field `class_`:\n{row_struct}"
    );
}

#[test]
fn java_jdbc_mangles_the_class_keyword() {
    let row_struct = generate_row_struct("java-jdbc");
    assert!(
        !row_struct.contains(" class;") && !row_struct.contains(" class,") && !row_struct.contains(" class)"),
        "a bare `class` field/param is not valid Java (class is a keyword):\n{row_struct}"
    );
    assert!(
        row_struct.contains("class_"),
        "expected the reserved-word mangled field `class_`:\n{row_struct}"
    );
}

#[test]
fn kotlin_jdbc_mangles_the_class_keyword() {
    let row_struct = generate_row_struct("kotlin-jdbc");
    assert!(
        !row_struct.contains("val class:"),
        "`val class: ...` is not valid Kotlin (class is a hard keyword):\n{row_struct}"
    );
    assert!(
        row_struct.contains("val class_:"),
        "expected the reserved-word mangled field `class_`:\n{row_struct}"
    );
}

#[test]
fn elixir_postgrex_mangles_the_fn_keyword() {
    let row_struct = generate_row_struct("elixir-postgrex");
    // #151's own reported case: `fn` is a reserved word in Elixir.
    assert!(
        !row_struct.contains(":fn,") && !row_struct.contains(":fn]"),
        "a bare `fn` field is not valid Elixir (fn is reserved):\n{row_struct}"
    );
    assert!(
        row_struct.contains(":fn_"),
        "expected the reserved-word mangled field `fn_`:\n{row_struct}"
    );
}

#[test]
fn go_pgx_leaves_pascal_cased_field_alone() {
    // Go PascalCases exported struct fields (`Type`), which never collides
    // with the lowercase keyword `type` -- issue #180 calls this out as
    // already-accidentally-safe, and `to_pascal_case` normalizes away the
    // mangled `type_`'s trailing underscore (an all-lowercase single word
    // has no second part for the underscore to separate), so the exported
    // field is the same `Type` it always was. The struct still compiles
    // (Go has no `class`/`type` restriction on struct field *names*, only
    // on the package-level `type` keyword), which is what this asserts.
    let row_struct = generate_row_struct("go-pgx");
    assert!(row_struct.contains("Type string"), "got:\n{row_struct}");
    assert!(row_struct.contains("Class string"), "got:\n{row_struct}");

    // Real-compiler check: `gofmt -e` parses the struct without needing its
    // package/import context resolved, mirroring `validate_go_tools`.
    let source = format!("package generated\n\n{row_struct}\n");
    if Command::new("gofmt").arg("-h").output().is_ok() {
        let path = std::env::temp_dir().join("scythe_reserved_word_check.go");
        std::fs::write(&path, &source).expect("failed to write temp file");
        let output = Command::new("gofmt")
            .args(["-e", path.to_str().unwrap()])
            .output()
            .expect("gofmt could not be executed");
        let _ = std::fs::remove_file(&path);
        assert!(
            output.status.success(),
            "gofmt rejected the mangled struct:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    } else {
        eprintln!("skipping gofmt check: not on PATH");
    }
}

#[test]
fn csharp_npgsql_mangles_the_class_keyword_in_the_function_parameter_list() {
    // C# backends emit function *parameters* with the raw field_name
    // (unlike row-struct properties, which are PascalCased downstream) --
    // this is the exact shape #180 reported: `string class` in a signature.
    let backend = get_backend("csharp-npgsql", "postgresql").unwrap();
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).unwrap();
    const PARAM_QUERY: &str = "-- @name FindByClass\n-- @returns :one\n\
        SELECT id, type, class, fn FROM items WHERE class = $1;";
    let parsed = parse_query_with_dialect(PARAM_QUERY, &SqlDialect::PostgreSQL).unwrap();
    let analyzed = analyze(&catalog, &parsed).unwrap();
    let code = generate_with_backend(&analyzed, &*backend).unwrap();
    let query_fn = code.query_fn.expect("expected a query fn");
    assert!(
        !query_fn.contains("string class,") && !query_fn.contains("string class)"),
        "`string class` is not valid C# (class is a keyword):\n{query_fn}"
    );
    assert!(
        query_fn.contains("class_"),
        "expected the reserved-word mangled parameter `class_`:\n{query_fn}"
    );
}

/// TypeScript mangles a keyword only where it lands in a binding.
///
/// The asymmetry `[naming] reserved_bindings` exists for, asserted in both
/// directions in one test so neither half can be "fixed" without the other
/// failing: `function q(class: string)` is `TS1390`, but the row type must
/// keep declaring `class` because the driver's rows are cast straight onto
/// it (`client.query<FindByClassRow>(...)`). Mangling the column instead
/// would have swapped a compile error for a row type that describes an
/// object `pg` never returns.
#[test]
fn typescript_pg_mangles_class_in_a_binding_but_leaves_the_row_key_alone() {
    let backend = get_backend("typescript-pg", "postgresql").unwrap();
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).unwrap();
    const PARAM_QUERY: &str = "-- @name FindByClass\n-- @returns :one\n\
        SELECT id, type, class, fn FROM items WHERE class = $1;";
    let parsed = parse_query_with_dialect(PARAM_QUERY, &SqlDialect::PostgreSQL).unwrap();
    let analyzed = analyze(&catalog, &parsed).unwrap();
    let code = generate_with_backend(&analyzed, &*backend).unwrap();

    let query_fn = code.query_fn.expect("expected a query fn");
    assert!(
        !query_fn.contains("\tclass: string,"),
        "`class` is not allowed as a TypeScript parameter name (TS1390):\n{query_fn}"
    );
    assert!(
        query_fn.contains("\tclass_: string,"),
        "expected the mangled parameter binding `class_`:\n{query_fn}"
    );

    let row_struct = code.row_struct.expect("expected a row struct");
    assert!(
        row_struct.contains("\tclass: string;"),
        "the row key must stay `class` -- it names a column `pg` returns:\n{row_struct}"
    );
    assert!(
        !row_struct.contains("class_"),
        "mangling the row key breaks the cast onto the driver's rows:\n{row_struct}"
    );
}
