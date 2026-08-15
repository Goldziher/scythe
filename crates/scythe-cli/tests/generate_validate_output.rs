//! Regression tests for board #187: `validate_generated_code` /
//! `validate_with_tools` (`crates/scythe-codegen/src/validation.rs`) had no
//! production caller at all -- every call site outside `validation.rs` was a
//! test, so `scythe generate` never checked its own output. `scythe generate
//! --validate-output` wires it in as an opt-in flag.
//!
//! `ValidationOutcome` is deliberately a three-state type (validated /
//! nothing to check because no tool ran / a real tool found a problem), not
//! pass/fail -- see that type's doc comment. These tests pin that
//! `--validate-output` surfaces all three states distinctly, not just
//! "the flag exists".
//!
//! Every test drives the compiled binary via
//! `Command::new(env!("CARGO_BIN_EXE_scythe"))`, following the idiom in
//! `cli_honesty_defects.rs`.

use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const SCHEMA_SQL: &str = "CREATE TABLE widgets (id bigint PRIMARY KEY, name text NOT NULL);\n";
const QUERY_SQL: &str = "-- @name GetWidget\n-- @returns :one\nSELECT id, name FROM widgets WHERE id = $1;\n";

/// Writes a minimal project targeting `backend` and returns the
/// `scythe.toml` path.
fn write_project(dir: &std::path::Path, backend: &str) -> String {
    std::fs::write(dir.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(dir.join("queries.sql"), QUERY_SQL).unwrap();
    let config = format!(
        "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
        schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n\n\
        [[sql.gen]]\nbackend = \"{backend}\"\noutput = \"out\"\n"
    );
    let config_path = dir.join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();
    config_path.to_string_lossy().into_owned()
}

/// Writes a minimal project targeting the `python-psycopg3` backend (its
/// only real-tool checker is `poly`, wired through
/// `scythe_codegen::validation::validate_python_tools`) and returns the
/// `scythe.toml` path.
fn write_python_project(dir: &std::path::Path) -> String {
    write_project(dir, "python-psycopg3")
}

/// `scythe generate` without `--validate-output` must never run the
/// generated-code validators at all -- opt-in, not default, because it shells
/// out to external toolchains (`poly`, `tsc`, `javac`, ...) that may not be
/// installed on every machine `generate` runs on.
#[test]
fn generate_without_validate_output_never_reports_validation() {
    let dir = TempDir::new().unwrap();
    let config_path = write_python_project(dir.path());

    let output = scythe_bin()
        .args(["generate", "--config", &config_path])
        .output()
        .expect("run scythe generate");

    let err = stderr(&output);
    assert!(
        output.status.success(),
        "plain `generate` must still succeed; stderr: {err}"
    );
    assert!(
        !err.contains("generated-code validation"),
        "generated-code validation must not run at all without --validate-output; stderr: {err}"
    );
}

/// `scythe generate --validate-output` with the real tool (`poly`) reachable
/// on `PATH` must report the target as validated, distinctly from a skip.
///
/// `poly` is this repository's own bundled linter and is required to be on
/// `PATH` for this repo's own tooling (`poly lint .`), so the inherited
/// `PATH` is used unmodified here -- unlike the skip test below, which
/// deliberately breaks it.
#[test]
fn generate_validate_output_reports_validated_when_the_tool_is_present() {
    let dir = TempDir::new().unwrap();
    let config_path = write_python_project(dir.path());

    let output = scythe_bin()
        .args(["generate", "--config", &config_path, "--validate-output"])
        .output()
        .expect("run scythe generate");

    let err = stderr(&output);
    assert!(
        output.status.success(),
        "a target that passes real-tool validation must not move the exit code; stderr: {err}"
    );
    assert!(
        err.contains("generated-code validation VALIDATED (poly)"),
        "must report the validated state by name, naming the tool that actually ran; stderr: {err}"
    );
    assert!(
        !err.contains("generated-code validation SKIPPED"),
        "a target that was actually checked must not also be reported as skipped; stderr: {err}"
    );
}

/// `scythe generate --validate-output` with the checker unreachable (`PATH`
/// stripped of it) must report SKIPPED, not silently report success.
///
/// `PATH` is overridden to a directory with nothing in it so `poly` -- the
/// only checker `python-psycopg3` output uses -- is genuinely not found by
/// `tool_present`'s real probe spawn, regardless of what happens to be
/// installed on the machine running this test. This is the exact case board
/// #187 calls out: `ValidationOutcome::Passed` with an empty `tools_run` is
/// the underlying library's own "pass" under its non-strict policy, and
/// reporting that as validated would recreate the unfalsifiable gate this
/// flag exists to close.
#[test]
fn generate_validate_output_reports_skipped_when_the_tool_is_absent() {
    let dir = TempDir::new().unwrap();
    let config_path = write_python_project(dir.path());
    let empty_path_dir = TempDir::new().unwrap();

    let output = scythe_bin()
        .args(["generate", "--config", &config_path, "--validate-output"])
        .env("PATH", empty_path_dir.path())
        .output()
        .expect("run scythe generate");

    let err = stderr(&output);
    assert!(
        output.status.success(),
        "a skip (no tool available) must not move the exit code, matching the non-strict \
         default policy `ValidationOutcome`/`ToolValidation::into_result` already document; \
         stderr: {err}"
    );
    assert!(
        err.contains("generated-code validation SKIPPED (poly not installed; nothing was verified)"),
        "must report the skip distinctly, naming the missing tool; stderr: {err}"
    );
    assert!(
        !err.contains("generated-code validation VALIDATED"),
        "a skip must never be reported as validated -- that is exactly the gate-that-cannot-fail \
         this flag exists to close; stderr: {err}"
    );
}

/// `scythe generate --validate-output` against `rust-sqlx` -- a backend
/// `validate_with_tools`'s trailing `_ => return ToolValidation::Unsupported`
/// arm covers, i.e. one whose language was never wired to a real-tool
/// checker at all -- must report SKIPPED and name the backend, regardless of
/// what happens to be on `PATH`.
///
/// Distinct from `generate_validate_output_reports_skipped_when_the_tool_is_absent`
/// above: that test exercises a backend *with* a validator whose tool is
/// merely missing from `PATH`; this one exercises a backend with no
/// validator wired up in the first place, which `ValidationOutcome::Unsupported`
/// -- not `Passed` with an empty `tools_run` -- represents.
#[test]
fn generate_validate_output_reports_skipped_for_a_backend_with_no_validator() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(dir.path(), "rust-sqlx");

    let output = scythe_bin()
        .args(["generate", "--config", &config_path, "--validate-output"])
        .output()
        .expect("run scythe generate");

    let err = stderr(&output);
    assert!(
        output.status.success(),
        "a backend with no validator at all must not move the exit code; stderr: {err}"
    );
    assert!(
        err.contains("generated-code validation SKIPPED (no tool-based validator for backend 'rust-sqlx')"),
        "must name the backend and say no validator exists for it; stderr: {err}"
    );
    assert!(
        !err.contains("generated-code validation VALIDATED"),
        "a backend with no validator must never be reported as validated; stderr: {err}"
    );
}
