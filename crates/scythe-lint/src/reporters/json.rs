//! Flat JSON reporter — for CI consumers that don't ingest SARIF.

use std::io::{self, Write};

use serde::Serialize;

use crate::types::Severity;

use super::Finding;

#[derive(Serialize)]
struct JsonFinding<'a> {
    file: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    query_name: Option<&'a str>,
    rule_id: &'a str,
    severity: &'static str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cwe: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
}

pub fn emit(findings: &[Finding], out: &mut dyn Write) -> io::Result<()> {
    let payload: Vec<JsonFinding<'_>> = findings
        .iter()
        .filter(|f| !matches!(f.severity, Severity::Off))
        .map(|f| JsonFinding {
            file: &f.file,
            query_name: f.query_name.as_deref(),
            rule_id: &f.rule_id,
            severity: match f.severity {
                Severity::Error => "error",
                Severity::Warn => "warning",
                Severity::Off => unreachable!(),
            },
            message: &f.message,
            line: f.line,
            column: f.column,
            cwe: f.cwe.iter().map(|s| s.as_str()).collect(),
            source: f.source.as_deref(),
        })
        .collect();

    let s = serde_json::to_string_pretty(&payload).map_err(io::Error::other)?;
    out.write_all(s.as_bytes())?;
    out.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_emits_array_of_findings() {
        let findings = vec![Finding {
            file: "q.sql".into(),
            query_name: None,
            rule_id: "SC-SEC02".into(),
            rule_name: None,
            rule_description: None,
            severity: Severity::Error,
            message: "GRANT ALL".into(),
            line: None,
            column: None,
            cwe: vec!["CWE-269".into()],
            source: None,
        }];
        let mut buf = Vec::new();
        emit(&findings, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["rule_id"], "SC-SEC02");
        assert_eq!(arr[0]["severity"], "error");
        assert_eq!(arr[0]["cwe"][0], "CWE-269");
    }

    #[test]
    fn json_empty_is_empty_array() {
        let mut buf = Vec::new();
        emit(&[], &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.trim(), "[]");
    }
}
