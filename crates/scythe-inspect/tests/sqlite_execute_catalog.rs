use std::path::{Path, PathBuf};

use scythe_core::catalog::{Catalog, CatalogObjectName, Column, GeneratedColumnKind, RelationKind};
use scythe_inspect::execute_sqlite_schema_files;

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
fn should_execute_files_in_order_and_preserve_sqlite_catalog_metadata() {
    let directory = tempfile::tempdir().expect("temp directory");
    let first = write_schema(
        directory.path(),
        "001_base.sql",
        "CREATE TABLE membership (
            org_id INTEGER NOT NULL,
            user_id INTEGER NOT NULL,
            nickname TEXT DEFAULT 'guest',
            slug TEXT GENERATED ALWAYS AS (org_id || '-' || user_id) STORED,
            PRIMARY KEY (org_id, user_id)
        );
        CREATE TABLE counted (id INTEGER PRIMARY KEY AUTOINCREMENT);",
    );
    let second = write_schema(
        directory.path(),
        "002_view.sql",
        "ALTER TABLE membership ADD COLUMN active INTEGER NOT NULL DEFAULT 1;
         CREATE VIEW active_membership AS SELECT org_id, user_id FROM membership WHERE active = 1;",
    );

    let catalog = execute_sqlite_schema_files(&[first, second]).expect("execute schema");
    assert_catalog_relations(
        &catalog,
        &[
            ExpectedRelationMetadata {
                lookup_name: "membership",
                qualified_name: CatalogObjectName::new("membership"),
                raw_name: "membership",
                kind: RelationKind::Table,
                columns: &[
                    ExpectedColumnMetadata {
                        name: "org_id",
                        resolved_type: "integer",
                        raw_type: "INTEGER",
                        default: None,
                        nullable: false,
                        primary_key: true,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "user_id",
                        resolved_type: "integer",
                        raw_type: "INTEGER",
                        default: None,
                        nullable: false,
                        primary_key: true,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "nickname",
                        resolved_type: "text",
                        raw_type: "TEXT",
                        default: Some("'guest'"),
                        nullable: true,
                        primary_key: false,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "slug",
                        resolved_type: "text",
                        raw_type: "TEXT",
                        default: None,
                        nullable: true,
                        primary_key: false,
                        generated: Some(GeneratedColumnKind::Stored),
                    },
                    ExpectedColumnMetadata {
                        name: "active",
                        resolved_type: "integer",
                        raw_type: "INTEGER",
                        default: Some("1"),
                        nullable: false,
                        primary_key: false,
                        generated: None,
                    },
                ],
            },
            ExpectedRelationMetadata {
                lookup_name: "counted",
                qualified_name: CatalogObjectName::new("counted"),
                raw_name: "counted",
                kind: RelationKind::Table,
                columns: &[ExpectedColumnMetadata {
                    name: "id",
                    resolved_type: "integer",
                    raw_type: "INTEGER",
                    default: None,
                    nullable: false,
                    primary_key: true,
                    generated: None,
                }],
            },
            ExpectedRelationMetadata {
                lookup_name: "active_membership",
                qualified_name: CatalogObjectName::new("active_membership"),
                raw_name: "active_membership",
                kind: RelationKind::View,
                columns: &[
                    ExpectedColumnMetadata {
                        name: "org_id",
                        resolved_type: "integer",
                        raw_type: "INTEGER",
                        default: None,
                        nullable: true,
                        primary_key: false,
                        generated: None,
                    },
                    ExpectedColumnMetadata {
                        name: "user_id",
                        resolved_type: "integer",
                        raw_type: "INTEGER",
                        default: None,
                        nullable: true,
                        primary_key: false,
                        generated: None,
                    },
                ],
            },
        ],
    );
    assert!(!catalog.tables().any(|name| name.starts_with("sqlite_")));
}

#[test]
fn should_support_typeless_and_virtual_generated_columns() {
    let directory = tempfile::tempdir().expect("temp directory");
    let schema = write_schema(
        directory.path(),
        "schema.sql",
        "CREATE TABLE loose (
            value,
            doubled INTEGER GENERATED ALWAYS AS (value * 2) VIRTUAL
        );",
    );

    let catalog = execute_sqlite_schema_files(&[schema]).expect("execute schema");
    assert_catalog_relations(
        &catalog,
        &[ExpectedRelationMetadata {
            lookup_name: "loose",
            qualified_name: CatalogObjectName::new("loose"),
            raw_name: "loose",
            kind: RelationKind::Table,
            columns: &[
                ExpectedColumnMetadata {
                    name: "value",
                    resolved_type: "blob",
                    raw_type: "",
                    default: None,
                    nullable: true,
                    primary_key: false,
                    generated: None,
                },
                ExpectedColumnMetadata {
                    name: "doubled",
                    resolved_type: "integer",
                    raw_type: "INTEGER",
                    default: None,
                    nullable: true,
                    primary_key: false,
                    generated: Some(GeneratedColumnKind::Virtual),
                },
            ],
        }],
    );
}

#[test]
fn should_report_engine_file_and_operation_for_invalid_sql() {
    let directory = tempfile::tempdir().expect("temp directory");
    let schema = write_schema(
        directory.path(),
        "broken.sql",
        "CREATE TABL broken (secret TEXT DEFAULT 'SCYTHE_SECRET_SENTINEL');",
    );

    let error = execute_sqlite_schema_files(std::slice::from_ref(&schema)).expect_err("invalid SQL must fail");
    let rendered = error.to_string();
    assert!(rendered.contains("sqlite"), "{rendered}");
    assert!(rendered.contains(&schema.display().to_string()), "{rendered}");
    assert!(rendered.contains("executing schema DDL"), "{rendered}");
    assert!(rendered.contains("SCHEMA_SQL_REJECTED"), "{rendered}");
    assert!(!rendered.contains(SECRET_SENTINEL), "{rendered}");
    assert!(!rendered.contains("CREATE TABL"), "{rendered}");
}

#[test]
fn should_isolate_repeated_and_concurrent_loads() {
    let directory = tempfile::tempdir().expect("temp directory");
    let schema = write_schema(
        directory.path(),
        "schema.sql",
        "CREATE TABLE users (id INTEGER PRIMARY KEY);",
    );

    let first = execute_sqlite_schema_files(std::slice::from_ref(&schema)).expect("first load");
    let second = execute_sqlite_schema_files(std::slice::from_ref(&schema)).expect("second load");
    assert_eq!(first.fingerprint(), second.fingerprint());

    let handles = (0..4)
        .map(|_| {
            let schema = schema.clone();
            std::thread::spawn(move || {
                execute_sqlite_schema_files(&[schema])
                    .expect("concurrent load")
                    .fingerprint()
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        assert_eq!(handle.join().expect("thread did not panic"), first.fingerprint());
    }
}
