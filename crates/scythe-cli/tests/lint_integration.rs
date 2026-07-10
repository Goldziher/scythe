//! End-to-end integration tests for `scythe lint`'s auto-inspect feature
//! introduced in sub-phase 1D.
//!
//! Two scenarios:
//! 1. No DB URL configured — lint output is byte-identical to v0.10 behaviour
//!    (i.e. zero inspect findings appended). Runs in any environment.
//! 2. A working DB URL — inspect findings appear, marked `[inspect]`.
//!    Gated behind the `SCYTHE_TEST_DATABASE_URL` env var; the tests skip
//!    silently when the env var is absent, matching the `pg_live.rs` pattern.
//!
//! All `env_remove` calls are scoped to the child `assert_cmd::Command`
//! instance, so they affect only the child process and never race other tests
//! running in the same Rust test harness.

use std::fs;

use assert_cmd::Command;
use tempfile::TempDir;

/// Spawn `scythe lint --config <path>` with a clean environment: both
/// `DATABASE_URL` and `SCYTHE_DATABASE_URL` removed from the child's env.
/// Returns the `assert_cmd::assert::Assert` for further assertions.
fn scythe_lint_no_db(config_path: &str) -> assert_cmd::assert::Assert {
    Command::cargo_bin("scythe")
        .expect("scythe binary must exist")
        .args(["lint", "--config", config_path])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .assert()
}

/// Write a minimal `scythe.toml` with a `[[sql]]` block pointing at a
/// benign SQL fixture that does NOT trigger any lint rules.
///
/// Returns the `TempDir` that owns the files (must be kept alive).
fn write_benign_fixture(dir: &TempDir) -> String {
    let sql_content = "-- @name GetUser\nSELECT id, name FROM users WHERE id = $1;\n";
    let schema_content = "CREATE TABLE users (id bigint PRIMARY KEY, name text NOT NULL);\n";

    let sql_path = dir.path().join("queries.sql");
    let schema_path = dir.path().join("schema.sql");
    fs::write(&sql_path, sql_content).expect("write queries.sql");
    fs::write(&schema_path, schema_content).expect("write schema.sql");

    let config_content = r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]
"#
    .to_string();
    let config_path = dir.path().join("scythe.toml");
    fs::write(&config_path, config_content).expect("write scythe.toml");

    config_path.to_string_lossy().into_owned()
}

/// Write a `scythe.toml` that includes an `[inspect]` block with the given
/// `database_url`.
fn write_fixture_with_inspect_url(dir: &TempDir, database_url: &str) -> String {
    let sql_content = "-- @name GetUser\nSELECT id, name FROM users WHERE id = $1;\n";
    let schema_content = "CREATE TABLE users (id bigint PRIMARY KEY, name text NOT NULL);\n";

    let sql_path = dir.path().join("queries.sql");
    let schema_path = dir.path().join("schema.sql");
    fs::write(&sql_path, sql_content).expect("write queries.sql");
    fs::write(&schema_path, schema_content).expect("write schema.sql");

    let config_content = format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[inspect]
database_url = "{database_url}"
"#
    );
    let config_path = dir.path().join("scythe.toml");
    fs::write(&config_path, config_content).expect("write scythe.toml");

    config_path.to_string_lossy().into_owned()
}

/// When neither `DATABASE_URL` nor `SCYTHE_DATABASE_URL` is set, and the
/// `scythe.toml` has no `[inspect].database_url`, `scythe lint` must:
/// - Exit 0 (no violations in the benign fixture).
/// - Emit "No lint violations found." on stderr.
/// - Emit NO line containing `[inspect]` or `SC-INS` on stdout or stderr.
#[test]
fn lint_without_db_url_emits_zero_inspect_findings() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_benign_fixture(&dir);

    let assert = scythe_lint_no_db(&config_path);

    let output = assert.get_output().clone();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "lint on a clean fixture must exit 0; stderr: {stderr}"
    );

    assert!(
        stderr.contains("No lint violations found."),
        "expected 'No lint violations found.' on stderr; got: {stderr}"
    );

    for line in stderr.lines().chain(stdout.lines()) {
        assert!(
            !line.contains("[inspect]"),
            "unexpected [inspect] tag without a DB URL; line: {line:?}"
        );
        assert!(
            !line.contains("SC-INS"),
            "unexpected SC-INS finding without a DB URL; line: {line:?}"
        );
    }
}

