//! End-to-end integration tests for `scythe lint`'s auto-inspect feature
//! introduced in sub-phase 1D, and for #210's fix to how it decides whether
//! to run at all.
//!
//! Scenarios:
//! 1. No DB URL configured — lint output is byte-identical to v0.10 behaviour
//!    (i.e. zero inspect findings appended). Runs in any environment.
//! 2. A bare `$DATABASE_URL`/`$SCYTHE_DATABASE_URL` env var, with no
//!    `--database-url` flag and no `[inspect].database_url` in the config —
//!    ignored entirely; `lint` must never even attempt a connection (#210).
//!    Runs in any environment, using an unreachable `.invalid` host.
//! 3. An explicit `--database-url` flag or a configured
//!    `[inspect].database_url` pointed at an unreachable host — connection
//!    failure is reported visibly (no `tracing` subscriber is installed, so
//!    the diagnostic must not rely on one) without failing the run. Runs in
//!    any environment.
//! 4. A working, configured `[inspect].database_url` against a real
//!    Postgres — inspect findings appear, marked `[inspect]`. Gated behind
//!    the `SCYTHE_TEST_DATABASE_URL` env var; the test skips silently when
//!    the env var is absent, matching the `pg_live.rs` pattern.
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

/// #210: `scythe lint` must NOT auto-connect purely because `$DATABASE_URL`
/// happens to be set in the environment -- unlike the old behaviour (which
/// this test used to verify the opposite of), a bare env var is no longer
/// enough on its own. Only an explicit `--database-url` flag or
/// `[inspect].database_url` in `scythe.toml` opts in.
///
/// Deliberately does not need a live database (or `SCYTHE_TEST_DATABASE_URL`):
/// the point is that `lint` must never even attempt a connection here, so an
/// unreachable host is sufficient to prove it, and the test runs in any
/// environment.
#[test]
fn lint_ignores_bare_database_url_env_var_for_auto_inspect() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_benign_fixture(&dir);

    let output = Command::cargo_bin("scythe")
        .expect("scythe binary")
        .args(["lint", "--config", &config_path])
        .env("DATABASE_URL", "postgres://does-not-exist.invalid:1/x")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("spawn scythe lint");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "a bare (ignored) DATABASE_URL must not affect a clean lint run; stderr: {stderr}"
    );

    for line in stderr.lines().chain(stdout.lines()) {
        assert!(
            !line.contains("[inspect]"),
            "no [inspect] tag expected -- a bare env var must not trigger auto-inspect; line: {line:?}"
        );
        assert!(
            !line.contains("could not connect"),
            "lint must never attempt a connection from a bare env var alone; line: {line:?}"
        );
    }
}

/// The explicit `--database-url` flag is the opt-in `--database-url` (bare
/// env vars are not, per the test above) -- and a connection failure through
/// it must be visible (not silently swallowed) while still not failing the
/// run. Uses a deliberately unreachable host, so this needs no live database.
#[test]
fn lint_with_explicit_database_url_flag_reports_failed_connection_and_does_not_fail() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_benign_fixture(&dir);
    let bad_url = "postgres://does-not-exist.invalid:1/x";

    let output = Command::cargo_bin("scythe")
        .expect("scythe binary")
        .args(["lint", "--config", &config_path, "--database-url", bad_url])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("spawn scythe lint");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code();

    assert!(
        exit_code != Some(2),
        "a failed inspect connection alone must not cause exit 2; exit: {exit_code:?}; stderr: {stderr}"
    );
    assert!(
        stderr.contains("could not connect"),
        "the explicit --database-url path must report a visible diagnostic on connection failure \
         (no tracing subscriber is installed, so a tracing::warn! alone would be invisible); stderr: {stderr}"
    );
    for line in stderr.lines() {
        assert!(
            !line.contains("[inspect]"),
            "no [inspect] finding expected from a failed connection; line: {line:?}"
        );
    }
}

/// Same as the explicit-flag case above, but the URL comes from
/// `[inspect].database_url` in `scythe.toml` -- the other opt-in source
/// `try_run_inspect` still honours. Also needs no live database.
#[test]
fn lint_with_configured_inspect_url_reports_failed_connection_and_does_not_fail() {
    let dir = TempDir::new().expect("tempdir");
    let config_path = write_fixture_with_inspect_url(&dir, "postgres://does-not-exist.invalid:1/x");

    let output = Command::cargo_bin("scythe")
        .expect("scythe binary")
        .args(["lint", "--config", &config_path])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("spawn scythe lint");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code();

    assert!(
        exit_code != Some(2),
        "a failed inspect connection alone must not cause exit 2; exit: {exit_code:?}; stderr: {stderr}"
    );
    assert!(
        stderr.contains("could not connect"),
        "a configured [inspect].database_url must still report a visible diagnostic on connection \
         failure; stderr: {stderr}"
    );
}

