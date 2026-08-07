//! End-to-end tests for the per-target manifest override half of GitHub
//! issue #82: `manifest = "..."` on a `[[sql.gen]]` target names a *partial*
//! manifest that is merged over the backend's compiled-in one.
//!
//! The other half of #82 — removing the implicit, working-directory-relative
//! `backends/<name>/manifest.toml` lookup — is covered by
//! `manifest_determinism.rs`. That lookup was engine-blind: all five
//! `rust-sqlx` engine arms probed the same path and `java-jdbc` collapsed
//! nine engines onto one, so a MySQL target could silently receive
//! PostgreSQL type mappings. The replacement is therefore keyed per
//! `[[sql.gen]]` target — which already names a backend and inherits its
//! engine from the enclosing `[[sql]]` block — and never globally by backend
//! name. `override_is_per_target_not_global_across_engines` is the
//! regression test for that specific property.
//!
//! Every test drives the compiled binary via
//! `Command::new(env!("CARGO_BIN_EXE_scythe"))` with `.current_dir(...)` set
//! on the *child* process — never a process-wide CWD mutation — so tests stay
//! parallel-safe, following the idiom in `config_relative_paths.rs` and
//! `migrate_integration.rs`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Render a path as a string safe to embed in a TOML basic string (`"..."`).
/// `Path::display()` uses the native separator, which on Windows is `\` — an
/// escape character in TOML basic strings.
fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

const SCHEMA_SQL: &str = "CREATE TABLE widgets (id bigint PRIMARY KEY, name text NOT NULL);\n";

const QUERY_SQL: &str = "-- @name GetWidget\n-- @returns :one\nSELECT id, name FROM widgets WHERE id = $1;\n";

/// A type name that appears in no compiled-in manifest, so finding it in the
/// generated output can only mean the overlay was applied.
const SENTINEL_TYPE: &str = "ScytheOverrideSentinelInt";

/// Write `schema.sql` and `queries.sql` into `dir`.
fn write_sources(dir: &Path) {
    std::fs::write(dir.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(dir.join("queries.sql"), QUERY_SQL).unwrap();
}

/// A minimal overlay remapping the `int64` neutral type, which the `bigint`
/// primary key in `SCHEMA_SQL` resolves to.
fn int64_overlay(rust_type: &str) -> String {
    format!("[types.scalars]\nint64 = \"{rust_type}\"\n")
}

/// A single-block `scythe.toml` with one `rust-sqlx` target, optionally
/// carrying a `manifest = "..."` line.
fn config_with_manifest(manifest_line: &str) -> String {
    format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[[sql.gen]]
backend = "rust-sqlx"
output = "out"
{manifest_line}
"#
    )
}

/// Build a self-contained project in a fresh temp dir and return it plus the
/// path to its `scythe.toml`.
fn project(config: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    write_sources(dir.path());
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();
    (dir, config_path)
}

/// Run `scythe generate --config <config_path>` from `cwd`.
fn generate_from(config_path: &Path, cwd: &Path) -> Output {
    scythe_bin()
        .args(["generate", "--config", &config_path.display().to_string()])
        .current_dir(cwd)
        .output()
        .expect("failed to run scythe generate")
}

