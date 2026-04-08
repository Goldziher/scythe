pub mod csharp_microsoft_sqlite;
pub mod csharp_mysqlconnector;
pub mod csharp_npgsql;
pub mod elixir_ecto;
pub mod elixir_exqlite;
pub mod elixir_myxql;
pub mod elixir_postgrex;
pub mod go_database_sql;
pub mod go_pgx;
pub mod java_jdbc;
pub mod kotlin_jdbc;
pub mod php_amphp;
pub mod php_pdo;
pub mod python_aiomysql;
pub mod python_aiosqlite;
pub mod python_asyncpg;
pub mod python_common;
pub mod python_psycopg3;
pub mod ruby_mysql2;
pub mod ruby_pg;
pub mod ruby_sqlite3;
pub mod ruby_trilogy;
pub mod sqlx;
pub mod tokio_postgres;
pub mod typescript_better_sqlite3;
pub mod typescript_common;
pub mod typescript_mysql2;
pub mod typescript_pg;
pub mod typescript_postgres;

use scythe_core::analyzer::AnalyzedParam;
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::CodegenBackend;

/// Strip SQL comments, trailing semicolons, and excess whitespace.
/// Preserves newlines between lines.
pub(crate) fn clean_sql(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Like clean_sql but joins lines with spaces (for languages that embed SQL inline).
pub(crate) fn clean_sql_oneline(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Rewrite SQL for optional parameters.
///
/// For each optional param, finds `column = $N` (or `column <> $N`, `column != $N`)
/// and rewrites to `($N IS NULL OR column = $N)`. This allows callers to pass NULL
/// to skip a filter condition at runtime.
///
/// This operates on the raw SQL before any backend-specific placeholder rewriting.
pub(crate) fn rewrite_optional_params(
    sql: &str,
    optional_params: &[String],
    params: &[AnalyzedParam],
) -> String {
    if optional_params.is_empty() {
        return sql.to_string();
    }

    let mut result = sql.to_string();

    for opt_name in optional_params {
        let Some(param) = params.iter().find(|p| p.name == *opt_name) else {
            continue;
        };
        let placeholder = format!("${}", param.position);

        // Try each comparison operator
        for op in &["=", "<>", "!="] {
            result = rewrite_comparison(&result, &placeholder, op);
        }
    }

    result
}

/// Rewrite a single `column <op> $N` pattern to `($N IS NULL OR column <op> $N)`.
/// Handles both `column <op> $N` and `$N <op> column` orderings.
fn rewrite_comparison(sql: &str, placeholder: &str, op: &str) -> String {
    let mut result = String::with_capacity(sql.len() + 32);
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Try to match `identifier <op> $N` at this position
        if let Some((start, col, end)) = try_match_col_op_ph(&chars, i, op, placeholder) {
            // Write everything before the match start
            if start > i {
                // This shouldn't happen since we iterate char by char
            }
            result.push_str(&format!(
                "({placeholder} IS NULL OR {col} {op} {placeholder})"
            ));
            i = end;
            continue;
        }

        // Try to match `$N <op> identifier` at this position
        if let Some((end, col)) = try_match_ph_op_col(&chars, i, op, placeholder) {
            result.push_str(&format!(
                "({placeholder} IS NULL OR {col} {op} {placeholder})"
            ));
            i = end;
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Try to match `identifier <ws>* <op> <ws>* placeholder` starting at position `i`.
/// Returns `(match_start, column_name, match_end)` if found.
fn try_match_col_op_ph(
    chars: &[char],
    i: usize,
    op: &str,
    placeholder: &str,
) -> Option<(usize, String, usize)> {
    // Must start with an identifier character (word char)
    if !is_ident_char(chars[i]) {
        return None;
    }
    // Must not be preceded by another ident char (whole-word boundary)
    if i > 0 && is_ident_char(chars[i - 1]) {
        return None;
    }

    // Read the identifier
    let ident_start = i;
    let mut j = i;
    while j < chars.len() && is_ident_char(chars[j]) {
        j += 1;
    }
    let ident: String = chars[ident_start..j].iter().collect();

    // Skip whitespace
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }

    // Match operator
    let op_chars: Vec<char> = op.chars().collect();
    if j + op_chars.len() > chars.len() {
        return None;
    }
    for (k, oc) in op_chars.iter().enumerate() {
        if chars[j + k] != *oc {
            return None;
        }
    }
    j += op_chars.len();

    // Skip whitespace
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }

    // Match placeholder
    let ph_chars: Vec<char> = placeholder.chars().collect();
    if j + ph_chars.len() > chars.len() {
        return None;
    }
    for (k, pc) in ph_chars.iter().enumerate() {
        if chars[j + k] != *pc {
            return None;
        }
    }
    j += ph_chars.len();

    // Ensure placeholder is not followed by a digit (e.g., $1 vs $10)
    if j < chars.len() && chars[j].is_ascii_digit() {
        return None;
    }

    Some((i, ident, j))
}

/// Try to match `placeholder <ws>* <op> <ws>* identifier` starting at position `i`.
/// Returns `(match_end, column_name)` if found.
fn try_match_ph_op_col(
    chars: &[char],
    i: usize,
    op: &str,
    placeholder: &str,
) -> Option<(usize, String)> {
    let ph_chars: Vec<char> = placeholder.chars().collect();
    if i + ph_chars.len() > chars.len() {
        return None;
    }

    // Must not be preceded by $ or digit (boundary check)
    if i > 0 && (chars[i - 1] == '$' || chars[i - 1].is_ascii_digit()) {
        return None;
    }

    // Match placeholder
    for (k, pc) in ph_chars.iter().enumerate() {
        if chars[i + k] != *pc {
            return None;
        }
    }
    let mut j = i + ph_chars.len();

    // Ensure placeholder is not followed by a digit
    if j < chars.len() && chars[j].is_ascii_digit() {
        return None;
    }

    // Skip whitespace
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }

    // Match operator
    let op_chars: Vec<char> = op.chars().collect();
    if j + op_chars.len() > chars.len() {
        return None;
    }
    for (k, oc) in op_chars.iter().enumerate() {
        if chars[j + k] != *oc {
            return None;
        }
    }
    j += op_chars.len();

    // Skip whitespace
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }

    // Read the identifier
    if j >= chars.len() || !is_ident_char(chars[j]) {
        return None;
    }
    let ident_start = j;
    while j < chars.len() && is_ident_char(chars[j]) {
        j += 1;
    }
    let ident: String = chars[ident_start..j].iter().collect();

    // Avoid matching "NULL" (from already-rewritten text)
    if ident == "NULL" {
        return None;
    }

    Some((j, ident))
}

