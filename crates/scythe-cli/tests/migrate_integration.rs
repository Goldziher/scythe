//! End-to-end regression tests for GitHub issue #88.
//!
//! `convert_query_files` in `commands/migrate.rs` built the glob pattern used
//! to find `.sql` query files by joining `base_dir` onto the user's
//! `queries` pattern with `Path::join` + `Path::display()`, then deciding
//! "is this already a glob, or a bare directory" by inspecting the *joined*
//! string. Neither step escaped glob metacharacters in `base_dir`. So a
//! project directory named e.g. `a[b]` had its `[b]` compiled as a glob
//! character class, `queries` files were never found, and `scythe migrate`
//! printed a cheerful `Migration complete: 0 file(s) converted` instead of
//! failing or converting anything — the exact silent-zero failure mode
//! issue #84 fixed for `generate`/`check`/`lint`/`audit`/`fmt` in 0.13.0.
//!
//! Every test here drives the compiled binary via
//! `Command::new(env!("CARGO_BIN_EXE_scythe"))` with `.current_dir(...)` set
//! on the *child* process — never a process-wide CWD mutation — so tests
//! stay parallel-safe, following the idiom in `config_relative_paths.rs`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const SCHEMA_SQL: &str = "CREATE TABLE widgets (id bigint PRIMARY KEY, name text NOT NULL);\n";

/// A single sqlc-flavored query using `-- name: X :one` and `sqlc.arg(...)`
/// — the syntax `migrate` is meant to translate into scythe's `-- @name` /
/// `-- @returns` / `-- @param` annotations and positional `$n` params.
const SQLC_QUERY_SQL: &str = "-- name: GetWidget :one\nSELECT id, name FROM widgets WHERE id = sqlc.arg(widget_id);\n";

fn v2_yaml_config(queries: &str) -> String {
    format!(
        "version: \"2\"\n\
         sql:\n\
         \x20 - engine: postgresql\n\
         \x20\x20\x20 schema: schema.sql\n\
         \x20\x20\x20 queries: {queries}\n\
         \x20\x20\x20 codegen:\n\
         \x20\x20\x20\x20\x20 - plugin: rust\n\
         \x20\x20\x20\x20\x20\x20\x20 out: generated\n"
    )
}

fn v2_json_config(queries: &str) -> String {
    format!(
        r#"{{
  "version": "2",
  "sql": [
    {{
      "engine": "postgresql",
      "schema": "schema.sql",
      "queries": "{queries}",
      "codegen": [
        {{ "plugin": "rust", "out": "generated" }}
      ]
    }}
  ]
}}
"#
    )
}

fn v1_yaml_config(queries: &str) -> String {
    format!(
        "version: \"1\"\n\
         packages:\n\
         \x20 - name: main\n\
         \x20\x20\x20 path: generated\n\
         \x20\x20\x20 queries: {queries}\n\
         \x20\x20\x20 schema: schema.sql\n\
         \x20\x20\x20 engine: postgresql\n"
    )
}

fn v1_json_config(queries: &str) -> String {
    format!(
        r#"{{
  "version": "1",
  "packages": [
    {{
      "name": "main",
      "path": "generated",
      "queries": "{queries}",
      "schema": "schema.sql",
      "engine": "postgresql"
    }}
  ]
}}
"#
    )
}

