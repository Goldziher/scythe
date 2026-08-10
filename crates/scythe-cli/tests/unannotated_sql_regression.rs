//! Regression tests for issue #204: a query file with real SQL content but
//! no `-- name:`/`-- @name` annotation used to be silently reduced to zero
//! query blocks by `split_query_file` -- `scythe generate` wrote an empty
//! (or incomplete) output file and exited 0, and `scythe check` reported
//! "All queries valid." on a file `scythe audit` found real findings in.
//!
//! Every test drives the compiled binary via
//! `Command::new(env!("CARGO_BIN_EXE_scythe"))` with `.current_dir(...)` set
//! on the child process, following the idiom in `config_relative_paths.rs`.

use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const SCHEMA_SQL: &str = "CREATE TABLE users (id bigint PRIMARY KEY, name text NOT NULL);\n";

/// SQL with real statements but no `-- name:`/`-- @name` annotation at all
/// -- exactly issue #204's reproduction.
const UNANNOTATED_SQL: &str = "SELECT id, name FROM users WHERE id = $1;\nDELETE FROM users;\n";

fn write_project(dir: &std::path::Path, queries_sql: &str) -> String {
    std::fs::write(dir.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(dir.join("queries.sql"), queries_sql).unwrap();
    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
        schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n";
    let config_path = dir.join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();
    config_path.to_string_lossy().into_owned()
}

/// `scythe generate` must refuse (not silently write an empty/partial
/// output file and exit 0) when a query file has SQL content that would be
/// silently dropped for lack of an annotation.
#[test]
fn generate_refuses_a_query_file_with_no_annotation() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(dir.path(), UNANNOTATED_SQL);

    let output = scythe_bin()
        .args(["generate", "--config", &config_path])
        .output()
        .expect("run scythe generate");

    let err = stderr(&output);
    assert!(
        !output.status.success(),
        "generate must not silently succeed on a query file with no annotation; stderr: {err}"
    );
    assert!(
        err.contains("annotation"),
        "the error must explain the missing-annotation problem; stderr: {err}"
    );
    assert!(
        err.contains("queries.sql"),
        "the error must name the offending file; stderr: {err}"
    );
}

/// `scythe check` must not report "All queries valid." (or exit 0) on a
/// file whose real content never made it into a query block at all.
#[test]
fn check_refuses_a_query_file_with_no_annotation() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(dir.path(), UNANNOTATED_SQL);

    let output = scythe_bin()
        .args(["check", "--config", &config_path])
        .output()
        .expect("run scythe check");

    let err = stderr(&output);
    assert!(
        !output.status.success(),
        "check must not silently succeed on a query file with no annotation; stderr: {err}"
    );
    assert!(
        !err.contains("All queries valid."),
        "check must not claim validity for content it never examined; stderr: {err}"
    );
    assert!(err.contains("annotation"), "stderr: {err}");
}

/// `scythe lint` (config mode) must not report "No lint violations found."
/// on a file whose content was silently dropped for lack of an annotation.
#[test]
fn lint_refuses_a_query_file_with_no_annotation() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(dir.path(), UNANNOTATED_SQL);

    let output = scythe_bin()
        .args(["lint", "--config", &config_path])
        .output()
        .expect("run scythe lint");

    let err = stderr(&output);
    assert!(
        !output.status.success(),
        "lint must not silently succeed on a query file with no annotation; stderr: {err}"
    );
    assert!(
        !err.contains("No lint violations found."),
        "lint must not claim a clean run over content it never examined; stderr: {err}"
    );
}

/// Negative control: a file whose only pre-annotation content is an
/// ordinary comment header (not a real statement) must be accepted, exactly
/// as before -- this is the common, legitimate case (a license header, or
/// an explanatory comment) and must not be flagged.
#[test]
fn generate_accepts_a_leading_comment_header() {
    let dir = TempDir::new().unwrap();
    let sql = "-- Copyright 2024 Example Corp\n-- SPDX-License-Identifier: MIT\n\n\
        -- @name GetUser\n-- @returns :one\nSELECT id, name FROM users WHERE id = $1;\n";
    let config_path = write_project(dir.path(), sql);

    let output = scythe_bin()
        .args(["generate", "--config", &config_path])
        .output()
        .expect("run scythe generate");

    assert!(
        output.status.success(),
        "a leading comment header (not a real statement) must not be flagged; stderr: {}",
        stderr(&output)
    );
}

/// Negative control: a well-formed file (annotation always precedes its
/// SQL) must be accepted by `check`.
#[test]
fn check_accepts_a_well_formed_query_file() {
    let dir = TempDir::new().unwrap();
    let sql = "-- @name GetUser\n-- @returns :one\nSELECT id, name FROM users WHERE id = $1;\n";
    let config_path = write_project(dir.path(), sql);

    let output = scythe_bin()
        .args(["check", "--config", &config_path])
        .output()
        .expect("run scythe check");

    let err = stderr(&output);
    assert!(
        output.status.success(),
        "well-formed queries must pass check; stderr: {err}"
    );
    assert!(err.contains("All queries valid."), "stderr: {err}");
}