/// A bad `[lint.sqruff.rules]` entry must be reported as the configuration
/// error it is, naming the `[[sql]]` block — not as an error against whichever
/// query file happened to be read first.
///
/// Regression guard for the blast radius introduced when sqruff config errors
/// became fatal: the per-file `lint_sql` call sits ahead of the native rule
/// engine in the same loop, so a lazy config error aborted the whole run and
/// discarded every scythe-native finding, security rules included.
#[test]
fn invalid_sqruff_rule_value_fails_as_a_config_error() {
    let dir = TempDir::new().expect("create temp dir");
    let config_path = write_fixture_with_sqruff_rule(&dir, "LT02", "warn");

    let assert = scythe_lint_no_db(&config_path).failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();

    assert!(
        stderr.contains("invalid [lint.sqruff] configuration"),
        "must report a configuration error, got: {stderr}"
    );
    assert!(
        stderr.contains("[test]"),
        "must name the offending [[sql]] block, got: {stderr}"
    );
    assert!(
        stderr.contains("LT02") && stderr.contains("warn"),
        "must name the offending rule and value, got: {stderr}"
    );
    assert!(
        !stderr.contains("queries.sql"),
        "must not blame a query file for a config error, got: {stderr}"
    );
}

/// Same guard for a typo'd rule code, which sqruff itself rejects.
#[test]
fn unknown_sqruff_rule_code_fails_as_a_config_error() {
    let dir = TempDir::new().expect("create temp dir");
    let config_path = write_fixture_with_sqruff_rule(&dir, "LT0", "off");

    let assert = scythe_lint_no_db(&config_path).failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();

    assert!(
        stderr.contains("invalid [lint.sqruff] configuration"),
        "must report a configuration error, got: {stderr}"
    );
    assert!(
        !stderr.contains("queries.sql"),
        "must not blame a query file for a config error, got: {stderr}"
    );
}

/// `enabled = false` must not validate `[lint.sqruff.rules]` at all — the
/// lint paths never reach sqruff, so a stale rules table is inert rather than
/// fatal. The native rule engine must still run and report.
#[test]
fn disabled_sqruff_ignores_an_otherwise_fatal_rules_table() {
    let dir = TempDir::new().expect("create temp dir");
    let sql_content = "-- @name SelectStar\n-- @returns :many\nSELECT * FROM users;\n";
    let schema_content = "CREATE TABLE users (id bigint PRIMARY KEY, name text NOT NULL);\n";
    fs::write(dir.path().join("queries.sql"), sql_content).expect("write queries.sql");
    fs::write(dir.path().join("schema.sql"), schema_content).expect("write schema.sql");

    let config_content = r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[lint.sqruff]
enabled = false

[lint.sqruff.rules]
LT0 = "off"
"#;
    let config_path = dir.path().join("scythe.toml");
    fs::write(&config_path, config_content).expect("write scythe.toml");

    let assert = scythe_lint_no_db(&config_path.to_string_lossy());
    let output = assert.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        stderr.contains("SC-S03"),
        "native rules must still run when sqruff is disabled, got: {stderr}"
    );
    assert!(
        !stderr.contains("invalid [lint.sqruff] configuration"),
        "a rules table must be inert, not fatal, when sqruff is disabled: {stderr}"
    );
}

/// Write a `scythe.toml` carrying a single `[lint.sqruff.rules]` entry, over a
/// query that trips a native rule so the test can tell "config rejected" apart
/// from "nothing to report".
fn write_fixture_with_sqruff_rule(dir: &TempDir, rule: &str, value: &str) -> String {
    let sql_content = "-- @name SelectStar\n-- @returns :many\nSELECT * FROM users;\n";
    let schema_content = "CREATE TABLE users (id bigint PRIMARY KEY, name text NOT NULL);\n";

    fs::write(dir.path().join("queries.sql"), sql_content).expect("write queries.sql");
    fs::write(dir.path().join("schema.sql"), schema_content).expect("write schema.sql");

    let config_content = format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[lint.sqruff.rules]
{rule} = "{value}"
"#
    );
    let config_path = dir.path().join("scythe.toml");
    fs::write(&config_path, config_content).expect("write scythe.toml");

    config_path.to_string_lossy().into_owned()
}
