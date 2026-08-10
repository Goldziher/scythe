//! Regression tests for issue #206: passing a file path to `audit`/`fmt`
//! instead of relying on `scythe.toml` used to silently change what ran --
//! `audit <file>` ignored `[lint]`/`[audit]` entirely (a rule turned "off"
//! still fired, and user-defined rules never ran), `fmt` never read or
//! validated `[lint.sqruff]`, and a malformed `--config` was silently
//! swallowed rather than reported, degrading the dialect to `ansi`.
//!
//! Every test drives the compiled binary via
//! `Command::new(env!("CARGO_BIN_EXE_scythe"))`.

use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// `GRANT ALL ON t TO PUBLIC` trips both SC-SEC02 (enumerate exact
/// privileges) and SC-SEC03 (don't grant to PUBLIC).
const GRANT_ALL_SQL: &str = "GRANT ALL ON users TO PUBLIC;\n";

/// #206, item 2: `scythe audit <file>` must honour `[lint]` rule severity
/// overrides from `scythe.toml` -- a rule turned `"off"` must not fire in
/// explicit-file mode just because it still fires in config mode.
#[test]
fn audit_explicit_file_honours_a_rule_turned_off_in_config() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("grant.sql");
    std::fs::write(&sql_path, GRANT_ALL_SQL).unwrap();

    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\nschema = []\nqueries = []\n\n\
        [lint.rules]\nSC-SEC02 = \"off\"\n";
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args([
            "audit",
            "--config",
            config_path.to_str().unwrap(),
            sql_path.to_str().unwrap(),
        ])
        .output()
        .expect("run scythe audit");

    let out = stdout(&output);
    assert!(
        !out.contains("SC-SEC02"),
        "SC-SEC02 is configured off; explicit-file mode must not fire it: {out}"
    );
    assert!(
        out.contains("SC-SEC03"),
        "SC-SEC03 is not disabled and must still fire: {out}"
    );
}

/// #206, item 2: `scythe audit <file>` must also register `[[audit.rule]]`
/// user-defined rules -- not just apply severity overrides to canonical ones.
#[test]
fn audit_explicit_file_registers_user_defined_rules_from_config() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("evil.sql");
    std::fs::write(&sql_path, "SELECT evil_function();\n").unwrap();

    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\nschema = []\nqueries = []\n\n\
        [[audit.rule]]\n\
        id = \"USER-001\"\n\
        name = \"no-evil-function\"\n\
        category = \"security\"\n\
        severity = \"error\"\n\
        description = \"evil_function should not be called\"\n\
        message = \"USER RULE FIRED: call to {func}\"\n\
        matcher = \"function_name_in_set\"\n\n\
        [audit.rule.matcher_args]\n\
        functions = [\"evil_function\"]\n";
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args([
            "audit",
            "--config",
            config_path.to_str().unwrap(),
            sql_path.to_str().unwrap(),
        ])
        .output()
        .expect("run scythe audit");

    let out = stdout(&output);
    let err = stderr(&output);
    assert!(
        out.contains("USER-001") || err.contains("USER-001"),
        "explicit-file mode must run the user-defined [[audit.rule]]; stdout: {out}\nstderr: {err}"
    );
}

/// #206, item 3: `scythe fmt` must validate `[lint.sqruff]` -- a
/// configuration `scythe lint` rejects must not be silently accepted (and
/// ignored) by `fmt`.
#[test]
fn fmt_rejects_the_same_invalid_lint_sqruff_config_lint_does() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("t.sql");
    std::fs::write(&sql_path, "SELECT 1;\n").unwrap();

    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\nschema = []\nqueries = [\"t.sql\"]\n\n\
        [lint.sqruff.rules]\nLT02 = \"warn\"\n";
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let lint_output = scythe_bin()
        .args(["lint", "--config", config_path.to_str().unwrap()])
        .current_dir(dir.path())
        .output()
        .expect("run scythe lint");
    assert!(
        !lint_output.status.success(),
        "sanity check: lint must reject this config; stderr: {}",
        stderr(&lint_output)
    );

    let fmt_output = scythe_bin()
        .args(["fmt", "--config", config_path.to_str().unwrap(), "--check"])
        .current_dir(dir.path())
        .output()
        .expect("run scythe fmt");

    assert!(
        !fmt_output.status.success(),
        "fmt must reject the same invalid [lint.sqruff] config lint rejects, instead of silently \
         ignoring it; stderr: {}",
        stderr(&fmt_output)
    );
}

