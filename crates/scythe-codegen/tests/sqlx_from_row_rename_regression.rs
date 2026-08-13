//! Closes the silent-wrong-answer hazard `[naming] sanitize_field_names`
//! (#168/40f6351) introduced for rust-sqlx specifically.
//!
//! `field_shape_regression.rs` documents the general justification for
//! `sanitize_field_names`: "none of [the mangling backends] read a column
//! back by the generated field name -- they use the position or the raw SQL
//! name". That is true for tokio-postgres and tiberius, whose hand-written
//! `from_row` always looks a column up by the literal SQL name string
//! (`row.get("my col")`) while only the *Rust field* is sanitized, and true
//! for rust-sibyl, which reads columns by position. It is false for
//! rust-sqlx: every row struct it emits derives `sqlx::FromRow`, and that
//! derive resolves a column *by the Rust field's own name* unless told
//! otherwise. Mangling `"my col"` into the field `my_col` without an
//! accompanying `#[sqlx(rename = "my col")]` makes `FromRow::from_row` search
//! the row for a column literally named `my_col`, which does not exist --
//! the generated code compiles cleanly (that was the whole point of the
//! mangling) and then fails, or worse silently resolves to the wrong column,
//! at runtime. The fix is emitting `#[sqlx(rename = "my col")]` directly
//! ahead of any field whose generated name differs from its SQL column name.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TABLE items (id INT PRIMARY KEY, \"my col\" TEXT NOT NULL);";

const QUERY: &str = "-- @name FindItem\n-- @returns :one\n\
    SELECT id, \"my col\" FROM items WHERE id = $1;";

fn generate_row_struct(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.row_struct.expect("expected a row struct")
}

/// The regression itself: a mangled field must carry the rename attribute
/// naming the original, unmangled SQL column, immediately ahead of the field
/// declaration so it is unambiguous which field it applies to.
#[test]
fn rust_sqlx_renames_a_mangled_field_back_to_its_sql_column_name() {
    let row_struct = generate_row_struct("rust-sqlx");
    assert!(
        row_struct.contains("pub my_col: String,"),
        "expected the sanitized field `my_col`:\n{row_struct}"
    );
    assert!(
        row_struct.contains("#[sqlx(rename = \"my col\")]\n    pub my_col: String,"),
        "sqlx::FromRow must be told the original SQL column name via \
         #[sqlx(rename = \"my col\")] directly ahead of the mangled field -- otherwise it looks \
         up a column literally named `my_col`, which does not exist:\n{row_struct}"
    );
}

/// A field whose generated name already matches its column must not carry a
/// rename attribute -- it would be redundant, and its presence on every field
/// (not just the mangled ones) would suggest the attribute was emitted
/// unconditionally rather than derived from an actual name mismatch.
#[test]
fn rust_sqlx_does_not_rename_a_field_that_was_never_mangled() {
    let row_struct = generate_row_struct("rust-sqlx");
    assert!(
        !row_struct.contains("#[sqlx(rename = \"id\")]"),
        "a field whose name already matches its column must not carry a redundant rename:\n{row_struct}"
    );
}

/// The struct must still derive `sqlx::FromRow` -- this regression is about
/// telling that derive the right column name, not about working around it by
/// dropping it.
#[test]
fn rust_sqlx_row_struct_still_derives_from_row() {
    let row_struct = generate_row_struct("rust-sqlx");
    assert!(
        row_struct.contains("sqlx::FromRow"),
        "the row struct must keep deriving sqlx::FromRow:\n{row_struct}"
    );
}
