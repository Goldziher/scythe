//! Regression tests for array-typed columns on the JVM backends.
//!
//! The JVM manifests used to map the `array` container to `List<{T}>`, but no
//! JVM backend has an array reader: `java_jdbc`/`kotlin_jdbc`/`kotlin_exposed`
//! read any non-scalar column with `rs.getObject(col)` (static type `Object` /
//! `Any!`) and the two R2DBC backends with `row.get(col, Object.class)` /
//! `row.get(col, Any::class.java)`. Neither is assignable to `List<T>`, so
//! every array column produced a file that `javac`/`kotlinc` rejected --
//! `incompatible types: Object cannot be converted to List<String>` on Java,
//! `argument type mismatch: actual type is 'Any!'` on Kotlin -- and
//! `array<int32>`/`array<bool>` additionally produced `java.util.List<int>`,
//! which is not even valid Java syntax.
//!
//! No integration-test schema declares an array column, and the shared
//! `tool_validation.rs` fixture has none either, so nothing compiled such a
//! file: the breakage was latent for every element type, `array<json>`
//! included. The manifests now declare arrays as the plain text form the
//! string reader actually returns, exactly like the `range` container.

use scythe_codegen::{CodegenBackend, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// Every neutral array spelling reachable from PostgreSQL DDL that used to
/// render as a `List<...>`: a JSON element type, a string one, a boxed-in-name
/// -only primitive one (`int[]` -> `java.util.List<int>`), and a nullable one.
const SCHEMA: &str = "CREATE TABLE events (\
    id INTEGER PRIMARY KEY, \
    payload JSON[] NOT NULL, \
    payload_opt JSONB[], \
    tags TEXT[] NOT NULL, \
    counts INTEGER[] NOT NULL, \
    flags BOOLEAN[] NOT NULL\
);";

const QUERY: &str = "-- @name GetEvent\n-- @returns :one\n\
    SELECT id, payload, payload_opt, tags, counts, flags FROM events WHERE id = $1;";

/// The array columns in [`SCHEMA`], by generated field name.
const ARRAY_FIELDS: [&str; 5] = ["payload", "payload_opt", "tags", "counts", "flags"];

fn generate_row_struct(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    generate_row_struct_with(&*backend)
}

fn generate_row_struct_with(backend: &dyn CodegenBackend) -> String {
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, backend).expect("codegen must succeed");
    code.row_struct.expect("expected a row struct")
}

/// Assert that no array column is declared as a list type. A `List<...>`
/// declaration is precisely what none of the JVM readers can produce.
fn assert_no_list_declaration(backend_name: &str, row_struct: &str) {
    for field in ARRAY_FIELDS {
        for list_type in [
            "List<String>",
            "List<Int>",
            "List<int>",
            "List<Boolean>",
            "List<boolean>",
        ] {
            let declaration = format!("{list_type} {field}");
            assert!(
                !row_struct.contains(&declaration),
                "{backend_name}: array column `{field}` is declared `{list_type}`, which no JVM \
                 reader can produce; got:\n{row_struct}"
            );
            let kotlin_declaration = format!("{field}: {list_type}");
            assert!(
                !row_struct.contains(&kotlin_declaration),
                "{backend_name}: array column `{field}` is declared `{list_type}`, which no JVM \
                 reader can produce; got:\n{row_struct}"
            );
        }
    }
}

/// Assert every array column is both declared as `String` and read with the
/// backend's string accessor -- the declaration and the reader must agree, or
/// the file does not compile.
fn assert_string_reader(backend_name: &str, row_struct: &str, declaration: impl Fn(&str) -> String, reader: &str) {
    for field in ARRAY_FIELDS {
        let declared = declaration(field);
        assert!(
            row_struct.contains(&declared),
            "{backend_name}: expected array column `{field}` declared as `{declared}`; got:\n{row_struct}"
        );
        let read = reader.replace("{col}", field);
        assert!(
            row_struct.contains(&read),
            "{backend_name}: expected array column `{field}` read with `{read}`; got:\n{row_struct}"
        );
    }
}

#[test]
fn java_jdbc_reads_array_columns_as_strings() {
    let row_struct = generate_row_struct("java-jdbc");
    assert_no_list_declaration("java-jdbc", &row_struct);
    assert_string_reader(
        "java-jdbc",
        &row_struct,
        |field| format!("String {field}"),
        "rs.getString(\"{col}\")",
    );
}

#[test]
fn java_r2dbc_reads_array_columns_as_strings() {
    let row_struct = generate_row_struct("java-r2dbc");
    assert_no_list_declaration("java-r2dbc", &row_struct);
    // java-r2dbc puts the reader in the query fn, not the row record, so only
    // the declaration is checked here; the reader is checked below.
    for field in ARRAY_FIELDS {
        let declared = format!("String {field}");
        assert!(
            row_struct.contains(&declared),
            "java-r2dbc: expected array column `{field}` declared as `{declared}`; got:\n{row_struct}"
        );
    }
}

#[test]
fn java_r2dbc_query_fn_reads_array_columns_with_the_string_class() {
    let backend = get_backend("java-r2dbc", "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    let query_fn = code.query_fn.expect("expected a query fn");
    for field in ARRAY_FIELDS {
        let read = format!("row.get(\"{field}\", String.class)");
        assert!(
            query_fn.contains(&read),
            "java-r2dbc: expected array column `{field}` read with `{read}`; got:\n{query_fn}"
        );
    }
}

#[test]
fn kotlin_jdbc_reads_array_columns_as_strings() {
    let row_struct = generate_row_struct("kotlin-jdbc");
    assert_no_list_declaration("kotlin-jdbc", &row_struct);
    for field in ARRAY_FIELDS {
        assert!(
            row_struct.contains(&format!("val {field}: String")),
            "kotlin-jdbc: expected array column `{field}` declared as `String`; got:\n{row_struct}"
        );
    }
}

#[test]
fn kotlin_exposed_reads_array_columns_as_strings() {
    let row_struct = generate_row_struct("kotlin-exposed");
    assert_no_list_declaration("kotlin-exposed", &row_struct);
    for field in ARRAY_FIELDS {
        assert!(
            row_struct.contains(&format!("val {field}: String")),
            "kotlin-exposed: expected array column `{field}` declared as `String`; got:\n{row_struct}"
        );
    }
}

#[test]
fn kotlin_r2dbc_reads_array_columns_as_strings() {
    let row_struct = generate_row_struct("kotlin-r2dbc");
    assert_no_list_declaration("kotlin-r2dbc", &row_struct);
    for field in ARRAY_FIELDS {
        assert!(
            row_struct.contains(&format!("val {field}: String")),
            "kotlin-r2dbc: expected array column `{field}` declared as `String`; got:\n{row_struct}"
        );
    }
}

/// The JDBC-family readers (`java-jdbc`, `kotlin-jdbc`, `kotlin-exposed`) must
/// never fall back to `getObject` for an array column: that is the accessor
/// whose `Object`/`Any!` static type started this.
#[test]
fn jdbc_backends_never_read_array_columns_with_get_object() {
    for backend_name in ["java-jdbc", "kotlin-jdbc", "kotlin-exposed"] {
        let row_struct = generate_row_struct(backend_name);
        for field in ARRAY_FIELDS {
            let get_object = format!("getObject(\"{field}\")");
            assert!(
                !row_struct.contains(&get_object),
                "{backend_name}: array column `{field}` is still read with `{get_object}`, whose \
                 static type is Object/Any; got:\n{row_struct}"
            );
        }
    }
}
