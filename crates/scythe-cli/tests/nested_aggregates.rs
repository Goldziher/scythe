//! End-to-end coverage for nested-aggregate codegen (#78).
//!
//! These go through the CLI binary rather than the `scythe-codegen` API
//! because the things they pin only exist at file-assembly time: enums and
//! nested structs are emitted once per output file, deduplicated across
//! queries, and ordered relative to the row structs that reference them.
//! None of that is observable from a single `generate_with_backend` call.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

/// Render a path for embedding in a TOML basic string. Forward slashes are
/// accepted by globs on every platform; `\` would be a TOML escape.
fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

const SCHEMA: &str = "\
CREATE TYPE order_status AS ENUM ('pending', 'in progress', 'shipped');
CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL,
    status order_status NOT NULL,
    notes TEXT,
    \"createdAt\" TIMESTAMPTZ NOT NULL
);
";

const QUERIES: &str = "\
-- @name GetUserOrders
-- @returns :many
SELECT u.id, json_agg(o.*) AS orders FROM users u JOIN orders o ON o.user_id = u.id GROUP BY u.id;

-- @name GetUserOrdersOuter
-- @returns :many
SELECT u.id, json_agg(o.*) AS orders FROM users u LEFT JOIN orders o ON o.user_id = u.id GROUP BY u.id;
";

/// Run `scythe generate` for one backend over `schema`/`queries` and return
/// the single generated file's contents.
fn generate(backend: &str, extension: &str, schema: &str, queries: &str) -> Result<String, String> {
    let temp = tempfile::TempDir::new().unwrap();
    let schema_path = temp.path().join("schema.sql");
    let queries_path = temp.path().join("queries.sql");
    let output_dir = temp.path().join("out");
    std::fs::write(&schema_path, schema).unwrap();
    std::fs::write(&queries_path, queries).unwrap();

    let config = format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["{schema}"]
queries = ["{queries}"]

[[sql.gen]]
backend = "{backend}"
output = "{output}"
"#,
        schema = toml_path(&schema_path),
        queries = toml_path(&queries_path),
        backend = backend,
        output = toml_path(&output_dir),
    );
    let config_path = temp.path().join("scythe.toml");
    std::fs::write(&config_path, &config).unwrap();

    let output = scythe_bin()
        .args(["generate", "--config", config_path.to_str().unwrap()])
        .current_dir(temp.path())
        .output()
        .expect("failed to run scythe generate");

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    let generated: PathBuf = output_dir.join(format!("queries.{extension}"));
    Ok(std::fs::read_to_string(&generated).expect("generated file"))
}

#[test]
fn sqlx_emits_a_nested_struct_per_query_with_json_key_renames() {
    let code = generate("rust-sqlx", "rs", SCHEMA, QUERIES).expect("generate");

    assert!(
        code.contains("pub struct GetUserOrdersRowOrders"),
        "expected a nested struct; got:\n{code}"
    );
    // The JSON key json_agg emits is the raw SQL column name, so the
    // snake_cased Rust field needs an explicit serde rename. Without it,
    // deserialization fails at runtime with `missing field created_at`.
    assert!(
        code.contains("#[serde(rename = \"createdAt\")]"),
        "expected a serde rename for the quoted mixed-case column; got:\n{code}"
    );
    assert!(
        code.contains("pub created_at: chrono::DateTime<chrono::Utc>,"),
        "got:\n{code}"
    );
    // A field whose snake_case form already matches the key must not be
    // renamed -- the attribute is emitted only where it changes behaviour.
    assert!(!code.contains("#[serde(rename = \"user_id\")]"), "got:\n{code}");
}

#[test]
fn sqlx_nested_struct_derives_both_serde_traits() {
    let code = generate("rust-sqlx", "rs", SCHEMA, QUERIES).expect("generate");

    // Deserialize decodes the column; Serialize is required because
    // `sqlx::types::Json<T>: Serialize` is bounded on `T: Serialize` and a
    // row struct generated with `serde = true` derives Serialize.
    assert!(
        code.contains(
            "#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]\npub struct GetUserOrdersRowOrders"
        ),
        "got:\n{code}"
    );
}

#[test]
fn sqlx_emits_the_enum_reached_only_through_a_nested_struct() {
    let code = generate("rust-sqlx", "rs", SCHEMA, QUERIES).expect("generate");

    // `orders.status` appears in no top-level column -- only inside the
    // nested struct. Without scanning nested fields the file would name
    // `OrderStatus` with no definition anywhere (E0412).
    assert!(code.contains("pub enum OrderStatus"), "got:\n{code}");
    assert!(code.contains("pub status: OrderStatus,"), "got:\n{code}");
    // Decoded by serde_json here, not by the driver, so it needs the serde
    // derives the plain enum path omits...
    assert!(
        code.contains("sqlx::Type, serde::Serialize, serde::Deserialize)]"),
        "got:\n{code}"
    );
    // ...and per-variant renames, because serde would otherwise look for the
    // PascalCase identifier rather than the SQL label.
    assert!(code.contains("#[serde(rename = \"in progress\")]"), "got:\n{code}");
}

#[test]
fn sqlx_left_join_element_is_optional() {
    let code = generate("rust-sqlx", "rs", SCHEMA, QUERIES).expect("generate");

    assert!(
        code.contains("pub orders: Option<sqlx::types::Json<Vec<GetUserOrdersRowOrders>>>,"),
        "an INNER JOIN cannot produce a null element; got:\n{code}"
    );
    assert!(
        code.contains("pub orders: Option<sqlx::types::Json<Vec<Option<GetUserOrdersOuterRowOrders>>>>,"),
        "json_agg over a LEFT JOIN with no match yields [null]; got:\n{code}"
    );
}

