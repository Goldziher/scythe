//! `scythe lint` regressions for the four ways it used to report success
//! without having checked anything:
//!
//! 1. It never called `LintEngine::build_report`, so the engine's cross-query
//!    checks -- duplicate `@name` (SC-C03) -- had no caller at all.
//! 2. It scraped CWE ids out of the violation *message*, which no rule ever
//!    puts them in, so `--format sarif` carried no CWE tags.
//! 3. It built no `SuppressionSet`, so the `-- scythe-audit: ignore[ID]`
//!    annotations `scythe audit` honours did nothing under `scythe lint`.
//! 4. A query that failed to analyze was skipped -- every query-level rule
//!    silently dropped -- while lint printed "No lint violations found." and
//!    exited 0, disagreeing with `scythe check` on the same project (#158).
//!
//! Every test drives the compiled binary and asserts exact exit codes.

use std::fs;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const SCHEMA: &str = "CREATE TABLE users (id bigint PRIMARY KEY, email text NOT NULL);\n";

/// Write `scythe.toml` + `schema.sql` + `queries.sql` into `dir` and return
/// the config path.
fn write_project(dir: &TempDir, engine: &str, schema: &str, queries: &str) -> String {
    fs::write(dir.path().join("schema.sql"), schema).expect("write schema.sql");
    fs::write(dir.path().join("queries.sql"), queries).expect("write queries.sql");

    let config = format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "{engine}"
schema = ["schema.sql"]
queries = ["queries.sql"]
"#
    );
    let config_path = dir.path().join("scythe.toml");
    fs::write(&config_path, config).expect("write scythe.toml");
    config_path.to_string_lossy().into_owned()
}

fn run_lint(config_path: &str, extra: &[&str]) -> std::process::Output {
    let mut cmd = Command::cargo_bin("scythe").expect("scythe binary must exist");
    cmd.args(["lint", "--config", config_path]);
    cmd.args(extra);
    cmd.env_remove("DATABASE_URL")
        .env_remove("SCYTHE_DATABASE_URL")
        .output()
        .expect("run scythe lint")
}

/// Every JSON finding's `rule_id`, in emission order.
fn rule_ids(stdout: &str) -> Vec<String> {
    let parsed: Value = serde_json::from_str(stdout).expect("lint --format json must emit valid JSON");
    parsed
        .as_array()
        .expect("json reporter emits an array")
        .iter()
        .map(|f| f["rule_id"].as_str().expect("rule_id is a string").to_string())
        .collect()
}

