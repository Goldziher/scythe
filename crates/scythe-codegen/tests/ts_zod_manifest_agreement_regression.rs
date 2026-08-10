//! End-to-end regression tests for #216: `row_type = "zod"` must derive its
//! schemas from the backend manifest, exactly as `row_type = "interface"`
//! does.
//!
//! The Zod emitter used to consult a hardcoded neutral-type table
//! (`neutral_to_zod`) while the interface emitter resolved the same column
//! through `[types.scalars]`. Every TypeScript manifest departs from that
//! table somewhere, so the two modes declared *different types for the same
//! column on the same backend* -- on `typescript-node-sqlite`, four of six
//! columns in one query.
//!
//! Enum values had two further problems on this path: they were spliced into
//! string literals unescaped, and the variant keys were built with a raw
//! `to_pascal_case` instead of the manifest-aware
//! `scythe_backend::naming::enum_variant_name` the interface path uses, so a
//! value like `in-active` became `InActive` under `interface` and
//! `In-active` -- not a valid object key at all -- under `zod`.
//!
//! Assertions here are unconditional; `validate_with_tools` is additive.

use std::collections::HashMap;

use scythe_codegen::validation::{strict_mode_enabled, validate_with_tools};
use scythe_codegen::{GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// Every column whose neutral type the old hardcoded table disagreed with
/// at least one TypeScript manifest about.
const SQLITE_SCHEMA: &str = "CREATE TABLE things (\
    id INTEGER PRIMARY KEY, \
    active BOOLEAN NOT NULL, \
    price DECIMAL(10, 2) NOT NULL, \
    created_at DATETIME NOT NULL, \
    payload BLOB NOT NULL, \
    meta JSON NOT NULL\
);";

const SQLITE_QUERY: &str = "-- @name GetThing\n-- @returns :one\n\
    SELECT id, active, price, created_at, payload, meta FROM things WHERE id = ?;";

fn generate(backend_name: &str, engine: &str, dialect: &SqlDialect, schema: &str, query: &str, zod: bool) -> String {
    let mut backend = get_backend(backend_name, engine).expect("backend must support engine");
    if zod {
        backend
            .apply_options(&HashMap::from([("row_type".to_string(), "zod".to_string())]))
            .expect("row_type = zod must apply");
    }
    let catalog = Catalog::from_ddl_with_dialect(&[schema], dialect).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, dialect).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code: GeneratedCode = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.row_struct.expect("expected a row struct")
}

/// Pull `field: type` out of an `export interface` body.
fn interface_fields(row_struct: &str) -> Vec<(String, String)> {
    row_struct
        .lines()
        .filter_map(|line| line.trim().strip_suffix(';'))
        .filter_map(|line| line.split_once(": "))
        .map(|(name, ty)| (name.trim().to_string(), ty.trim().to_string()))
        .collect()
}

/// Pull `field: schema` out of a `z.object({ ... })` body.
fn zod_fields(row_struct: &str) -> Vec<(String, String)> {
    row_struct
        .lines()
        .filter_map(|line| line.trim().strip_suffix(','))
        .filter_map(|line| line.split_once(": "))
        .map(|(name, schema)| (name.trim().to_string(), schema.trim().to_string()))
        .collect()
}

/// The Zod schema whose `z.infer` is the given TypeScript type. This is the
/// agreement the two `row_type` modes have to satisfy, spelled out
/// independently of the implementation so the test is not a mirror of it.
fn expected_schema_for(ts_type: &str) -> String {
    match ts_type {
        "boolean" => "z.boolean()".to_string(),
        "number" => "z.number()".to_string(),
        "bigint" => "z.bigint()".to_string(),
        "string" => "z.string()".to_string(),
        "Date" => "z.date()".to_string(),
        "Buffer" | "Uint8Array" => format!("z.instanceof({ts_type})"),
        "Record<string, unknown>" => "z.record(z.string(), z.unknown())".to_string(),
        other => panic!("fixture produced an unmapped TypeScript type: {other}"),
    }
}

