//! Regression test for issue #208: `scythe audit` used to abandon a whole
//! file at the first statement it could not parse, discarding every
//! finding from statements already parsed successfully (including real
//! security findings) and printing "No findings." with exit 0.
//!
//! Drives the compiled binary via `Command::new(env!("CARGO_BIN_EXE_scythe"))`.

use std::process::Command;

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

/// A real `GRANT ALL ... TO PUBLIC` finding (SC-SEC02 + SC-SEC03) precedes a
/// statement using syntax the parser cannot handle.
const MIXED_SQL: &str =
    "GRANT ALL ON users TO PUBLIC;\nCREATE POLICY p ON t USING (true) NOT VALID GARBAGE SYNTAX HERE;\n";

/// `scythe audit` must still report the GRANT ALL finding from the
/// statement before the unparseable one, and must exit non-zero (not the
/// silent "No findings." + exit 0 issue #208 reproduces).
#[test]
fn audit_reports_findings_from_statements_before_an_unparseable_one() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("bad.sql");
    std::fs::write(&sql_path, MIXED_SQL).unwrap();

    let output = scythe_bin()
        .args(["audit", sql_path.to_str().unwrap()])
        .output()
        .expect("run scythe audit");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.contains("SC-SEC02") || stdout.contains("SC-SEC03"),
        "the GRANT ALL finding from the parseable statement must survive; \
         stdout: {stdout}\nstderr: {stderr}"
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "a file with a real security finding must not exit 0, regardless of the unparseable statement; \
         stdout: {stdout}"
    );
    assert!(
        !stdout.trim().is_empty() && stdout != "No findings.\n",
        "audit must not silently report a clean run over a file with a real finding; stdout: {stdout}"
    );
}

/// The unparseable statement itself must be visible as a finding (not just
/// a stderr note a CI job checking the exit code would never see).
#[test]
fn audit_reports_the_unparseable_statement_as_its_own_finding() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("bad.sql");
    std::fs::write(&sql_path, MIXED_SQL).unwrap();

    let output = scythe_bin()
        .args(["audit", sql_path.to_str().unwrap()])
        .output()
        .expect("run scythe audit");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("SC-PARSE01"),
        "the unparseable statement must itself be an error finding, not only a stderr note: {stdout}"
    );
}

/// Negative control: a file with no parse failures at all must behave
/// exactly as before -- every statement's findings reported, exit non-zero
/// on a real finding.
#[test]
fn audit_reports_findings_normally_when_everything_parses() {
    let dir = TempDir::new().unwrap();
    let sql_path = dir.path().join("clean_grant.sql");
    std::fs::write(&sql_path, "GRANT ALL ON users TO PUBLIC;\n").unwrap();

    let output = scythe_bin()
        .args(["audit", sql_path.to_str().unwrap()])
        .output()
        .expect("run scythe audit");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("SC-SEC02") || stdout.contains("SC-SEC03"),
        "stdout: {stdout}"
    );
    assert!(
        !stdout.contains("SC-PARSE01"),
        "a fully-parseable file must not report a parse finding: {stdout}"
    );
}
