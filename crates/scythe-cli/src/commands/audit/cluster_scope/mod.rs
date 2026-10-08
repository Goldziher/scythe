//! Fail-closed PostgreSQL cluster-scope gate for planned-schema inputs.
//!
//! See issue #269. `scythe audit --cluster-scope <file>...` parses each input
//! with a version-pinned PostgreSQL grammar (15 or 18, never sqlparser) and
//! refuses, by default, every statement that is not proven database-local:
//! role/membership, database, tablespace, subscription, parameter-privilege,
//! `ALTER SYSTEM`, and shared-ownership writes. Routine bodies are inspected
//! recursively, and dynamic SQL is refused unless it is statically recoverable
//! and local.
//!
//! Nothing is ever skipped silently: a parse gap, an uninspectable routine
//! language, or an unresolved `EXECUTE` is a finding, and the process exits
//! non-zero even under a severity floor or `--exit-zero`.

pub mod helper;
pub mod policy;
pub mod response;
mod walk;

use serde_json::Value;

use self::helper::{HelperSet, PgVersion};
use self::policy::Verdict;
use self::response::ParseMode;

/// A gate finding, carrying a stable machine code alongside the human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateFinding {
    pub file: String,
    pub line: Option<usize>,
    pub code: &'static str,
    pub message: String,
}

/// Aggregate result of gating one or more inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateReport {
    /// Number of input files inspected.
    pub inputs: usize,
    /// Number of executable statements inspected, including routine bodies.
    pub statements: usize,
    pub findings: Vec<GateFinding>,
}

/// A parse failure, missing helper, unreadable input, or empty input.
#[derive(Debug)]
pub struct GateError(pub String);

impl std::fmt::Display for GateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GateError {}

/// Run the cluster-scope gate over explicit planned-schema `files`.
pub fn run(files: &[String], version: PgVersion) -> Result<GateReport, GateError> {
    if files.is_empty() {
        return Err(GateError(
            "cluster-scope gate requires explicit synthesized planned-schema file(s)".to_string(),
        ));
    }

    let helpers = HelperSet::discover().map_err(GateError)?;
    let mut report = GateReport {
        inputs: 0,
        statements: 0,
        findings: Vec::new(),
    };

    for path in files {
        let sql = std::fs::read_to_string(path)
            .map_err(|error| GateError(format!("cluster-scope: failed to read '{path}': {error}")))?;
        if sql.trim().is_empty() {
            return Err(GateError(format!("{path}: planned schema input is empty")));
        }
        gate_file(&helpers, version, path, &sql, &mut report)?;
        report.inputs += 1;
    }

    Ok(report)
}

fn gate_file(
    helpers: &HelperSet,
    version: PgVersion,
    path: &str,
    sql: &str,
    report: &mut GateReport,
) -> Result<(), GateError> {
    // ~keep A whole-file parse gap is a fail-closed finding, never an empty success:
    // ~keep `--exit-zero` and a severity floor must not be able to turn an unparsed
    // ~keep statement green. The finding still exits non-zero at the CLI.
    let ast = match helpers.parse(version, ParseMode::Sql, sql) {
        Ok(ast) => ast,
        Err(error) => {
            report.findings.push(GateFinding {
                file: path.to_string(),
                line: None,
                code: "SC-CLUSTER04",
                message: format!("planned schema input could not be parsed (fail closed): {error}"),
            });
            return Ok(());
        }
    };

    let statements = ast
        .as_value()
        .get("stmts")
        .and_then(Value::as_array)
        .ok_or_else(|| GateError(format!("{path}: parser returned no statement list")))?;

    for entry in statements {
        let node = entry
            .get("stmt")
            .and_then(Value::as_object)
            .and_then(|object| object.iter().next())
            .ok_or_else(|| GateError(format!("{path}: parser returned a statement with no node")))?;
        let (node_type, value) = (node.0.as_str(), node.1);
        let line = entry
            .get("stmt_location")
            .and_then(Value::as_i64)
            .map(|loc| line_of(sql, loc));
        let slice = statement_slice(sql, entry);

        report.statements += 1;
        match policy::classify(node_type, value) {
            Verdict::Local => {}
            Verdict::ClusterScope { kind } => report.findings.push(GateFinding {
                file: path.to_string(),
                line,
                code: "SC-CLUSTER01",
                message: format!("cluster-scoped privilege statement ({kind}): {node_type}"),
            }),
            Verdict::NotAllowlisted => report.findings.push(GateFinding {
                file: path.to_string(),
                line,
                code: "SC-CLUSTER02",
                message: format!("statement kind outside the database-local allow-list: {node_type}"),
            }),
            Verdict::Routine(spec) => {
                report.statements +=
                    walk::inspect_routine(helpers, version, &spec, slice, path, line, &mut report.findings);
            }
        }
    }

    Ok(())
}

/// 1-based line number of a byte offset.
fn line_of(sql: &str, offset: i64) -> usize {
    let offset = offset.max(0) as usize;
    let offset = offset.min(sql.len());
    sql.as_bytes()[..offset].iter().filter(|b| **b == b'\n').count() + 1
}

/// The statement's original source text, for re-parsing routine bodies with the
/// PL/pgSQL grammar.
fn statement_slice<'a>(sql: &'a str, entry: &Value) -> &'a str {
    let location = entry.get("stmt_location").and_then(Value::as_i64).unwrap_or(0).max(0) as usize;
    let length = entry.get("stmt_len").and_then(Value::as_i64).unwrap_or(0).max(0) as usize;
    let start = location.min(sql.len());
    let end = if length == 0 {
        sql.len()
    } else {
        (start + length).min(sql.len())
    };
    &sql[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_list_is_an_error() {
        assert!(run(&[], PgVersion::Pg18).is_err());
    }

    #[test]
    fn line_of_is_one_based() {
        assert_eq!(line_of("a\nb\nc", 0), 1);
        assert_eq!(line_of("a\nb\nc", 2), 2);
        assert_eq!(line_of("a\nb\nc", 4), 3);
    }
}
