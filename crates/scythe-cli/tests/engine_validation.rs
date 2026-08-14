//! Regression tests for #165, item 3: `scythe lint` and `scythe audit` used
//! to accept an unrecognized `[[sql]] engine = "..."` value and silently
//! analyze it as PostgreSQL (`SqlDialect::from_str(&engine).unwrap_or(PostgreSQL)`
//! in `lint_cmd.rs` and `audit.rs`). A typo like `mysql8` produced no
//! diagnostic at all -- the run parsed the schema and queries under the
//! wrong dialect, matched them against the wrong dialect-gated rule set, and
//! reported success regardless. `scythe generate` already rejected the same
//! mistake loudly; `lint`/`audit` did not.
//!
//! Every test drives the compiled binary and checks that an unknown engine
//! now fails the run (exit 1, the "operational failure" code -- not exit 0,
//! and not the exit-2 "findings present" code, since this is a config
//! mistake, not a lint finding) with a message naming the offending value.
//! On the old code every one of these commands exited 0.

use std::fs;
use std::process::{Command, Output};

use tempfile::TempDir;

const SCHEMA: &str = "CREATE TABLE widgets (id bigint PRIMARY KEY, name text NOT NULL);\n";
const QUERIES: &str = "-- @name ListWidgets\n-- @returns :many\nSELECT id, name FROM widgets;\n";

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Write `scythe.toml` + `schema.sql` + `queries.sql` into `dir`, with the
/// given (possibly bogus) `engine`, and return the config path.
fn write_project(dir: &TempDir, engine: &str) -> String {
    fs::write(dir.path().join("schema.sql"), SCHEMA).expect("write schema.sql");
    fs::write(dir.path().join("queries.sql"), QUERIES).expect("write queries.sql");

    let config = format!(
        "[scythe]\nversion = \"1\"\n\n\
         [[sql]]\nname = \"main\"\nengine = \"{engine}\"\n\
         schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n"
    );
    let config_path = dir.path().join("scythe.toml");
    fs::write(&config_path, config).expect("write scythe.toml");
    config_path.to_string_lossy().into_owned()
}

/// `scythe lint --config scythe.toml` (config-resolved mode, `lint_from_config`)
/// must reject an unrecognized `engine`, not silently analyze it as
/// PostgreSQL. `mysql8` is valid-looking SQL under `SCHEMA`/`QUERIES` above
/// regardless of dialect, so on the old code this run parsed and linted
/// clean and exited 0 -- the failure mode this test guards against is
/// exactly that silent success, not a syntax error the wrong dialect happens
/// to trip.
#[test]
fn lint_from_config_rejects_unknown_engine() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(&dir, "mysql8");

    let output = scythe_bin()
        .args(["lint", "--config", &config_path])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("run scythe lint");

    let err = stderr(&output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "an unknown engine is an operational failure (exit 1), not a lint finding (exit 2) or \
         success (exit 0); stderr: {err}"
    );
    assert!(
        err.contains("unknown engine 'mysql8'"),
        "the error must name the offending value; stderr: {err}"
    );
}

/// `scythe lint <file>` (explicit-file mode, `load_native_lint_context`) must
/// reject the same unrecognized `engine` when it builds its native-rule
/// catalog from `scythe.toml`'s first `[[sql]]` block -- a separate code path
/// from `lint_from_config`, with its own `SqlDialect::from_str(..).unwrap_or(..)`
/// before the fix.
#[test]
fn lint_explicit_file_mode_rejects_unknown_engine_from_config() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(&dir, "postgersql");
    let query_path = dir.path().join("queries.sql");

    let output = scythe_bin()
        .args(["lint", "--config", &config_path, query_path.to_str().unwrap()])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("run scythe lint");

    let err = stderr(&output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "explicit-file mode must reject the config's unknown engine exactly like config mode \
         does, not silently skip native-rule construction; stderr: {err}"
    );
    assert!(
        err.contains("unknown engine 'postgersql'"),
        "the error must name the offending value; stderr: {err}"
    );
}

/// `scythe audit --config scythe.toml` (`audit_from_config`) must reject an
/// unrecognized `engine` the same way `lint` does, instead of auditing the
/// block as PostgreSQL and reporting whatever that finds as the whole truth.
#[test]
fn audit_from_config_rejects_unknown_engine() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(&dir, "mysql8");

    let output = scythe_bin()
        .args(["audit", "--config", &config_path])
        .output()
        .expect("run scythe audit");

    let err = stderr(&output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "an unknown engine is an operational failure (exit 1), not a clean audit (exit 0); \
         stderr: {err}"
    );
    assert!(
        err.contains("unknown engine 'mysql8'"),
        "the error must name the offending value; stderr: {err}"
    );
}

/// A recognized alias other than the canonical spelling (`mysql`, not
/// `mysql8`) must keep working -- the fix must add validation, not narrow
/// what `engine` accepts. Checked against the "unknown engine" failure mode
/// specifically (rather than requiring exit 0) so this test does not become
/// coupled to which lint rules happen to be active by default.
#[test]
fn lint_from_config_still_accepts_a_known_engine_alias() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(&dir, "mysql");

    let output = scythe_bin()
        .args(["lint", "--config", &config_path])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("run scythe lint");

    let err = stderr(&output);
    assert_ne!(
        output.status.code(),
        Some(1),
        "a known engine alias must not be rejected as an operational failure; stderr: {err}"
    );
    assert!(
        !err.contains("unknown engine"),
        "a known engine alias must not be reported as unknown; stderr: {err}"
    );
}
