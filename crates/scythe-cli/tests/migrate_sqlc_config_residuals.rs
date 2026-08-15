//! Regression tests for #152's third residual: `scythe migrate` used to parse
//! sqlc's top-level `plugins:` array (`SqlcConfig::plugins`) and each `gen.<lang>.package`
//! field (`SqlcGenTarget::package`) and then discard both with no diagnostic at all --
//! `migrate` printed "Generated config: scythe.toml" and exited 0 while those settings
//! never appeared anywhere in the written config. Neither has a `scythe.toml` equivalent
//! (scythe has no wasm/process plugin system, and no backend supports a configurable
//! generated-code package/module name), so both are now a `warning:` on stderr naming
//! what was dropped, rather than silence.
//!
//! Every test drives the compiled binary via `Command::new(env!("CARGO_BIN_EXE_scythe"))`
//! with `.current_dir(...)` set on the child process, following the idiom in
//! `migrate_integration.rs` and `migrate_config_and_stats.rs`.

use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const SCHEMA_SQL: &str = "CREATE TABLE widgets (id bigint PRIMARY KEY, name text NOT NULL);\n";
const SQLC_QUERY_SQL: &str = "-- name: GetWidget :one\nSELECT id, name FROM widgets WHERE id = sqlc.arg(widget_id);\n";

/// #152: a `plugins:` declaration must be named on stderr, not silently dropped.
#[test]
fn migrate_warns_about_undigested_plugins_declaration() {
    let project = TempDir::new().unwrap();
    std::fs::write(project.path().join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::create_dir_all(project.path().join("queries")).unwrap();
    std::fs::write(project.path().join("queries/q.sql"), SQLC_QUERY_SQL).unwrap();

    let sqlc_config = "version: \"2\"\n\
         plugins:\n\
         \x20 - name: golang\n\
         \x20\x20\x20 wasm:\n\
         \x20\x20\x20\x20\x20 url: https://example.com/sqlc-gen-go.wasm\n\
         \x20\x20\x20\x20\x20 sha256: deadbeef\n\
         sql:\n\
         \x20 - schema: schema.sql\n\
         \x20\x20\x20 queries: queries\n\
         \x20\x20\x20 engine: postgresql\n\
         \x20\x20\x20 gen:\n\
         \x20\x20\x20\x20\x20 go:\n\
         \x20\x20\x20\x20\x20\x20\x20 out: generated\n";
    std::fs::write(project.path().join("sqlc.yaml"), sqlc_config).unwrap();

    let output = scythe_bin()
        .args(["migrate", "sqlc.yaml"])
        .current_dir(project.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "migrate must still succeed -- an undigested `plugins:` entry is a warning, not a \
         hard failure; stderr: {}",
        stderr(&output)
    );

    let err = stderr(&output);
    assert!(
        err.contains("warning:") && err.contains("golang") && err.contains("plugins:"),
        "expected a warning naming the undigested plugin 'golang' and the `plugins:` key it came \
         from, got stderr: {err}"
    );
}

/// #152: a `gen.<lang>.package` value must be named on stderr, not silently dropped --
/// scythe has no `scythe.toml` field for it (every generated Go file hardcodes `package
/// queries`), so migrate cannot carry it forward; it must at least say so.
#[test]
fn migrate_warns_about_go_package_with_no_scythe_equivalent() {
    let project = TempDir::new().unwrap();
    std::fs::write(project.path().join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::create_dir_all(project.path().join("queries")).unwrap();
    std::fs::write(project.path().join("queries/q.sql"), SQLC_QUERY_SQL).unwrap();

    let sqlc_config = "version: \"2\"\n\
         sql:\n\
         \x20 - schema: schema.sql\n\
         \x20\x20\x20 queries: queries\n\
         \x20\x20\x20 engine: postgresql\n\
         \x20\x20\x20 gen:\n\
         \x20\x20\x20\x20\x20 go:\n\
         \x20\x20\x20\x20\x20\x20\x20 out: generated\n\
         \x20\x20\x20\x20\x20\x20\x20 package: mydb\n";
    std::fs::write(project.path().join("sqlc.yaml"), sqlc_config).unwrap();

    let output = scythe_bin()
        .args(["migrate", "sqlc.yaml"])
        .current_dir(project.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "migrate must still succeed -- an undigested `gen.go.package` is a warning, not a hard \
         failure; stderr: {}",
        stderr(&output)
    );

    let err = stderr(&output);
    assert!(
        err.contains("warning:") && err.contains("gen.go.package") && err.contains("mydb"),
        "expected a warning naming the undigested package 'mydb' and the `gen.go.package` key it \
         came from, got stderr: {err}"
    );

    let scythe_toml = std::fs::read_to_string(project.path().join("scythe.toml")).expect("scythe.toml must be written");
    assert!(
        !scythe_toml.contains("mydb"),
        "scythe.toml has no field for a generated-code package name, so 'mydb' must not appear \
         in it; got:\n{scythe_toml}"
    );
}
