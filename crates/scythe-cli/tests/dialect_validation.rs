//! Regression tests for issue #205: `--dialect postgresql` (scythe's own
//! canonical engine name, the value every `scythe.toml` in the repo puts in
//! `engine = "..."`) used to panic (exit 101) on `fmt`/`lint` because it was
//! passed straight to sqruff-lib without translation, and any unrecognized
//! `--dialect` value (a typo, or gibberish like `klingon`) panicked the same
//! way instead of producing a diagnostic.
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

fn write_sql_file(dir: &std::path::Path) -> String {
    let path = dir.join("t.sql");
    std::fs::write(&path, "SELECT 1;\n").unwrap();
    path.to_string_lossy().into_owned()
}

/// `scythe fmt --dialect postgresql` must not panic (exit 101) -- it must
/// either succeed or fail with an ordinary diagnostic exit code.
#[test]
fn fmt_accepts_canonical_postgresql_dialect_alias() {
    let dir = TempDir::new().unwrap();
    let sql_path = write_sql_file(dir.path());

    let output = scythe_bin()
        .args(["fmt", "--dialect", "postgresql", "--check", &sql_path])
        .output()
        .expect("run scythe fmt");

    assert_ne!(
        output.status.code(),
        Some(101),
        "fmt --dialect postgresql must not panic; stderr: {}",
        stderr(&output)
    );
}

/// `scythe lint --dialect postgresql` must not panic either.
#[test]
fn lint_accepts_canonical_postgresql_dialect_alias() {
    let dir = TempDir::new().unwrap();
    let sql_path = write_sql_file(dir.path());

    let output = scythe_bin()
        .args(["lint", "--dialect", "postgresql", &sql_path])
        .output()
        .expect("run scythe lint");

    assert_ne!(
        output.status.code(),
        Some(101),
        "lint --dialect postgresql must not panic; stderr: {}",
        stderr(&output)
    );
}

/// An unrecognized `--dialect` (gibberish, not a typo of a real one) must
/// be rejected with a clear diagnostic naming the accepted values -- not a
/// panic, and not a silent fallback to `ansi`.
#[test]
fn fmt_rejects_unknown_dialect_with_a_clear_error() {
    let dir = TempDir::new().unwrap();
    let sql_path = write_sql_file(dir.path());

    let output = scythe_bin()
        .args(["fmt", "--dialect", "klingon", "--check", &sql_path])
        .output()
        .expect("run scythe fmt");

    let err = stderr(&output);
    assert_ne!(
        output.status.code(),
        Some(101),
        "fmt --dialect klingon must not panic; stderr: {err}"
    );
    assert!(
        !output.status.success(),
        "an unknown dialect must not be silently accepted; stderr: {err}"
    );
    assert!(
        err.contains("klingon"),
        "the error must name the offending value; stderr: {err}"
    );
}

#[test]
fn lint_rejects_unknown_dialect_with_a_clear_error() {
    let dir = TempDir::new().unwrap();
    let sql_path = write_sql_file(dir.path());

    let output = scythe_bin()
        .args(["lint", "--dialect", "klingon", &sql_path])
        .output()
        .expect("run scythe lint");

    let err = stderr(&output);
    assert_ne!(
        output.status.code(),
        Some(101),
        "lint --dialect klingon must not panic; stderr: {err}"
    );
    assert!(
        !output.status.success(),
        "an unknown dialect must not be silently accepted; stderr: {err}"
    );
    assert!(
        err.contains("klingon"),
        "the error must name the offending value; stderr: {err}"
    );
}

/// `--dialect postgres` (sqruff's own spelling) must still work -- the fix
/// must not narrow accepted values, only add validation for the rest.
#[test]
fn fmt_still_accepts_sqruff_native_dialect_name() {
    let dir = TempDir::new().unwrap();
    let sql_path = write_sql_file(dir.path());

    let output = scythe_bin()
        .args(["fmt", "--dialect", "postgres", "--check", &sql_path])
        .output()
        .expect("run scythe fmt");

    assert!(
        output.status.success(),
        "fmt --dialect postgres must still succeed on already-formatted SQL; stderr: {}",
        stderr(&output)
    );
}
