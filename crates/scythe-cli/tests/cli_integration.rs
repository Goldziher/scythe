//! Integration tests that exercise the scythe CLI binary via std::process::Command.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

/// Returns the workspace root (two levels up from crate manifest dir).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn schema_dir(relative: &str) -> PathBuf {
    workspace_root().join("tests/schemas").join(relative)
}

/// Render a path as a string safe to embed in a TOML basic string (`"..."`).
///
/// `Path::display()` uses the platform's native separator, which on Windows
/// is `\` — an escape character in TOML basic strings. Glob patterns accept
/// `/` as a separator on every platform, so forward-slashing the path here
/// keeps the generated `scythe.toml` fixtures parseable as TOML on Windows
/// without changing what they match.
fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

#[test]
fn test_help_exits_zero() {
    let output = scythe_bin()
        .arg("--help")
        .output()
        .expect("failed to run scythe --help");

    assert!(output.status.success(), "scythe --help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("SQL-to-code") || stdout.contains("scythe"),
        "help output should mention scythe"
    );
}

#[test]
fn test_version_exits_zero() {
    let output = scythe_bin()
        .arg("--version")
        .output()
        .expect("failed to run scythe --version");

    assert!(output.status.success(), "scythe --version should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scythe"), "version output should contain 'scythe'");
}

#[test]
fn test_check_basemind_exits_zero() {
    let dir = schema_dir("simple/basemind");
    let output = scythe_bin()
        .args(["check", "--config", "scythe.toml"])
        .current_dir(&dir)
        .output()
        .expect("failed to run scythe check");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "scythe check on basemind should exit 0.\nstderr: {}",
        stderr
    );
    // `run_check` reports per-`[[sql]]`-block success as "[<name>] All queries
    // valid." (see crates/scythe-cli/src/commands/generate.rs); it never
    // prints "Check passed.".
    assert!(
        stderr.contains("All queries valid."),
        "check should report success for the block.\nstderr: {}",
        stderr
    );
}

/// Regression test for the `analyzed_queries`/`verifiable` retention guard in
/// `run_check` (see `should_retain_for_verification` in
/// `crates/scythe-cli/src/commands/generate.rs`): supplying `--database-url`
/// must still drive analyzed queries through to `verify_against_database`,
/// which attempts a live connection and reports a connection failure. This
/// proves the retention-skip optimization for the no-URL path (exercised by
/// `test_check_basemind_exits_zero` above) left the `--database-url` path
/// behaviorally unchanged.
#[test]
fn test_check_with_database_url_attempts_live_verification() {
    let dir = schema_dir("simple/basemind");
    let output = scythe_bin()
        .args([
            "check",
            "--config",
            "scythe.toml",
            "--database-url",
            "postgres://127.0.0.1:1/scythe_nonexistent_db",
        ])
        .current_dir(&dir)
        .output()
        .expect("failed to run scythe check --database-url");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "scythe check against an unreachable database should exit non-zero.\nstderr: {}",
        stderr
    );
    assert!(
        stderr.contains("failed to connect"),
        "expected a connection failure from verify_against_database (proving analyzed \
         queries were retained and handed off for verification), got:\nstderr: {}",
        stderr
    );
}

#[test]
fn test_check_pagila_exits_zero() {
    let dir = schema_dir("medium/pagila");
    let output = scythe_bin()
        .args(["check", "--config", "scythe.toml"])
        .current_dir(&dir)
        .output()
        .expect("failed to run scythe check");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "scythe check on pagila should exit 0.\nstderr: {}",
        stderr
    );
}

