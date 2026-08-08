//! End-to-end regression tests for GitHub issue #97: `scythe migrate` wrote
//! `target = "<lang>"` verbatim from the sqlc plugin name into
//! `[sql.gen.<lang>]`, and `scythe generate` builds the backend name as
//! `format!("{lang}-{target}")` -- so a migrated Go project got
//! `target = "go"` and the unusable backend `go-go`, and a migrated Kotlin
//! project (for which the legacy `[sql.gen.kotlin]` table had no field at
//! all in `LegacyGenConfig`) was silently dropped and generated `rust-sqlx`
//! output instead, with no error whatsoever.
//!
//! Every test here drives the compiled binary via
//! `Command::new(env!("CARGO_BIN_EXE_scythe"))` with `.current_dir(...)` set
//! on the *child* process -- never a process-wide CWD mutation -- following
//! the idiom in `config_relative_paths.rs` and `migrate_integration.rs`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const SCHEMA_SQL: &str = "CREATE TABLE users (id bigint PRIMARY KEY, name text NOT NULL, email text NOT NULL);\n";

const SQLC_QUERY_SQL: &str = "-- name: GetUser :one\nSELECT id, name, email FROM users WHERE id = sqlc.arg(user_id);\n";

/// Write `schema.sql`, `queries/q.sql`, and a stock sqlc v2 yaml config whose
/// `gen:` block generates only `gen_lang` (e.g. `"go"` or `"kotlin"`) into a
/// fresh project directory. Mirrors the shape a real `sqlc generate --lang
/// go` / `--lang kotlin` project ships with.
fn write_sqlc_project(project_dir: &Path, gen_lang: &str) -> PathBuf {
    std::fs::create_dir_all(project_dir).unwrap();
    std::fs::write(project_dir.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::create_dir_all(project_dir.join("queries")).unwrap();
    std::fs::write(project_dir.join("queries").join("q.sql"), SQLC_QUERY_SQL).unwrap();

    let config = format!(
        "version: \"2\"\n\
         sql:\n\
         \x20 - schema: schema.sql\n\
         \x20\x20\x20 queries: queries\n\
         \x20\x20\x20 engine: postgresql\n\
         \x20\x20\x20 gen:\n\
         \x20\x20\x20\x20\x20 {gen_lang}:\n\
         \x20\x20\x20\x20\x20\x20\x20 out: db\n"
    );
    let config_path = project_dir.join("sqlc.yaml");
    std::fs::write(&config_path, config).unwrap();
    config_path
}

fn run_migrate(config_path: &Path, run_from: &Path) -> Output {
    scythe_bin()
        .args(["migrate", config_path.to_str().unwrap()])
        .current_dir(run_from)
        .output()
        .expect("run scythe migrate")
}

fn run_generate(scythe_toml: &Path, run_from: &Path) -> Output {
    scythe_bin()
        .args(["generate", "--config", scythe_toml.to_str().unwrap()])
        .current_dir(run_from)
        .output()
        .expect("run scythe generate")
}

/// THE REGRESSION TEST for the Go half of issue #97. Before the fix,
/// `migrate` wrote `[sql.gen.go] target = "go"` (the sqlc plugin name,
/// verbatim) and `generate` built the backend as `format!("go-{target}")`,
/// producing the nonexistent backend `go-go` -- `scythe generate` reported
/// success from `migrate` but then hard-failed with
/// `unknown backend: go-go`. After the fix, `migrate` picks a real driver
/// (`pgx`, for the `postgresql` engine) so the round trip actually works.
#[test]
fn migrate_then_generate_go_produces_a_working_backend() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();
    let config_path = write_sqlc_project(project.path(), "go");

    let migrate_output = run_migrate(&config_path, run_from.path());
    assert!(
        migrate_output.status.success(),
        "migrate must succeed; stderr: {}",
        stderr(&migrate_output)
    );

    let scythe_toml = project.path().join("scythe.toml");
    let toml_content = std::fs::read_to_string(&scythe_toml).unwrap();
    assert!(
        !toml_content.contains("target = \"go\""),
        "migrate must not write the bare language name back as the driver -- that produces the \
         unusable backend 'go-go'; got:\n{toml_content}"
    );
    assert!(
        toml_content.contains("[sql.gen.go]") && toml_content.contains("target = \"pgx\""),
        "expected [sql.gen.go] with a real driver target (pgx for the postgresql engine), \
         got:\n{toml_content}"
    );

    let generate_output = run_generate(&scythe_toml, run_from.path());
    assert!(
        generate_output.status.success(),
        "generate must succeed against a migrated Go config; stderr: {}",
        stderr(&generate_output)
    );
    assert!(
        !stderr(&generate_output).contains("unknown backend"),
        "must not hit the go-go 'unknown backend' failure; stderr: {}",
        stderr(&generate_output)
    );

    let generated_file = project.path().join("db").join("queries.go");
    let content = std::fs::read_to_string(&generated_file)
        .unwrap_or_else(|e| panic!("expected generated Go file at {}: {e}", generated_file.display()));
    assert!(
        content.to_lowercase().contains("user"),
        "expected real Go code for the GetUser query, got:\n{content}"
    );
}