/// Clean SQL and apply optional parameter rewriting.
pub(crate) fn clean_sql_with_optional(
    sql: &str,
    optional_params: &[String],
    params: &[AnalyzedParam],
) -> String {
    let cleaned = clean_sql(sql);
    rewrite_optional_params(&cleaned, optional_params, params)
}

/// Clean SQL (oneline) and apply optional parameter rewriting.
pub(crate) fn clean_sql_oneline_with_optional(
    sql: &str,
    optional_params: &[String],
    params: &[AnalyzedParam],
) -> String {
    let cleaned = clean_sql_oneline(sql);
    rewrite_optional_params(&cleaned, optional_params, params)
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Get a backend by name and database engine.
///
/// The `engine` parameter (e.g., "postgresql", "mysql", "sqlite") determines
/// which manifest is loaded for type mappings. PG-only backends reject non-PG engines.
pub fn get_backend(name: &str, engine: &str) -> Result<Box<dyn CodegenBackend>, ScytheError> {
    let backend: Box<dyn CodegenBackend> = match name {
        "rust-sqlx" | "sqlx" | "rust" => Box::new(sqlx::SqlxBackend::new(engine)?),
        "rust-tokio-postgres" | "tokio-postgres" => {
            Box::new(tokio_postgres::TokioPostgresBackend::new(engine)?)
        }
        "python-psycopg3" | "python" => {
            Box::new(python_psycopg3::PythonPsycopg3Backend::new(engine)?)
        }
        "python-asyncpg" => Box::new(python_asyncpg::PythonAsyncpgBackend::new(engine)?),
        "python-aiomysql" => Box::new(python_aiomysql::PythonAiomysqlBackend::new(engine)?),
        "python-aiosqlite" => Box::new(python_aiosqlite::PythonAiosqliteBackend::new(engine)?),
        "typescript-postgres" | "ts" | "typescript" => {
            Box::new(typescript_postgres::TypescriptPostgresBackend::new(engine)?)
        }
        "typescript-pg" => Box::new(typescript_pg::TypescriptPgBackend::new(engine)?),
        "typescript-mysql2" => Box::new(typescript_mysql2::TypescriptMysql2Backend::new(engine)?),
        "typescript-better-sqlite3" => {
            Box::new(typescript_better_sqlite3::TypescriptBetterSqlite3Backend::new(engine)?)
        }
        "go-database-sql" => Box::new(go_database_sql::GoDatabaseSqlBackend::new(engine)?),
        "go-pgx" | "go" => Box::new(go_pgx::GoPgxBackend::new(engine)?),
        "java-jdbc" | "java" => Box::new(java_jdbc::JavaJdbcBackend::new(engine)?),
        "kotlin-jdbc" | "kotlin" | "kt" => Box::new(kotlin_jdbc::KotlinJdbcBackend::new(engine)?),
        "csharp-npgsql" | "csharp" | "c#" | "dotnet" => {
            Box::new(csharp_npgsql::CsharpNpgsqlBackend::new(engine)?)
        }
        "csharp-mysqlconnector" => Box::new(
            csharp_mysqlconnector::CsharpMysqlConnectorBackend::new(engine)?,
        ),
        "csharp-microsoft-sqlite" => Box::new(
            csharp_microsoft_sqlite::CsharpMicrosoftSqliteBackend::new(engine)?,
        ),
        "elixir-postgrex" | "elixir" | "ex" => {
            Box::new(elixir_postgrex::ElixirPostgrexBackend::new(engine)?)
        }
        "elixir-ecto" | "ecto" => Box::new(elixir_ecto::ElixirEctoBackend::new(engine)?),
        "elixir-myxql" => Box::new(elixir_myxql::ElixirMyxqlBackend::new(engine)?),
        "elixir-exqlite" => Box::new(elixir_exqlite::ElixirExqliteBackend::new(engine)?),
        "ruby-pg" | "ruby" | "rb" => Box::new(ruby_pg::RubyPgBackend::new(engine)?),
        "ruby-mysql2" => Box::new(ruby_mysql2::RubyMysql2Backend::new(engine)?),
        "ruby-sqlite3" => Box::new(ruby_sqlite3::RubySqlite3Backend::new(engine)?),
        "ruby-trilogy" | "trilogy" => Box::new(ruby_trilogy::RubyTrilogyBackend::new(engine)?),
        "php-pdo" | "php" => Box::new(php_pdo::PhpPdoBackend::new(engine)?),
        "php-amphp" | "amphp" => Box::new(php_amphp::PhpAmphpBackend::new(engine)?),
        _ => {
            return Err(ScytheError::new(
                ErrorCode::InternalError,
                format!("unknown backend: {}", name),
            ));
        }
    };

    // Validate engine is supported by this backend
    let normalized_engine = normalize_engine(engine);
    if !backend
        .supported_engines()
        .iter()
        .any(|e| normalize_engine(e) == normalized_engine)
    {
        return Err(ScytheError::new(
            ErrorCode::InternalError,
            format!(
                "backend '{}' does not support engine '{}'. Supported: {:?}",
                name,
                engine,
                backend.supported_engines()
            ),
        ));
    }

    Ok(backend)
}

/// Normalize engine name to canonical form.
fn normalize_engine(engine: &str) -> &str {
    match engine {
        "postgresql" | "postgres" | "pg" => "postgresql",
        "mysql" | "mariadb" => "mysql",
        "sqlite" | "sqlite3" => "sqlite",
        other => other,
    }
}