/// This must fail before the fix on every SQLite-backed TypeScript backend:
/// the manifest maps `bool` to `number`, `decimal` to `number`, `datetime`
/// to `string` and `json` to `Record<string, unknown>`, while the hardcoded
/// table produced `z.boolean()`, `z.string()`, `z.date()` and `z.unknown()`.
#[test]
fn zod_schemas_agree_with_the_interface_the_same_backend_declares() {
    for backend_name in [
        "typescript-better-sqlite3",
        "typescript-node-sqlite",
        "typescript-wasm-sqlite",
    ] {
        let interface = generate(
            backend_name,
            "sqlite",
            &SqlDialect::SQLite,
            SQLITE_SCHEMA,
            SQLITE_QUERY,
            false,
        );
        let zod = generate(
            backend_name,
            "sqlite",
            &SqlDialect::SQLite,
            SQLITE_SCHEMA,
            SQLITE_QUERY,
            true,
        );

        let declared = interface_fields(&interface);
        let schemas: HashMap<String, String> = zod_fields(&zod).into_iter().collect();
        assert_eq!(declared.len(), 6, "{backend_name}: fixture must project 6 columns");

        for (field, ts_type) in declared {
            let schema = schemas
                .get(&field)
                .unwrap_or_else(|| panic!("{backend_name}: zod mode dropped the field `{field}`:\n{zod}"));
            assert_eq!(
                schema,
                &expected_schema_for(&ts_type),
                "{backend_name}: `{field}` is `{ts_type}` under row_type = \"interface\" but \
                 `{schema}` under row_type = \"zod\" (#216)"
            );
        }
    }
}

/// The two binary runtimes must each keep their own manifest type -- the
/// pre-existing half of this bug, still covered.
#[test]
fn zod_bytes_schema_follows_the_manifest_not_a_hardcoded_buffer() {
    let pg = generate(
        "typescript-pg",
        "postgresql",
        &SqlDialect::PostgreSQL,
        "CREATE TABLE blobs (id INT PRIMARY KEY, payload BYTEA NOT NULL);",
        "-- @name GetBlob\n-- @returns :one\nSELECT id, payload FROM blobs WHERE id = $1;",
        true,
    );
    assert!(pg.contains("z.instanceof(Buffer)"), "{pg}");

    let sqlite = generate(
        "typescript-node-sqlite",
        "sqlite",
        &SqlDialect::SQLite,
        "CREATE TABLE blobs (id INTEGER PRIMARY KEY, payload BLOB NOT NULL);",
        "-- @name GetBlob\n-- @returns :one\nSELECT id, payload FROM blobs WHERE id = ?;",
        true,
    );
    assert!(sqlite.contains("z.instanceof(Uint8Array)"), "{sqlite}");
    assert!(!sqlite.contains("Buffer"), "{sqlite}");
}

/// `int64` is `bigint` on the duckdb manifest and `number` on pg -- the
/// clearest case of a single neutral type the old table could not have got
/// right for both.
#[test]
fn zod_int64_follows_each_backends_own_int64_mapping() {
    let duck = generate(
        "typescript-duckdb",
        "duckdb",
        &SqlDialect::PostgreSQL,
        "CREATE TABLE counters (id INT PRIMARY KEY, total BIGINT NOT NULL);",
        "-- @name GetCounter\n-- @returns :one\nSELECT id, total FROM counters WHERE id = $1;",
        true,
    );
    assert!(
        duck.contains("total: z.bigint()"),
        "duckdb maps int64 to bigint; got:\n{duck}"
    );

    let pg = generate(
        "typescript-pg",
        "postgresql",
        &SqlDialect::PostgreSQL,
        "CREATE TABLE counters (id INT PRIMARY KEY, total BIGINT NOT NULL);",
        "-- @name GetCounter\n-- @returns :one\nSELECT id, total FROM counters WHERE id = $1;",
        true,
    );
    assert!(pg.contains("total: z.number()"), "pg maps int64 to number; got:\n{pg}");
}

