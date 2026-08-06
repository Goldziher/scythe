//! Verify statically-inferred query types against a live database.
//!
//! Scythe infers result columns and parameters by parsing schema DDL and the
//! query itself.  Nothing in that pipeline ever asks the database whether the
//! answer is right, so a parser that drifts from an engine's grammar can
//! produce a plausible row type that does not match what the server returns.
//!
//! This module closes half that gap.  Preparing a statement server-side makes
//! PostgreSQL report the result column types and parameter types without
//! executing anything, which catches:
//!
//! - a column count mismatch (the projection was parsed incorrectly)
//! - a column type mismatch (the wrong catalog type was mapped)
//! - a parameter count or type mismatch
//! - a query the server rejects outright but the parser accepted
//!
//! ## What this deliberately does not verify
//!
//! **Nullability.** The describe response carries type OIDs but no nullability
//! information, and nullability is the part that matters most — outer joins and
//! aggregates over empty sets are exactly where inference is hardest.  This
//! verifies the half the server knows about; the other half remains scythe's
//! responsibility.  A drifted parser will nearly always show up in the half the
//! server does know about, which is what makes the check worth running.

use scythe_core::analyzer::AnalyzedQuery;
use scythe_lint::reporters::Finding;
use scythe_lint::types::Severity;
use tokio_postgres::Client;

pub mod pg_types;

/// A query the server rejected outright.
pub const SC_VER01: &str = "SC-VER01";
/// The server returned a different number of result columns than was inferred.
pub const SC_VER02: &str = "SC-VER02";
/// A result column's type does not match the inferred type.
pub const SC_VER03: &str = "SC-VER03";
/// The server expects a different number of parameters than was inferred.
pub const SC_VER04: &str = "SC-VER04";
/// A parameter's type does not match the inferred type.
pub const SC_VER05: &str = "SC-VER05";

/// Tag applied to `Finding::source` so reporters can distinguish these from
/// lint, audit and inspect findings.
const SOURCE: &str = "check";

fn finding(file: &str, query_name: &str, rule_id: &str, rule_name: &str, message: String) -> Finding {
    Finding {
        file: file.to_string(),
        query_name: Some(query_name.to_string()),
        rule_id: rule_id.to_string(),
        rule_name: Some(rule_name.to_string()),
        rule_description: Some("statically-inferred query shape does not match what the database reports".to_string()),
        severity: Severity::Error,
        message,
        line: None,
        column: None,
        cwe: Vec::new(),
        source: Some(SOURCE.to_string()),
    }
}

/// Prepare every query server-side and report where the server disagrees with
/// static inference.
///
/// `file` is used only to label the findings.  Queries are prepared, never
/// executed, so this is safe to run against a production database.
pub async fn verify_queries(client: &Client, file: &str, queries: &[AnalyzedQuery]) -> Vec<Finding> {
    let mut findings = Vec::new();

    for query in queries {
        let statement = match client.prepare(&query.sql).await {
            Ok(statement) => statement,
            Err(error) => {
                findings.push(finding(
                    file,
                    &query.name,
                    SC_VER01,
                    "query-rejected-by-server",
                    format!(
                        "the database rejected this query: {}",
                        crate::error::error_chain(&error)
                    ),
                ));
                continue;
            }
        };

        verify_columns(&mut findings, file, query, &statement);
        verify_params(&mut findings, file, query, &statement);
    }

    findings
}

fn verify_columns(
    findings: &mut Vec<Finding>,
    file: &str,
    query: &AnalyzedQuery,
    statement: &tokio_postgres::Statement,
) {
    let reported = statement.columns();

    if reported.len() != query.columns.len() {
        findings.push(finding(
            file,
            &query.name,
            SC_VER02,
            "result-column-count-mismatch",
            format!(
                "inferred {} result column(s) but the database reports {}: [{}]",
                query.columns.len(),
                reported.len(),
                reported.iter().map(|c| c.name()).collect::<Vec<_>>().join(", ")
            ),
        ));
        return;
    }

    for (inferred, actual) in query.columns.iter().zip(reported) {
        let Some(actual_neutral) = pg_types::neutral_type_for(actual.type_()) else {
            continue;
        };

        if !pg_types::types_are_compatible(&inferred.neutral_type, &actual_neutral) {
            findings.push(finding(
                file,
                &query.name,
                SC_VER03,
                "result-column-type-mismatch",
                format!(
                    "column `{}` was inferred as `{}` but the database reports `{}` ({})",
                    inferred.name,
                    inferred.neutral_type,
                    actual_neutral,
                    actual.type_().name()
                ),
            ));
        }
    }
}

fn verify_params(
    findings: &mut Vec<Finding>,
    file: &str,
    query: &AnalyzedQuery,
    statement: &tokio_postgres::Statement,
) {
    let reported = statement.params();

    if reported.len() != query.params.len() {
        findings.push(finding(
            file,
            &query.name,
            SC_VER04,
            "parameter-count-mismatch",
            format!(
                "inferred {} parameter(s) but the database expects {}",
                query.params.len(),
                reported.len()
            ),
        ));
        return;
    }

    for (inferred, actual) in query.params.iter().zip(reported) {
        let Some(actual_neutral) = pg_types::neutral_type_for(actual) else {
            continue;
        };

        if !pg_types::types_are_compatible(&inferred.neutral_type, &actual_neutral) {
            findings.push(finding(
                file,
                &query.name,
                SC_VER05,
                "parameter-type-mismatch",
                format!(
                    "parameter `{}` (${}) was inferred as `{}` but the database expects `{}` ({})",
                    inferred.name,
                    inferred.position,
                    inferred.neutral_type,
                    actual_neutral,
                    actual.name()
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_are_tagged_as_check_source() {
        let f = finding(
            "q.sql",
            "GetUser",
            SC_VER02,
            "result-column-count-mismatch",
            "boom".into(),
        );
        assert_eq!(f.source.as_deref(), Some("check"));
        assert_eq!(f.query_name.as_deref(), Some("GetUser"));
        assert_eq!(f.rule_id, SC_VER02);
        assert_eq!(f.severity, Severity::Error);
    }

    /// The rule IDs are part of the user-facing contract (suppression, SARIF
    /// taxonomies), so guard them against accidental renaming.
    #[test]
    fn rule_ids_are_stable() {
        assert_eq!(SC_VER01, "SC-VER01");
        assert_eq!(SC_VER02, "SC-VER02");
        assert_eq!(SC_VER03, "SC-VER03");
        assert_eq!(SC_VER04, "SC-VER04");
        assert_eq!(SC_VER05, "SC-VER05");
    }
}