/// Write `schema.sql` and `queries/q.sql` under `project_dir`, plus the sqlc
/// config named `config_name` (its extension selects the json/yaml loader
/// branch in `run_migrate`) built from `config_body(queries_field)`.
fn write_project(
    project_dir: &Path,
    config_name: &str,
    queries_field: &str,
    config_body: impl FnOnce(&str) -> String,
) -> PathBuf {
    std::fs::create_dir_all(project_dir).unwrap();
    std::fs::write(project_dir.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::create_dir_all(project_dir.join("queries")).unwrap();
    std::fs::write(project_dir.join("queries").join("q.sql"), SQLC_QUERY_SQL).unwrap();

    let config_path = project_dir.join(config_name);
    std::fs::write(&config_path, config_body(queries_field)).unwrap();
    config_path
}

/// Assert `query_file` was rewritten from sqlc syntax to scythe syntax and
/// that a `.sql.bak` backup with the original content was left behind.
fn assert_query_converted(query_file: &Path) {
    let content = std::fs::read_to_string(query_file)
        .unwrap_or_else(|e| panic!("expected converted query at {}: {e}", query_file.display()));
    assert!(
        content.contains("-- @name GetWidget"),
        "expected sqlc annotation translated to @name, got:\n{content}"
    );
    assert!(
        content.contains("-- @returns :one"),
        "expected @returns annotation, got:\n{content}"
    );
    assert!(
        content.contains("-- @param widget_id"),
        "expected sqlc.arg(widget_id) translated to a @param line, got:\n{content}"
    );
    assert!(
        content.contains("id = $1"),
        "expected sqlc.arg(widget_id) translated to positional $1, got:\n{content}"
    );

    let bak = query_file.with_extension("sql.bak");
    let bak_content =
        std::fs::read_to_string(&bak).unwrap_or_else(|e| panic!("expected backup file at {}: {e}", bak.display()));
    assert_eq!(
        bak_content, SQLC_QUERY_SQL,
        "backup must preserve the original sqlc-flavored content"
    );
}

/// THE REGRESSION TEST for issue #88: a project directory literally named
/// `a[b]`. `[b]` is legal in directory names on every platform this repo's
/// CI matrix targets (Windows forbids `<>:"/\|?*`, not brackets), so this
/// test needs no platform skip — matches the sibling regression test for
/// `generate` in `config_relative_paths.rs`.
#[test]
fn migrate_converts_project_in_bracket_named_directory() {
    let root = TempDir::new().unwrap();
    let project_dir = root.path().join("a[b]");
    let run_from = TempDir::new().unwrap();

    let config_path = write_project(&project_dir, "sqlc.yaml", "queries", v2_yaml_config);

    let output = scythe_bin()
        .args(["migrate", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "migrate must succeed for a base dir containing glob metacharacters; stderr: {}",
        stderr(&output)
    );

    let out = stdout(&output);
    assert!(
        !out.contains("0 file(s) converted"),
        "before the fix, the bracketed dir name was compiled as a glob character class and \
         matched nothing; stdout: {out}"
    );
    assert!(
        out.contains("1 file(s) converted"),
        "expected exactly the one query file to be converted; stdout: {out}"
    );

    assert_query_converted(&project_dir.join("queries").join("q.sql"));
}

/// `?` in the directory name — but unlike `[...]`, a bare `?` or `*` happens
/// to *self-match* the literal character sitting at its position (a glob
/// `?` matches "any one character", including a literal `?`), so a naive
/// single-directory version of this test passes even against the unfixed
/// code and proves nothing. The real, observable bug for `?`/`*` is
/// over-matching, not under-matching: an unescaped `a?b` pattern also
/// matches any *sibling* directory that differs from `a?b` by exactly one
/// character in that position (e.g. `axb`). This test creates that sibling
/// with its own untouched-marker query file and asserts it is left
/// completely alone — only the intended `a?b/queries/q.sql` is converted.
/// `?` is forbidden in Windows filenames, so this is Unix-only.
#[cfg(not(windows))]
#[test]
fn migrate_converts_project_in_question_mark_named_directory_without_touching_sibling() {
    let root = TempDir::new().unwrap();
    let project_dir = root.path().join("a?b");
    let sibling_dir = root.path().join("axb");
    let run_from = TempDir::new().unwrap();

    let config_path = write_project(&project_dir, "sqlc.yaml", "queries", v2_yaml_config);

    // A sibling an unescaped `?` wildcard would also match.
    std::fs::create_dir_all(sibling_dir.join("queries")).unwrap();
    let sibling_query = sibling_dir.join("queries").join("q.sql");
    std::fs::write(&sibling_query, SQLC_QUERY_SQL).unwrap();

    let output = scythe_bin()
        .args(["migrate", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "migrate must succeed for a base dir containing '?'; stderr: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("1 file(s) converted"),
        "expected exactly the intended file converted, not the sibling too; stdout: {out}"
    );
    assert_query_converted(&project_dir.join("queries").join("q.sql"));

    let sibling_content = std::fs::read_to_string(&sibling_query).unwrap();
    assert_eq!(
        sibling_content, SQLC_QUERY_SQL,
        "an unescaped '?' in the base dir must not let migrate reach into an unrelated sibling directory"
    );
    assert!(
        !sibling_query.with_extension("sql.bak").exists(),
        "sibling file must not have been converted (no backup expected)"
    );
}

/// Same shape as the `?` test above, for `*`: an unescaped `a*b` pattern
/// also matches a sibling directory like `axyzb` (`*` matches any sequence,
/// including the literal substring that replaces it), so the observable bug
/// is over-matching into that sibling, not a `0 file(s) converted` result.
/// `*` is forbidden in Windows filenames, so this is Unix-only.
#[cfg(not(windows))]
#[test]
fn migrate_converts_project_in_star_named_directory_without_touching_sibling() {
    let root = TempDir::new().unwrap();
    let project_dir = root.path().join("a*b");
    let sibling_dir = root.path().join("axyzb");
    let run_from = TempDir::new().unwrap();

    let config_path = write_project(&project_dir, "sqlc.yaml", "queries", v2_yaml_config);

    // A sibling an unescaped `*` wildcard would also match.
    std::fs::create_dir_all(sibling_dir.join("queries")).unwrap();
    let sibling_query = sibling_dir.join("queries").join("q.sql");
    std::fs::write(&sibling_query, SQLC_QUERY_SQL).unwrap();

    let output = scythe_bin()
        .args(["migrate", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "migrate must succeed for a base dir containing '*'; stderr: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("1 file(s) converted"),
        "expected exactly the intended file converted, not the sibling too; stdout: {out}"
    );
    assert_query_converted(&project_dir.join("queries").join("q.sql"));

    let sibling_content = std::fs::read_to_string(&sibling_query).unwrap();
    assert_eq!(
        sibling_content, SQLC_QUERY_SQL,
        "an unescaped '*' in the base dir must not let migrate reach into an unrelated sibling directory"
    );
    assert!(
        !sibling_query.with_extension("sql.bak").exists(),
        "sibling file must not have been converted (no backup expected)"
    );
}

/// Ordinary happy path, no metacharacters anywhere: `scythe.toml` must be
/// written next to the sqlc config, and the query file must be converted.
/// v2 `sql[].queries`, yaml loader, bare-directory `queries` value
/// (exercises the `/*.sql` append).
#[test]
fn migrate_happy_path_v2_yaml_writes_config_and_converts_queries() {
    let root = TempDir::new().unwrap();
    let project_dir = root.path().join("project");
    let run_from = TempDir::new().unwrap();

    let config_path = write_project(&project_dir, "sqlc.yaml", "queries", v2_yaml_config);

    let output = scythe_bin()
        .args(["migrate", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "migrate must succeed; stderr: {}",
        stderr(&output)
    );

    let scythe_toml = project_dir.join("scythe.toml");
    assert!(
        scythe_toml.exists(),
        "expected scythe.toml at {}",
        scythe_toml.display()
    );
    let toml_content = std::fs::read_to_string(&scythe_toml).unwrap();
    assert!(
        toml_content.contains("queries/*.sql"),
        "expected the bare 'queries' dir normalized to a glob in scythe.toml, got:\n{toml_content}"
    );

    assert_query_converted(&project_dir.join("queries").join("q.sql"));
}

/// v1 `packages[].queries`, json loader, explicit glob `queries` value
/// (bypasses the `/*.sql` append branch entirely).
#[test]
fn migrate_v1_packages_json_explicit_glob_query_path() {
    let root = TempDir::new().unwrap();
    let project_dir = root.path().join("project");
    let run_from = TempDir::new().unwrap();

    let config_path = write_project(&project_dir, "sqlc.json", "queries/*.sql", v1_json_config);

    let output = scythe_bin()
        .args(["migrate", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "migrate must succeed; stderr: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("1 file(s) converted"),
        "expected the query file to be converted; stdout: {out}"
    );
    assert_query_converted(&project_dir.join("queries").join("q.sql"));
}

/// v1 `packages[].queries`, yaml loader, bare directory `queries` value.
#[test]
fn migrate_v1_packages_yaml_bare_directory_query_path() {
    let root = TempDir::new().unwrap();
    let project_dir = root.path().join("project");
    let run_from = TempDir::new().unwrap();

    let config_path = write_project(&project_dir, "sqlc.yaml", "queries", v1_yaml_config);

    let output = scythe_bin()
        .args(["migrate", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "migrate must succeed; stderr: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("1 file(s) converted"),
        "expected the query file to be converted; stdout: {out}"
    );
    assert_query_converted(&project_dir.join("queries").join("q.sql"));
}

/// v2 `sql[].queries`, json loader.
#[test]
fn migrate_v2_sql_json_query_path() {
    let root = TempDir::new().unwrap();
    let project_dir = root.path().join("project");
    let run_from = TempDir::new().unwrap();

    let config_path = write_project(&project_dir, "sqlc.json", "queries", v2_json_config);

    let output = scythe_bin()
        .args(["migrate", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "migrate must succeed; stderr: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("1 file(s) converted"),
        "expected the query file to be converted; stdout: {out}"
    );
    assert_query_converted(&project_dir.join("queries").join("q.sql"));
}

/// A `queries` pattern that matches no files must be reported (naming the
/// pattern and the base dir it was resolved against) rather than silently
/// producing a "0 file(s) converted" success message with no explanation.
/// `migrate` treats this as a *warning*, not a hard error: it still exits 0,
/// since one stale `queries` entry should not abort an otherwise-successful
/// migration of the rest of the project.
#[test]
fn migrate_warns_but_does_not_fail_on_zero_match_query_path() {
    let root = TempDir::new().unwrap();
    let project_dir = root.path().join("project");
    let run_from = TempDir::new().unwrap();

    // No `queries/` directory is created at all — `nonexistent` matches
    // nothing.
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(project_dir.join("schema.sql"), SCHEMA_SQL).unwrap();
    let config_path = project_dir.join("sqlc.yaml");
    std::fs::write(&config_path, v2_yaml_config("nonexistent")).unwrap();

    let output = scythe_bin()
        .args(["migrate", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe migrate");

    assert!(
        output.status.success(),
        "a zero-match queries entry must be a warning, not a hard failure; stderr: {}",
        stderr(&output)
    );

    let out = stdout(&output);
    assert!(
        out.contains("0 file(s) converted"),
        "expected zero files converted, got stdout:\n{out}"
    );

    let err = stderr(&output);
    assert!(
        err.contains("warning:"),
        "expected a warning naming the unmatched pattern; stderr: {err}"
    );
    assert!(
        err.contains("nonexistent"),
        "warning must name the offending pattern; stderr: {err}"
    );
    assert!(
        err.contains(&project_dir.display().to_string()),
        "warning must name the base directory patterns were resolved against; stderr: {err}"
    );
}
