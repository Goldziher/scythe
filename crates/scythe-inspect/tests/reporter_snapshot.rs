//! Reporter snapshot tests — lock down the human/JSON/SARIF output shape for a
//! canonical inspect finding.
//!
//! Uses hand-rolled inline assertions rather than an insta snapshot crate so no
//! new dev-dep is required.  When the Finding shape changes the assertions will
//! fail immediately, surfacing the break before a release.

use scythe_inspect::CheckRegistry;
use scythe_lint::reporters::Format;
use scythe_lint::{Finding, Severity, emit_findings};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a canonical, deterministic Finding for snapshot tests.
fn canonical_finding() -> Finding {
    Finding {
        file: String::new(),
        query_name: None,
        rule_id: "SC-INS04".to_string(),
        rule_name: Some("no-primary-key".to_string()),
        rule_description: Some("ordinary tables without a PRIMARY KEY".to_string()),
        severity: Severity::Warn,
        message: "public.accounts has no primary key".to_string(),
        line: None,
        column: None,
        cwe: vec![],
        source: Some("inspect".to_string()),
    }
}

fn emit_to_string(format: Format, findings: &[Finding]) -> String {
    let mut buf = Vec::new();
    emit_findings(format, "scythe-inspect", "0.10.0", findings, &mut buf).expect("emit_findings must not fail");
    String::from_utf8(buf).expect("output must be valid UTF-8")
}

// ---------------------------------------------------------------------------
// JSON snapshot
// ---------------------------------------------------------------------------

#[test]
fn json_snapshot_contains_required_fields() {
    let finding = canonical_finding();
    let output = emit_to_string(Format::Json, &[finding]);

    let parsed: serde_json::Value = serde_json::from_str(&output).expect("JSON output must parse as valid JSON");

    let arr = parsed.as_array().expect("JSON output is an array");
    assert_eq!(arr.len(), 1, "one finding should produce one JSON object");

    let obj = &arr[0];
    assert_eq!(obj["rule_id"], "SC-INS04", "rule_id must be SC-INS04; got: {obj}");
    assert_eq!(
        obj["severity"], "warning",
        "warn severity serialises as 'warning'; got: {obj}"
    );
    assert_eq!(
        obj["message"], "public.accounts has no primary key",
        "message must match; got: {obj}"
    );
    assert_eq!(obj["source"], "inspect", "source field must be 'inspect'; got: {obj}");
    // file is empty string — must be present in JSON
    assert_eq!(obj["file"], "", "empty file field must be present; got: {obj}");
}

#[test]
fn json_snapshot_empty_findings_emits_empty_array() {
    let output = emit_to_string(Format::Json, &[]);
    let parsed: serde_json::Value = serde_json::from_str(&output).expect("empty JSON output must parse");
    assert!(
        parsed.as_array().expect("is array").is_empty(),
        "no findings → empty JSON array"
    );
}

// ---------------------------------------------------------------------------
// SARIF snapshot
// ---------------------------------------------------------------------------

#[test]
fn sarif_snapshot_has_version_and_runs() {
    let finding = canonical_finding();
    let output = emit_to_string(Format::Sarif, &[finding]);

    let parsed: serde_json::Value = serde_json::from_str(&output).expect("SARIF output must parse as valid JSON");

    assert_eq!(
        parsed["version"], "2.1.0",
        "SARIF version must be '2.1.0'; got: {parsed}"
    );
    let runs = parsed["runs"].as_array().expect("SARIF must have 'runs' array");
    assert_eq!(runs.len(), 1, "one run per invocation");
    let results = runs[0]["results"].as_array().expect("runs[0].results must be array");
    assert_eq!(results.len(), 1, "one result for one finding");
    assert_eq!(
        results[0]["ruleId"], "SC-INS04",
        "SARIF ruleId must match; got: {}",
        results[0]
    );
    assert_eq!(
        results[0]["level"], "warning",
        "warn severity maps to SARIF 'warning' level; got: {}",
        results[0]
    );
    // message.text must be present
    assert_eq!(
        results[0]["message"]["text"], "public.accounts has no primary key",
        "SARIF message.text must match; got: {}",
        results[0]["message"]
    );
}

#[test]
fn sarif_snapshot_tool_driver_name_matches_inspect() {
    let finding = canonical_finding();
    let output = emit_to_string(Format::Sarif, &[finding]);
    let parsed: serde_json::Value = serde_json::from_str(&output).expect("SARIF must parse");
    assert_eq!(
        parsed["runs"][0]["tool"]["driver"]["name"], "scythe-inspect",
        "tool driver name must be scythe-inspect"
    );
    assert_eq!(
        parsed["runs"][0]["tool"]["driver"]["version"], "0.10.0",
        "tool driver version must match"
    );
}

#[test]
fn sarif_snapshot_empty_findings_has_empty_results() {
    let output = emit_to_string(Format::Sarif, &[]);
    let parsed: serde_json::Value = serde_json::from_str(&output).expect("SARIF must parse");
    let results = parsed["runs"][0]["results"].as_array().expect("results array");
    assert!(results.is_empty(), "no findings → empty SARIF results");
}

// ---------------------------------------------------------------------------
// Human snapshot
// ---------------------------------------------------------------------------

#[test]
fn human_snapshot_contains_source_tag_severity_rule_and_message() {
    let finding = canonical_finding();
    let output = emit_to_string(Format::Human, &[finding]);

    assert!(
        output.contains("[inspect]"),
        "human output must contain '[inspect]' source tag; got:\n{output}"
    );
    assert!(
        output.contains("warning"),
        "human output must contain 'warning' severity label; got:\n{output}"
    );
    assert!(
        output.contains("[SC-INS04]"),
        "human output must contain '[SC-INS04]' rule id; got:\n{output}"
    );
    assert!(
        output.contains("public.accounts has no primary key"),
        "human output must contain the message; got:\n{output}"
    );
}

#[test]
fn human_snapshot_empty_findings_says_no_findings() {
    let output = emit_to_string(Format::Human, &[]);
    assert_eq!(
        output.trim(),
        "No findings.",
        "empty findings must produce 'No findings.' (got: {output:?})"
    );
}

// ---------------------------------------------------------------------------
// Registry round-trip — check that SC-INS04 is in the canonical registry with
// the expected shape so snapshot tests use data that actually reflects what
// scythe-inspect ships.
// ---------------------------------------------------------------------------

#[test]
fn canonical_registry_sc_ins04_has_expected_shape() {
    let registry = CheckRegistry::canonical();
    let spec = registry
        .get("SC-INS04")
        .expect("SC-INS04 must be in the canonical registry");

    assert_eq!(spec.id, "SC-INS04");
    assert_eq!(spec.name, "no-primary-key");
    assert!(
        spec.engines.iter().any(|e| e == "postgres"),
        "SC-INS04 must apply to postgres"
    );
    assert!(spec.explanation.is_some(), "SC-INS04 must have an explanation field");
    assert!(spec.remediation.is_some(), "SC-INS04 must have a remediation field");
}