/// When `scythe.toml` contains `[inspect].database_url` pointing at a live PG
/// with a table missing a primary key (SC-INS04), `scythe lint` must emit a
/// finding tagged `[inspect]` containing `SC-INS04`.
///
/// Skips silently when `SCYTHE_TEST_DATABASE_URL` is not set.
#[test]
fn lint_with_db_url_emits_inspect_findings() {
    let url = match std::env::var("SCYTHE_TEST_DATABASE_URL").ok() {
        Some(u) => u,
        None => {
            eprintln!("lint_with_db_url_emits_inspect_findings: skipping (SCYTHE_TEST_DATABASE_URL not set)");
            return;
        }
    };

    let schema_name = "lint_integ_test_t2";
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
                "DROP SCHEMA IF EXISTS {schema_name} CASCADE;
                 CREATE SCHEMA {schema_name};
                 CREATE TABLE {schema_name}.nopk (col text);"
            ))
            .await
            .expect("seed schema");
    });

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_fixture_with_inspect_url(&dir, &url);

    let output = Command::cargo_bin("scythe")
        .expect("scythe binary")
        .args(["lint", "--config", &config_path])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("spawn scythe lint");

    let stderr = String::from_utf8_lossy(&output.stderr);

    rt.block_on(async {
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect for teardown");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        client
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema_name} CASCADE"))
            .await
            .ok();
    });

    assert!(
        stderr.contains("[inspect]"),
        "expected [inspect] tag in lint output when DB is configured; stderr: {stderr}"
    );
    assert!(
        stderr.contains("SC-INS04"),
        "expected SC-INS04 (no-primary-key) finding; stderr: {stderr}"
    );
    assert!(
        stderr.contains(schema_name),
        "finding message must reference the seeded schema {schema_name}; stderr: {stderr}"
    );
}

/// Same as Test 2 but the URL is passed via the `DATABASE_URL` environment
/// variable rather than `[inspect].database_url` in the config.  Verifies the
/// env-var precedence path end-to-end.
///
/// Skips silently when `SCYTHE_TEST_DATABASE_URL` is not set.
#[test]
fn lint_with_db_url_via_env_var_emits_inspect_findings() {
    let url = match std::env::var("SCYTHE_TEST_DATABASE_URL").ok() {
        Some(u) => u,
        None => {
            eprintln!(
                "lint_with_db_url_via_env_var_emits_inspect_findings: skipping (SCYTHE_TEST_DATABASE_URL not set)"
            );
            return;
        }
    };

    let schema_name = "lint_integ_test_t3";
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
                "DROP SCHEMA IF EXISTS {schema_name} CASCADE;
                 CREATE SCHEMA {schema_name};
                 CREATE TABLE {schema_name}.nopk (col text);"
            ))
            .await
            .expect("seed schema");
    });

    let dir = TempDir::new().expect("tempdir");
    let config_path = write_benign_fixture(&dir);

    let output = Command::cargo_bin("scythe")
        .expect("scythe binary")
        .args(["lint", "--config", &config_path])
        .env("DATABASE_URL", &url)
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("spawn scythe lint");

    let stderr = String::from_utf8_lossy(&output.stderr);

    rt.block_on(async {
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("connect for teardown");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        client
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema_name} CASCADE"))
            .await
            .ok();
    });

    assert!(
        stderr.contains("[inspect]"),
        "expected [inspect] tag when DATABASE_URL is set; stderr: {stderr}"
    );
    assert!(
        stderr.contains("SC-INS04"),
        "expected SC-INS04 (no-primary-key) finding via DATABASE_URL; stderr: {stderr}"
    );
    assert!(
        stderr.contains(schema_name),
        "finding must reference schema {schema_name}; stderr: {stderr}"
    );
}

/// When `DATABASE_URL` points at a non-existent host, `scythe lint` must:
/// - Exit 0 (benign fixture, no lint/audit violations).
/// - Emit NO `[inspect]` tag (connection was skipped silently after warn).
/// - Never exit with code 2 due to the failed connection alone.
///
/// Skips silently when `SCYTHE_TEST_DATABASE_URL` is not set (we use that as
/// a proxy for "a live-capable environment").
#[test]
fn lint_with_misconfigured_db_url_does_not_fail() {
    if std::env::var("SCYTHE_TEST_DATABASE_URL").is_err() {
        eprintln!("lint_with_misconfigured_db_url_does_not_fail: skipping (SCYTHE_TEST_DATABASE_URL not set)");
        return;
    }

    let bad_url = "postgres://does-not-exist:1/x";
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_benign_fixture(&dir);

    let output = Command::cargo_bin("scythe")
        .expect("scythe binary")
        .args(["lint", "--config", &config_path])
        .env("DATABASE_URL", bad_url)
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("spawn scythe lint");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code();

    assert!(
        exit_code != Some(2),
        "misconfigured DB URL must not cause exit 2; exit: {exit_code:?}; stderr: {stderr}"
    );

    for line in stderr.lines() {
        assert!(
            !line.contains("[inspect]"),
            "no [inspect] tag expected with bad DB URL; line: {line:?}"
        );
        assert!(
            !line.contains("SC-INS"),
            "no SC-INS finding expected with bad DB URL; line: {line:?}"
        );
    }
}
