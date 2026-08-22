//! GH #147: `degrade_unsupported_nested_structs` rewrites a `json_agg`/
//! `row_to_json` column's `json_nested<...>` type to a plain `json` (or,
//! when the manifest declares it, `json_array`) scalar for any backend that
//! does not implement `generate_nested_struct_def` -- measured on `main` as
//! 98 of 102 manifests. Before this fix, `scythe generate` exited 0 with no
//! signal that the rewrite happened at all: a user asking for a structured
//! row (e.g. `java-jdbc`, whose `json` scalar is `String`, read back via
//! `rs.getString`) silently got an opaque string instead.
//!
//! `degrade_unsupported_nested_structs` now returns a third element, one
//! [`scythe_codegen::NestedStructDegradation`] per rewritten column, and
//! [`scythe_codegen::generate_with_backend`] threads it onto
//! `GeneratedCode::degraded_nested_structs`. These tests pin that the field
//! actually reaches the top-level `generate_with_backend` result -- not just
//! the internal function -- and that it stays empty for a backend that
//! genuinely supports the struct, so a regression that silently drops the
//! third tuple element or forgets to assign the field fails loudly here.

use scythe_codegen::{GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::{NestedFieldInfo, NestedStructInfo, analyze};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::errors::ScytheError;
use scythe_core::parser::parse_query_with_dialect;

const NESTED_ARRAY_SCHEMA: &str = "\
    CREATE TYPE order_status AS ENUM ('pending', 'shipped'); \
    CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL); \
    CREATE TABLE orders (id SERIAL PRIMARY KEY, user_id INTEGER NOT NULL, status order_status NOT NULL);";

const NESTED_ARRAY_QUERY: &str = "-- @name GetUserOrders\n-- @returns :many\n\
    SELECT u.id, json_agg(o.*) AS orders FROM users u JOIN orders o ON o.user_id = u.id GROUP BY u.id;";

fn generate(backend_name: &str) -> Result<GeneratedCode, ScytheError> {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog =
        Catalog::from_ddl_with_dialect(&[NESTED_ARRAY_SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(NESTED_ARRAY_QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    generate_with_backend(&analyzed, &*backend)
}

/// `java-jdbc` implements neither `generate_nested_struct_def` nor the
/// `json_array` scalar marker, so `orders` falls all the way back to plain
/// `json`. This is the exact defect shape from the issue: `generate` must
/// still succeed (this is not a hard error by default), but the result must
/// name the column, the struct that could not be built, the fallback it got
/// instead, and the backend responsible.
#[test]
fn unsupported_backend_reports_the_degraded_column_struct_and_backend() {
    let code = generate("java-jdbc").expect("degradation is not a hard error by default");

    assert_eq!(
        code.degraded_nested_structs.len(),
        1,
        "exactly one column (orders) referenced the unsupported struct: {:?}",
        code.degraded_nested_structs
    );
    let degradation = &code.degraded_nested_structs[0];
    assert_eq!(degradation.column, "orders");
    assert_eq!(degradation.struct_name, "GetUserOrdersRowOrders");
    assert_eq!(degradation.fallback_type, "json");
    assert_eq!(degradation.backend, "java-jdbc");

    // ~keep The rewrite itself still happened -- this pins that the diagnostic is
    // additive, not a replacement for the existing (already-tested) rewrite.
    assert!(code.row_struct.is_some(), "codegen must still produce a row struct");
}

#[test]
fn typescript_postgresql_backends_emit_native_nested_structs() {
    for backend in ["typescript-pg", "typescript-postgres", "typescript-kysely"] {
        let code = generate(backend).expect("TypeScript nested JSON codegen must succeed");
        assert!(
            code.degraded_nested_structs.is_empty(),
            "{backend} must preserve nested JSON structure: {:?}",
            code.degraded_nested_structs
        );
        assert_eq!(
            code.nested_struct_defs.len(),
            1,
            "{backend} must emit one nested interface"
        );
        assert!(
            code.nested_struct_defs[0].code.contains("status: OrderStatus;"),
            "{backend} must preserve an enum inside the JSON object"
        );
        assert!(
            code.row_struct
                .as_deref()
                .is_some_and(|row| row.contains("orders: Array<GetUserOrdersRowOrders> | null;")),
            "{backend} must use Array<T> around the nested interface"
        );
    }
}

#[test]
fn javascript_aliases_keep_the_plain_json_fallback() {
    for backend in ["javascript-pg", "javascript-postgres"] {
        let code = generate(backend).expect("JavaScript fallback codegen must succeed");
        assert_eq!(code.degraded_nested_structs.len(), 1);
        assert_eq!(code.degraded_nested_structs[0].fallback_type, "json_array");
        assert!(code.nested_struct_defs.is_empty());
    }
}

#[test]
fn unsupported_future_json_field_degrades_the_entire_nested_struct() {
    let nested = NestedStructInfo {
        name: "future_payload".to_string(),
        fields: vec![NestedFieldInfo {
            name: "location".to_string(),
            neutral_type: "future_geography".to_string(),
            nullable: false,
        }],
    };
    for backend_name in ["typescript-pg", "typescript-postgres", "typescript-kysely"] {
        let backend = get_backend(backend_name, "postgresql").expect("backend must support PostgreSQL");
        assert!(
            backend
                .generate_nested_struct_def(&nested)
                .expect("unsupported nested field must not be a hard error")
                .is_none(),
            "{backend_name} must request deterministic plain-JSON degradation"
        );
    }
}

/// `rust-sqlx` implements `generate_nested_struct_def`, so nothing here was
/// degraded: the field must stay empty rather than report a false positive
/// for a backend that actually generated the struct.
#[test]
fn supporting_backend_reports_no_degradation() {
    let code = generate("rust-sqlx").expect("codegen must succeed");
    assert!(
        code.degraded_nested_structs.is_empty(),
        "rust-sqlx supports the nested struct and must not report a degradation: {:?}",
        code.degraded_nested_structs
    );
    assert_eq!(
        code.nested_struct_defs.len(),
        1,
        "the struct definition must still be emitted"
    );
}
