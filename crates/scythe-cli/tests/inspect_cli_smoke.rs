//! CLI smoke tests for `scythe inspect`.
//!
//! All tests here are DB-free — they exercise flags that return without
//! connecting to a database (`--list-checks`, `--explain`, `--help`).
//! Live-database tests live in `crates/scythe-inspect/tests/pg_live.rs`.

use assert_cmd::Command;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn scythe() -> Command {
    Command::cargo_bin("scythe").expect("scythe binary must exist")
}

// ---------------------------------------------------------------------------
// 1. list_checks_prints_thirteen_rows
// ---------------------------------------------------------------------------

/// `scythe inspect --list-checks` must exit 0 and list all 13 canonical
/// SC-INS* checks.
#[test]
fn list_checks_prints_thirteen_rows() {
    let assert = scythe().args(["inspect", "--list-checks"]).assert().success();

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();

    // All 13 canonical IDs must be present.
    for id in [
        "SC-INS01", "SC-INS02", "SC-INS03", "SC-INS04", "SC-INS05", "SC-INS06", "SC-INS07", "SC-INS08", "SC-INS09",
        "SC-INS10", "SC-INS11", "SC-INS12", "SC-INS13",
    ] {
        assert!(
            stdout.contains(id),
            "--list-checks must include {id}; stdout:\n{stdout}"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. list_checks_with_dialect_mysql_says_no_checks
// ---------------------------------------------------------------------------

/// `scythe inspect --list-checks --dialect mysql` — exits 0 and emits the
/// "no checks available" message (MySQL checks are stubbed in Phase 1).
#[test]
fn list_checks_with_dialect_mysql_says_no_checks() {
    let assert = scythe()
        .args(["inspect", "--list-checks", "--dialect", "mysql"])
        .assert()
        .success();

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();

    // The output must either be empty or explicitly say no checks are available.
    // Current impl emits a "no checks available for engine `mysql`" message.
    assert!(
        stdout.contains("no checks") || stdout.trim().is_empty(),
        "--list-checks --dialect mysql must say 'no checks' or produce empty output; got:\n{stdout}"
    );

    // No SC-INS* checks must appear under the mysql dialect.
    assert!(
        !stdout.contains("SC-INS"),
        "mysql dialect must not list postgres-only SC-INS checks; got:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// 3. explain_known_id_prints_body
// ---------------------------------------------------------------------------

/// `scythe inspect --explain SC-INS04` must exit 0 and print the check name,
/// Explanation section, and Remediation section.
#[test]
fn explain_known_id_prints_body() {
    let assert = scythe().args(["inspect", "--explain", "SC-INS04"]).assert().success();

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();

    assert!(
        stdout.contains("no-primary-key"),
        "--explain SC-INS04 must include check name 'no-primary-key'; got:\n{stdout}"
    );
    assert!(
        stdout.contains("Explanation"),
        "--explain SC-INS04 must include 'Explanation' section; got:\n{stdout}"
    );
    assert!(
        stdout.contains("Remediation"),
        "--explain SC-INS04 must include 'Remediation' section; got:\n{stdout}"
    );
    assert!(
        stdout.contains("SC-INS04"),
        "--explain SC-INS04 must include the check id; got:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// 4. explain_unknown_id_exits_nonzero
// ---------------------------------------------------------------------------

/// `scythe inspect --explain SC-NOPE` must exit ≠ 0 and stderr must mention
/// the unknown id.
#[test]
fn explain_unknown_id_exits_nonzero() {
    let output = scythe()
        .args(["inspect", "--explain", "SC-NOPE"])
        .output()
        .expect("command must run");

    assert!(
        !output.status.success(),
        "--explain SC-NOPE must exit non-zero; exit code: {:?}",
        output.status.code()
    );

    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(
        stderr.contains("SC-NOPE"),
        "stderr must mention the unknown id 'SC-NOPE'; got:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// 5. explain_postgres_check_under_mysql_dialect_errors
// ---------------------------------------------------------------------------

/// `scythe inspect --dialect mysql --explain SC-INS04` — SC-INS04 is a
/// postgres-only check.  Under the mysql dialect it must not be found, so the
/// command must exit ≠ 0 and stderr must mention both the id and the dialect.
#[test]
fn explain_postgres_check_under_mysql_dialect_errors() {
    let output = scythe()
        .args(["inspect", "--dialect", "mysql", "--explain", "SC-INS04"])
        .output()
        .expect("command must run");

    assert!(
        !output.status.success(),
        "--dialect mysql --explain SC-INS04 must exit non-zero; exit code: {:?}",
        output.status.code()
    );

    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(
        stderr.contains("SC-INS04"),
        "stderr must mention 'SC-INS04'; got:\n{stderr}"
    );
    assert!(stderr.contains("mysql"), "stderr must mention 'mysql'; got:\n{stderr}");
}

// ---------------------------------------------------------------------------
// 7. output_flag_writes_to_file
// ---------------------------------------------------------------------------

/// `scythe inspect --list-checks --output <path>` — exits 0 and creates the
/// specified file; the file content must match what stdout would contain.
#[test]
fn output_flag_writes_to_file() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let out_path = temp.path().join("inspect-list.txt");

    // Run with --output to write to a file.
    scythe()
        .args(["inspect", "--list-checks", "--output", out_path.to_str().unwrap()])
        .assert()
        .success();

    assert!(
        out_path.exists(),
        "--output file must be created at {}",
        out_path.display()
    );

    let content = std::fs::read_to_string(&out_path).expect("read output file");
    assert!(
        content.contains("SC-INS01"),
        "output file must contain SC-INS01; got:\n{content}"
    );
    assert!(
        content.contains("SC-INS13"),
        "output file must contain SC-INS13; got:\n{content}"
    );

    // Run without --output and compare stdout.
    let stdout_run = scythe()
        .args(["inspect", "--list-checks"])
        .output()
        .expect("command must run");
    let stdout_content = std::str::from_utf8(&stdout_run.stdout).unwrap();

    assert_eq!(content, stdout_content, "--output file content must match stdout");
}

// ---------------------------------------------------------------------------
// 8. version_flag_or_inspect_help_shows_flags
// ---------------------------------------------------------------------------

/// `scythe inspect --help` must document every flag that the CLI surface
/// exposes per `docs/guide/cli-reference.md`.
#[test]
fn inspect_help_shows_expected_flags() {
    let output = scythe()
        .args(["inspect", "--help"])
        .output()
        .expect("help command must run");

    assert!(
        output.status.success(),
        "inspect --help must exit 0; stderr: {}",
        std::str::from_utf8(&output.stderr).unwrap()
    );

    let stdout = std::str::from_utf8(&output.stdout).unwrap();

    for flag in [
        "--explain",
        "--list-checks",
        "--severity",
        "--exit-zero",
        "--output",
        "--dialect",
    ] {
        assert!(
            stdout.contains(flag),
            "inspect --help must mention flag {flag}; got:\n{stdout}"
        );
    }
}

// ---------------------------------------------------------------------------
// 9. format_json_with_list_checks_errors_cleanly_or_works
// ---------------------------------------------------------------------------

/// `scythe inspect --list-checks --format json`:
/// The `--format` flag applies to finding emission, not to `--list-checks`
/// (which has its own table format).  The command must exit 0; `--list-checks`
/// with `--format json` is not a documented combo and the current implementation
/// ignores `--format` when `--list-checks` is set, printing the table as usual.
///
/// This test asserts the EXISTING behaviour: exit 0 and the table still
/// contains SC-INS01.
#[test]
fn format_json_with_list_checks_exits_zero_and_lists_checks() {
    let output = scythe()
        .args(["inspect", "--list-checks", "--format", "json"])
        .output()
        .expect("command must run");

    assert!(
        output.status.success(),
        "--list-checks --format json must exit 0; stderr: {}",
        std::str::from_utf8(&output.stderr).unwrap()
    );

    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    assert!(
        stdout.contains("SC-INS01"),
        "--list-checks --format json must still emit the catalog table; got:\n{stdout}"
    );
}

// ---------------------------------------------------------------------------
// Additional: list_checks_postgres_dialect_shows_all_checks
// ---------------------------------------------------------------------------

/// Explicit `--dialect postgres` must show all 13 checks — the same as the
/// default.
#[test]
fn list_checks_postgres_dialect_shows_all_checks() {
    let assert = scythe()
        .args(["inspect", "--list-checks", "--dialect", "postgres"])
        .assert()
        .success();

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    for id in [
        "SC-INS01", "SC-INS02", "SC-INS03", "SC-INS04", "SC-INS05", "SC-INS06", "SC-INS07", "SC-INS08", "SC-INS09",
        "SC-INS10", "SC-INS11", "SC-INS12", "SC-INS13",
    ] {
        assert!(
            stdout.contains(id),
            "--dialect postgres --list-checks must include {id}; stdout:\n{stdout}"
        );
    }
}

// ---------------------------------------------------------------------------
// Additional: inspect_without_db_url_and_no_list_checks_errors
// ---------------------------------------------------------------------------

/// `scythe inspect` without a DB URL and without `--list-checks` must exit
/// non-zero with a diagnostic about the missing URL.
#[test]
fn inspect_without_db_url_exits_nonzero() {
    // Unset known DB URL env vars by running with cleared env — we can't
    // fully control the test runner's env, so instead we pass a deliberately
    // invalid config with no database_url so we get the "missing URL" error.
    let temp = tempfile::TempDir::new().expect("temp dir");
    let config_path = temp.path().join("scythe.toml");
    // Empty config — no [inspect].database_url
    std::fs::write(&config_path, "").unwrap();

    let output = scythe()
        .args(["inspect", "--config", config_path.to_str().unwrap()])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("command must run");

    assert!(
        !output.status.success(),
        "inspect with no DB URL must exit non-zero; exit code: {:?}",
        output.status.code()
    );

    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(
        stderr.contains("DATABASE_URL") || stderr.contains("no database URL") || stderr.contains("url"),
        "stderr must mention the missing URL; got:\n{stderr}"
    );
}
