use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

const QUERY: &str = "-- @name ListUsers\n-- @returns :many\nSELECT id, name FROM users;\n";

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn write_project(directory: &Path, schema: &str, source_line: &str) {
    std::fs::write(directory.join("schema.sql"), schema).unwrap();
    std::fs::write(directory.join("queries.sql"), QUERY).unwrap();
    std::fs::write(
        directory.join("scythe.toml"),
        format!(
            r#"[scythe]
version = "1"

[[sql]]
name = "main"
engine = "sqlite"
{source_line}schema = ["schema.sql"]
queries = ["queries.sql"]
output = "generated"
"#,
        ),
    )
    .unwrap();
}

#[test]
fn omitted_schema_source_matches_explicit_parse_output_byte_for_byte() {
    let omitted = TempDir::new().unwrap();
    let explicit = TempDir::new().unwrap();
    let schema = "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);\n";
    write_project(omitted.path(), schema, "");
    write_project(explicit.path(), schema, "schema_source = \"parse\"\n");

    for project in [&omitted, &explicit] {
        let output = scythe_bin()
            .args(["generate", "--config", "scythe.toml"])
            .current_dir(project.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "generate failed: {}", stderr(&output));
    }

    let omitted_code = std::fs::read(omitted.path().join("generated/queries.rs")).unwrap();
    let explicit_code = std::fs::read(explicit.path().join("generated/queries.rs")).unwrap();
    assert_eq!(omitted_code, explicit_code);
}

#[test]
fn all_configured_commands_use_execute_without_parse_fallback() {
    let project = TempDir::new().unwrap();
    write_project(
        project.path(),
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);\nCREATE TABLE users (id INTEGER);\n",
        "schema_source = \"execute\"\n",
    );

    for command in ["generate", "check", "lint", "audit"] {
        let output = scythe_bin()
            .args([command, "--config", "scythe.toml"])
            .current_dir(project.path())
            .env_remove("DATABASE_URL")
            .env_remove("SCYTHE_DATABASE_URL")
            .output()
            .unwrap();
        let error = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{command} stderr: {error}");
        assert!(
            error.contains(&format!("{command} [main]")),
            "{command} stderr: {error}"
        );
        assert!(error.contains("schema_source=execute"), "{command} stderr: {error}");
        assert!(error.contains("engine `sqlite`"), "{command} stderr: {error}");
        assert!(error.contains("schema.sql"), "{command} stderr: {error}");
        assert!(error.contains("already exists"), "{command} stderr: {error}");
    }
}

#[test]
fn unsupported_execute_engine_fails_before_schema_glob_resolution() {
    let project = TempDir::new().unwrap();
    std::fs::write(
        project.path().join("scythe.toml"),
        r#"[scythe]
version = "1"

[[sql]]
name = "warehouse"
engine = "postgresql"
schema_source = "execute"
schema = ["missing-schema.sql"]
queries = ["missing-queries.sql"]
"#,
    )
    .unwrap();

    for command in ["generate", "check", "lint", "audit"] {
        let output = scythe_bin()
            .args([command, "--config", "scythe.toml"])
            .current_dir(project.path())
            .env_remove("DATABASE_URL")
            .env_remove("SCYTHE_DATABASE_URL")
            .output()
            .unwrap();
        let error = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{command} stderr: {error}");
        assert!(
            error.contains(&format!("{command} [warehouse]")),
            "{command} stderr: {error}"
        );
        assert!(
            error.contains("unsupported for engine `postgresql`"),
            "{command} stderr: {error}"
        );
        assert!(!error.contains("matched no files"), "{command} stderr: {error}");
    }
}