#[test]
fn test_generate_writes_file() {
    let dir = schema_dir("simple/basemind");
    let temp = tempfile::TempDir::new().unwrap();
    let output_dir = temp.path().join("generated");

    // `schema`/`queries` are absolute paths into `tests/schemas/simple/basemind`
    // (not relative to `dir`) since 0.13.0 resolves relative glob patterns
    // against the config file's directory (the temp dir here), not the
    // process's current working directory. This also doubles as coverage for
    // absolute glob patterns passing through `rebase_pattern` unchanged.
    let config_content = format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["{schema}"]
queries = ["{queries}"]
output = "{output}"
"#,
        schema = toml_path(&dir.join("schema.sql")),
        queries = toml_path(&dir.join("queries/*.sql")),
        output = toml_path(&output_dir)
    );

    let config_path = temp.path().join("scythe.toml");
    std::fs::write(&config_path, &config_content).unwrap();

    // `current_dir` is set to an unrelated temp dir (not `dir`, where the
    // schema/queries actually live) to prove resolution no longer depends on
    // the process's CWD. `output` above is an absolute path, which #207
    // rejects by default as escaping the project root; `--allow-output-escape`
    // opts back in since this test's absolute `output` is deliberate.
    let output = scythe_bin()
        .args([
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--allow-output-escape",
        ])
        .current_dir(temp.path())
        .output()
        .expect("failed to run scythe generate");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "scythe generate should exit 0.\nstderr: {}",
        stderr
    );

    let generated_file = output_dir.join("queries.rs");
    assert!(
        generated_file.exists(),
        "should create queries.rs at {}",
        generated_file.display()
    );

    let content = std::fs::read_to_string(&generated_file).unwrap();
    assert!(
        content.contains("scythe:provenance"),
        "generated file should have a scythe provenance header"
    );
    assert!(
        content.len() > 100,
        "generated file should have substantial content, got {} bytes",
        content.len()
    );
}

#[test]
fn test_generate_pagila_writes_file() {
    let dir = schema_dir("medium/pagila");
    let temp = tempfile::TempDir::new().unwrap();
    let output_dir = temp.path().join("generated");

    // See `test_generate_writes_file`: schema/queries are absolute (into
    // `tests/schemas/medium/pagila`), not relative to `dir`, since 0.13.0
    // resolves relative glob patterns against the config file's directory.
    let config_content = format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "pagila"
engine = "postgresql"
schema = ["{schema}"]
queries = ["{customers}", "{rentals}"]
output = "{output}"
"#,
        schema = toml_path(&dir.join("schema.sql")),
        customers = toml_path(&dir.join("queries/customers.sql")),
        rentals = toml_path(&dir.join("queries/rentals.sql")),
        output = toml_path(&output_dir)
    );

    let config_path = temp.path().join("scythe.toml");
    std::fs::write(&config_path, &config_content).unwrap();

    // `output` above is an absolute path, which #207 rejects by default as
    // escaping the project root; `--allow-output-escape` opts back in.
    let output = scythe_bin()
        .args([
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--allow-output-escape",
        ])
        .current_dir(temp.path())
        .output()
        .expect("failed to run scythe generate");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "scythe generate on pagila should exit 0.\nstderr: {}",
        stderr
    );

    let generated_file = output_dir.join("queries.rs");
    assert!(generated_file.exists(), "should create queries.rs for pagila");

    let content = std::fs::read_to_string(&generated_file).unwrap();
    assert!(
        content.len() > 500,
        "pagila should generate substantial code, got {} bytes",
        content.len()
    );
}