/// `SELECT *` over a table with an `email` column trips SC-SEC07, whose spec
/// declares `cwe = ["CWE-200"]`. Lint used to derive the CWE list by scanning
/// the rendered violation message for a `CWE-\d+` pattern -- which no rule
/// message contains -- so the SARIF `properties.cwe` array was absent for
/// every finding lint has ever produced.
#[test]
fn lint_sarif_carries_the_cwe_the_rule_declares() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_project(
        &dir,
        "postgresql",
        SCHEMA,
        "-- @name ListUsers\n-- @returns :many\nSELECT * FROM users;\n",
    );

    let output = run_lint(&config, &["--format", "sarif"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let sarif: Value = serde_json::from_str(&stdout).expect("lint --format sarif must emit valid JSON");

    let results = sarif["runs"][0]["results"]
        .as_array()
        .expect("sarif log has a results array");
    let sec07 = results
        .iter()
        .find(|r| r["ruleId"] == "SC-SEC07")
        .unwrap_or_else(|| panic!("SC-SEC07 must be reported; sarif was: {stdout}"));

    assert_eq!(
        sec07["properties"]["cwe"],
        serde_json::json!(["CWE-200"]),
        "SC-SEC07 declares cwe = [\"CWE-200\"] in its spec and SARIF must carry it"
    );
}

/// `-- scythe-audit: ignore[SC-SEC07]` inside a query block must drop exactly
/// that rule's violation for exactly that query, leaving every other rule's
/// findings intact. Lint built no `SuppressionSet` at all before this, so the
/// annotation was inert.
#[test]
fn lint_honours_an_inline_suppression_for_one_rule_only() {
    let dir = TempDir::new().expect("tempdir");
    let unsuppressed = write_project(
        &dir,
        "postgresql",
        SCHEMA,
        "-- @name ListUsers\n-- @returns :many\nSELECT * FROM users;\n",
    );
    let before = run_lint(&unsuppressed, &["--format", "json"]);
    let before_ids = rule_ids(&String::from_utf8_lossy(&before.stdout));
    assert!(
        before_ids.contains(&"SC-SEC07".to_string()) && before_ids.contains(&"SC-S03".to_string()),
        "without a suppression both SC-SEC07 and SC-S03 must fire; got {before_ids:?}"
    );

    let dir2 = TempDir::new().expect("tempdir");
    let suppressed = write_project(
        &dir2,
        "postgresql",
        SCHEMA,
        "-- @name ListUsers\n-- @returns :many\n-- scythe-audit: ignore[SC-SEC07]\nSELECT * FROM users;\n",
    );
    let after = run_lint(&suppressed, &["--format", "json"]);
    let after_ids = rule_ids(&String::from_utf8_lossy(&after.stdout));

    assert!(
        !after_ids.contains(&"SC-SEC07".to_string()),
        "the suppressed rule must not be reported; got {after_ids:?}"
    );
    assert!(
        after_ids.contains(&"SC-S03".to_string()),
        "a suppression for one rule must not suppress any other; got {after_ids:?}"
    );
}

/// Two queries sharing one `@name` is SC-C03, and only
/// `LintEngine::build_report` detects it -- lint's own per-query loop had no
/// cross-query step, so `scythe lint` reported nothing while `scythe check`
/// failed the same project.
#[test]
fn lint_reports_duplicate_query_names_and_exits_two() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_project(
        &dir,
        "postgresql",
        SCHEMA,
        "-- @name GetUser\n-- @returns :one\nSELECT id, email FROM users WHERE id = $1;\n\
         -- @name GetUser\n-- @returns :one\nSELECT id FROM users WHERE id = $1;\n",
    );

    let output = run_lint(&config, &["--format", "json"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ids = rule_ids(&String::from_utf8_lossy(&output.stdout));

    assert!(
        ids.contains(&"SC-C03".to_string()),
        "a duplicate @name must be reported as SC-C03; got {ids:?}, stderr: {stderr}"
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "SC-C03 is error severity, so lint must exit 2; stderr: {stderr}"
    );
    assert!(
        stderr.contains("[test] checked 2 query(ies) against"),
        "lint must report how much it checked; stderr: {stderr}"
    );
}

/// A duplicate `@name` downgraded to `warn` in `[lint.rules]` must be
/// reported as a warning and exit 0 -- the severity comes from the registry,
/// not from a hardcoded constant.
#[test]
fn lint_duplicate_query_name_honours_configured_severity() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("schema.sql"), SCHEMA).expect("write schema.sql");
    fs::write(
        dir.path().join("queries.sql"),
        "-- @name GetUser\n-- @returns :one\nSELECT id, email FROM users WHERE id = $1;\n\
         -- @name GetUser\n-- @returns :one\nSELECT id FROM users WHERE id = $1;\n",
    )
    .expect("write queries.sql");
    let config_path = dir.path().join("scythe.toml");
    fs::write(
        &config_path,
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[lint.rules]
"SC-C03" = "warn"
"#,
    )
    .expect("write scythe.toml");

    let output = run_lint(&config_path.to_string_lossy(), &["--format", "json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout).expect("valid JSON");
    let sc_c03 = parsed
        .as_array()
        .expect("array")
        .iter()
        .find(|f| f["rule_id"] == "SC-C03")
        .unwrap_or_else(|| panic!("SC-C03 must still be reported; stdout: {stdout}"));

    assert_eq!(sc_c03["severity"], "warning", "the configured severity must win");
    assert_eq!(
        output.status.code(),
        Some(0),
        "a warning-only run exits 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Issue #158: a query the analyzer rejects used to be skipped, taking every
/// query-level rule with it, while lint printed "No lint violations found."
/// and exited 0. It is now an SC-PARSE02 error finding -- the same rule id
/// `scythe check` already uses for an analysis failure.
#[test]
fn lint_reports_an_unanalysable_query_as_an_error_and_exits_two() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_project(
        &dir,
        "postgresql",
        "CREATE TABLE users (id bigint PRIMARY KEY, name text NOT NULL);\n",
        "-- @name GetUsers\n-- @returns :many\nSELECT * FROM users WHERE name = 42;\n",
    );

    let output = run_lint(&config, &["--format", "json"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ids = rule_ids(&String::from_utf8_lossy(&output.stdout));

    assert_eq!(
        ids,
        vec!["SC-PARSE02".to_string()],
        "the analysis failure must be the reported finding; stderr: {stderr}"
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "an unanalysable query must fail the run; stderr: {stderr}"
    );
    assert!(
        stderr.contains("failed to analyze query 'GetUsers'"),
        "the diagnostic must name the query; stderr: {stderr}"
    );
    assert!(
        !stderr.contains("No lint violations found."),
        "lint must not claim a clean project it never linted; stderr: {stderr}"
    );
}

/// A query that does not parse is the same defect shape as one that does not
/// analyze, and gets the same treatment: an SC-PARSE01 error, not a skip.
#[test]
fn lint_reports_an_unparseable_query_as_an_error_and_exits_two() {
    let dir = TempDir::new().expect("tempdir");
    let config = write_project(
        &dir,
        "postgresql",
        SCHEMA,
        "-- @name Broken\n-- @returns :one\nSELECT * FROM users WHERE ((;\n",
    );

    let output = run_lint(&config, &["--format", "json"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ids = rule_ids(&String::from_utf8_lossy(&output.stdout));

    assert!(
        ids.contains(&"SC-PARSE01".to_string()),
        "a parse failure must be reported; got {ids:?}, stderr: {stderr}"
    );
    assert_eq!(output.status.code(), Some(2), "stderr: {stderr}");
}

/// `SC-PARSE01` joined a registry so `[lint.rules]` could reach it. Before
/// that, `lint_from_config` hardcoded `Severity::Error` at the point this
/// finding was constructed, so `"SC-PARSE01" = "warn"` here had no effect:
/// the exit code stayed `2` and the JSON finding stayed `"error"`.
#[test]
fn lint_unparseable_query_honours_configured_severity() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("schema.sql"), SCHEMA).expect("write schema.sql");
    fs::write(
        dir.path().join("queries.sql"),
        "-- @name Broken\n-- @returns :one\nSELECT * FROM users WHERE ((;\n",
    )
    .expect("write queries.sql");
    let config_path = dir.path().join("scythe.toml");
    fs::write(
        &config_path,
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[lint.rules]
"SC-PARSE01" = "warn"
"#,
    )
    .expect("write scythe.toml");

    let output = run_lint(&config_path.to_string_lossy(), &["--format", "json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout).expect("valid JSON");
    let sc_parse01 = parsed
        .as_array()
        .expect("array")
        .iter()
        .find(|f| f["rule_id"] == "SC-PARSE01")
        .unwrap_or_else(|| panic!("SC-PARSE01 must still be reported; stdout: {stdout}"));

    assert_eq!(sc_parse01["severity"], "warning", "the configured severity must win");
    assert_eq!(
        output.status.code(),
        Some(0),
        "a warning-only run exits 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Same regression as `lint_unparseable_query_honours_configured_severity`,
/// for `SC-PARSE02` (analysis failure).
#[test]
fn lint_unanalysable_query_honours_configured_severity() {
    let dir = TempDir::new().expect("tempdir");
    fs::write(dir.path().join("schema.sql"), SCHEMA).expect("write schema.sql");
    fs::write(
        dir.path().join("queries.sql"),
        "-- @name GetUsers\n-- @returns :many\nSELECT * FROM users WHERE email = 42;\n",
    )
    .expect("write queries.sql");
    let config_path = dir.path().join("scythe.toml");
    fs::write(
        &config_path,
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[lint.rules]
"SC-PARSE02" = "warn"
"#,
    )
    .expect("write scythe.toml");

    let output = run_lint(&config_path.to_string_lossy(), &["--format", "json"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout).expect("valid JSON");
    let sc_parse02 = parsed
        .as_array()
        .expect("array")
        .iter()
        .find(|f| f["rule_id"] == "SC-PARSE02")
        .unwrap_or_else(|| panic!("SC-PARSE02 must still be reported; stdout: {stdout}"));

    assert_eq!(sc_parse02["severity"], "warning", "the configured severity must win");
    assert_eq!(
        output.status.code(),
        Some(0),
        "a warning-only run exits 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `LintRule::is_applicable_to` gates most canonical migration and security
/// rules to PostgreSQL. Running them against a MySQL block produced nothing,
/// which is indistinguishable from "they ran and found nothing" -- lint now
/// names the rules it did not run.
#[test]
fn lint_names_the_rules_skipped_for_a_non_matching_engine() {
    let queries = "-- @name ListUsers\n-- @returns :many\nSELECT id, email FROM users;\n";

    let mysql_dir = TempDir::new().expect("tempdir");
    let mysql_config = write_project(&mysql_dir, "mysql", SCHEMA, queries);
    let mysql = run_lint(&mysql_config, &[]);
    let mysql_stderr = String::from_utf8_lossy(&mysql.stderr);

    assert!(
        mysql_stderr.contains("rule(s) skipped: not applicable to engine 'mysql'"),
        "lint must say which engine the skipped rules did not apply to; stderr: {mysql_stderr}"
    );
    assert!(
        mysql_stderr.contains("SC-MIG01"),
        "the skipped rule ids must be named; stderr: {mysql_stderr}"
    );

    let pg_dir = TempDir::new().expect("tempdir");
    let pg_config = write_project(&pg_dir, "postgresql", SCHEMA, queries);
    let pg = run_lint(&pg_config, &[]);
    let pg_stderr = String::from_utf8_lossy(&pg.stderr);

    assert!(
        !pg_stderr.contains("rule(s) skipped"),
        "no rule is skipped for the engine every canonical rule targets; stderr: {pg_stderr}"
    );
    assert_eq!(
        pg.status.code(),
        Some(0),
        "the postgres fixture is clean; stderr: {pg_stderr}"
    );
}
