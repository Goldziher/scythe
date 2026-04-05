use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::errors::ScytheError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryCommand {
    One,
    Many,
    Exec,
    ExecResult,
    ExecRows,
    Batch,
}

impl std::fmt::Display for QueryCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryCommand::One => write!(f, "one"),
            QueryCommand::Many => write!(f, "many"),
            QueryCommand::Exec => write!(f, "exec"),
            QueryCommand::ExecResult => write!(f, "exec_result"),
            QueryCommand::ExecRows => write!(f, "exec_rows"),
            QueryCommand::Batch => write!(f, "batch"),
        }
    }
}

impl QueryCommand {
    fn from_str(s: &str) -> Result<Self, ScytheError> {
        match s {
            "one" => Ok(QueryCommand::One),
            "many" => Ok(QueryCommand::Many),
            "exec" => Ok(QueryCommand::Exec),
            "exec_result" => Ok(QueryCommand::ExecResult),
            "exec_rows" => Ok(QueryCommand::ExecRows),
            "batch" => Ok(QueryCommand::Batch),
            other => Err(ScytheError::invalid_annotation(format!(
                "invalid @returns value: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDoc {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonMapping {
    pub column: String,
    pub rust_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotations {
    pub name: String,
    pub command: QueryCommand,
    pub param_docs: Vec<ParamDoc>,
    pub nullable_overrides: Vec<String>,
    pub nonnull_overrides: Vec<String>,
    pub json_mappings: Vec<JsonMapping>,
    pub deprecated: Option<String>,
}

#[derive(Debug)]
pub struct Query {
    pub name: String,
    pub command: QueryCommand,
    pub sql: String,
    pub stmt: sqlparser::ast::Statement,
    pub annotations: Annotations,
}

/// Parse a single annotated SQL query into a `Query`.
pub fn parse_query(query_sql: &str) -> Result<Query, ScytheError> {
    let mut name: Option<String> = None;
    let mut command: Option<QueryCommand> = None;
    let mut param_docs = Vec::new();
    let mut nullable_overrides = Vec::new();
    let mut nonnull_overrides = Vec::new();
    let mut json_mappings = Vec::new();
    let mut deprecated: Option<String> = None;

    let mut sql_lines = Vec::new();

    for line in query_sql.lines() {
        let trimmed = line.trim();

        // Check for annotation: "-- @..." or "--@..."
        let annotation_body = if let Some(rest) = trimmed.strip_prefix("--") {
            let rest = rest.trim_start();
            rest.strip_prefix('@')
        } else {
            None
        };

        if let Some(body) = annotation_body {
            // Parse the annotation keyword and value
            let (keyword, value) = match body.find(|c: char| c.is_whitespace()) {
                Some(pos) => (&body[..pos], body[pos..].trim()),
                None => (body, ""),
            };

            match keyword {
                "name" => {
                    name = Some(value.to_string());
                }
                "returns" => {
                    let cmd_str = value.strip_prefix(':').unwrap_or(value);
                    command = Some(QueryCommand::from_str(cmd_str)?);
                }
                "param" => {
                    // format: "<name>: <description>" or "<name>:<description>"
                    if let Some(colon_pos) = value.find(':') {
                        let param_name = value[..colon_pos].trim().to_string();
                        let description = value[colon_pos + 1..].trim().to_string();
                        param_docs.push(ParamDoc {
                            name: param_name,
                            description,
                        });
                    } else {
                        param_docs.push(ParamDoc {
                            name: value.to_string(),
                            description: String::new(),
                        });
                    }
                }
                "nullable" => {
                    for col in value.split(',') {
                        let col = col.trim();
                        if !col.is_empty() {
                            nullable_overrides.push(col.to_string());
                        }
                    }
                }
                "nonnull" => {
                    for col in value.split(',') {
                        let col = col.trim();
                        if !col.is_empty() {
                            nonnull_overrides.push(col.to_string());
                        }
                    }
                }
                "json" => {
                    // format: "<col> = <Type>"
                    if let Some(eq_pos) = value.find('=') {
                        let column = value[..eq_pos].trim().to_string();
                        let rust_type = value[eq_pos + 1..].trim().to_string();
                        json_mappings.push(JsonMapping { column, rust_type });
                    }
                }
                "deprecated" => {
                    deprecated = Some(value.to_string());
                }
                _ => {
                    // Unknown annotation — ignore or could error
                }
            }
        } else {
            sql_lines.push(line);
        }
    }

    let name = name.ok_or_else(|| ScytheError::missing_annotation("name"))?;
    let command = command.ok_or_else(|| ScytheError::missing_annotation("returns"))?;

    let sql = sql_lines.join("\n").trim().to_string();

    if sql.is_empty() {
        return Err(ScytheError::syntax("empty SQL body"));
    }

    let dialect = PostgreSqlDialect {};
    let statements = Parser::parse_sql(&dialect, &sql)
        .map_err(|e| ScytheError::syntax(format!("syntax error: {}", e)))?;

    if statements.len() != 1 {
        // sqlparser may produce an extra empty statement from a trailing semicolon —
        // filter those out by checking for exactly one non-empty statement.
        let non_empty: Vec<_> = statements
            .into_iter()
            .filter(|s| {
                !matches!(s, sqlparser::ast::Statement::Flush { .. }) && format!("{s}") != ""
            })
            .collect();
        if non_empty.len() != 1 {
            return Err(ScytheError::syntax("expected exactly one SQL statement"));
        }
        let stmt = non_empty.into_iter().next().unwrap();
        let annotations = Annotations {
            name: name.clone(),
            command: command.clone(),
            param_docs,
            nullable_overrides,
            nonnull_overrides,
            json_mappings,
            deprecated,
        };
        return Ok(Query {
            name,
            command,
            sql,
            stmt,
            annotations,
        });
    }

    let stmt = statements.into_iter().next().unwrap();

    let annotations = Annotations {
        name: name.clone(),
        command: command.clone(),
        param_docs,
        nullable_overrides,
        nonnull_overrides,
        json_mappings,
        deprecated,
    };

    Ok(Query {
        name,
        command,
        sql,
        stmt,
        annotations,
    })
}