#[test]
fn test_missing_config_exits_one() {
    let output = scythe_bin()
        .args(["check", "--config", "nonexistent.toml"])
        .output()
        .expect("failed to run scythe check");

    // Exit 1 is the operational-failure code, distinct from exit 2 (which
    // means "the config was readable and error-severity findings were
    // reported"). A missing config is never a finding, so it must not share
    // the findings exit code.
    assert_eq!(
        output.status.code(),
        Some(1),
        "scythe check with missing config should exit 1 (operational failure), not 2 (findings); stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Write a minimal `check` project whose only query fires SC-S01
/// (update-without-where), an `Error`-severity rule by default, and return
/// the config path.
fn write_check_error_project(temp: &tempfile::TempDir) -> PathBuf {
    std::fs::write(
        temp.path().join("schema.sql"),
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join("queries.sql"),
        "-- @name DisableAllUsers\n-- @returns :exec\nUPDATE users SET name = 'x';\n",
    )
    .unwrap();
    let config_path = temp.path().join("scythe.toml");
    std::fs::write(
        &config_path,
        concat!(
            "[scythe]\nversion = \"1\"\n\n",
            "[[sql]]\nname = \"main\"\nengine = \"postgresql\"\n",
            "schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n",
        ),
    )
    .unwrap();
    config_path
}

#[test]
fn test_check_error_finding_exits_two() {
    let temp = tempfile::TempDir::new().unwrap();
    let config_path = write_check_error_project(&temp);

    let output = scythe_bin()
        .args(["check", "--config", config_path.to_str().unwrap()])
        .output()
        .expect("failed to run scythe check");

    assert_eq!(
        output.status.code(),
        Some(2),
        "an error-severity finding (SC-S01) must yield check exit code 2; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("SC-S01"),
        "expected SC-S01 (update-without-where) in the report; stdout: {stdout}"
    );
}

#[test]
fn test_check_exit_zero_overrides_error_exit_code() {
    let temp = tempfile::TempDir::new().unwrap();
    let config_path = write_check_error_project(&temp);

    let output = scythe_bin()
        .args(["check", "--config", config_path.to_str().unwrap(), "--exit-zero"])
        .output()
        .expect("failed to run scythe check --exit-zero");

    assert!(
        output.status.success(),
        "--exit-zero must produce exit 0 even with error-severity findings; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("SC-S01"),
        "finding must still be emitted under --exit-zero; got: {stdout}"
    );
}

#[test]
fn test_no_subcommand_exits_nonzero() {
    let output = scythe_bin().output().expect("failed to run scythe");

    assert!(!output.status.success(), "scythe with no subcommand should fail");
}

/// Write a small SQL file that fires SC-SEC02 (GRANT ALL) and returns its path.
fn write_grant_all_sql(temp: &tempfile::TempDir) -> PathBuf {
    let sql_path = temp.path().join("grant_all.sql");
    std::fs::write(&sql_path, "GRANT ALL ON users TO bob;\n").unwrap();
    sql_path
}

#[test]
fn test_audit_list_rules_exits_zero_and_shows_sec01() {
    let output = scythe_bin()
        .args(["audit", "--list-rules"])
        .output()
        .expect("failed to run scythe audit --list-rules");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "audit --list-rules should exit 0; stdout: {stdout}"
    );
    assert!(
        stdout.contains("SC-SEC01") && stdout.contains("SC-SEC11"),
        "rule catalog must include SC-SEC01..11; got: {stdout}"
    );
    assert!(
        stdout.contains("[security]"),
        "rule catalog must be grouped by category; got: {stdout}"
    );
}

/// Regression test: SC-A02 and SC-C01 are registered lint rules that default
/// to `Off` severity. `--list-rules` used to build its catalog from
/// `active_rules`, which filters `Off` rules out, so both silently vanished
/// from the printed catalog even though the lint registry holds 23 rules
/// (not 21). See `RuleRegistry::all_rules`.
#[test]
fn test_audit_list_rules_includes_off_by_default_rules() {
    let output = scythe_bin()
        .args(["audit", "--list-rules"])
        .output()
        .expect("failed to run scythe audit --list-rules");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "audit --list-rules should exit 0; stdout: {stdout}"
    );
    assert!(
        stdout.contains("SC-A02"),
        "rule catalog must include off-by-default rule SC-A02; got: {stdout}"
    );
    assert!(
        stdout.contains("SC-C01"),
        "rule catalog must include off-by-default rule SC-C01; got: {stdout}"
    );
}

/// Same hole as `test_audit_list_rules_includes_off_by_default_rules`, but
/// for `--explain`: it looked rules up in `active_rules` too, so explaining
/// an off-by-default rule id failed as if the id didn't exist at all.
#[test]
fn test_audit_explain_off_by_default_rule_succeeds() {
    for id in ["SC-A02", "SC-C01"] {
        let output = scythe_bin()
            .args(["audit", "--explain", id])
            .output()
            .unwrap_or_else(|e| panic!("failed to run scythe audit --explain {id}: {e}"));

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "audit --explain {id} should exit 0; stdout: {stdout} stderr: {stderr}"
        );
        assert!(
            stdout.contains(id),
            "explanation for {id} must name the rule; got: {stdout}"
        );
    }
}

