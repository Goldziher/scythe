//! CLI smoke tests for `scythe inspect`.
//!
//! All tests here are DB-free — they exercise flags that return without
//! connecting to a database (`--list-checks`, `--explain`, `--help`).
//! Live-database tests live in `crates/scythe-inspect/tests/pg_live.rs`.

use assert_cmd::Command;

// ---------------------------------------------------------------------------

fn scythe() -> Command {
    Command::cargo_bin("scythe").expect("scythe binary must exist")
}

/// `scythe inspect --list-checks` must exit 0 and list all 13 canonical
/// SC-INS* checks.
#[test]
fn list_checks_prints_thirteen_rows() {
    let assert = scythe().args(["inspect", "--list-checks"]).assert().success();

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();

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

/// `scythe inspect --list-checks --dialect mysql` lists MySQL's own checks
/// and none of PostgreSQL's.
///
/// ~keep This asserted the opposite until MySQL got a real driver: it
/// required "no checks" or empty output, which was true only because every
/// non-PostgreSQL engine fell through to `UnsupportedDriver`. Left as it
/// was, it would have failed the moment the gap it described was closed --
/// it guarded the absence, not the behaviour. The postgres-only half is
/// kept and made exact: `SC-INS01`..`SC-INS13` must not appear, which a
/// bare `contains("SC-INS")` can no longer express now that MySQL's own ids
/// are spelled `SC-INS-MY01`.
#[test]
fn list_checks_with_dialect_mysql_lists_mysql_checks_and_no_postgres_ones() {
    let assert = scythe()
        .args(["inspect", "--list-checks", "--dialect", "mysql"])
        .assert()
        .success();

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();

    for id in ["SC-INS-MY01", "SC-INS-MY02", "SC-INS-MY03", "SC-INS-MY04"] {
        assert!(
            stdout.contains(id),
            "--list-checks --dialect mysql must include {id}; stdout:\n{stdout}"
        );
    }

    for id in [
        "SC-INS01", "SC-INS02", "SC-INS03", "SC-INS04", "SC-INS05", "SC-INS06", "SC-INS07", "SC-INS08", "SC-INS09",
        "SC-INS10", "SC-INS11", "SC-INS12", "SC-INS13",
    ] {
        assert!(
            !stdout.contains(id),
            "mysql dialect must not list the postgres-only check {id}; stdout:\n{stdout}"
        );
    }
}

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

/// `scythe inspect --list-checks --output <path>` — exits 0 and creates the
/// specified file; the file content must match what stdout would contain.
#[test]
fn output_flag_writes_to_file() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let out_path = temp.path().join("inspect-list.txt");

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

    let stdout_run = scythe()
        .args(["inspect", "--list-checks"])
        .output()
        .expect("command must run");
    let stdout_content = std::str::from_utf8(&stdout_run.stdout).unwrap();

    assert_eq!(content, stdout_content, "--output file content must match stdout");
}

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

/// `scythe inspect <sqlite-url>` — SQLite has no `scythe inspect` driver.
/// Before #131, every unimplemented engine fell through to a MySQL stub that
/// refused to connect and reported itself as `mysql`, so a SQLite user was
/// told about an engine they never asked for. The command must instead fail
/// immediately (no socket ever opens — `UnsupportedDriver::connect` never
/// touches the URL) naming `sqlite`, and must never mention `mysql`.
///
/// Revert the `UnsupportedDriver` dispatch in `build_driver_with_config` back
/// to the old `_ => Box::new(MySqlDriver::new())` catch-all and this test
/// fails on the `!stderr.contains("mysql")` assertion — the stub still
/// exits non-zero (it also refuses to connect), so only the *message*
/// distinguishes an honest refusal from a misreported one.
#[test]
fn inspect_unsupported_engine_errors_naming_the_engine_not_a_stub() {
    let output = scythe()
        .args(["inspect", "sqlite:///nonexistent.db"])
        .env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("command must run");

    assert!(
        !output.status.success(),
        "inspect against sqlite (no driver) must exit non-zero; exit code: {:?}",
        output.status.code()
    );

    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(
        stderr.contains("engine `sqlite` is not supported"),
        "stderr must name the engine actually requested; got:\n{stderr}"
    );
    // ~keep Not `!stderr.contains("mysql")`: the honest refusal names every supported
    // engine, so it mentions mysql legitimately. What must not appear is a *connection
    // attempt* against an engine the user did not ask for -- `InspectError::Connect`
    // renders as "connection to {engine} failed", which is exactly what the old
    // `_ => MySqlDriver::new()` catch-all produced for a sqlite URL.
    assert!(
        !stderr.contains("connection to mysql"),
        "stderr must not report a mysql connection attempt for a sqlite URL -- the original \
         #131 defect; got:\n{stderr}"
    );
}

/// `scythe inspect` without a DB URL and without `--list-checks` must exit
/// non-zero with a diagnostic about the missing URL.
#[test]
fn inspect_without_db_url_exits_nonzero() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let config_path = temp.path().join("scythe.toml");
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
