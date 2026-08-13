//! Regression tests for `typescript-duckdb`'s `bytes` mapping in the READ
//! direction.
//!
//! The manifest's `bytes` scalar used to be `"Uint8Array"`. That is exactly
//! right for the *bind* direction -- `@duckdb/node-api`'s prepared
//! statements accept a raw `Uint8Array` for a `BLOB` parameter at runtime
//! (see `write_bind_and_run`'s doc comment in `typescript_duckdb.rs`, and
//! the `as DuckDBValue[]` cast that forces it through the type checker) --
//! but it was never checked against the *read* direction. `getRowObjects()`
//! hands a `BLOB` column back wrapped in the driver's own `DuckDBBlobValue`
//! class, not a bare `Uint8Array`: the driver's own `DuckDBValue` union
//! (the type `stmt.bind()` accepts) explicitly excludes `Uint8Array` while
//! including "the driver's wrapper classes" -- which is why binding a raw
//! `Uint8Array` needs the `as DuckDBValue[]` force cast in the first place.
//! A `bytes` column declared `Uint8Array` and blindly cast from the row
//! object (`firstRow<Row>(rows)`, or `row['col'] as Uint8Array` under
//! `field_case = "camelCase"`) was therefore a compile-time type that never
//! matched what the driver actually puts there at runtime -- exactly the
//! kind of declared-vs-actual mismatch that produces silent wrong answers
//! (or, more likely, a runtime `TypeError` the first time calling code
//! tries to treat the value as a `Uint8Array`).
//!
//! These tests pin the corrected read type, `DuckDBBlobValue`.

use std::collections::HashMap;