#[test]
fn test_audit_explain_unknown_rule_returns_error() {
    let output = scythe_bin()
        .args(["audit", "--explain", "DOES-NOT-EXIST"])
        .output()
        .expect("failed to run scythe audit --explain");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "unknown rule id must exit non-zero; stderr: {stderr}"
    );
    assert!(
        stderr.contains("DOES-NOT-EXIST") && stderr.contains("--list-rules"),
        "error must name the offending id and hint --list-rules; got: {stderr}"
    );
}

#[test]
fn test_audit_exit_zero_overrides_error_exit_code() {
    let temp = tempfile::TempDir::new().unwrap();
    let sql_path = write_grant_all_sql(&temp);

    let base = scythe_bin()
        .args(["audit", sql_path.to_str().unwrap()])
        .output()
        .expect("audit run");
    assert_eq!(
        base.status.code(),
        Some(2),
        "GRANT ALL must yield audit exit code 2; stderr: {}",
        String::from_utf8_lossy(&base.stderr)
    );

    let lenient = scythe_bin()
        .args(["audit", "--exit-zero", sql_path.to_str().unwrap()])
        .output()
        .expect("audit run with --exit-zero");
    assert!(
        lenient.status.success(),
        "--exit-zero must produce exit 0 even with errors; stderr: {}",
        String::from_utf8_lossy(&lenient.stderr)
    );
    let stdout = String::from_utf8_lossy(&lenient.stdout);
    assert!(
        stdout.contains("SC-SEC02"),
        "finding must still be emitted under --exit-zero; got: {stdout}"
    );
}

#[test]
fn test_audit_output_file_writes_sarif() {
    let temp = tempfile::TempDir::new().unwrap();
    let sql_path = write_grant_all_sql(&temp);
    let out_path = temp.path().join("audit.sarif");

    let output = scythe_bin()
        .args([
            "audit",
            "--format",
            "sarif",
            "-o",
            out_path.to_str().unwrap(),
            "--exit-zero",
            sql_path.to_str().unwrap(),
        ])
        .output()
        .expect("audit run with -o");
    assert!(
        output.status.success(),
        "audit with -o + --exit-zero should exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sarif = std::fs::read_to_string(&out_path).expect("read sarif output");
    assert!(
        sarif.contains("\"$schema\"") && sarif.contains("\"2.1.0\""),
        "output file must contain SARIF 2.1.0 envelope; got: {sarif}"
    );
}

#[test]
fn test_audit_ignore_suppressions_resurfaces_finding() {
    let temp = tempfile::TempDir::new().unwrap();
    let sql_path = temp.path().join("suppressed.sql");
    std::fs::write(
        &sql_path,
        "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON users TO bob;\n",
    )
    .unwrap();

    let suppressed = scythe_bin()
        .args(["audit", sql_path.to_str().unwrap()])
        .output()
        .expect("audit run");
    assert!(
        suppressed.status.success(),
        "suppressed run should exit 0; stderr: {}",
        String::from_utf8_lossy(&suppressed.stderr)
    );

    let strict = scythe_bin()
        .args(["audit", "--ignore-suppressions", sql_path.to_str().unwrap()])
        .output()
        .expect("audit run with --ignore-suppressions");
    assert_eq!(
        strict.status.code(),
        Some(2),
        "strict run must surface the finding and exit 2; stderr: {}",
        String::from_utf8_lossy(&strict.stderr)
    );
    let stdout = String::from_utf8_lossy(&strict.stdout);
    assert!(
        stdout.contains("SC-SEC02"),
        "strict run must emit SC-SEC02; got: {stdout}"
    );
}

#[test]
fn test_audit_severity_filter_drops_warnings() {
    let temp = tempfile::TempDir::new().unwrap();
    let sql_path = temp.path().join("mixed.sql");
    std::fs::write(
        &sql_path,
        "GRANT ALL ON users TO bob;\nSELECT * FROM users WHERE name LIKE '%abc%';\n",
    )
    .unwrap();

    let output = scythe_bin()
        .args([
            "audit",
            "--severity",
            "error",
            "--exit-zero",
            sql_path.to_str().unwrap(),
        ])
        .output()
        .expect("audit run");
    assert!(output.status.success(), "--exit-zero forces success");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("SC-SEC02"),
        "error-level finding must remain; got: {stdout}"
    );
    assert!(
        !stdout.contains("SC-SEC09"),
        "warn-level finding must be filtered out; got: {stdout}"
    );
}

