//! Regression test for the mixed-engine `--database-url` verification bug:
//! `scythe check --database-url` used to run *every* `[[sql]]` block's
//! queries through the PostgreSQL wire protocol, even blocks configured for
//! a non-PostgreSQL engine. A MySQL/MSSQL/etc. block in an otherwise
//! PostgreSQL config produced a flood of spurious SC-VER01 errors and made
//! `check` exit non-zero.
//!
//! This test builds a config with one `postgresql` block and one `mysql`
//! block, points `--database-url` at a live PostgreSQL instance, and asserts
//! the mysql block is skipped (with a warning naming it) rather than sent
//! through `tokio-postgres::prepare`, while the postgres block still gets
//! verified normally.
//!
//! Skips silently when `SCYTHE_TEST_DATABASE_URL` is not set, matching the
//! gating pattern used by `lint_integration.rs` and
//! `scythe-inspect/tests/verify_live.rs`.
//!
//! The database-free half of this regression — proving the block-filtering
//! logic itself is correct — lives in
//! `crates/scythe-cli/src/commands/generate.rs` as
//! `partition_verifiable_blocks_*` unit tests.

use std::fs;

use assert_cmd::Command;
use tempfile::TempDir;

/// Write a `scythe.toml` with one `postgresql` block (pointed at a real
/// table, `table_name`, that the caller must create in the live database
/// before invoking `scythe check`) and one `mysql` block that never touches
/// the database.
fn write_mixed_engine_fixture(dir: &TempDir, table_name: &str) -> String {
    let pg_query = format!("-- @name GetUser\n-- @returns :one\nSELECT id, name FROM {table_name} WHERE id = $1;\n");
    let pg_schema = format!("CREATE TABLE {table_name} (id bigint PRIMARY KEY, name text NOT NULL);\n");
    fs::write(dir.path().join("pg_queries.sql"), pg_query).expect("write pg_queries.sql");
    fs::write(dir.path().join("pg_schema.sql"), pg_schema).expect("write pg_schema.sql");

    let mysql_query = "-- @name ListItems\n-- @returns :many\nSELECT id, name FROM items;\n";
    let mysql_schema = "CREATE TABLE items (id INT PRIMARY KEY, name VARCHAR(100) NOT NULL);\n";
    fs::write(dir.path().join("mysql_queries.sql"), mysql_query).expect("write mysql_queries.sql");
    fs::write(dir.path().join("mysql_schema.sql"), mysql_schema).expect("write mysql_schema.sql");

    let config_content = r#"[scythe]
version = "1"

[[sql]]
name = "pg_block"
engine = "postgresql"
schema = ["pg_schema.sql"]
queries = ["pg_queries.sql"]

[[sql]]
name = "mysql_block"
engine = "mysql"
schema = ["mysql_schema.sql"]
queries = ["mysql_queries.sql"]
"#
    .to_string();
    let config_path = dir.path().join("scythe.toml");
    fs::write(&config_path, config_content).expect("write scythe.toml");

    config_path.to_string_lossy().into_owned()
}

/// End-to-end: a mixed-engine config with `--database-url` must verify only
/// the postgres block, skip the mysql block with a named warning, produce no
/// findings attributable to the mysql block, and exit 0.
///
/// Skips silently when `SCYTHE_TEST_DATABASE_URL` is not set.
#[test]
fn check_with_mixed_engine_config_skips_non_postgres_block() {
    let url = match std::env::var("SCYTHE_TEST_DATABASE_URL").ok() {
        Some(u) => u,
        None => {
            eprintln!(
                "check_with_mixed_engine_config_skips_non_postgres_block: skipping (SCYTHE_TEST_DATABASE_URL not set)"
            );
            return;
        }
    };

    let table_name = "check_mixed_engine_users";
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    rt.block_on(async {
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect for setup");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        client
            .batch_execute(&format!(
                "DROP TABLE IF EXISTS {table_name};
                 CREATE TABLE {table_name} (id bigint PRIMARY KEY, name text NOT NULL);"
            ))
            .await
            .expect("seed table");
    });

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_mixed_engine_fixture(&dir, table_name);

    let output = Command::cargo_bin("scythe")
        .expect("scythe binary")
        .args([
            "check",
            "--config",
            &config_path,
            "--database-url",
            &url,
            "--format",
            "json",
        ])
        // The schema/query globs in the fixture are relative paths, so the
        // child process must run with `dir` as its cwd for them to resolve —
        // otherwise both `[[sql]]` blocks silently analyze zero queries and
        // this test would pass without ever exercising verification.
        .current_dir(dir.path())
        .output()
        .expect("spawn scythe check");

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    rt.block_on(async {
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect for teardown");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        client
            .batch_execute(&format!("DROP TABLE IF EXISTS {table_name}"))
            .await
            .ok();
    });

    assert!(
        output.status.success(),
        "a mysql block must never fail `check` just because --database-url is set; \
         exit: {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status.code()
    );

    assert!(
        stderr.contains("mysql_block") && stderr.contains("mysql") && stderr.contains("Skipping database verification"),
        "expected a skip warning naming the mysql block and its engine; stderr: {stderr}"
    );

    assert!(
        stderr.contains("[pg_block] Verifying"),
        "the postgres block must still be verified against the live database; stderr: {stderr}"
    );

    let findings: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("expected valid JSON findings on stdout: {e}\nstdout: {stdout}");
    });
    let findings = findings.as_array().expect("findings must be a JSON array");

    for finding in findings {
        let file = finding.get("file").and_then(|v| v.as_str()).unwrap_or_default();
        assert_ne!(
            file, "mysql_block",
            "no finding may be attributed to the skipped mysql block: {finding:?}"
        );
    }
}
