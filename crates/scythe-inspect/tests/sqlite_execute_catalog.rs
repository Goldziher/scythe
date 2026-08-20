use std::path::{Path, PathBuf};

use scythe_core::catalog::{GeneratedColumnKind, RelationKind};
use scythe_inspect::execute_sqlite_schema_files;

const SECRET_SENTINEL: &str = "SCYTHE_SECRET_SENTINEL";

fn write_schema(directory: &Path, name: &str, sql: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, sql).expect("write schema file");
    path
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
    let membership = catalog.get_table("membership").expect("membership table");
    assert_eq!(membership.columns.len(), 5);
    assert!(membership.columns[0].primary_key);
    assert!(membership.columns[1].primary_key);
    assert_eq!(membership.columns[2].default.as_deref(), Some("'guest'"));
    assert_eq!(membership.columns[4].default.as_deref(), Some("1"));
    assert_eq!(catalog.relation_kind("membership"), Some(RelationKind::Table));
    assert_eq!(catalog.relation_kind("active_membership"), Some(RelationKind::View));
    assert_eq!(catalog.column_raw_sql_type("membership", "nickname"), Some("TEXT"));
    assert_eq!(
        catalog.column_generated_kind("membership", "slug"),
        Some(GeneratedColumnKind::Stored)
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
    let loose = catalog.get_table("loose").expect("loose table");
    assert_eq!(loose.columns[0].sql_type, "blob");
    assert_eq!(catalog.column_raw_sql_type("loose", "value"), Some(""));
    assert_eq!(
        catalog.column_generated_kind("loose", "doubled"),
        Some(GeneratedColumnKind::Virtual)
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