#[test]
fn nested_struct_definitions_precede_the_row_structs_that_use_them() {
    let code = generate("python-psycopg3", "py", SCHEMA, QUERIES).expect("generate");

    let nested = code.find("class GetUserOrdersRowOrders").expect("nested class");
    let row = code.find("class GetUserOrdersRow:").expect("row class");
    assert!(
        nested < row,
        "Python evaluates class annotations eagerly, so the nested class must be bound first; got:\n{code}"
    );
}

#[test]
fn psycopg3_maps_json_keys_through_a_from_json_classmethod() {
    let code = generate("python-psycopg3", "py", SCHEMA, QUERIES).expect("generate");

    // `Cls(**item)` would pass `createdAt` as an unexpected keyword
    // argument, so the key mapping is spelled out once per class.
    assert!(code.contains("created_at=obj[\"createdAt\"],"), "got:\n{code}");
    assert!(
        code.contains("[GetUserOrdersRowOrders._from_json(item) for item in r[1]]"),
        "got:\n{code}"
    );
    assert!(
        code.contains("[None if item is None else GetUserOrdersOuterRowOrders._from_json(item) for item in r[1]]"),
        "the LEFT JOIN form must tolerate a null element; got:\n{code}"
    );
}

/// A user's own `@json` annotation resolves to `json_typed<T>`, not
/// `json_nested<T>`. `T` there is a type the user named and scythe knows
/// nothing about -- it may be a `TypedDict`, may describe a JSON array, may
/// not be constructible from a mapping at all -- so psycopg3 must keep
/// assigning the raw decoded value rather than calling a constructor on it.
#[test]
fn psycopg3_leaves_a_user_json_mapping_as_a_raw_assignment() {
    let schema = "CREATE TABLE events (id SERIAL PRIMARY KEY, data JSONB NOT NULL);";
    let queries = "\
-- @name GetEventsTyped
-- @returns :many
-- @json data = EventData
SELECT id, data FROM events;
";

    let code = generate("python-psycopg3", "py", schema, queries).expect("generate");

    assert!(code.contains("GetEventsTypedRow(id=r[0], data=r[1])"), "got:\n{code}");
    assert!(
        !code.contains("EventData._from_json") && !code.contains("EventData(**"),
        "a user-declared @json type must not be constructed; got:\n{code}"
    );
}

/// The degradation guarantee: a backend that does not opt in must produce
/// exactly what it produced before nested-aggregate inference existed. The
/// baseline is a nullable plain-`json` column, which is exactly what the old
/// `json_agg` arm inferred (`TypeInfo::new("json", true)`).
#[test]
fn unopted_backend_output_matches_a_plain_json_baseline() {
    let baseline_schema = format!("{SCHEMA}CREATE TABLE blobs (id INTEGER NOT NULL, payload JSON);\n");
    let baseline_queries = "\
-- @name GetUserOrders
-- @returns :many
SELECT u.id, b.payload AS orders FROM users u JOIN blobs b ON b.id = u.id;

-- @name GetUserOrdersOuter
-- @returns :many
SELECT u.id, b.payload AS orders FROM users u JOIN blobs b ON b.id = u.id;
";

    let nested = generate("java-jdbc", "java", SCHEMA, QUERIES).expect("generate");
    let baseline = generate("java-jdbc", "java", &baseline_schema, baseline_queries).expect("generate");

    // The SQL text embedded in each query function differs by construction;
    // everything else -- type declarations, record fields, the enum block
    // that must NOT appear -- has to match.
    let strip_sql = |code: &str| {
        code.lines()
            .filter(|line| !line.contains("prepareStatement("))
            .collect::<Vec<_>>()
            .join("\n")
    };

    assert_eq!(
        strip_sql(&nested),
        strip_sql(&baseline),
        "java-jdbc does not opt into nested structs, so its output must be identical to the plain-json form"
    );
    assert!(
        !nested.contains("OrderStatus"),
        "the enum is reachable only through a nested struct this backend degraded away, so it must not be \
         emitted; got:\n{nested}"
    );
}

/// Two queries whose names collapse onto the same snake_case stem derive the
/// same nested struct name. Identical shapes deduplicate; different shapes
/// are unresolvable and must fail loudly rather than silently give one query
/// the other's type.
#[test]
fn duplicate_nested_struct_names_with_identical_shapes_emit_one_definition() {
    let queries = "\
-- @name GetUserOrders
-- @returns :many
SELECT json_agg(o.*) AS orders FROM orders o;

-- @name GETUserOrders
-- @returns :many
SELECT json_agg(o.*) AS orders FROM orders o WHERE o.user_id = $1;
";

    let code = generate("rust-sqlx", "rs", SCHEMA, queries).expect("generate");

    assert_eq!(
        code.matches("pub struct GetUserOrdersRowOrders {").count(),
        1,
        "a second identical definition is E0428; got:\n{code}"
    );
}

#[test]
fn duplicate_nested_struct_names_with_different_shapes_are_rejected() {
    let queries = "\
-- @name GetUserOrders
-- @returns :many
SELECT json_agg(o.*) AS orders FROM orders o;

-- @name GETUserOrders
-- @returns :many
SELECT json_agg(u.*) AS orders FROM users u;
";

    let err = generate("rust-sqlx", "rs", SCHEMA, queries).expect_err("should fail");

    assert!(
        err.contains("different row shapes"),
        "expected a name-collision diagnostic, got: {err}"
    );
}
