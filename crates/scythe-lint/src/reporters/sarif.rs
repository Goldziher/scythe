//! SARIF 2.1.0 reporter.
//!
//! Produces a minimal, GitHub code-scanning-compatible SARIF log:
//! - one `run` per invocation
//! - `tool.driver` carries name + version
//! - per-finding: `ruleId`, `level`, `message.text`, optional `physicalLocation`
//! - per-finding: CWE tags in `properties.cwe` (array of `CWE-NNN` strings).
//!
//! Reference: <https://docs.oasis-open.org/sarif/sarif/v2.1.0/cs01/sarif-v2.1.0-cs01.html>

use std::collections::BTreeMap;
use std::io::{self, Write};

use serde::Serialize;

use crate::types::Severity;

use super::Finding;

const SARIF_VERSION: &str = "2.1.0";
const SARIF_SCHEMA: &str =
    "https://docs.oasis-open.org/sarif/sarif/v2.1.0/cos02/schemas/sarif-schema-2.1.0.json";

#[derive(Serialize)]
struct SarifLog<'a> {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: Vec<SarifRun<'a>>,
}

#[derive(Serialize)]
struct SarifRun<'a> {
    tool: SarifTool<'a>,
    results: Vec<SarifResult<'a>>,
}

#[derive(Serialize)]
struct SarifTool<'a> {
    driver: SarifDriver<'a>,
}

#[derive(Serialize)]
struct SarifDriver<'a> {
    name: &'a str,
    version: &'a str,
    #[serde(rename = "informationUri", skip_serializing_if = "Option::is_none")]
    information_uri: Option<&'a str>,
    rules: Vec<SarifRule<'a>>,
}

#[derive(Serialize)]
struct SarifRule<'a> {
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(rename = "shortDescription", skip_serializing_if = "Option::is_none")]
    short_description: Option<SarifText<'a>>,
}

#[derive(Serialize)]
struct SarifResult<'a> {
    #[serde(rename = "ruleId")]
    rule_id: &'a str,
    level: &'static str,
    message: SarifText<'a>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    locations: Vec<SarifLocation<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    properties: Option<SarifProperties<'a>>,
}

#[derive(Serialize)]
struct SarifText<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct SarifLocation<'a> {
    #[serde(rename = "physicalLocation")]
    physical_location: SarifPhysicalLocation<'a>,
}

#[derive(Serialize)]
struct SarifPhysicalLocation<'a> {
    #[serde(rename = "artifactLocation")]
    artifact_location: SarifArtifactLocation<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<SarifRegion>,
}

#[derive(Serialize)]
struct SarifArtifactLocation<'a> {
    uri: &'a str,
}

#[derive(Serialize)]
struct SarifRegion {
    #[serde(rename = "startLine", skip_serializing_if = "Option::is_none")]
    start_line: Option<usize>,
    #[serde(rename = "startColumn", skip_serializing_if = "Option::is_none")]
    start_column: Option<usize>,
}

#[derive(Serialize)]
struct SarifProperties<'a> {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cwe: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
}

pub fn emit(
    tool_name: &str,
    tool_version: &str,
    findings: &[Finding],
    out: &mut dyn Write,
) -> io::Result<()> {
    // Build the rule descriptor set deterministically (sorted by rule id).
    let mut rules_seen: BTreeMap<&str, (Option<&str>, Option<&str>)> = BTreeMap::new();
    for f in findings {
        rules_seen
            .entry(&f.rule_id)
            .or_insert((f.rule_name.as_deref(), f.rule_description.as_deref()));
    }

    let rules: Vec<SarifRule<'_>> = rules_seen
        .into_iter()
        .map(|(id, (name, description))| SarifRule {
            id,
            name,
            short_description: description.map(|d| SarifText { text: d }),
        })
        .collect();

    let results: Vec<SarifResult<'_>> = findings
        .iter()
        .filter(|f| !matches!(f.severity, Severity::Off))
        .map(|f| {
            let locations = if f.file.is_empty() {
                Vec::new()
            } else {
                vec![SarifLocation {
                    physical_location: SarifPhysicalLocation {
                        artifact_location: SarifArtifactLocation { uri: &f.file },
                        region: match (f.line, f.column) {
                            (None, None) => None,
                            (line, column) => Some(SarifRegion {
                                start_line: line,
                                start_column: column,
                            }),
                        },
                    },
                }]
            };

            let cwe: Vec<&str> = f.cwe.iter().map(|s| s.as_str()).collect();
            let properties = if cwe.is_empty() && f.source.is_none() {
                None
            } else {
                Some(SarifProperties {
                    cwe,
                    source: f.source.as_deref(),
                })
            };

            SarifResult {
                rule_id: &f.rule_id,
                level: match f.severity {
                    Severity::Error => "error",
                    Severity::Warn => "warning",
                    Severity::Off => unreachable!(),
                },
                message: SarifText { text: &f.message },
                locations,
                properties,
            }
        })
        .collect();

    let log = SarifLog {
        schema: SARIF_SCHEMA,
        version: SARIF_VERSION,
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: tool_name,
                    version: tool_version,
                    information_uri: Some("https://docs.scythe.kreuzberg.dev/"),
                    rules,
                },
            },
            results,
        }],
    };

    let s = serde_json::to_string_pretty(&log).map_err(io::Error::other)?;
    out.write_all(s.as_bytes())?;
    out.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sarif_emits_valid_top_level_shape() {
        let findings = vec![Finding {
            file: "q.sql".into(),
            query_name: None,
            rule_id: "SC-SEC02".into(),
            rule_name: Some("grant-all".into()),
            rule_description: Some("CWE-269".into()),
            severity: Severity::Error,
            message: "GRANT ALL".into(),
            line: Some(3),
            column: Some(1),
            cwe: vec!["CWE-269".into()],
            source: None,
        }];
        let mut buf = Vec::new();
        emit("scythe-audit", "0.0.0", &findings, &mut buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed["version"], "2.1.0");
        assert_eq!(parsed["runs"][0]["tool"]["driver"]["name"], "scythe-audit");
        assert_eq!(parsed["runs"][0]["results"][0]["ruleId"], "SC-SEC02");
        assert_eq!(parsed["runs"][0]["results"][0]["level"], "error");
        assert_eq!(
            parsed["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]
                ["uri"],
            "q.sql"
        );
        assert_eq!(
            parsed["runs"][0]["results"][0]["properties"]["cwe"][0],
            "CWE-269"
        );
    }

    #[test]
    fn sarif_empty_findings_emits_empty_results() {
        let mut buf = Vec::new();
        emit("scythe-audit", "0.0.0", &[], &mut buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert!(parsed["runs"][0]["results"].as_array().unwrap().is_empty());
    }
}
