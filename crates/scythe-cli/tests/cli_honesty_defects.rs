//! Regression tests for four "the CLI claims to have checked more than it
//! actually did" defects:
//!
//! 1. `scythe fmt <file>` (explicit-file mode) hardcoded `sqruff_config:
//!    None`, silently dropping a user's `[lint.sqruff]` table -- only the
//!    *dialect* half of #206 was ever fixed.
//! 2. `scythe lint <file>` (explicit-file mode) never constructed a
//!    `LintEngine`, so it ran zero scythe-native rules (SC-*) even when
//!    `scythe.toml`'s schema was sitting right next to the file passed on
//!    the command line.
//! 3. `scythe check` never called `resolve_contained_output`, so it could
//!    report a config clean when `scythe generate` on the very same config
//!    refuses to write outside the project root (#207). Covered by unit
//!    tests inside `crates/scythe-cli/src/commands/generate.rs` (see
//!    `check_reports_a_gen_target_output_that_escapes_the_project_root`),
//!    not here.
//! 4. `scythe fmt --check` signalled "needs formatting" as a plain `Err`,
//!    which fell through to `main`'s generic exit(1) path -- the same code
//!    used for an operational failure (bad config, unreadable file). #212
//!    already draws this line for `lint`/`check` (exit 2 for findings, exit
//!    1 for operational failure); `fmt --check` did not follow it.
//!
//! Every test drives the compiled binary via
//! `Command::new(env!("CARGO_BIN_EXE_scythe"))`.

use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Deliberately inconsistent indentation so sqruff's `LT02` indentation rule
/// reports the file as needing formatting -- the same fixture
/// `config_relative_paths.rs`'s `fmt_check_resolves_globs_relative_to_config_not_cwd`
/// already relies on for the identical purpose.
const UNFORMATTED_SQL: &str = "SELECT\n  id,\n    name\nFROM widgets\nWHERE id = $1;\n";

/// Item 1: `scythe fmt <file>` (files passed explicitly on the command line,
/// not resolved from `scythe.toml`) must reject the same unusable
/// `[lint.sqruff]` configuration that `scythe fmt --config scythe.toml`
/// (files resolved from the config) already rejects.
///
/// `LT02 = "warn"` is rejected by `sqruff_adapter::make_config` regardless
/// of whether `LT02` is a real rule code: the adapter only ever supports
/// `"off"` as a rule value (see `SqruffConfigError::UnsupportedRuleValue`).
///
/// Before the fix, explicit-file mode built its linter with
/// `sqruff_config: None`, so this table was never even looked at and the
/// run succeeded -- silently dropping the user's config instead of
/// reporting the same error `scythe lint` reports for it.
#[test]
fn fmt_explicit_file_honours_invalid_lint_sqruff_config_not_just_dialect() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("t.sql");
    std::fs::write(&sql_path, "SELECT 1;\n").unwrap();

    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\nschema = []\nqueries = [\"t.sql\"]\n\n\
        [lint.sqruff.rules]\nLT02 = \"warn\"\n";
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args([
            "fmt",
            "--config",
            config_path.to_str().unwrap(),
            "--check",
            sql_path.to_str().unwrap(),
        ])
        .output()
        .expect("run scythe fmt");

    let err = stderr(&output);
    assert!(
        !output.status.success(),
        "fmt in explicit-file mode must reject the same invalid [lint.sqruff] config \
         config-file-mode fmt rejects, not silently drop it; stderr: {err}"
    );
    assert!(
        err.contains("invalid [lint.sqruff] configuration"),
        "must be reported as a configuration error, matching config-file mode; stderr: {err}"
    );
}

/// Item 2: `scythe lint <file>` (explicit-file mode) must run scythe-native
/// rules (the `SC-*` rule family from `LintEngine`) when `scythe.toml`'s
/// schema is resolvable, not sqruff rules only.
///
/// `SELECT * FROM widgets` trips `SC-S03` (no-select-star), a warn-severity
/// rule -- chosen so a real finding does not also flip the exit code, which
/// would conflate "did native rules run" with "did anything fail".
///
/// Before the fix, explicit-file mode never constructed a `LintEngine` at
/// all (the old code even said so in a comment), so no `SC-*` finding could
/// ever appear here regardless of what `scythe.toml` declared.
#[test]
fn lint_explicit_file_runs_native_scythe_rules_when_config_schema_is_available() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("schema.sql"),
        "CREATE TABLE widgets (id bigint PRIMARY KEY, name text NOT NULL);\n",
    )
    .unwrap();
    let query_path = dir.path().join("queries.sql");
    std::fs::write(
        &query_path,
        "-- @name ListWidgets\n-- @returns :many\nSELECT * FROM widgets;\n",
    )
    .unwrap();

    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
        schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n";
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args([
            "lint",
            "--config",
            config_path.to_str().unwrap(),
            "--format",
            "json",
            query_path.to_str().unwrap(),
        ])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("run scythe lint");

    let out = stdout(&output);
    let err = stderr(&output);
    assert!(
        out.contains("SC-S03"),
        "explicit-file mode must run scythe-native rules (SC-S03 no-select-star) once a \
         schema is resolvable from scythe.toml, not sqruff rules only; stdout: {out}, stderr: {err}"
    );
}

/// Item 4: `scythe fmt --check` on a file that needs formatting must exit 2
/// (the same "findings present" code `lint`/`check` use), not the generic
/// exit 1 `main` produces for a plain `Err` -- exit 1 must stay reserved for
/// an operational failure.
#[test]
fn fmt_check_needing_formatting_exits_two_not_one() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("t.sql");
    std::fs::write(&sql_path, UNFORMATTED_SQL).unwrap();

    let output = scythe_bin()
        .args(["fmt", "--check", sql_path.to_str().unwrap()])
        .output()
        .expect("run scythe fmt");

    let err = stderr(&output);
    assert!(
        !output.status.success(),
        "fmt --check on a file needing formatting must fail; stderr: {err}"
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "'needs formatting' must be its own exit code (2), distinct from exit 1 (operational \
         failure) -- matching the #212 contract lint/check already follow; stderr: {err}"
    );
}

/// Companion to the above: an operational failure in `fmt --check` (here, an
/// unreadable file) must still be exit 1 -- proving the two codes are
/// actually distinguished by cause, not just uniformly changed.
#[test]
fn fmt_check_operational_failure_still_exits_one() {
    let dir = TempDir::new().unwrap();
    let missing_path = dir.path().join("does-not-exist.sql");

    let output = scythe_bin()
        .args(["fmt", "--check", missing_path.to_str().unwrap()])
        .output()
        .expect("run scythe fmt");

    let err = stderr(&output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "an operational failure (unreadable file) must remain exit 1, distinct from exit 2 \
         for 'needs formatting'; stderr: {err}"
    );
}
