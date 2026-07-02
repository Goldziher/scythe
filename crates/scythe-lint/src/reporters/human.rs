//! Human-readable reporter — matches the existing `scythe lint` text format so
//! `scythe audit` output diffs cleanly against current expectations.

use std::io::{self, Write};

use crate::types::Severity;

use super::Finding;

pub fn emit(findings: &[Finding], out: &mut dyn Write) -> io::Result<()> {
    if findings.is_empty() {
        writeln!(out, "No findings.")?;
        return Ok(());
    }

    let mut current_file: Option<&str> = None;
    for f in findings {
        if current_file != Some(f.file.as_str()) {
            if !f.file.is_empty() {
                writeln!(out)?;
                writeln!(out, "{}:", f.file)?;
            }
            current_file = Some(f.file.as_str());
        }

        let severity_str = match f.severity {
            Severity::Error => "error",
            Severity::Warn => "warning",
            Severity::Off => continue,
        };

        let location = match (f.line, f.column) {
            (Some(line), Some(col)) => format!("{}:{}", line, col),
            _ => match &f.query_name {
                Some(name) => format!("query:{}", name),
                None => String::new(),
            },
        };

        let source_tag = match f.source.as_deref() {
            Some(s) if !s.is_empty() => format!("[{}] ", s),
            _ => String::new(),
        };

        if location.is_empty() {
            writeln!(out, "  {}{}: [{}] {}", source_tag, severity_str, f.rule_id, f.message)?;
        } else {
            writeln!(
                out,
                "  {} {}{}: [{}] {}",
                location, source_tag, severity_str, f.rule_id, f.message
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Finding> {
        vec![Finding {
            file: "queries.sql".into(),
            query_name: Some("DropAll".into()),
            rule_id: "SC-SEC02".into(),
            rule_name: Some("grant-all".into()),
            rule_description: None,
            severity: Severity::Error,
            message: "GRANT ALL".into(),
            line: None,
            column: None,
            cwe: vec!["CWE-269".into()],
            source: None,
        }]
    }

    #[test]
    fn human_emits_grouped_by_file() {
        let mut buf = Vec::new();
        emit(&fixture(), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("queries.sql:"));
        assert!(s.contains("[SC-SEC02]"));
        assert!(s.contains("GRANT ALL"));
    }

    #[test]
    fn human_empty_says_so() {
        let mut buf = Vec::new();
        emit(&[], &mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "No findings.\n");
    }
}