/// #130: `fmt` builds its sqruff linter **once for the whole run**, before it
/// reads any file, and that is directly observable.
///
/// `fmt` formats whether or not sqruff-based *linting* is switched off, so
/// `enabled = false` does not excuse it from an unusable
/// `[lint.sqruff.rules]` -- it is a table `fmt` will act on. The question is
/// where that failure is reported. Constructing the linter inside the
/// per-file loop, as this used to, necessarily blames whichever file happened
/// to be read first for a mistake that is in the config and in no file at
/// all. Hoisting construction out of the loop is what makes the error name
/// the configuration instead.
///
/// Two query files, so "reported once, against no file" is distinguishable
/// from "reported against the first file".
#[test]
fn fmt_blames_an_unusable_rules_table_on_the_config_not_on_the_first_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("first.sql"), "SELECT 1;\n").unwrap();
    std::fs::write(dir.path().join("second.sql"), "SELECT 2;\n").unwrap();

    let config = "[scythe]\nversion = \"1\"\n\n\
        [[sql]]\nname = \"main\"\nengine = \"postgresql\"\nschema = []\n\
        queries = [\"first.sql\", \"second.sql\"]\n\n\
        [lint.sqruff]\nenabled = false\n\n\
        [lint.sqruff.rules]\nLT0 = \"off\"\n";
    let config_path = dir.path().join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();

    let output = scythe_bin()
        .args(["fmt", "--config", config_path.to_str().unwrap(), "--check"])
        .current_dir(dir.path())
        .output()
        .expect("run scythe fmt");
    let err = stderr(&output);

    assert!(
        !output.status.success(),
        "an unknown rule code in a table fmt will act on must fail the run; stderr: {err}"
    );
    assert!(
        err.contains("invalid [lint.sqruff] configuration"),
        "must report a configuration error; stderr: {err}"
    );
    assert!(err.contains("LT0"), "must name the offending rule code; stderr: {err}");
    assert!(
        !err.contains("first.sql") && !err.contains("second.sql"),
        "must not blame a query file for a mistake that is in the config -- the linter is built \
         once, before any file is read; stderr: {err}"
    );
}

/// #206, item 4: a malformed `--config` must be reported, not silently
/// swallowed into "no dialect configured, default to ansi" -- in explicit
/// file mode with no query files this manifests as `lint`/`fmt` reporting a
/// config error instead of quietly proceeding.
#[test]
fn lint_reports_a_malformed_explicit_config_instead_of_silently_ignoring_it() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("t.sql");
    std::fs::write(&sql_path, "SELECT 1;\n").unwrap();

    let broken_config = dir.path().join("broken.toml");
    std::fs::write(&broken_config, "this is not [valid toml\n").unwrap();

    let output = scythe_bin()
        .args([
            "lint",
            "--config",
            broken_config.to_str().unwrap(),
            sql_path.to_str().unwrap(),
        ])
        .output()
        .expect("run scythe lint");

    let err = stderr(&output);
    assert!(
        !output.status.success(),
        "a malformed --config must be reported as an error, not silently ignored; stderr: {err}"
    );
    assert!(
        err.contains("broken.toml") || err.contains("parse"),
        "the error should mention the config file or a parse failure; stderr: {err}"
    );
}

/// Same malformed-config check for `fmt`.
#[test]
fn fmt_reports_a_malformed_explicit_config_instead_of_silently_ignoring_it() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("t.sql");
    std::fs::write(&sql_path, "SELECT 1;\n").unwrap();

    let broken_config = dir.path().join("broken.toml");
    std::fs::write(&broken_config, "this is not [valid toml\n").unwrap();

    let output = scythe_bin()
        .args([
            "fmt",
            "--config",
            broken_config.to_str().unwrap(),
            "--check",
            sql_path.to_str().unwrap(),
        ])
        .output()
        .expect("run scythe fmt");

    let err = stderr(&output);
    assert!(
        !output.status.success(),
        "a malformed --config must be reported as an error, not silently ignored; stderr: {err}"
    );
}
