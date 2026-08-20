use std::path::{Path, PathBuf};

use scythe_core::catalog::{Catalog, CatalogObjectName, Column, GeneratedColumnKind, RelationKind};
use scythe_inspect::execute_duckdb_schema_files;

const SECRET_SENTINEL: &str = "SCYTHE_SECRET_SENTINEL";

fn write_schema(directory: &Path, name: &str, sql: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, sql).expect("write schema file");
    path
}

struct ExpectedColumnMetadata<'a> {
    name: &'a str,
    resolved_type: &'a str,
    raw_type: &'a str,
    default: Option<&'a str>,
    nullable: bool,
    primary_key: bool,
    generated: Option<GeneratedColumnKind>,
}

struct ExpectedRelationMetadata<'a> {
    lookup_name: &'a str,
    qualified_name: CatalogObjectName,
    raw_name: &'a str,
    kind: RelationKind,
    columns: &'a [ExpectedColumnMetadata<'a>],
}

fn assert_catalog_relations(catalog: &Catalog, expected_relations: &[ExpectedRelationMetadata<'_>]) {
    assert_eq!(catalog.tables().count(), expected_relations.len());
    for expected_relation in expected_relations {
        assert_relation(catalog, expected_relation);
    }
}

fn assert_relation(catalog: &Catalog, expected: &ExpectedRelationMetadata<'_>) {
    let relation = catalog
        .get_table(expected.lookup_name)
        .unwrap_or_else(|| panic!("missing relation {}", expected.lookup_name));
    assert_eq!(
        catalog.relation_name(expected.lookup_name),
        Some(&expected.qualified_name)
    );
    assert_eq!(relation.raw_name, expected.raw_name);
    assert_eq!(catalog.relation_kind(expected.lookup_name), Some(expected.kind));
    assert_eq!(relation.columns.len(), expected.columns.len());
    for (column, expected_column) in relation.columns.iter().zip(expected.columns) {
        assert_column(catalog, expected.lookup_name, column, expected_column);
    }
}

fn assert_column(catalog: &Catalog, relation_name: &str, column: &Column, expected: &ExpectedColumnMetadata<'_>) {
    let column_path = format!("{relation_name}.{}", expected.name);
    assert_eq!(column.name, expected.name, "name for {column_path}");
    assert_eq!(
        column.sql_type, expected.resolved_type,
        "resolved type for {column_path}"
    );
    assert_eq!(
        catalog.column_raw_sql_type(relation_name, expected.name),
        Some(expected.raw_type),
        "raw type for {column_path}"
    );
    assert_eq!(column.default.as_deref(), expected.default, "default for {column_path}");
    assert_eq!(column.nullable, expected.nullable, "nullability for {column_path}");
    assert_eq!(
        column.primary_key, expected.primary_key,
        "primary-key flag for {column_path}"
    );
    assert_eq!(
        catalog.column_generated_kind(relation_name, expected.name),
        expected.generated,
        "generated kind for {column_path}"
    );
}

#[test]
fn should_execute_files_in_order_and_preserve_duckdb_catalog_metadata() {
    let directory = tempfile::tempdir().expect("temp directory");
    let first = write_schema(
        directory.path(),
        "001_base.sql",
        "CREATE SCHEMA app;
         CREATE TYPE app.status AS ENUM ('open', 'closed');
         CREATE TABLE app.tasks (
             tenant_id BIGINT NOT NULL,
             task_id BIGINT NOT NULL,
             status app.status NOT NULL DEFAULT 'open',
             note VARCHAR,
             active BOOLEAN NOT NULL DEFAULT true,
             PRIMARY KEY (tenant_id, task_id)
         );",
    );
    let second = write_schema(
        directory.path(),
        "002_view.sql",
        "CREATE VIEW app.open_tasks AS SELECT tenant_id, task_id, status FROM app.tasks WHERE active;",
    );

    let catalog = execute_duckdb_schema_files(&[first, second]).expect("execute schema");
    assert_catalog_relations(
        &catalog,
        &[
            ExpectedRelationMetadata {
                lookup_name: "app.tasks",
                qualified_name: CatalogObjectName::qualified("app", "tasks"),
                raw_name: "tasks",
                kind: RelationKind::Table,
                columns: &[
                    ExpectedColumnMetadata {
                        name: "tenant_id",
                        resolved_type: "bigint",
                        raw_type: "BIGINT",
                        default: None,
                        nullable: false,
                        primary_key: true,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "task_id",
                        resolved_type: "bigint",
                        raw_type: "BIGINT",
                        default: None,
                        nullable: false,
                        primary_key: true,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "status",
                        resolved_type: "app.status",
                        raw_type: "ENUM('open', 'closed')",
                        default: Some("'open'"),
                        nullable: false,
                        primary_key: false,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "note",
                        resolved_type: "varchar",
                        raw_type: "VARCHAR",
                        default: None,
                        nullable: true,
                        primary_key: false,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "active",
                        resolved_type: "boolean",
                        raw_type: "BOOLEAN",
                        default: Some("CAST('t' AS BOOLEAN)"),
                        nullable: false,
                        primary_key: false,
                        generated: None,
                    },
                ],
            },
            ExpectedRelationMetadata {
                lookup_name: "app.open_tasks",
                qualified_name: CatalogObjectName::qualified("app", "open_tasks"),
                raw_name: "open_tasks",
                kind: RelationKind::View,
                columns: &[
                    ExpectedColumnMetadata {
                        name: "tenant_id",
                        resolved_type: "bigint",
                        raw_type: "BIGINT",
                        default: None,
                        nullable: true,
                        primary_key: false,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "task_id",
                        resolved_type: "bigint",
                        raw_type: "BIGINT",
                        default: None,
                        nullable: true,
                        primary_key: false,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "status",
                        resolved_type: "app.status",
                        raw_type: "ENUM('open', 'closed')",
                        default: None,
                        nullable: true,
                        primary_key: false,
                        generated: None,
                    },
                ],
            },
        ],
    );
    assert_eq!(
        catalog.get_enum("app.status").expect("status enum").values,
        ["open", "closed"]
    );
}

#[test]
fn should_report_engine_file_and_operation_for_invalid_sql() {
    let directory = tempfile::tempdir().expect("temp directory");
    let schema = write_schema(
        directory.path(),
        "broken.sql",
        "CREATE TABL broken (secret VARCHAR DEFAULT 'SCYTHE_SECRET_SENTINEL');",
    );

    let error = execute_duckdb_schema_files(std::slice::from_ref(&schema)).expect_err("invalid SQL must fail");
    let rendered = error.to_string();
    assert!(rendered.contains("duckdb"), "{rendered}");
    assert!(rendered.contains(&schema.display().to_string()), "{rendered}");
    assert!(rendered.contains("executing schema DDL"), "{rendered}");
    assert!(rendered.contains("SCHEMA_SQL_REJECTED"), "{rendered}");
    assert!(!rendered.contains(SECRET_SENTINEL), "{rendered}");
    assert!(!rendered.contains("CREATE TABL"), "{rendered}");
}

#[test]
fn should_resolve_identical_label_enums_by_catalog_schema_and_support_arrays() {
    let directory = tempfile::tempdir().expect("temp directory");
    let schema = write_schema(
        directory.path(),
        "enums.sql",
        "CREATE SCHEMA one;
         CREATE SCHEMA two;
         CREATE TYPE one.status AS ENUM ('open', 'closed');
         CREATE TYPE two.status AS ENUM ('open', 'closed');
         CREATE TABLE one.items (direct one.status, many one.status[]);
         CREATE TABLE two.items (direct two.status, many two.status[]);",
    );

    let catalog = execute_duckdb_schema_files(&[schema]).expect("resolve enum identities");
    let one = catalog.get_table("one.items").expect("one.items");
    assert_eq!(one.columns[0].sql_type, "one.status");
    assert_eq!(one.columns[1].sql_type, "one.status[]");
    assert_eq!(
        catalog.column_raw_sql_type("one.items", "direct"),
        Some("ENUM('open', 'closed')")
    );
    assert_eq!(
        catalog.column_raw_sql_type("one.items", "many"),
        Some("ENUM('open', 'closed')[]")
    );

    let two = catalog.get_table("two.items").expect("two.items");
    assert_eq!(two.columns[0].sql_type, "two.status");
    assert_eq!(two.columns[1].sql_type, "two.status[]");
}

#[test]
fn should_preserve_valid_schema_when_enum_identity_is_ambiguous() {
    let directory = tempfile::tempdir().expect("temp directory");
    let schema = write_schema(
        directory.path(),
        "ambiguous.sql",
        "CREATE TYPE status_a AS ENUM ('open', 'closed');
         CREATE TYPE status_b AS ENUM ('open', 'closed');
         CREATE TABLE items (first status_a, second status_b);",
    );

    let catalog = execute_duckdb_schema_files(&[schema]).expect("valid DuckDB schema");
    let table = catalog.get_table("main.items").expect("main.items");
    assert_eq!(table.columns[0].sql_type, "enum('open', 'closed')");
    assert_eq!(table.columns[1].sql_type, "enum('open', 'closed')");
    assert_eq!(
        catalog.column_raw_sql_type("main.items", "first"),
        Some("ENUM('open', 'closed')")
    );
    assert_eq!(
        catalog.column_raw_sql_type("main.items", "second"),
        Some("ENUM('open', 'closed')")
    );
}

#[test]
fn should_disable_external_access_and_extension_loading_before_schema_execution() {
    let directory = tempfile::tempdir().expect("temp directory");
    let output = directory.path().join("escaped.csv");
    let external = write_schema(
        directory.path(),
        "external.sql",
        &format!("COPY (SELECT 1) TO '{}';", output.display()),
    );
    let error = execute_duckdb_schema_files(&[external]).expect_err("external access must be disabled");
    assert!(error.to_string().contains("executing schema DDL"));
    assert!(!output.exists());

    let install = write_schema(directory.path(), "install.sql", "INSTALL httpfs;");
    let error = execute_duckdb_schema_files(&[install]).expect_err("extension installation must be disabled");
    assert!(error.to_string().contains("executing schema DDL"));

    let load = write_schema(directory.path(), "load.sql", "LOAD httpfs;");
    let error = execute_duckdb_schema_files(&[load]).expect_err("extension loading must be disabled");
    assert!(error.to_string().contains("executing schema DDL"));
}

#[test]
fn should_isolate_repeated_and_concurrent_loads() {
    let directory = tempfile::tempdir().expect("temp directory");
    let schema = write_schema(
        directory.path(),
        "schema.sql",
        "CREATE TABLE users (id BIGINT PRIMARY KEY);",
    );

    let first = execute_duckdb_schema_files(std::slice::from_ref(&schema)).expect("first load");
    let second = execute_duckdb_schema_files(std::slice::from_ref(&schema)).expect("second load");
    assert_eq!(first.fingerprint(), second.fingerprint());

    let handles = (0..2)
        .map(|_| {
            let schema = schema.clone();
            std::thread::spawn(move || {
                execute_duckdb_schema_files(&[schema])
                    .expect("concurrent load")
                    .fingerprint()
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        assert_eq!(handle.join().expect("thread did not panic"), first.fingerprint());
    }
}
