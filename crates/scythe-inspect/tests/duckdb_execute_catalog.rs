use std::path::{Path, PathBuf};

use scythe_core::catalog::{CatalogObjectName, RelationKind};
use scythe_inspect::execute_duckdb_schema_files;

fn write_schema(directory: &Path, name: &str, sql: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, sql).expect("write schema file");
    path
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
    let tasks = catalog.get_table("app.tasks").expect("tasks table");
    assert_eq!(tasks.columns.len(), 5);
    assert!(tasks.columns[0].primary_key);
    assert!(tasks.columns[1].primary_key);
    assert_eq!(tasks.columns[2].sql_type, "app.status");
    assert!(
        tasks.columns[2]
            .default
            .as_deref()
            .is_some_and(|default| default.contains("open"))
    );
    assert!(
        tasks.columns[4]
            .default
            .as_deref()
            .is_some_and(|default| default.contains("BOOLEAN"))
    );
    assert_eq!(catalog.relation_kind("app.tasks"), Some(RelationKind::Table));
    assert_eq!(catalog.relation_kind("app.open_tasks"), Some(RelationKind::View));
    assert_eq!(
        catalog.relation_name("app.tasks"),
        Some(&CatalogObjectName::qualified("app", "tasks"))
    );
    assert_eq!(catalog.column_raw_sql_type("app.tasks", "note"), Some("VARCHAR"));
    assert_eq!(
        catalog.get_enum("app.status").expect("status enum").values,
        ["open", "closed"]
    );
}

#[test]
fn should_report_engine_file_and_operation_for_invalid_sql() {
    let directory = tempfile::tempdir().expect("temp directory");
    let schema = write_schema(directory.path(), "broken.sql", "CREATE TABL broken (id BIGINT);");

    let error = execute_duckdb_schema_files(std::slice::from_ref(&schema)).expect_err("invalid SQL must fail");
    let rendered = error.to_string();
    assert!(rendered.contains("duckdb"), "{rendered}");
    assert!(rendered.contains(&schema.display().to_string()), "{rendered}");
    assert!(rendered.contains("executing schema DDL"), "{rendered}");
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
