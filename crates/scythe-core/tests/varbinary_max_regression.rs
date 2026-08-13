//! Regression tests for board #174: T-SQL's `MAX` length keyword
//! (`VARBINARY(MAX)`, `NVARCHAR(MAX)`, `VARCHAR(MAX)`) must resolve to the
//! same neutral type as the corresponding bounded/unbounded spelling, in
//! both keyword casings, without regressing the bounded (`VARBINARY(100)`)
//! form.

use scythe_core::SqlDialect;
use scythe_core::analyzer::sql_type_to_neutral;
use scythe_core::catalog::Catalog;

/// Builds a single-column MSSQL table from `column_type_ddl` and returns the
/// neutral type the full DDL pipeline (parse -> `normalize_data_type` ->
/// `sql_type_to_neutral`) resolves the column to.
fn resolve_mssql_column_neutral_type(column_type_ddl: &str) -> String {
    let ddl = format!("CREATE TABLE t (col {});", column_type_ddl);
    let catalog = Catalog::from_ddl_with_dialect(&[&ddl], &SqlDialect::MsSql).unwrap();
    let table = catalog.get_table("t").unwrap();
    sql_type_to_neutral(&table.columns[0].sql_type, &catalog).into_owned()
}

// ~keep Before the fix, `normalize_data_type` had no match arm for
// `DataType::Varbinary` at all, so it fell through to the generic
// `other => other.to_string().to_lowercase()` branch, storing the catalog
// column's `sql_type` as the literal string `"varbinary(max)"`. In
// `sql_type_to_neutral`, `strip_precision` only strips a trailing
// `(<digits>)` and leaves non-digit parens (`"(max)"`) untouched, so that
// string never matched the bare `"varbinary"` arm and fell to the
// catch-all, resolving to the invalid neutral type `"varbinary(max)"`
// instead of `"bytes"`. Both casings hit the same code path because
// sqlparser's tokenizer matches keywords case-insensitively.
#[test]
fn should_resolve_varbinary_max_uppercase_to_bytes() {
    assert_eq!(resolve_mssql_column_neutral_type("VARBINARY(MAX)"), "bytes");
}

#[test]
fn should_resolve_varbinary_max_lowercase_to_bytes() {
    assert_eq!(resolve_mssql_column_neutral_type("varbinary(max)"), "bytes");
}

#[test]
fn should_resolve_varbinary_max_mixed_case_to_bytes() {
    assert_eq!(resolve_mssql_column_neutral_type("VarBinary(Max)"), "bytes");
}

// ~keep `Varchar`'s existing match arm already routes anything other than
// `Some(CharacterLength::IntegerLength { .. })` -- including
// `CharacterLength::Max` -- to `"text"`, which `sql_type_to_neutral` maps to
// `"string"`. These two cases already passed before the fix; they are kept
// here so a future change to the `Varchar`/`Nvarchar` arms cannot silently
// regress the sibling of the reported `VARBINARY(MAX)` bug.
#[test]
fn should_resolve_varchar_max_uppercase_to_string() {
    assert_eq!(resolve_mssql_column_neutral_type("VARCHAR(MAX)"), "string");
}

#[test]
fn should_resolve_varchar_max_lowercase_to_string() {
    assert_eq!(resolve_mssql_column_neutral_type("varchar(max)"), "string");
}

#[test]
fn should_resolve_nvarchar_max_uppercase_to_string() {
    assert_eq!(resolve_mssql_column_neutral_type("NVARCHAR(MAX)"), "string");
}

#[test]
fn should_resolve_nvarchar_max_lowercase_to_string() {
    assert_eq!(resolve_mssql_column_neutral_type("nvarchar(max)"), "string");
}

/// The bounded form must keep resolving exactly as before: a fix that
/// breaks `VARBINARY(100)` while fixing `VARBINARY(MAX)` is a regression,
/// not a fix.
#[test]
fn should_resolve_varbinary_bounded_length_to_bytes() {
    assert_eq!(resolve_mssql_column_neutral_type("VARBINARY(100)"), "bytes");
    assert_eq!(resolve_mssql_column_neutral_type("varbinary(100)"), "bytes");
}

#[test]
fn should_resolve_varchar_bounded_length_to_string() {
    assert_eq!(resolve_mssql_column_neutral_type("VARCHAR(50)"), "string");
}

#[test]
fn should_resolve_nvarchar_bounded_length_to_string() {
    assert_eq!(resolve_mssql_column_neutral_type("NVARCHAR(50)"), "string");
}

/// `BINARY(MAX)` is not valid T-SQL (only `VARBINARY` supports `MAX`), but
/// sqlparser shares `BinaryLength` between `DataType::Binary` and
/// `DataType::Varbinary`, so it parses. It must resolve the same way
/// `VARBINARY(MAX)` does rather than reviving the unknown-type bug for a
/// sibling `DataType` variant the fix did not touch.
#[test]
fn should_resolve_binary_bounded_length_to_bytes() {
    assert_eq!(resolve_mssql_column_neutral_type("BINARY(10)"), "bytes");
}