use scythe_codegen::{CodegenBackend, GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// `BLOB` parses under `SqlDialect::PostgreSQL` (the dialect every
/// `typescript-duckdb` test in this crate parses DuckDB SQL with -- there is
/// no dedicated DuckDB `SqlDialect` variant) and lands on the neutral type
/// `bytes`, exactly as it does for the five other backends'
/// `bytes_fixture` in `csharp_reader_type_regression.rs`.
const SCHEMA: &str = "CREATE TABLE blobs (id INTEGER PRIMARY KEY, payload BLOB NOT NULL, maybe_payload BLOB);";
const QUERY_ONE: &str =
    "-- @name GetBlob\n-- @returns :one\nSELECT id, payload, maybe_payload FROM blobs WHERE id = $1;";
const QUERY_MANY: &str = "-- @name ListBlobs\n-- @returns :many\nSELECT id, payload, maybe_payload FROM blobs;";

fn generate(query: &str, options: &HashMap<String, String>) -> (GeneratedCode, Box<dyn CodegenBackend>) {
    let mut backend = get_backend("typescript-duckdb", "duckdb").expect("typescript-duckdb must support duckdb");
    if !options.is_empty() {
        backend.apply_options(options).expect("options must apply");
    }
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    (code, backend)
}

/// This must fail before the fix: the manifest declared `bytes` as
/// `Uint8Array`, so the row interface's `payload`/`maybe_payload` fields
/// -- and the blind `firstRow<GetBlobRow>(rows)` cast that produces them
/// under the default `field_case = "snake_case"` -- claimed a type
/// `getRowObjects()` never actually returns for a `BLOB` column.
#[test]
fn duckdb_blob_column_row_struct_declares_duckdb_blob_value_not_uint8array() {
    let (code, _backend) = generate(QUERY_ONE, &HashMap::new());
    let row_struct = code.row_struct.expect("expected a row struct");

    assert!(
        row_struct.contains("payload: DuckDBBlobValue;"),
        "the non-null BLOB column must be declared DuckDBBlobValue; got:\n{row_struct}"
    );
    assert!(
        row_struct.contains("maybe_payload: DuckDBBlobValue | null;"),
        "the nullable BLOB column must keep its null guard around the corrected type; got:\n{row_struct}"
    );
    assert!(
        !row_struct.contains("Uint8Array"),
        "the row struct must not declare a BLOB column Uint8Array -- that is not what \
         @duckdb/node-api returns for one; got:\n{row_struct}"
    );
}

/// The default `field_case = "snake_case"` reads through a blind
/// `firstRow<GetBlobRow>(rows)` cast with no per-column expression to
/// inspect, so the only place this direction's type surfaces in the query
/// function itself is the cast's type argument.
#[test]
fn duckdb_blob_column_one_query_fn_uses_the_blind_cast_with_the_corrected_row_type() {
    let (code, _backend) = generate(QUERY_ONE, &HashMap::new());
    let query_fn = code.query_fn.expect("expected a query function");

    assert!(
        query_fn.contains("firstRow<GetBlobRow>(rows)"),
        "default field_case must keep the blind cast onto the row interface; got:\n{query_fn}"
    );
    assert!(!query_fn.contains("Uint8Array"), "got:\n{query_fn}");
}

/// Under `field_case = "camelCase"`, `:one` reconstructs each field with an
/// explicit `row['col'] as {type}` cast (see
/// `test_one_query_fn_remaps_fields_under_camel_case` in
/// `typescript_duckdb.rs`), so the corrected type must appear directly in
/// the cast expression, not just the interface.
#[test]
fn duckdb_blob_column_one_query_fn_casts_to_duckdb_blob_value_under_camel_case() {
    let (code, _backend) = generate(
        QUERY_ONE,
        &HashMap::from([("field_case".to_string(), "camelCase".to_string())]),
    );
    let query_fn = code.query_fn.expect("expected a query function");

    assert!(
        query_fn.contains("payload: row['payload'] as DuckDBBlobValue,"),
        "the non-null BLOB column must be cast to DuckDBBlobValue; got:\n{query_fn}"
    );
    assert!(
        query_fn.contains("maybePayload: row['maybe_payload'] as DuckDBBlobValue | null,"),
        "the nullable BLOB column must be cast to DuckDBBlobValue | null; got:\n{query_fn}"
    );
    assert!(!query_fn.contains("Uint8Array"), "got:\n{query_fn}");
}

/// `:many` goes through `generate_ts_many_row_remap` instead of `:one`'s
/// `generate_ts_one_row_remap`, a different code path in
/// `typescript_common.rs` that must independently agree on the type.
#[test]
fn duckdb_blob_column_many_query_fn_casts_to_duckdb_blob_value_under_camel_case() {
    let (code, _backend) = generate(
        QUERY_MANY,
        &HashMap::from([("field_case".to_string(), "camelCase".to_string())]),
    );
    let query_fn = code.query_fn.expect("expected a query function");

    assert!(
        query_fn.contains("payload: row['payload'] as DuckDBBlobValue,"),
        "got:\n{query_fn}"
    );
    assert!(!query_fn.contains("Uint8Array"), "got:\n{query_fn}");
}

/// `DuckDBBlobValue` has to be imported wherever it is referenced, exactly
/// as `DuckDBValue` already is for the bind side -- an unused import is a
/// lint finding, a missing one is `TS2304: Cannot find name`.
#[test]
fn duckdb_blob_value_import_is_present_only_when_a_blob_column_is_read() {
    let (blob_code, backend) = generate(QUERY_ONE, &HashMap::new());
    let header_with_blob = backend.file_header_for_results(&[blob_code]);
    assert!(
        header_with_blob
            .contains("import type { DuckDBConnection, DuckDBValue, DuckDBBlobValue } from \"@duckdb/node-api\";"),
        "got:\n{header_with_blob}"
    );

    let (no_blob_code, backend) = generate(
        "-- @name CountBlobs\n-- @returns :one\nSELECT id FROM blobs WHERE id = $1;",
        &HashMap::new(),
    );
    let header_without_blob = backend.file_header_for_results(&[no_blob_code]);
    assert!(
        !header_without_blob.contains("DuckDBBlobValue"),
        "a file with no BLOB column must not import DuckDBBlobValue; got:\n{header_without_blob}"
    );
    assert!(
        header_without_blob.contains("import type { DuckDBConnection, DuckDBValue } from \"@duckdb/node-api\";"),
        "got:\n{header_without_blob}"
    );
}
