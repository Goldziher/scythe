//! Regression tests for issue #84: `schema`/`queries` glob patterns and
//! `[[sql.gen]]`/legacy `output` directories in `scythe.toml` must resolve
//! relative to the *config file's* directory, not the process's current
//! working directory. Prior to the fix, `resolve_globs` (and `fmt`'s
//! standalone glob loop) called `glob::glob(pattern)` directly, so
//! `scythe generate --config /path/to/project/scythe.toml` run from anywhere
//! else silently found nothing.
//!
//! Every test here drives the compiled binary via
//! `Command::new(env!("CARGO_BIN_EXE_scythe"))` with `.current_dir(...)` set
//! on the *child* process — never a process-wide CWD mutation — so tests stay
//! parallel-safe. Each test builds a self-contained temp project and (except
//! where the scenario specifically requires otherwise) runs it from a second,
//! unrelated temp directory to prove resolution does not depend on the
//! caller's CWD.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

/// Render a path as a string safe to embed in a TOML basic string (`"..."`)
/// and safe as a glob pattern on every platform. `Path::display()` uses the
/// native separator, which on Windows is `\` — an escape character in TOML
/// basic strings. `/` is a valid glob and TOML-string separator everywhere.
fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

const SCHEMA_SQL: &str = "CREATE TABLE widgets (id bigint PRIMARY KEY, name text NOT NULL);\n";

/// A single well-formed query, annotated so `generate`/`check` accept it.
const VALID_QUERY_SQL: &str = "-- @name GetWidget\n-- @returns :one\nSELECT id, name FROM widgets WHERE id = $1;\n";

/// An UPDATE with no WHERE clause — trips `SC-S01` (error severity by
/// default), giving `lint` an unambiguous, non-zero-exit signal.
const LINTY_QUERY_SQL: &str = "-- @name UpdateAllWidgets\n-- @returns :exec\nUPDATE widgets SET name = $1;\n";

/// `GRANT ALL` — trips `SC-SEC02` (error severity by default). Audit parses
/// raw SQL statements directly (no `-- @name` annotation required).
const AUDIT_QUERY_SQL: &str = "GRANT ALL ON widgets TO someuser;\n";

/// Deliberately inconsistent indentation (2 spaces then 4) so sqruff's LT02
/// indentation rule reports it needs formatting.
const UNFORMATTED_QUERY_SQL: &str =
    "-- @name GetWidget\n-- @returns :one\nSELECT\n  id,\n    name\nFROM widgets\nWHERE id = $1;\n";

