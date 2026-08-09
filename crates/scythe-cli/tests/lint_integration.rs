//! End-to-end integration tests for `scythe lint`'s auto-inspect feature
//! introduced in sub-phase 1D.
//!
//! Only the DB-free scenario lives here: no `[inspect].database_url`
//! configured, lint output is byte-identical to v0.10 behaviour (i.e. zero
//! inspect findings appended). Runs in any environment, no gating needed.
//!
//! The live-DB scenarios (a working `[inspect].database_url`/`DATABASE_URL`
//! producing real `[inspect]` findings, and a misconfigured one being
//! tolerated) live in `tests/lint_live.rs`, gated behind the `live-tests`
//! feature -- see that file's header for why they were split out (#162).
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
