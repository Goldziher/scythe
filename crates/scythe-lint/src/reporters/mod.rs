//! Output formats for lint / audit findings.
//!
//! Three reporters ship today:
//! - [`human`]: matches the existing `scythe lint` text format.
//! - [`sarif`]: SARIF 2.1.0, GitHub code-scanning compatible.
//! - [`json`]: flat machine-readable JSON for CI tooling that doesn't ingest
//!   SARIF.
//!
//! Reporters operate on a shared [`Finding`] type so callers can build the
//! finding list once and dispatch to any format.

pub mod human;
pub mod json;
pub mod sarif;

use std::io;

use crate::types::Severity;

/// Output format selector parsed from CLI flags / config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Sarif,
    Json,
}

impl Format {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "human" | "text" | "pretty" => Some(Self::Human),
            "sarif" => Some(Self::Sarif),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// A reported finding — a violation enriched with its source location, the
/// effective severity (after config overrides), and optional CWE tags supplied
/// by the rule's description for SARIF emission.
#[derive(Debug, Clone)]
pub struct Finding {
    pub file: String,
    pub query_name: Option<String>,
    pub rule_id: String,
    pub rule_name: Option<String>,
    pub rule_description: Option<String>,
    pub severity: Severity,
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    /// CWE identifiers (e.g. `CWE-78`) extracted from the rule description.
    pub cwe: Vec<String>,
}

/// Emit findings in the chosen format.
pub fn emit(
    format: Format,
    tool_name: &str,
    tool_version: &str,
    findings: &[Finding],
    out: &mut dyn io::Write,
) -> io::Result<()> {
    match format {
        Format::Human => human::emit(findings, out),
        Format::Sarif => sarif::emit(tool_name, tool_version, findings, out),
        Format::Json => json::emit(findings, out),
    }
}

/// Pull every `CWE-NNN` token out of a description string.
pub fn extract_cwe(description: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = description.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..i + 4].eq_ignore_ascii_case(b"CWE-") {
            let mut j = i + 4;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 4 {
                out.push(format!("CWE-{}", &description[i + 4..j]));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parse_round_trip() {
        assert_eq!(Format::parse("human"), Some(Format::Human));
        assert_eq!(Format::parse("SARIF"), Some(Format::Sarif));
        assert_eq!(Format::parse("json"), Some(Format::Json));
        assert_eq!(Format::parse("xml"), None);
    }

    #[test]
    fn extract_cwe_finds_single() {
        assert_eq!(extract_cwe("see CWE-78"), vec!["CWE-78".to_string()]);
    }

    #[test]
    fn extract_cwe_finds_multiple() {
        assert_eq!(
            extract_cwe("CWE-78 / CWE-732 — combined"),
            vec!["CWE-78".to_string(), "CWE-732".to_string()]
        );
    }

    #[test]
    fn extract_cwe_skips_non_matches() {
        assert!(extract_cwe("nothing tagged here").is_empty());
        assert!(extract_cwe("CWE-").is_empty());
    }
}
