use sqlparser::parser::Parser;

use crate::dialect::SqlDialect;
use crate::errors::ScytheError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryCommand {
    One,
    Many,
    Exec,
    ExecResult,
    ExecRows,
    Batch,
    Grouped,
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
            QueryCommand::Grouped => write!(f, "grouped"),
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
            "grouped" => Ok(QueryCommand::Grouped),
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
    pub optional_params: Vec<String>,
    pub group_by: Option<String>,
}

#[derive(Debug)]
pub struct Query {
    pub name: String,
    pub command: QueryCommand,
    pub sql: String,
    pub stmt: sqlparser::ast::Statement,
    pub annotations: Annotations,
}

/// Parse a single annotated SQL query into a `Query` using the PostgreSQL dialect.
pub fn parse_query(query_sql: &str) -> Result<Query, ScytheError> {
    parse_query_with_dialect(query_sql, &SqlDialect::PostgreSQL)
}

/// Parse a single annotated SQL query into a `Query` using the specified dialect.
pub fn parse_query_with_dialect(
    query_sql: &str,
    dialect: &SqlDialect,
) -> Result<Query, ScytheError> {
    let mut name: Option<String> = None;
    let mut command: Option<QueryCommand> = None;
    let mut param_docs = Vec::new();
    let mut nullable_overrides = Vec::new();
    let mut nonnull_overrides = Vec::new();
    let mut json_mappings = Vec::new();
    let mut deprecated: Option<String> = None;
    let mut optional_params = Vec::new();
    let mut group_by: Option<String> = None;

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

            match keyword.to_ascii_lowercase().as_str() {
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
                "group_by" => {
                    group_by = Some(value.to_string());
                }
                "optional" => {
                    for param in value.split(',') {
                        let param = param.trim();
                        if !param.is_empty() {
                            optional_params.push(param.to_string());
                        }
                    }
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

    if command == QueryCommand::Grouped && group_by.is_none() {
        return Err(ScytheError::invalid_annotation(
            "@returns :grouped requires a @group_by annotation (e.g. @group_by users.id)",
        ));
    }

    let sql = sql_lines.join("\n").trim().to_string();

    if sql.is_empty() {
        return Err(ScytheError::syntax("empty SQL body"));
    }

    let parser_dialect = dialect.to_sqlparser_dialect();
    let statements = Parser::parse_sql(parser_dialect.as_ref(), &sql)
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
        let stmt = non_empty
            .into_iter()
            .next()
            .expect("filtered to exactly one statement");
        let annotations = Annotations {
            name: name.clone(),
            command: command.clone(),
            param_docs,
            nullable_overrides,
            nonnull_overrides,
            json_mappings,
            deprecated,
            optional_params,
            group_by: group_by.clone(),
        };
        return Ok(Query {
            name,
            command,
            sql,
            stmt,
            annotations,
        });
    }

    let stmt = statements
        .into_iter()
        .next()
        .expect("filtered to exactly one statement");

    let annotations = Annotations {
        name: name.clone(),
        command: command.clone(),
        param_docs,
        nullable_overrides,
        nonnull_overrides,
        json_mappings,
        deprecated,
        optional_params,
        group_by,
    };

    Ok(Query {
        name,
        command,
        sql,
        stmt,
        annotations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorCode;

    fn parse(sql: &str) -> Result<Query, ScytheError> {
        parse_query(sql)
    }

    #[test]
    fn test_basic_parse() {
        let input = "-- @name GetUsers\n-- @returns :many\nSELECT * FROM users;";
        let q = parse(input).unwrap();
        assert_eq!(q.name, "GetUsers");
        assert_eq!(q.command, QueryCommand::Many);
        assert!(q.sql.contains("SELECT"));
    }

    #[test]
    fn test_all_command_types() {
        let cases = vec![
            (":one", QueryCommand::One),
            (":many", QueryCommand::Many),
            (":exec", QueryCommand::Exec),
            (":exec_result", QueryCommand::ExecResult),
            (":exec_rows", QueryCommand::ExecRows),
        ];
        for (tag, expected) in cases {
            let input = format!("-- @name Q\n-- @returns {}\nSELECT 1", tag);
            let q = parse(&input).unwrap();
            assert_eq!(q.command, expected, "failed for {}", tag);
        }
    }

    #[test]
    fn test_case_insensitive_keywords() {
        let input = "-- @Name GetUsers\n-- @RETURNS :many\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.name, "GetUsers");
        assert_eq!(q.command, QueryCommand::Many);
    }

    #[test]
    fn test_missing_name_errors() {
        let input = "-- @returns :many\nSELECT 1";
        let err = parse(input).unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingAnnotation);
        assert!(err.message.contains("name"));
    }

    #[test]
    fn test_missing_returns_errors() {
        let input = "-- @name Foo\nSELECT 1";
        let err = parse(input).unwrap_err();
        assert_eq!(err.code, ErrorCode::MissingAnnotation);
        assert!(err.message.contains("returns"));
    }

    #[test]
    fn test_invalid_returns_value() {
        let input = "-- @name Foo\n-- @returns :invalid\nSELECT 1";
        let err = parse(input).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidAnnotation);
    }

    #[test]
    fn test_empty_name_value() {
        // An empty name is accepted by the parser (it stores "")
        let input = "-- @name\n-- @returns :one\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.name, "");
    }

    #[test]
    fn test_param_annotation() {
        let input = "-- @name Foo\n-- @returns :one\n-- @param id: the user ID\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.annotations.param_docs.len(), 1);
        assert_eq!(q.annotations.param_docs[0].name, "id");
        assert_eq!(q.annotations.param_docs[0].description, "the user ID");
    }

    #[test]
    fn test_param_no_description() {
        let input = "-- @name Foo\n-- @returns :one\n-- @param id\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.annotations.param_docs.len(), 1);
        assert_eq!(q.annotations.param_docs[0].name, "id");
        assert_eq!(q.annotations.param_docs[0].description, "");
    }

    #[test]
    fn test_nullable_annotation() {
        let input = "-- @name Foo\n-- @returns :one\n-- @nullable col1, col2\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.annotations.nullable_overrides, vec!["col1", "col2"]);
    }

    #[test]
    fn test_nonnull_annotation() {
        let input = "-- @name Foo\n-- @returns :one\n-- @nonnull col1\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.annotations.nonnull_overrides, vec!["col1"]);
    }

    #[test]
    fn test_json_annotation() {
        let input = "-- @name Foo\n-- @returns :one\n-- @json data = EventData\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.annotations.json_mappings.len(), 1);
        assert_eq!(q.annotations.json_mappings[0].column, "data");
        assert_eq!(q.annotations.json_mappings[0].rust_type, "EventData");
    }

    #[test]
    fn test_deprecated_annotation() {
        let input = "-- @name Foo\n-- @returns :one\n-- @deprecated Use V2\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.annotations.deprecated, Some("Use V2".to_string()));
    }

    #[test]
    fn test_sql_syntax_error() {
        let input = "-- @name Foo\n-- @returns :one\nSELCT * FROM users";
        let err = parse(input).unwrap_err();
        assert_eq!(err.code, ErrorCode::SyntaxError);
    }

    #[test]
    fn test_trailing_semicolon() {
        let input = "-- @name Foo\n-- @returns :one\nSELECT 1;";
        let q = parse(input).unwrap();
        assert_eq!(q.name, "Foo");
    }

    #[test]
    fn test_multiple_statements_error() {
        let input = "-- @name Foo\n-- @returns :one\nSELECT 1; SELECT 2;";
        let err = parse(input).unwrap_err();
        assert_eq!(err.code, ErrorCode::SyntaxError);
    }

    #[test]
    fn test_sql_preserved_without_annotations() {
        let input = "-- @name Foo\n-- @returns :one\nSELECT id, name FROM users WHERE id = $1";
        let q = parse(input).unwrap();
        assert_eq!(q.sql, "SELECT id, name FROM users WHERE id = $1");
    }

    #[test]
    fn test_returns_without_colon_prefix() {
        let input = "-- @name Foo\n-- @returns many\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.command, QueryCommand::Many);
    }

    #[test]
    fn test_batch_command() {
        let input = "-- @name Foo\n-- @returns :batch\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.command, QueryCommand::Batch);
    }

    #[test]
    fn test_grouped_command_with_group_by() {
        let input = "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\nSELECT u.id, u.name FROM users u JOIN orders o ON o.user_id = u.id";
        let q = parse(input).unwrap();
        assert_eq!(q.command, QueryCommand::Grouped);
        assert_eq!(q.annotations.group_by, Some("users.id".to_string()));
    }

    #[test]
    fn test_grouped_command_without_group_by_errors() {
        let input = "-- @name Foo\n-- @returns :grouped\nSELECT 1";
        let err = parse(input).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidAnnotation);
        assert!(err.message.contains("@group_by"));
    }

    #[test]
    fn test_group_by_without_grouped_is_ignored() {
        let input = "-- @name Foo\n-- @returns :many\n-- @group_by users.id\nSELECT 1";
        let q = parse(input).unwrap();
        assert_eq!(q.command, QueryCommand::Many);
        assert_eq!(q.annotations.group_by, Some("users.id".to_string()));
    }
}