/// Requirement 1: an override changes a scalar type mapping and the generated
/// code reflects it. Without the override wiring, `id` is emitted as the
/// compiled-in `i64`.
#[test]
fn scalar_override_changes_the_generated_type() {
    let (dir, config_path) = project(&config_with_manifest(r#"manifest = "custom.toml""#));
    std::fs::write(dir.path().join("custom.toml"), int64_overlay(SENTINEL_TYPE)).unwrap();

    let output = generate_from(&config_path, dir.path());
    assert!(
        output.status.success(),
        "generate should succeed, stderr:\n{}",
        stderr(&output)
    );

    let generated = std::fs::read_to_string(dir.path().join("out").join("queries.rs")).unwrap();
    assert!(
        generated.contains(SENTINEL_TYPE),
        "the overridden int64 mapping must appear in the generated code, got:\n{generated}"
    );
    assert!(
        !generated.contains("pub id: i64"),
        "the compiled-in i64 mapping must have been replaced, got:\n{generated}"
    );
}

/// The overlay is a *partial* manifest: keys it does not mention keep their
/// compiled-in values. `name text NOT NULL` must still resolve to `String`
/// even though the overlay only restates `int64`.
#[test]
fn override_is_an_overlay_not_a_replacement() {
    let (dir, config_path) = project(&config_with_manifest(r#"manifest = "custom.toml""#));
    std::fs::write(dir.path().join("custom.toml"), int64_overlay(SENTINEL_TYPE)).unwrap();

    let output = generate_from(&config_path, dir.path());
    assert!(output.status.success(), "generate failed:\n{}", stderr(&output));

    let generated = std::fs::read_to_string(dir.path().join("out").join("queries.rs")).unwrap();
    // Both halves are asserted together: "the unmentioned key survived" is
    // trivially true if the overlay was never applied at all, so on its own it
    // would still pass with the feature ripped out.
    assert!(
        generated.contains(SENTINEL_TYPE),
        "the mentioned scalar must be overridden, got:\n{generated}"
    );
    assert!(
        generated.contains("pub name: String"),
        "an unmentioned scalar must keep its compiled-in mapping, got:\n{generated}"
    );
}

/// Requirement 2: `manifest` resolves relative to the directory containing
/// `scythe.toml`, not the process's current working directory — the 0.13.0
/// (#84) convention every other path in the config follows. Resolving against
/// the CWD would reintroduce exactly the bug #82 reported.
#[test]
fn manifest_path_resolves_relative_to_the_config_file_not_the_cwd() {
    let (dir, config_path) = project(&config_with_manifest(r#"manifest = "manifests/custom.toml""#));
    std::fs::create_dir(dir.path().join("manifests")).unwrap();
    std::fs::write(
        dir.path().join("manifests").join("custom.toml"),
        int64_overlay(SENTINEL_TYPE),
    )
    .unwrap();

    // An unrelated directory with no `manifests/` subdirectory at all: if
    // resolution were CWD-relative, the read would fail outright.
    let elsewhere = TempDir::new().unwrap();

    let output = generate_from(&config_path, elsewhere.path());
    assert!(
        output.status.success(),
        "generate from an unrelated CWD should succeed, stderr:\n{}",
        stderr(&output)
    );

    let generated = std::fs::read_to_string(dir.path().join("out").join("queries.rs")).unwrap();
    assert!(
        generated.contains(SENTINEL_TYPE),
        "the config-relative override must be found from any CWD, got:\n{generated}"
    );
}

/// Running the same project from its own directory and from an unrelated one
/// must produce byte-identical output. This is the property #82 is about,
/// stated directly.
#[test]
fn output_is_byte_identical_regardless_of_cwd() {
    let (dir, config_path) = project(&config_with_manifest(r#"manifest = "manifests/custom.toml""#));
    std::fs::create_dir(dir.path().join("manifests")).unwrap();
    std::fs::write(
        dir.path().join("manifests").join("custom.toml"),
        int64_overlay(SENTINEL_TYPE),
    )
    .unwrap();

    let generated_path = dir.path().join("out").join("queries.rs");

    assert!(generate_from(&config_path, dir.path()).status.success());
    let from_project_dir = std::fs::read_to_string(&generated_path).unwrap();

    let elsewhere = TempDir::new().unwrap();
    assert!(generate_from(&config_path, elsewhere.path()).status.success());
    let from_elsewhere = std::fs::read_to_string(&generated_path).unwrap();

    assert_eq!(
        from_project_dir, from_elsewhere,
        "generated output must not depend on the process working directory"
    );
}

/// Requirement 3a: a misspelled *section or field* is rejected by
/// `#[serde(deny_unknown_fields)]`, and the error names the backend, the
/// resolved override path, and the offending key.
#[test]
fn unknown_section_errors_naming_backend_and_key() {
    let (dir, config_path) = project(&config_with_manifest(r#"manifest = "custom.toml""#));
    // `scalar`, not `scalars`.
    std::fs::write(dir.path().join("custom.toml"), "[types.scalar]\nint64 = \"Whatever\"\n").unwrap();

    let output = generate_from(&config_path, dir.path());
    assert!(!output.status.success(), "a misspelled section must fail the run");

    let message = stderr(&output);
    assert!(
        message.contains("rust-sqlx"),
        "error must name the backend, got:\n{message}"
    );
    assert!(
        message.contains("scalar"),
        "error must name the offending key, got:\n{message}"
    );
    assert!(
        message.contains("custom.toml"),
        "error must name the override file, got:\n{message}"
    );
}

/// A `[backend]` table is not overridable: manifest selection stays a pure
/// function of `(backend, engine)`, so an override may not rewrite the
/// engine or the file extension out from under it.
#[test]
fn backend_section_is_rejected() {
    let (dir, config_path) = project(&config_with_manifest(r#"manifest = "custom.toml""#));
    std::fs::write(dir.path().join("custom.toml"), "[backend]\nengine = \"mysql\"\n").unwrap();

    let output = generate_from(&config_path, dir.path());
    assert!(!output.status.success(), "a [backend] override must fail the run");
    assert!(
        stderr(&output).contains("backend"),
        "error must name the rejected section, got:\n{}",
        stderr(&output)
    );
}

/// Requirement 3b: a misspelled *leaf key* in `[types.scalars]` is rejected
/// too. Neutral type names are a fixed vocabulary, so `int_64` is a typo, and
/// accepting it silently would leave `int64` mapped to its compiled-in value
/// and generate code the user did not ask for.
#[test]
fn unknown_scalar_key_errors_naming_backend_and_key() {
    let (dir, config_path) = project(&config_with_manifest(r#"manifest = "custom.toml""#));
    std::fs::write(
        dir.path().join("custom.toml"),
        format!("[types.scalars]\nint_64 = \"{SENTINEL_TYPE}\"\n"),
    )
    .unwrap();

    let output = generate_from(&config_path, dir.path());
    assert!(!output.status.success(), "a misspelled scalar key must fail the run");

    let message = stderr(&output);
    assert!(
        message.contains("rust-sqlx"),
        "error must name the backend, got:\n{message}"
    );
    assert!(
        message.contains("int_64"),
        "error must name the offending key, got:\n{message}"
    );
    assert!(
        message.contains("int64"),
        "error should suggest the near-miss key, got:\n{message}"
    );
}

/// Requirement 4: a missing override file is an error naming the resolved
/// absolute path — never a silent fallback to the compiled-in manifest, which
/// is what made the old implicit lookup unsafe.
#[test]
fn missing_override_file_errors_naming_the_resolved_path() {
    let (dir, config_path) = project(&config_with_manifest(r#"manifest = "manifests/nope.toml""#));

    let output = generate_from(&config_path, dir.path());
    assert!(!output.status.success(), "a missing override file must fail the run");

    let message = stderr(&output);
    let expected = toml_path(&dir.path().join("manifests").join("nope.toml"));
    assert!(
        message.replace('\\', "/").contains(&expected),
        "error must name the resolved absolute path '{expected}', got:\n{message}"
    );
    assert!(
        message.contains("rust-sqlx"),
        "error must name the backend, got:\n{message}"
    );
}

/// Requirement 5: the engine-blindness guard. Two `[[sql]]` blocks use the
/// *same* backend name (`rust-sqlx`) under *different* engines, each with its
/// own override. Each target must pick up its own file and only its own —
/// the failure mode of the removed global, engine-blind lookup was handing
/// one manifest to every engine.
#[test]
fn override_is_per_target_not_global_across_engines() {
    let dir = TempDir::new().unwrap();
    write_sources(dir.path());
    std::fs::write(dir.path().join("pg.toml"), int64_overlay("PostgresOnlySentinel")).unwrap();
    std::fs::write(dir.path().join("my.toml"), int64_overlay("MysqlOnlySentinel")).unwrap();

    let config = r#"[scythe]
version = "1"

[[sql]]
name = "pg"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[[sql.gen]]
backend = "rust-sqlx"
output = "out-pg"
manifest = "pg.toml"

[[sql]]
name = "my"
engine = "mysql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[[sql.gen]]
backend = "rust-sqlx"
output = "out-my"
manifest = "my.toml"
"#;
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = generate_from(&config_path, dir.path());
    assert!(output.status.success(), "generate failed:\n{}", stderr(&output));

    let pg = std::fs::read_to_string(dir.path().join("out-pg").join("queries.rs")).unwrap();
    let my = std::fs::read_to_string(dir.path().join("out-my").join("queries.rs")).unwrap();

    assert!(
        pg.contains("PostgresOnlySentinel"),
        "the postgres target must use its own override, got:\n{pg}"
    );
    assert!(
        !pg.contains("MysqlOnlySentinel"),
        "the mysql target's override must not leak into the postgres target, got:\n{pg}"
    );
    assert!(
        my.contains("MysqlOnlySentinel"),
        "the mysql target must use its own override, got:\n{my}"
    );
    assert!(
        !my.contains("PostgresOnlySentinel"),
        "the postgres target's override must not leak into the mysql target, got:\n{my}"
    );
}

/// The complement of the test above: within a single `[[sql]]` block, a
/// target *without* `manifest` must be untouched by a sibling target that has
/// one. The override is a property of the target, not of the run.
#[test]
fn override_does_not_leak_to_sibling_targets_without_one() {
    let dir = TempDir::new().unwrap();
    write_sources(dir.path());
    std::fs::write(dir.path().join("custom.toml"), int64_overlay(SENTINEL_TYPE)).unwrap();

    let config = r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]

[[sql.gen]]
backend = "rust-sqlx"
output = "out-overridden"
manifest = "custom.toml"

[[sql.gen]]
backend = "rust-sqlx"
output = "out-plain"
"#;
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = generate_from(&config_path, dir.path());
    assert!(output.status.success(), "generate failed:\n{}", stderr(&output));

    let overridden = std::fs::read_to_string(dir.path().join("out-overridden").join("queries.rs")).unwrap();
    let plain = std::fs::read_to_string(dir.path().join("out-plain").join("queries.rs")).unwrap();

    assert!(
        overridden.contains(SENTINEL_TYPE),
        "the target with `manifest` must be overridden, got:\n{overridden}"
    );
    assert!(
        !plain.contains(SENTINEL_TYPE),
        "the sibling target without `manifest` must be untouched, got:\n{plain}"
    );
    assert!(
        plain.contains("pub id: i64"),
        "the sibling target must keep the compiled-in mapping, got:\n{plain}"
    );
}

/// `manifest` must not be swallowed by the `#[serde(flatten)]` options
/// catch-all on `[[sql.gen]]`. If it were, it would be handed to
/// `apply_options` as a string option, every backend that does not recognise
/// the key would ignore it, and the line would silently do nothing.
#[test]
fn manifest_key_is_not_treated_as_a_backend_option() {
    let (dir, config_path) = project(&config_with_manifest(r#"manifest = "custom.toml""#));
    std::fs::write(dir.path().join("custom.toml"), int64_overlay(SENTINEL_TYPE)).unwrap();

    let output = generate_from(&config_path, dir.path());
    assert!(output.status.success(), "generate failed:\n{}", stderr(&output));

    let generated = std::fs::read_to_string(dir.path().join("out").join("queries.rs")).unwrap();
    assert!(
        generated.contains(SENTINEL_TYPE),
        "`manifest` must reach the overlay machinery, not apply_options, got:\n{generated}"
    );
}

/// An absolute `manifest` path is used as-is rather than being joined onto
/// the config directory, matching how `output` behaves.
#[test]
fn absolute_manifest_path_is_used_as_is() {
    let overlay_dir = TempDir::new().unwrap();
    let overlay_path = overlay_dir.path().join("custom.toml");
    std::fs::write(&overlay_path, int64_overlay(SENTINEL_TYPE)).unwrap();

    let config = config_with_manifest(&format!(r#"manifest = "{}""#, toml_path(&overlay_path)));
    let (dir, config_path) = project(&config);

    let output = generate_from(&config_path, dir.path());
    assert!(output.status.success(), "generate failed:\n{}", stderr(&output));

    let generated = std::fs::read_to_string(dir.path().join("out").join("queries.rs")).unwrap();
    assert!(
        generated.contains(SENTINEL_TYPE),
        "an absolute override path must be honoured verbatim, got:\n{generated}"
    );
}
