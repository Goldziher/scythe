//! Live-PG integration tests for `scythe lint`'s auto-inspect feature —
//! only run when the `live-tests` feature is enabled AND
//! `$SCYTHE_TEST_DATABASE_URL` is set. Split out of `lint_integration.rs`
//! (#162): those three tests used to be gated behind
//! `$SCYTHE_TEST_DATABASE_URL` alone, printing `ok` and returning early when
//! it was unset, with no CI job anywhere setting it for `scythe-cli` — so
//! they ran zero times while reporting success in every run. Living in a
//! dedicated `_live.rs` file, gated on the `live-tests` feature the same way
//! `scythe-inspect`'s `pg_live.rs`/`verify_live.rs`/`schema_diff_live.rs` are,
//! means the default `cargo test -p scythe-cli` doesn't even compile them —
//! there is no env var to silently skip past.
//!
//! The DB-free scenario (no `[inspect].database_url`, zero inspect findings)
//! stays in `lint_integration.rs`: it runs in any environment and needs no
//! feature gate.
//!
//! All `env_remove`/`env` calls are scoped to the child `assert_cmd::Command`
//! instance, so they affect only the child process and never race other
//! tests running in the same Rust test harness.

#![cfg(feature = "live-tests")]

use std::fs;

use assert_cmd::Command;
use tempfile::TempDir;

fn url() -> String {
    std::env::var("SCYTHE_TEST_DATABASE_URL").expect(
        "SCYTHE_TEST_DATABASE_URL must be set for live-tests (e.g. \
         postgres://postgres:postgres@localhost/postgres)",
    )
}

/// Write a minimal `scythe.toml` with a `[[sql]]` block pointing at a
/// benign SQL fixture that does NOT trigger any lint rules.
///
/// Returns the `TempDir` that owns the files (must be kept alive).
fn write_benign_fixture(dir: &TempDir) -> String {
    let sql_content = "-- @name GetUser\n-- @returns :one\nSELECT id, name FROM users WHERE id = $1;\n";
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
    let sql_content = "-- @name GetUser\n-- @returns :one\nSELECT id, name FROM users WHERE id = $1;\n";
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

/// When `scythe.toml` contains `[inspect].database_url` pointing at a live PG
/// with a table missing a primary key (SC-INS04), `scythe lint` must emit a
/// finding tagged `[inspect]` containing `SC-INS04`.
#[test]
fn lint_with_db_url_emits_inspect_findings() {
    let url = url();

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

/// The inverse of the test above: a bare `DATABASE_URL` must NOT be enough to
/// make `scythe lint` open a database connection.
///
/// This is the #210 contract. Before it, `lint` connected purely because the
/// variable happened to be set — ambient network I/O from a linter, with no
/// flag requesting it and no visible diagnostic either way. A URL now has to
/// arrive through `--database-url` or `[inspect].database_url`.
///
/// The seeded `nopk` table is what gives this test teeth: the URL is live and
/// SC-INS04 is genuinely there to be found, so the absence of any `[inspect]`
/// output can only mean `lint` declined to connect. Asserting a negative
/// against a URL that could not have produced a finding anyway would pass for
/// the wrong reason.
#[test]
fn lint_does_not_connect_on_a_bare_database_url_env_var() {
    let url = url();

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
        !stderr.contains("[inspect]"),
        "a bare DATABASE_URL must not make lint connect (#210); stderr: {stderr}"
    );
    assert!(
        !stderr.contains("SC-INS04"),
        "SC-INS04 is reachable at this URL, so reporting it proves lint connected \
         without being asked to (#210); stderr: {stderr}"
    );
    assert!(
        !stderr.contains(schema_name),
        "no output may reference the seeded schema {schema_name}; stderr: {stderr}"
    );
}

/// When `DATABASE_URL` points at a non-existent host, `scythe lint` must:
/// - Exit 0 (benign fixture, no lint/audit violations).
/// - Emit NO `[inspect]` tag (connection was skipped silently after warn).
/// - Never exit with code 2 due to the failed connection alone.
#[test]
fn lint_with_misconfigured_db_url_does_not_fail() {
    // Only proves the code path is reachable in a live-capable environment --
    // this test never actually connects to `url()`, but `live-tests` plus a
    // reachable database is still what gates it, matching the other two.
    let _ = url();

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