/// Write `schema.sql` + `queries.sql` + `scythe.toml` into `dir`. The config
/// uses relative `schema`/`queries` patterns (`"schema.sql"`, `"queries.sql"`)
/// and the given `output` string verbatim, so callers control whether output
/// resolution is exercised as relative or absolute.
fn write_project(dir: &Path, queries_sql: &str, output: &str) -> PathBuf {
    std::fs::write(dir.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(dir.join("queries.sql"), queries_sql).unwrap();

    let config = format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]
output = "{output}"
"#
    );
    let config_path = dir.join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();
    config_path
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn generate_resolves_globs_relative_to_config_not_cwd() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();
    let config_path = write_project(project.path(), VALID_QUERY_SQL, "generated");

    let output = scythe_bin()
        .args(["generate", "--config", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe generate");

    assert!(
        output.status.success(),
        "generate must succeed once schema/queries resolve against the config dir; stderr: {}",
        stderr(&output)
    );

    let generated = project.path().join("generated").join("queries.rs");
    let content = std::fs::read_to_string(&generated)
        .unwrap_or_else(|e| panic!("expected generated file at {}: {e}", generated.display()));

    // Before the fix, globs resolved against `run_from` (empty), so both
    // schema and queries matched nothing and the output was the
    // "no queries" placeholder rather than real generated code.
    assert!(
        content.contains("get_widget"),
        "expected a real get_widget query fn in generated output, got:\n{content}"
    );
    assert!(
        !content.contains("No queries generated"),
        "generated file must not be the empty placeholder, got:\n{content}"
    );
}

#[test]
fn check_resolves_globs_relative_to_config_not_cwd() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();
    let config_path = write_project(project.path(), VALID_QUERY_SQL, "generated");

    let output = scythe_bin()
        .args(["check", "--config", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe check");

    let err = stderr(&output);
    assert!(
        output.status.success(),
        "check on a valid query must exit 0; stderr: {err}"
    );

    // Before the fix, `resolve_globs` found zero query files against the
    // wrong CWD and printed "Checking 0 queries...".
    assert!(
        err.contains("Checking 1 queries"),
        "expected the real query to be counted (not 0); stderr: {err}"
    );
    assert!(
        !err.contains("Checking 0 queries"),
        "check must not fall back to 0 queries; stderr: {err}"
    );
}

#[test]
fn lint_resolves_globs_relative_to_config_not_cwd() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();
    let config_path = write_project(project.path(), LINTY_QUERY_SQL, "generated");

    let output = scythe_bin()
        .args(["lint", "--config", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe lint");

    let err = stderr(&output);

    // Before the fix, the query file was never found, so lint reported no
    // violations and exited 0 regardless of the UPDATE-without-WHERE bug.
    assert!(
        !output.status.success(),
        "lint must fail once the real (rule-tripping) query is analyzed; stderr: {err}"
    );
    assert!(
        err.contains("SC-S01"),
        "expected SC-S01 (update-without-where) to fire; stderr: {err}"
    );
    assert!(
        !err.contains("No lint violations found."),
        "lint must not report a clean run over a rule-tripping query; stderr: {err}"
    );
}

#[test]
fn audit_resolves_globs_relative_to_config_not_cwd() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();
    let config_path = write_project(project.path(), AUDIT_QUERY_SQL, "generated");

    let output = scythe_bin()
        .args(["audit", "--config", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe audit");

    // Before the fix, the query file was never found, so audit found nothing
    // to report and exited 0 despite the GRANT ALL.
    assert_eq!(
        output.status.code(),
        Some(2),
        "audit must find the GRANT ALL once queries.sql resolves against the config dir; stderr: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("SC-SEC02"),
        "expected SC-SEC02 (grant-all) to fire; stdout: {out}"
    );
}

#[test]
fn fmt_check_resolves_globs_relative_to_config_not_cwd() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();
    let config_path = write_project(project.path(), UNFORMATTED_QUERY_SQL, "generated");

    let output = scythe_bin()
        .args(["fmt", "--config", config_path.to_str().unwrap(), "--check"])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe fmt --check");

    let err = stderr(&output);

    // Before the fix, `resolve_files_from_config`'s inline glob loop (a 5th,
    // separate CWD-relative code path) found zero files against the wrong
    // CWD and printed "No SQL files found to format." with exit 0 — masking
    // the badly formatted query entirely.
    //
    // Assert on the POSITIVE signal — that fmt named this specific file as
    // needing formatting. A bare `!status.success()` check is not enough:
    // an unrebased pattern now matches nothing, which `resolve_globs` turns
    // into a hard error, which is also a non-zero exit with a message that
    // happens not to contain "No SQL files found to format". Both of the
    // obvious negative assertions therefore pass while the bug is fully
    // present, so they are kept only as corroboration.
    assert!(
        !output.status.success(),
        "fmt --check must fail once the badly formatted query is found; stderr: {err}"
    );
    assert!(
        err.contains("needs formatting"),
        "fmt must actually read the query file and report it as needing formatting, \
         not merely exit non-zero for some other reason; stderr: {err}"
    );
    assert!(
        !err.contains("matched no files"),
        "fmt --check failed because the glob resolved to nothing, not because it \
         found badly formatted SQL — the pattern was not rebased; stderr: {err}"
    );
    assert!(
        !err.contains("No SQL files found to format"),
        "fmt must find real files, not report an empty file set; stderr: {err}"
    );
}

#[test]
fn output_is_relative_to_config_not_cwd() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();
    let config_path = write_project(project.path(), VALID_QUERY_SQL, "generated");

    let output = scythe_bin()
        .args(["generate", "--config", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe generate");

    assert!(
        output.status.success(),
        "generate must succeed; stderr: {}",
        stderr(&output)
    );

    assert!(
        project.path().join("generated").join("queries.rs").exists(),
        "output must land beside the config, at {}",
        project.path().join("generated").display()
    );
    assert!(
        !run_from.path().join("generated").exists(),
        "no output must be written under the CWD the command was invoked from"
    );
}

#[test]
fn absolute_output_is_unchanged() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();
    let abs_output = TempDir::new().unwrap();

    let config_path = write_project(project.path(), VALID_QUERY_SQL, &toml_path(abs_output.path()));

    // #207: an `output` outside the project root is rejected by default
    // (the CI-against-a-PR-modified-config threat that fix closes) unless
    // the caller opts in. This test's whole premise is a deliberately
    // absolute `output`, so it is exactly the "genuinely wanted" case the
    // flag exists for.
    let output = scythe_bin()
        .args([
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--allow-output-escape",
        ])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe generate");

    assert!(
        output.status.success(),
        "generate must succeed; stderr: {}",
        stderr(&output)
    );

    assert!(
        abs_output.path().join("queries.rs").exists(),
        "an absolute `output` must be used exactly as given, at {}",
        abs_output.path().display()
    );

    // `PathBuf::push`/`Path::join` already replace the buffer outright when
    // the pushed path is absolute, so an incorrect re-join would still
    // resolve to `abs_output` (the assertion above would still pass) rather
    // than surfacing as a broken path. What an incorrect implementation
    // could actually do is create *extra* entries under the config dir (e.g.
    // a literal `generated` directory as a leftover default). Assert the
    // config dir contains exactly the three input files this test wrote and
    // nothing else.
    let mut project_entries: Vec<String> = std::fs::read_dir(project.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    project_entries.sort();
    assert_eq!(
        project_entries,
        vec![
            "queries.sql".to_string(),
            "schema.sql".to_string(),
            "scythe.toml".to_string()
        ],
        "an absolute output must not leave any extra entries under the config dir"
    );
}

#[test]
fn absolute_glob_patterns_bypass_config_dir() {
    // `schema.sql`/`queries.sql` live under `sql_dir`; `scythe.toml` lives
    // under an unrelated `config_dir`, referencing them by absolute path.
    let sql_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();

    std::fs::write(sql_dir.path().join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(sql_dir.path().join("queries.sql"), VALID_QUERY_SQL).unwrap();

    let output_dir = config_dir.path().join("generated");
    let config = format!(
        r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["{schema}"]
queries = ["{queries}"]
output = "{output}"
"#,
        schema = toml_path(&sql_dir.path().join("schema.sql")),
        queries = toml_path(&sql_dir.path().join("queries.sql")),
        output = toml_path(&output_dir),
    );
    let config_path = config_dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    // `output` here is also an absolute path (see `output_dir` above), so
    // this needs the same #207 opt-out as `absolute_output_is_unchanged`.
    let output = scythe_bin()
        .args([
            "generate",
            "--config",
            config_path.to_str().unwrap(),
            "--allow-output-escape",
        ])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe generate");

    assert!(
        output.status.success(),
        "absolute schema/queries patterns must resolve regardless of the config dir; stderr: {}",
        stderr(&output)
    );
    let content = std::fs::read_to_string(output_dir.join("queries.rs")).expect("generated queries.rs");
    assert!(
        content.contains("get_widget"),
        "expected real generated content, got:\n{content}"
    );
}

#[test]
fn default_config_path_reports_unprefixed_paths() {
    let project = TempDir::new().unwrap();
    std::fs::write(project.path().join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(project.path().join("queries.sql"), AUDIT_QUERY_SQL).unwrap();
    let config = r#"[scythe]
version = "1"

[[sql]]
name = "test"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["queries.sql"]
output = "generated"
"#;
    std::fs::write(project.path().join("scythe.toml"), config).unwrap();

    // CWD == project dir, and `--config scythe.toml` is the bare default
    // spelling (`config_dir("scythe.toml")` is the empty path). This pins
    // the `rebase_pattern` identity branch: the reported path must be
    // `queries.sql`, not `./queries.sql`.
    let output = scythe_bin()
        .args(["audit", "--config", "scythe.toml", "--format", "json", "--exit-zero"])
        .current_dir(project.path())
        .output()
        .expect("run scythe audit");

    assert!(
        output.status.success(),
        "--exit-zero forces success; stderr: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("\"queries.sql\""),
        "expected the unprefixed path 'queries.sql' in JSON output, got:\n{out}"
    );
    assert!(
        !out.contains("\"./queries.sql\""),
        "path must not be './'-prefixed just because '.' is an equally valid spelling of the same dir, got:\n{out}"
    );
}

#[test]
fn glob_metacharacters_in_config_dir_are_escaped() {
    // `[` and `]` are legal in directory names on every platform this repo's
    // CI matrix targets (Windows forbids `<>:"/\|?*`, not brackets), so this
    // test needs no platform skip.
    let root = TempDir::new().unwrap();
    let project_dir = root.path().join("pro[ject]");
    std::fs::create_dir(&project_dir).unwrap();
    let run_from = TempDir::new().unwrap();

    let config_path = write_project(&project_dir, VALID_QUERY_SQL, "generated");

    let output = scythe_bin()
        .args(["generate", "--config", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe generate");

    assert!(
        output.status.success(),
        "a config dir containing glob metacharacters must not break resolution; stderr: {}",
        stderr(&output)
    );
    let generated = project_dir.join("generated").join("queries.rs");
    let content = std::fs::read_to_string(&generated)
        .unwrap_or_else(|e| panic!("expected generated file at {}: {e}", generated.display()));
    assert!(
        content.contains("get_widget"),
        "expected real generated content despite the bracketed config dir, got:\n{content}"
    );
}

#[test]
fn empty_glob_is_an_error_naming_pattern_and_config_dir() {
    let project = TempDir::new().unwrap();
    let run_from = TempDir::new().unwrap();

    // Only `schema.sql` exists; `queries` points at a pattern that matches
    // nothing.
    std::fs::write(project.path().join("schema.sql"), SCHEMA_SQL).unwrap();
    let config = r#"[scythe]
version = "1"

[[sql]]
name = "widgets"
engine = "postgresql"
schema = ["schema.sql"]
queries = ["nonexistent/*.sql"]
output = "generated"
"#;
    let config_path = project.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args(["generate", "--config", config_path.to_str().unwrap()])
        .current_dir(run_from.path())
        .output()
        .expect("run scythe generate");

    assert!(
        !output.status.success(),
        "an empty glob must be a hard error, not a silent no-op"
    );
    let err = stderr(&output);
    assert!(
        err.contains("[widgets] queries"),
        "error must name the offending [[sql]] block and pattern kind; stderr: {err}"
    );
    assert!(
        err.contains("nonexistent/*.sql"),
        "error must name the offending pattern; stderr: {err}"
    );
    assert!(
        err.contains(&toml_path(project.path())) || err.contains(project.path().to_string_lossy().as_ref()),
        "error must name the config directory patterns were resolved against; stderr: {err}"
    );
    assert!(
        err.contains("current working directory"),
        "error must teach the new config-relative-path convention; stderr: {err}"
    );
}
