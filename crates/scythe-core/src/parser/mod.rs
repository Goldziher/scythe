use sqlparser::parser::Parser;

use crate::dialect::SqlDialect;
use crate::errors::ScytheError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryCommand {
    One,
    Opt,
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
            QueryCommand::Opt => write!(f, "opt"),
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
            "opt" => Ok(QueryCommand::Opt),
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

    // Preprocess dialect-specific syntax before parsing:
    //   * Oracle: strip `RETURNING ... INTO` output binds, convert `:N` → `?`.
    //   * MSSQL: convert `OUTPUT INSERTED.*` → `RETURNING` for parsing,
    //     convert `@pN` → `?` for parsing; keep original SQL for codegen.
    //   * PostgreSQL: strip `WHERE …` between `ON CONFLICT (cols)` and `DO …`
    //     for parsing (sqlparser-rs <= 0.61 doesn't recognise the
    //     partial-index inference form); keep original SQL for codegen.
    let (sql, parse_sql) = if *dialect == SqlDialect::Oracle {
        let processed = preprocess_oracle_sql(&sql);
        (processed.clone(), processed)
    } else if *dialect == SqlDialect::MsSql {
        // For codegen: only convert @pN → ? placeholders (keep OUTPUT syntax)
        let codegen_sql = convert_mssql_placeholders(&sql);
        // For parsing: also convert OUTPUT INSERTED → RETURNING
        let parse_sql = preprocess_mssql_sql(&sql);
        (codegen_sql, parse_sql)
    } else if *dialect == SqlDialect::PostgreSQL {
        let parse_sql = preprocess_postgres_sql(&sql);
        (sql.clone(), parse_sql)
    } else {
        (sql.clone(), sql)
    };

    let parser_dialect = dialect.to_sqlparser_dialect();
    let statements = Parser::parse_sql(parser_dialect.as_ref(), &parse_sql)
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

/// Strip the `WHERE …` predicate that PostgreSQL allows between
/// `ON CONFLICT (cols)` and `DO …` (the index-inference form for partial
/// unique indexes). sqlparser-rs through 0.61 does not parse this construct;
/// we lift it out for the parser and let the caller keep the original SQL
/// for codegen + runtime, where Postgres validates it.
fn preprocess_postgres_sql(sql: &str) -> String {
    // Strip line comments + string literals first so we only scan structural SQL.
    // (We still emit the original `sql` slice byte-for-byte; the upper-mask is
    //  only used to decide *where* to cut.)
    let mask = mask_postgres_for_scan(sql);
    let mask_bytes = mask.as_bytes();
    let bytes = sql.as_bytes();
    let mut search_from = 0;
    let mut result = String::with_capacity(sql.len());
    let mut last = 0;
    while let Some(rel) = find_keyword(&mask[search_from..], "ON CONFLICT") {
        let on_conflict_pos = search_from + rel;
        let after_on_conflict = on_conflict_pos + "ON CONFLICT".len();
        let mut idx = after_on_conflict;
        while idx < mask_bytes.len() && mask_bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= mask_bytes.len() || mask_bytes[idx] != b'(' {
            search_from = after_on_conflict;
            continue;
        }
        let mut depth = 0i32;
        let mut close = idx;
        while close < mask_bytes.len() {
            match mask_bytes[close] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            close += 1;
        }
        if depth != 0 {
            return sql.to_string();
        }
        let mut after_cols = close + 1;
        while after_cols < mask_bytes.len() && mask_bytes[after_cols].is_ascii_whitespace() {
            after_cols += 1;
        }
        if mask[after_cols..].starts_with("WHERE")
            && let Some(do_rel) = find_keyword(&mask[after_cols + "WHERE".len()..], "DO")
        {
            let do_abs = after_cols + "WHERE".len() + do_rel;
            // Slice from the original SQL (preserves casing + UTF-8) up to
            // the byte before WHERE; skip ahead to DO.
            result.push_str(std::str::from_utf8(&bytes[last..after_cols]).unwrap_or(""));
            last = do_abs;
            search_from = do_abs;
            continue;
        }
        search_from = close + 1;
    }
    result.push_str(std::str::from_utf8(&bytes[last..]).unwrap_or(""));
    result
}

/// Build an ASCII-uppercase, fixed-byte-offset mask of `sql` where `--` line
/// comments, `/* … */` block comments, and `'…'` / `$$…$$` string literals are
/// replaced with spaces. Multi-byte UTF-8 is collapsed to ASCII spaces of the
/// same byte length so positions in the mask line up with the original `sql`.
fn mask_postgres_for_scan(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = vec![b' '; bytes.len()];
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            // Line comment — replace through end-of-line with spaces.
            while i < bytes.len() && bytes[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            // Block comment — replace through `*/`.
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                out[i] = b' ';
                i += 1;
            }
            if i + 1 < bytes.len() {
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
            }
            continue;
        }
        if b == b'\'' {
            out[i] = b' ';
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        out[i] = b' ';
                        out[i + 1] = b' ';
                        i += 2;
                        continue;
                    }
                    out[i] = b' ';
                    i += 1;
                    break;
                }
                out[i] = b' ';
                i += 1;
            }
            continue;
        }
        // ASCII goes through as-uppercase; non-ASCII bytes become spaces so the
        // mask stays single-byte-per-position and positions line up.
        if b.is_ascii() {
            out[i] = b.to_ascii_uppercase();
        } else {
            out[i] = b' ';
        }
        i += 1;
    }
    String::from_utf8(out).expect("mask is ASCII by construction")
}

