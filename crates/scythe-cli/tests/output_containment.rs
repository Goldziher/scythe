//! Regression tests for issue #207: `scythe generate` used to create
//! directories and write wherever `[[sql.gen]].output` pointed, with no
//! containment check against the project root -- a `../../ESCAPED` or an
//! absolute path was silently honoured, making `scythe generate` run in CI
//! against a PR-modified `scythe.toml` an arbitrary
//! directory-create-and-write primitive. Two targets sharing one `output`
//! also silently clobbered each other, both reporting success.
//!
//! Every test drives the compiled binary via
//! `Command::new(env!("CARGO_BIN_EXE_scythe"))`.

use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const SCHEMA_SQL: &str = "CREATE TABLE users (id bigint PRIMARY KEY, name text NOT NULL);\n";
const QUERY_SQL: &str = "-- @name GetUser\n-- @returns :one\nSELECT id, name FROM users WHERE id = $1;\n";

/// A `[[sql.gen]]` target whose `output` escapes the project root via `../`
/// traversal must be rejected by default, and nothing must be written --
/// not even to the escaped location.
#[test]
fn generate_rejects_parent_traversal_output_by_default() {
    let root = TempDir::new().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(project.join("queries.sql"), QUERY_SQL).unwrap();

    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
        schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n\n\
        [[sql.gen]]\nbackend = \"rust-sqlx\"\noutput = \"../../ESCAPED\"\n";
    let config_path = project.join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args(["generate", "--config", config_path.to_str().unwrap()])
        .output()
        .expect("run scythe generate");

    let err = stderr(&output);
    assert!(
        !output.status.success(),
        "a ../ traversal output must be rejected by default; stderr: {err}"
    );
    assert!(
        err.contains("escapes"),
        "stderr should explain the containment violation: {err}"
    );
    assert!(
        !root.path().join("ESCAPED").exists(),
        "nothing must be written to the escaped location"
    );
}

/// The same config succeeds when `--allow-output-escape` is passed, proving
/// the opt-out actually works (not just that the default rejects).
#[test]
fn generate_allows_parent_traversal_output_with_opt_out_flag() {
    let root = TempDir::new().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(project.join("queries.sql"), QUERY_SQL).unwrap();

    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
        schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n\n\
        [[sql.gen]]\nbackend = \"rust-sqlx\"\noutput = \"../ESCAPED\"\n";
    let config_path = project.join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args([
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--allow-output-escape",
        ])
        .output()
        .expect("run scythe generate");

    assert!(
        output.status.success(),
        "--allow-output-escape must let a deliberate escape through; stderr: {}",
        stderr(&output)
    );
    assert!(
        root.path().join("ESCAPED/queries.rs").exists(),
        "expected the escaped output to actually be written when explicitly allowed"
    );
}

/// A `[[sql.gen]]` target whose `output` is an absolute path must also be
/// rejected by default, even though it happens to still land inside a
/// temp-dir sandbox -- the rejection is a purely lexical, absolute-path
/// check, not a filesystem containment probe.
#[test]
fn generate_rejects_absolute_output_by_default() {
    let root = TempDir::new().unwrap();
    let project = root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(project.join("queries.sql"), QUERY_SQL).unwrap();

    let abs_output = root.path().join("abs_out");
    let config = format!(
        "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
        schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n\n\
        [[sql.gen]]\nbackend = \"rust-sqlx\"\noutput = \"{}\"\n",
        abs_output.display().to_string().replace('\\', "/"),
    );
    let config_path = project.join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args(["generate", "--config", config_path.to_str().unwrap()])
        .output()
        .expect("run scythe generate");

    assert!(
        !output.status.success(),
        "an absolute output must be rejected by default; stderr: {}",
        stderr(&output)
    );
}

/// Two `[[sql.gen]]` targets that resolve to the same (output dir, filename)
/// pair must be rejected as a collision before either one is written --
/// previously the second target silently clobbered the first's generated
/// code and both reported success.
#[test]
fn generate_rejects_two_targets_that_would_clobber_each_other() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(dir.path().join("queries.sql"), QUERY_SQL).unwrap();

    // python-psycopg3 and python-asyncpg are both Python backends, so both
    // emit "queries.py" -- sharing `output = "o"` makes them collide.
    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
        schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n\n\
        [[sql.gen]]\nbackend = \"python-psycopg3\"\noutput = \"o\"\n\n\
        [[sql.gen]]\nbackend = \"python-asyncpg\"\noutput = \"o\"\n";
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args(["generate", "--config", config_path.to_str().unwrap()])
        .current_dir(dir.path())
        .output()
        .expect("run scythe generate");

    let err = stderr(&output);
    assert!(
        !output.status.success(),
        "two targets writing the same file must be rejected, not silently clobber each other; stderr: {err}"
    );
    assert!(
        err.contains("python-psycopg3") && err.contains("python-asyncpg"),
        "the error must name both colliding targets; stderr: {err}"
    );
    assert!(
        !dir.path().join("o/queries.py").exists(),
        "neither target's output should have been written once a collision is detected"
    );
}

/// Negative control: a normal, contained relative `output` must still work
/// with no flag needed.
#[test]
fn generate_allows_a_normal_relative_output() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(dir.path().join("queries.sql"), QUERY_SQL).unwrap();

    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
        schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n\n\
        [[sql.gen]]\nbackend = \"rust-sqlx\"\noutput = \"generated\"\n";
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args(["generate", "--config", config_path.to_str().unwrap()])
        .current_dir(dir.path())
        .output()
        .expect("run scythe generate");

    assert!(
        output.status.success(),
        "a normal relative output must not require --allow-output-escape; stderr: {}",
        stderr(&output)
    );
    assert!(dir.path().join("generated/queries.rs").exists());
}