const ENUM_SCHEMA: &str = "CREATE TYPE user_status AS ENUM ('active', 'in-active', 'on\"hold');\n\
    CREATE TABLE users (id INT PRIMARY KEY, status user_status NOT NULL);";
const ENUM_QUERY: &str = "-- @name GetUser\n-- @returns :one\nSELECT id, status FROM users WHERE id = $1;";

fn enum_def(backend_name: &str, zod: bool) -> String {
    let mut backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    if zod {
        backend
            .apply_options(&HashMap::from([("row_type".to_string(), "zod".to_string())]))
            .expect("row_type = zod must apply");
    }
    let catalog = Catalog::from_ddl_with_dialect(&[ENUM_SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(ENUM_QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.enum_def.expect("expected an enum def")
}

/// This must fail before the fix: `to_pascal_case("in-active")` keeps the
/// hyphen (`In-active`), which is not a valid object key, while the
/// interface path's `enum_variant_name` sanitises it to `InActive`.
#[test]
fn zod_enum_variant_keys_match_the_interface_paths_variant_names() {
    let interface = enum_def("typescript-pg", false);
    let zod = enum_def("typescript-pg", true);

    assert!(
        interface.contains("InActive: \"in-active\","),
        "interface mode names the variant through enum_variant_name; got:\n{interface}"
    );
    assert!(
        zod.contains("InActive: \"in-active\","),
        "zod mode must name it identically (#216); got:\n{zod}"
    );
    assert!(
        !zod.contains("In-active"),
        "`In-active` is not a valid object key; got:\n{zod}"
    );
}

/// This must fail before the fix: an enum value containing a `"` was
/// spliced into `z.enum(["..."])` and into the const's values raw, closing
/// the literal.
#[test]
fn zod_enum_values_are_escaped_for_the_string_literals_they_land_in() {
    let zod = enum_def("typescript-pg", true);

    assert!(
        zod.contains("z.enum([\"active\", \"in-active\", \"on\\\"hold\"]);"),
        "the double quote must be escaped inside z.enum; got:\n{zod}"
    );
    assert!(
        zod.contains(": \"on\\\"hold\","),
        "and inside the const's value too; got:\n{zod}"
    );
}

/// The whole generated file must still satisfy the repository's own
/// TypeScript checker under `row_type = "zod"`. Additive on top of the
/// assertions above.
#[test]
fn zod_output_passes_tool_validation() {
    for (backend_name, engine, dialect, schema, query) in [
        (
            "typescript-node-sqlite",
            "sqlite",
            SqlDialect::SQLite,
            SQLITE_SCHEMA,
            SQLITE_QUERY,
        ),
        (
            "typescript-pg",
            "postgresql",
            SqlDialect::PostgreSQL,
            ENUM_SCHEMA,
            ENUM_QUERY,
        ),
    ] {
        let mut backend = get_backend(backend_name, engine).unwrap();
        backend
            .apply_options(&HashMap::from([("row_type".to_string(), "zod".to_string())]))
            .unwrap();
        let catalog = Catalog::from_ddl_with_dialect(&[schema], &dialect).unwrap();
        let parsed = parse_query_with_dialect(query, &dialect).unwrap();
        let analyzed = analyze(&catalog, &parsed).unwrap();
        let code = generate_with_backend(&analyzed, &*backend).unwrap();

        let mut file = backend.file_header_for_results(std::slice::from_ref(&code));
        file.push('\n');
        if let Some(def) = &code.enum_def {
            file.push_str(def);
            file.push_str("\n\n");
        }
        for text in [&code.row_struct, &code.query_fn].into_iter().flatten() {
            file.push_str(text);
            file.push_str("\n\n");
        }

        let validation = validate_with_tools(&file, backend_name);
        assert!(
            validation.errors().is_empty(),
            "{backend_name}: {:#?}\n\nfile:\n{file}",
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
}