/// Locate a whitespace-separated keyword in an uppercase haystack. Returns the
/// byte offset of the keyword's start, or None if not found.
fn find_keyword(haystack: &str, keyword: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let key = keyword.as_bytes();
    let mut i = 0;
    while i + key.len() <= bytes.len() {
        if &bytes[i..i + key.len()] == key {
            let prev_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let next = i + key.len();
            let next_ok = next >= bytes.len() || !bytes[next].is_ascii_alphanumeric();
            if prev_ok && next_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Preprocess Oracle SQL before parsing:
/// 1. Strip `INTO :N, :N, ...` suffix from `RETURNING ... INTO` clauses.
/// 2. Convert `:N` positional placeholders to `?` (universally supported).
fn preprocess_oracle_sql(sql: &str) -> String {
    // Strip Oracle RETURNING ... INTO clause (output bind variables)
    // e.g. "INSERT ... RETURNING id, name INTO :4, :5" → "INSERT ... RETURNING id, name"
    let sql = strip_returning_into(sql);

    // Convert :N → ? (outside string literals)
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            // Skip string literals
            result.push(ch);
            while let Some(inner) = chars.next() {
                result.push(inner);
                if inner == '\'' {
                    if chars.peek() == Some(&'\'') {
                        result.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
            }
        } else if ch == ':' && chars.peek().is_some_and(|c| c.is_ascii_digit()) {
            // Convert :N → ?
            result.push('?');
            while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                chars.next();
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Convert MSSQL `@pN` positional placeholders to `?` (outside string literals).
/// MsSqlDialect treats `@` as an identifier start, so `@p1` becomes an identifier
/// rather than a `Placeholder` token — preprocessing normalises it to `?`.
fn convert_mssql_placeholders(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            // Skip string literals verbatim
            result.push(ch);
            while let Some(inner) = chars.next() {
                result.push(inner);
                if inner == '\'' {
                    if chars.peek() == Some(&'\'') {
                        // Escaped quote inside string literal
                        result.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
            }
        } else if ch == '@' && chars.peek().is_some_and(|c| *c == 'p' || *c == 'P') {
            // Peek ahead: must be `@p` followed by at least one digit
            let mut lookahead = chars.clone();
            lookahead.next(); // consume the 'p'/'P'
            if lookahead.peek().is_some_and(|c| c.is_ascii_digit()) {
                // It is an `@pN` placeholder — consume `p` and all digits
                chars.next(); // consume 'p'/'P'
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                }
                result.push('?');
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Preprocess MSSQL SQL before parsing:
/// 1. Strip `OUTPUT INSERTED.col, ...` clauses and convert to RETURNING
/// 2. Convert `@pN` positional placeholders to `?`
fn preprocess_mssql_sql(sql: &str) -> String {
    // First pass: convert OUTPUT INSERTED.col to RETURNING col
    let sql = strip_and_convert_mssql_output(sql);
    // Second pass: convert @pN to ?
    convert_mssql_placeholders(&sql)
}

/// Strip MSSQL `OUTPUT INSERTED.col1, INSERTED.col2, ...` from INSERT statements
/// and convert it to a `RETURNING col1, col2, ...` clause.
/// The OUTPUT clause appears between the column list and VALUES clause:
///   INSERT INTO table (cols) OUTPUT INSERTED.col1, INSERTED.col2, ... VALUES (...)
/// becomes:
///   INSERT INTO table (cols) VALUES (...) RETURNING col1, col2, ...
fn strip_and_convert_mssql_output(sql: &str) -> String {
    // Case-insensitive search for OUTPUT keyword in INSERT statements
    let upper = sql.to_uppercase();

    // Only process INSERT statements with OUTPUT
    if !upper.contains("INSERT") || !upper.contains("OUTPUT") {
        return sql.to_string();
    }

    // Find the OUTPUT keyword
    if let Some(output_pos) = find_word_position(&upper, "OUTPUT") {
        // Check if this is actually part of an INSERT statement by finding INSERT before it
        let before_output = &upper[..output_pos];
        if !before_output.contains("INSERT") {
            return sql.to_string();
        }

        // Look for the VALUES keyword after OUTPUT
        let after_output = &upper[output_pos + "OUTPUT".len()..];
        if let Some(values_offset) = find_word_position(after_output, "VALUES") {
            let values_pos = output_pos + "OUTPUT".len() + values_offset;

            // Extract the OUTPUT column list (between OUTPUT and VALUES)
            let output_cols_str = &sql[output_pos + "OUTPUT".len()..values_pos];

            // Parse column names: strip "INSERTED." prefix from each column name
            let cols = parse_inserted_columns(output_cols_str);

            if !cols.is_empty() {
                // Build result: keep everything before OUTPUT, then VALUES clause,
                // then RETURNING clause (before any trailing semicolon)
                let before_output_sql = sql[..output_pos].trim_end();
                let after_values = sql[values_pos..].trim_end();
                let (values_body, trailing) = if let Some(stripped) = after_values.strip_suffix(';')
                {
                    (stripped, ";")
                } else {
                    (after_values, "")
                };

                return format!(
                    "{}\n{} RETURNING {}{}",
                    before_output_sql, values_body, cols, trailing
                );
            }
        }
    }

    sql.to_string()
}

/// Find the position of a word (case-insensitive) in the text.
/// The word must be a separate word, not part of another identifier.
fn find_word_position(text: &str, word: &str) -> Option<usize> {
    let mut pos = 0;
    let word_len = word.len();
    while let Some(idx) = text[pos..].find(word) {
        let abs_idx = pos + idx;

        // Check character before
        let before_ok = abs_idx == 0
            || !text
                .as_bytes()
                .get(abs_idx - 1)
                .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_');

        // Check character after
        let after_idx = abs_idx + word_len;
        let after_ok = after_idx >= text.len()
            || !text
                .as_bytes()
                .get(after_idx)
                .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_');

        if before_ok && after_ok {
            return Some(abs_idx);
        }
        pos = abs_idx + 1;
    }
    None
}

/// Parse INSERTED.col1, INSERTED.col2, ... and extract column names as "col1, col2, ..."
fn parse_inserted_columns(output_str: &str) -> String {
    let mut cols = Vec::new();

    for part in output_str.split(',') {
        let trimmed = part.trim();

        // Try to extract column name after INSERTED.
        if let Some(after_inserted) = trimmed
            .strip_prefix("INSERTED.")
            .or_else(|| trimmed.strip_prefix("inserted."))
            .or_else(|| trimmed.strip_prefix("INSERTED"))
            .or_else(|| trimmed.strip_prefix("inserted"))
        {
            let col_name = after_inserted.trim().to_string();
            if !col_name.is_empty() {
                cols.push(col_name);
            }
        }
    }

    cols.join(", ")
}

/// Strip the `INTO :N, :N, ...` suffix from an Oracle `RETURNING ... INTO` clause.
fn strip_returning_into(sql: &str) -> String {
    // Case-insensitive search for "INTO" after "RETURNING" at the end of the statement
    let upper = sql.to_uppercase();
    if let Some(ret_pos) = upper.rfind("RETURNING") {
        let after_returning = &upper[ret_pos + "RETURNING".len()..];
        if let Some(into_offset) = after_returning.find("INTO") {
            let into_pos = ret_pos + "RETURNING".len() + into_offset;
            // Keep everything before INTO, trim trailing whitespace/semicolons
            let trimmed = sql[..into_pos].trim_end();
            return trimmed.to_string();
        }
    }
    sql.to_string()
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

    #[test]
    fn test_preprocess_postgres_strips_partial_index_where() {
        let sql = "INSERT INTO billing_events (project_id, stripe_event_id) \
                   VALUES ($1, $2) \
                   ON CONFLICT (stripe_event_id) WHERE stripe_event_id IS NOT NULL DO NOTHING";
        let cleaned = preprocess_postgres_sql(sql);
        assert!(
            !cleaned
                .to_uppercase()
                .contains("WHERE STRIPE_EVENT_ID IS NOT NULL"),
            "WHERE clause must be stripped between ON CONFLICT cols and DO; got: {cleaned}"
        );
        assert!(
            cleaned
                .to_uppercase()
                .contains("ON CONFLICT (STRIPE_EVENT_ID) DO NOTHING")
        );
        // sqlparser must accept the cleaned form.
        sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::PostgreSqlDialect {}, &cleaned)
            .expect("cleaned SQL should parse");
    }

    #[test]
    fn test_preprocess_postgres_no_op_when_no_partial_clause() {
        let sql = "INSERT INTO t (a) VALUES ($1) ON CONFLICT (a) DO UPDATE SET a = EXCLUDED.a";
        assert_eq!(preprocess_postgres_sql(sql), sql);
    }

    #[test]
    fn test_preprocess_postgres_leaves_on_conflict_on_constraint_alone() {
        let sql = "INSERT INTO t (a) VALUES ($1) ON CONFLICT ON CONSTRAINT t_a_uidx DO NOTHING";
        assert_eq!(preprocess_postgres_sql(sql), sql);
    }

    #[test]
    fn test_preprocess_postgres_handles_compound_index_cols() {
        let sql = "INSERT INTO t (a, b) VALUES ($1, $2) \
                   ON CONFLICT (a, b) WHERE a IS NOT NULL AND b > 0 DO UPDATE SET b = EXCLUDED.b";
        let cleaned = preprocess_postgres_sql(sql);
        assert!(
            cleaned
                .to_uppercase()
                .contains("ON CONFLICT (A, B) DO UPDATE")
        );
        assert!(!cleaned.to_uppercase().contains("WHERE A IS NOT NULL"));
    }

    #[test]
    fn test_preprocess_postgres_preserves_unrelated_where() {
        // The DELETE's WHERE is its own clause, not an ON-CONFLICT predicate;
        // it must survive untouched.
        let sql = "DELETE FROM t WHERE id = $1";
        assert_eq!(preprocess_postgres_sql(sql), sql);
    }

    #[test]
    fn test_preprocess_postgres_ignores_text_inside_line_comments() {
        // Earlier scans treated this as a real `ON CONFLICT (col) WHERE … DO`
        // and excised the entire comment + INSERT body up to the next `DO`.
        // Comments must be opaque to the predicate-stripping pass.
        let sql = "-- inline doc: `ON CONFLICT (col) WHERE …` is the partial form\n\
                   INSERT INTO t (a) VALUES ($1) \
                   ON CONFLICT (a) WHERE a IS NOT NULL DO NOTHING";
        let cleaned = preprocess_postgres_sql(sql);
        assert!(
            cleaned.contains("-- inline doc"),
            "comment must survive the pass; got: {cleaned}"
        );
        assert!(cleaned.contains("ON CONFLICT (a) DO NOTHING"));
    }

    #[test]
    fn test_preprocess_postgres_ignores_text_inside_string_literals() {
        let sql = "SELECT 'ON CONFLICT (a) WHERE a IS NOT NULL DO NOTHING' AS s";
        assert_eq!(preprocess_postgres_sql(sql), sql);
    }

    #[test]
    fn test_preprocess_oracle_colon_placeholders() {
        assert_eq!(
            preprocess_oracle_sql("SELECT * FROM users WHERE id = :1"),
            "SELECT * FROM users WHERE id = ?"
        );
        assert_eq!(
            preprocess_oracle_sql("INSERT INTO users (name, email) VALUES (:1, :2)"),
            "INSERT INTO users (name, email) VALUES (?, ?)"
        );
    }

    #[test]
    fn test_preprocess_oracle_preserves_string_literals() {
        assert_eq!(
            preprocess_oracle_sql("SELECT * FROM users WHERE name = ':1' AND id = :1"),
            "SELECT * FROM users WHERE name = ':1' AND id = ?"
        );
    }

    #[test]
    fn test_preprocess_oracle_strips_returning_into() {
        assert_eq!(
            preprocess_oracle_sql(
                "INSERT INTO users (name) VALUES (:1) RETURNING id, name INTO :2, :3"
            ),
            "INSERT INTO users (name) VALUES (?) RETURNING id, name"
        );
    }

    #[test]
    fn test_preprocess_oracle_full_insert_returning_into() {
        let sql = "INSERT INTO users (name, email, active) VALUES (:1, :2, :3) RETURNING id, name, email, active, created_at INTO :4, :5, :6, :7, :8";
        let result = preprocess_oracle_sql(sql);
        assert_eq!(
            result,
            "INSERT INTO users (name, email, active) VALUES (?, ?, ?) RETURNING id, name, email, active, created_at"
        );
    }

    #[test]
    fn test_preprocess_oracle_no_returning_into_unchanged() {
        assert_eq!(
            preprocess_oracle_sql("DELETE FROM users WHERE id = :1"),
            "DELETE FROM users WHERE id = ?"
        );
    }

    #[test]
    fn test_preprocess_mssql_single_placeholder() {
        assert_eq!(
            preprocess_mssql_sql("SELECT * FROM users WHERE id = @p1"),
            "SELECT * FROM users WHERE id = ?"
        );
    }

    #[test]
    fn test_preprocess_mssql_multiple_placeholders() {
        assert_eq!(
            preprocess_mssql_sql("INSERT INTO users (name, email) VALUES (@p1, @p2)"),
            "INSERT INTO users (name, email) VALUES (?, ?)"
        );
    }

    #[test]
    fn test_preprocess_mssql_preserves_string_literals() {
        assert_eq!(
            preprocess_mssql_sql("SELECT * FROM users WHERE name = '@p1' AND id = @p1"),
            "SELECT * FROM users WHERE name = '@p1' AND id = ?"
        );
    }

    #[test]
    fn test_preprocess_mssql_case_insensitive_p() {
        assert_eq!(
            preprocess_mssql_sql("SELECT * FROM users WHERE id = @P1"),
            "SELECT * FROM users WHERE id = ?"
        );
    }

    #[test]
    fn test_preprocess_mssql_non_placeholder_at_variable_unchanged() {
        // @variable (not @pN pattern) must not be touched
        assert_eq!(preprocess_mssql_sql("SELECT @myvar"), "SELECT @myvar");
    }

    #[test]
    fn test_preprocess_mssql_multi_digit_placeholder() {
        assert_eq!(preprocess_mssql_sql("SELECT @p10, @p2"), "SELECT ?, ?");
    }

    #[test]
    fn test_preprocess_mssql_output_inserted_simple() {
        let sql =
            "INSERT INTO users (id, name) OUTPUT INSERTED.id, INSERTED.name VALUES (@p1, @p2)";
        let result = preprocess_mssql_sql(sql);
        // Should convert OUTPUT INSERTED.col to RETURNING col and @pN to ?
        assert!(result.contains("RETURNING id, name"), "got: {}", result);
        assert!(result.contains("VALUES (?, ?)"), "got: {}", result);
        assert!(!result.contains("OUTPUT"), "got: {}", result);
    }

    #[test]
    fn test_preprocess_mssql_output_inserted_full_example() {
        let sql = "INSERT INTO users (id, name, email, active) OUTPUT INSERTED.id, INSERTED.name, INSERTED.email, INSERTED.active, INSERTED.created_at VALUES (@p1, @p2, @p3, @p4)";
        let result = preprocess_mssql_sql(sql);
        assert!(
            result.contains("RETURNING id, name, email, active, created_at"),
            "got: {}",
            result
        );
        assert!(result.contains("VALUES (?, ?, ?, ?)"), "got: {}", result);
    }

    #[test]
    fn test_preprocess_mssql_output_case_insensitive() {
        let sql = "INSERT INTO users (id) output inserted.id values (@p1)";
        let result = preprocess_mssql_sql(sql);
        assert!(result.contains("RETURNING id"), "got: {}", result);
        // The original lowercase "values" is preserved, then @p1 becomes ?
        assert!(
            result.contains("values (?)") || result.contains("VALUES (?)"),
            "got: {}",
            result
        );
    }

    #[test]
    fn test_preprocess_mssql_no_output_unchanged() {
        let sql = "INSERT INTO users (id, name) VALUES (@p1, @p2)";
        let result = preprocess_mssql_sql(sql);
        assert_eq!(result, "INSERT INTO users (id, name) VALUES (?, ?)");
    }

    #[test]
    fn test_preprocess_mssql_output_with_string_literal() {
        // @p1 inside a string should be preserved by placeholder conversion
        let sql =
            "INSERT INTO users (id, name) OUTPUT INSERTED.id, INSERTED.name VALUES (@p1, '@p2')";
        let result = preprocess_mssql_sql(sql);
        assert!(result.contains("RETURNING id, name"), "got: {}", result);
        assert!(result.contains("(?, '@p2')"), "got: {}", result);
    }

    #[test]
    fn test_preprocess_mssql_output_with_whitespace() {
        let sql =
            "INSERT INTO users (id, name)\nOUTPUT INSERTED.id,\n  INSERTED.name\nVALUES (@p1, @p2)";
        let result = preprocess_mssql_sql(sql);
        assert!(result.contains("RETURNING id, name"), "got: {}", result);
        assert!(result.contains("VALUES (?, ?)"), "got: {}", result);
    }
}
