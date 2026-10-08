//! Recursive inspection of routine bodies for cluster-scope writes.
//!
//! PL/pgSQL bodies are walked as the compiled tree libpg_query returns, looking
//! for static SQL (`PLpgSQL_stmt_execsql`) and dynamic SQL
//! (`PLpgSQL_stmt_dynexecute`). SQL-language bodies are re-parsed with the SQL
//! grammar. Anything that cannot be inspected is a fail-closed finding.

use serde_json::Value;

use super::GateFinding;
use super::helper::{HelperSet, PgVersion};
use super::policy::{self, RoutineLanguage, RoutineSpec, Verdict};
use super::response::ParseMode;

/// Inspect a routine body, returning the count of extra statements inspected.
pub(super) fn inspect_routine(
    helpers: &HelperSet,
    version: PgVersion,
    spec: &RoutineSpec,
    original_sql: &str,
    file: &str,
    line: Option<usize>,
    findings: &mut Vec<GateFinding>,
) -> usize {
    match &spec.language {
        RoutineLanguage::Plpgsql => match helpers.parse(version, ParseMode::Plpgsql, original_sql) {
            Ok(ast) => walk_plpgsql(helpers, version, ast.as_value(), file, line, findings),
            Err(error) => {
                findings.push(fail_closed(
                    file,
                    line,
                    format!("PL/pgSQL routine body could not be inspected (fail closed): {error}"),
                ));
                0
            }
        },
        RoutineLanguage::Sql => match helpers.parse(version, ParseMode::Sql, &spec.body) {
            Ok(ast) => inspect_inner_sql(ast.as_value(), file, line, findings),
            Err(error) => {
                findings.push(fail_closed(
                    file,
                    line,
                    format!("SQL routine body could not be inspected (fail closed): {error}"),
                ));
                0
            }
        },
        RoutineLanguage::Other(language) => {
            // ~keep A routine in a language the gate cannot read is refused rather
            // ~keep than assumed local (issue #269: never skip silently).
            findings.push(fail_closed(
                file,
                line,
                format!("routine body language '{language}' cannot be inspected (fail closed)"),
            ));
            0
        }
    }
}

/// Walk a PL/pgSQL AST, handling every `EXECUTE`/static statement it contains.
fn walk_plpgsql(
    helpers: &HelperSet,
    version: PgVersion,
    value: &Value,
    file: &str,
    line: Option<usize>,
    findings: &mut Vec<GateFinding>,
) -> usize {
    match value {
        Value::Object(map) => {
            let mut count = 0;
            for (key, child) in map {
                match key.as_str() {
                    "PLpgSQL_stmt_execsql" => {
                        count += 1;
                        match child
                            .get("sqlstmt")
                            .and_then(|s| s.get("PLpgSQL_expr"))
                            .and_then(|e| e.get("query"))
                            .and_then(Value::as_str)
                        {
                            Some(query) => {
                                inspect_static_sql(helpers, version, query, file, line, findings);
                            }
                            None => findings.push(fail_closed(
                                file,
                                line,
                                "PL/pgSQL statement has no recoverable SQL text (fail closed)".to_string(),
                            )),
                        }
                    }
                    "PLpgSQL_stmt_dynexecute" => {
                        count += 1;
                        inspect_dynamic(child, file, line, findings);
                    }
                    _ => {
                        count += walk_plpgsql(helpers, version, child, file, line, findings);
                    }
                }
            }
            count
        }
        Value::Array(items) => items
            .iter()
            .map(|item| walk_plpgsql(helpers, version, item, file, line, findings))
            .sum(),
        _ => 0,
    }
}

/// Classify a static SQL statement recovered from a routine body. Only
/// cluster-scope writes are rejected here: the database-local allow-list
/// applies to top-level planned-schema statements, not to a routine's body.
fn inspect_static_sql(
    helpers: &HelperSet,
    version: PgVersion,
    query: &str,
    file: &str,
    line: Option<usize>,
    findings: &mut Vec<GateFinding>,
) {
    if query.trim().is_empty() {
        return;
    }
    match helpers.parse(version, ParseMode::Sql, query) {
        Ok(ast) => {
            inspect_inner_sql(ast.as_value(), file, line, findings);
        }
        Err(error) => findings.push(fail_closed(
            file,
            line,
            format!("embeddable SQL in a routine body could not be parsed (fail closed): {error}"),
        )),
    }
}

fn inspect_dynamic(node: &Value, file: &str, line: Option<usize>, findings: &mut Vec<GateFinding>) {
    let Some(expression) = node
        .get("query")
        .and_then(|q| q.get("PLpgSQL_expr"))
        .and_then(|e| e.get("query"))
        .and_then(Value::as_str)
    else {
        findings.push(fail_closed(
            file,
            line,
            "PL/pgSQL dynamic statement has no recoverable expression (fail closed)".to_string(),
        ));
        return;
    };
    if let Err(reason) = policy::dynamic_sql_allowed(expression) {
        findings.push(GateFinding {
            file: file.to_string(),
            line,
            code: "SC-CLUSTER03",
            message: format!("unrecognized dynamic SQL execution in planned schema input: {expression} ({reason})"),
        });
    }
}

/// Reject cluster-scope statements inside an already-parsed statement list.
fn inspect_inner_sql(ast: &Value, file: &str, line: Option<usize>, findings: &mut Vec<GateFinding>) -> usize {
    let Some(statements) = ast.get("stmts").and_then(Value::as_array) else {
        return 0;
    };
    for entry in statements {
        if let Some((node_type, value)) = entry
            .get("stmt")
            .and_then(Value::as_object)
            .and_then(|object| object.iter().next())
            && let Verdict::ClusterScope { kind } = policy::classify(node_type, value)
        {
            findings.push(GateFinding {
                file: file.to_string(),
                line,
                code: "SC-CLUSTER01",
                message: format!("cluster-scoped privilege statement ({kind}) in routine body"),
            });
        }
    }
    statements.len()
}

fn fail_closed(file: &str, line: Option<usize>, message: String) -> GateFinding {
    GateFinding {
        file: file.to_string(),
        line,
        code: "SC-CLUSTER04",
        message,
    }
}