/// THE REGRESSION TEST for the Kotlin half of issue #97 -- the more severe
/// bug: before the fix, `LegacyGenConfig` in `generate.rs` had no `kotlin`
/// field at all, so a migrated Kotlin project's `[sql.gen.kotlin]` table was
/// silently ignored by serde's default unknown-field handling.
/// `resolve_gen_targets` then found zero targets and fell back to its own
/// `rust-sqlx` default -- `scythe generate` reported success and wrote
/// **Rust** code (`db/queries.rs`) for a Kotlin project, with no warning or
/// error of any kind. After the fix, `[sql.gen.kotlin]` resolves to a real
/// `kotlin-jdbc` backend and Kotlin code is what actually gets written.
#[test]
fn migrate_then_generate_kotlin_produces_kotlin_not_silent_rust_sqlx() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();
    let config_path = write_sqlc_project(project.path(), "kotlin");

    let migrate_output = run_migrate(&config_path, run_from.path());
    assert!(
        migrate_output.status.success(),
        "migrate must succeed; stderr: {}",
        stderr(&migrate_output)
    );

    let scythe_toml = project.path().join("scythe.toml");
    let toml_content = std::fs::read_to_string(&scythe_toml).unwrap();
    assert!(
        toml_content.contains("[sql.gen.kotlin]"),
        "migrate must preserve the kotlin gen target, got:\n{toml_content}"
    );
    assert!(
        toml_content.contains("target = \"jdbc\""),
        "expected a real kotlin driver target (jdbc), got:\n{toml_content}"
    );

    let generate_output = run_generate(&scythe_toml, run_from.path());
    assert!(
        generate_output.status.success(),
        "generate must succeed against a migrated Kotlin config; stderr: {}",
        stderr(&generate_output)
    );

    let rust_fallback = project.path().join("db").join("queries.rs");
    assert!(
        !rust_fallback.exists(),
        "must not silently fall back to the rust-sqlx default for a Kotlin project; found {}",
        rust_fallback.display()
    );

    let generated_file = project.path().join("db").join("queries.kt");
    let content = std::fs::read_to_string(&generated_file)
        .unwrap_or_else(|e| panic!("expected generated Kotlin file at {}: {e}", generated_file.display()));
    assert!(
        content.to_lowercase().contains("user"),
        "expected real Kotlin code for the GetUser query, got:\n{content}"
    );
}

/// sqlc v1 `packages` config predates the multi-language `gen:` block and
/// only ever generated Go code. Before the fix, `migrate` emitted no gen
/// block at all for v1 projects, so `resolve_gen_targets` silently defaulted
/// to `rust-sqlx` -- the same silent-wrong-language failure mode as the
/// Kotlin case above, just for every v1 migration.
#[test]
fn migrate_v1_packages_then_generate_produces_go_not_silent_rust_sqlx() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();

    std::fs::create_dir_all(project.path()).unwrap();
    std::fs::write(project.path().join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::create_dir_all(project.path().join("queries")).unwrap();
    std::fs::write(project.path().join("queries").join("q.sql"), SQLC_QUERY_SQL).unwrap();

    let config = "version: \"1\"\n\
                   packages:\n\
                   \x20 - name: main\n\
                   \x20\x20\x20 path: db\n\
                   \x20\x20\x20 queries: queries\n\
                   \x20\x20\x20 schema: schema.sql\n\
                   \x20\x20\x20 engine: postgresql\n";
    let config_path = project.path().join("sqlc.yaml");
    std::fs::write(&config_path, config).unwrap();

    let migrate_output = run_migrate(&config_path, run_from.path());
    assert!(
        migrate_output.status.success(),
        "migrate must succeed; stderr: {}",
        stderr(&migrate_output)
    );

    let scythe_toml = project.path().join("scythe.toml");
    let generate_output = run_generate(&scythe_toml, run_from.path());
    assert!(
        generate_output.status.success(),
        "generate must succeed against a migrated v1 config; stderr: {}",
        stderr(&generate_output)
    );

    let rust_fallback = project.path().join("db").join("queries.rs");
    assert!(
        !rust_fallback.exists(),
        "v1 sqlc configs are Go-only -- must not silently fall back to rust-sqlx; found {}",
        rust_fallback.display()
    );
    assert!(
        project.path().join("db").join("queries.go").exists(),
        "expected Go output for a migrated v1 (Go-only) sqlc project"
    );
}

/// A `gen:` language scythe has no backend for must fail migration loudly,
/// naming the offending language, rather than silently writing a
/// `scythe.toml` that omits it entirely.
#[test]
fn migrate_fails_loudly_on_unsupported_gen_language() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();

    std::fs::create_dir_all(project.path()).unwrap();
    std::fs::write(project.path().join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::create_dir_all(project.path().join("queries")).unwrap();
    std::fs::write(project.path().join("queries").join("q.sql"), SQLC_QUERY_SQL).unwrap();

    // "csharp" is not one of sqlc's built-in gen plugins (go/kotlin/python),
    // and scythe migrate has no mapping for it -- it must be rejected, not
    // silently dropped.
    let config = "version: \"2\"\n\
                   sql:\n\
                   \x20 - schema: schema.sql\n\
                   \x20\x20\x20 queries: queries\n\
                   \x20\x20\x20 engine: postgresql\n\
                   \x20\x20\x20 gen:\n\
                   \x20\x20\x20\x20\x20 csharp:\n\
                   \x20\x20\x20\x20\x20\x20\x20 out: db\n";
    let config_path = project.path().join("sqlc.yaml");
    std::fs::write(&config_path, config).unwrap();

    let migrate_output = run_migrate(&config_path, run_from.path());
    assert!(
        !migrate_output.status.success(),
        "migrate must fail loudly on a gen language it has no backend for, not silently drop it"
    );
    let err = stderr(&migrate_output);
    assert!(
        err.contains("csharp"),
        "error must name the unsupported gen language; stderr: {err}"
    );
}