#[test]
fn test_audit_dialect_flag_skips_pg_only_rule_on_sqlite() {
    let temp = tempfile::TempDir::new().unwrap();
    let sql_path = temp.path().join("set_role.sql");
    std::fs::write(&sql_path, "SET ROLE admin;\n").unwrap();

    let output = scythe_bin()
        .args([
            "audit",
            "--dialect",
            "sqlite",
            "--exit-zero",
            sql_path.to_str().unwrap(),
        ])
        .output()
        .expect("audit run");
    assert!(output.status.success(), "audit must exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("SC-SEC11"),
        "SC-SEC11 must not fire under --dialect sqlite; got: {stdout}"
    );
}

/// The four `javascript-*` backends reuse the TypeScript manifests, which
/// declare `file_extension = "ts"`. Nothing else in the tree exercises the
/// filename they actually write: there is no javascript integration project,
/// and the tool validator writes its own temp `.mjs` rather than going
/// through `output_filename`. So this asserts the end-to-end path -- untyped
/// JSDoc output landing in a `.ts` file is `noImplicitAny` under `--strict`
/// and is not something node will run.
#[test]
fn test_javascript_backend_writes_a_js_file() {
    let temp = tempfile::TempDir::new().unwrap();
    let schema_path = temp.path().join("schema.sql");
    let queries_path = temp.path().join("queries.sql");
    let output_dir = temp.path().join("out");
    std::fs::write(&schema_path, "CREATE TABLE users (id SERIAL PRIMARY KEY, bio TEXT);\n").unwrap();
    std::fs::write(
        &queries_path,
        "-- @name GetUserById\n-- @returns :one\nSELECT id, bio FROM users WHERE id = $1;\n",
    )
    .unwrap();

    let config = format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["{schema}"]
queries = ["{queries}"]

[[sql.gen]]
backend = "javascript-pg"
output = "{output}"
"#,
        schema = schema_path.display().to_string().replace('\\', "/"),
        queries = queries_path.display().to_string().replace('\\', "/"),
        output = output_dir.display().to_string().replace('\\', "/"),
    );
    let config_path = temp.path().join("scythe.toml");
    std::fs::write(&config_path, &config).unwrap();

    // `output` above is an absolute path, which #207 rejects by default as
    // escaping the project root; `--allow-output-escape` opts back in.
    let output = scythe_bin()
        .args([
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--allow-output-escape",
        ])
        .output()
        .expect("generate run");
    assert!(
        output.status.success(),
        "generate failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        output_dir.join("queries.js").exists(),
        "javascript-pg must write queries.js; found {:?}",
        std::fs::read_dir(&output_dir)
            .map(|entries| entries
                .filter_map(Result::ok)
                .map(|e| e.file_name())
                .collect::<Vec<_>>())
            .unwrap_or_default()
    );
    assert!(
        !output_dir.join("queries.ts").exists(),
        "javascript-pg must not write a .ts file"
    );

    // The provenance header has to name the js backend, not the ts one it
    // shares a manifest with, or `scythe check` reports backend drift against
    // this project's own output forever.
    let code = std::fs::read_to_string(output_dir.join("queries.js")).expect("generated file");
    assert!(
        code.contains("backend=javascript-pg"),
        "provenance must name javascript-pg; got:\n{code}"
    );
}
