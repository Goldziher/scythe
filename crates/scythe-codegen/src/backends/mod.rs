pub(crate) mod csharp_microsoft_sqlite;
pub(crate) mod csharp_mysqlconnector;
pub(crate) mod csharp_npgsql;
pub(crate) mod csharp_oracle;
pub(crate) mod csharp_snowflake;
pub(crate) mod csharp_sqlclient;
pub(crate) mod elixir_ecto;
pub(crate) mod elixir_exqlite;
pub(crate) mod elixir_jamdb;
pub(crate) mod elixir_myxql;
pub(crate) mod elixir_postgrex;
pub(crate) mod elixir_tds;
pub(crate) mod go_database_sql;
pub(crate) mod go_godror;
pub(crate) mod go_gosnowflake;
pub(crate) mod go_pgx;
pub(crate) mod java_jdbc;
pub(crate) mod java_r2dbc;
pub(crate) mod kotlin_exposed;
pub(crate) mod kotlin_jdbc;
pub(crate) mod kotlin_r2dbc;
pub(crate) mod php_amphp;
pub(crate) mod php_pdo;
pub(crate) mod python_aiomysql;
pub(crate) mod python_aiosqlite;
pub(crate) mod python_asyncpg;
pub(crate) mod python_common;
pub(crate) mod python_duckdb;
pub(crate) mod python_oracledb;
pub(crate) mod python_psycopg3;
pub(crate) mod python_pyodbc;
pub(crate) mod python_snowflake;
pub(crate) mod ruby_mysql2;
pub(crate) mod ruby_oci8;
pub(crate) mod ruby_pg;
pub(crate) mod ruby_rbs;
pub(crate) mod ruby_sqlite3;
pub(crate) mod ruby_tiny_tds;
pub(crate) mod ruby_trilogy;
pub(crate) mod rust_sibyl;
pub(crate) mod rust_tiberius;
pub(crate) mod sqlx;
pub(crate) mod tokio_postgres;
pub(crate) mod typescript_better_sqlite3;
pub(crate) mod typescript_common;
pub(crate) mod typescript_duckdb;
pub(crate) mod typescript_mssql;
pub(crate) mod typescript_mysql2;
pub(crate) mod typescript_oracledb;
pub(crate) mod typescript_pg;
pub(crate) mod typescript_postgres;
pub(crate) mod typescript_snowflake;

use scythe_backend::manifest::BackendManifest;
use scythe_core::analyzer::AnalyzedParam;
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::CodegenBackend;

/// Load a backend manifest, preferring a user-provided file at `override_path`
/// and falling back to the embedded `default_toml` string.
pub(crate) fn load_or_default_manifest(
    override_path: &str,
    default_toml: &str,
) -> Result<BackendManifest, ScytheError> {
    let path = std::path::Path::new(override_path);
    if path.exists() {
        scythe_backend::manifest::load_manifest(path)
            .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))
    } else {
        toml::from_str(default_toml)
            .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))
    }
}

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
        for op in &[
            ">=", "<=", "<>", "!=", ">", "<", "=", "ILIKE", "ilike", "LIKE", "like",
        ] {
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
        if let Some((_start, col, end)) = try_match_col_op_ph(&chars, i, op, placeholder) {
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
    c.is_alphanumeric() || c == '_' || c == '.'
}

/// Rewrite PostgreSQL `$1, $2, ...` positional placeholders to a target format.
/// Skips placeholders inside single-quoted SQL string literals.
/// The `formatter` closure receives the parameter number and returns the replacement string.
pub(crate) fn rewrite_pg_placeholders(sql: &str, formatter: impl Fn(u32) -> String) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
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
        } else if ch == '$' {
            if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                let mut num_str = String::new();
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    num_str.push(chars.next().unwrap());
                }
                let num: u32 = num_str.parse().unwrap_or(0);
                result.push_str(&formatter(num));
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Get a backend by name and database engine.
///
/// The `engine` parameter (e.g., "postgresql", "mysql", "sqlite") determines
/// which manifest is loaded for type mappings. PG-only backends reject non-PG engines.
pub fn get_backend(name: &str, engine: &str) -> Result<Box<dyn CodegenBackend>, ScytheError> {
    // Normalize engine aliases (e.g., "cockroachdb" -> "postgresql") before
    // passing to backend constructors so each backend only needs to match
    // canonical engine names.
    let canonical_engine = normalize_engine(engine);
    let backend: Box<dyn CodegenBackend> = match name {
        "rust-sqlx" | "sqlx" | "rust" => Box::new(sqlx::SqlxBackend::new(canonical_engine)?),
        "rust-tokio-postgres" | "tokio-postgres" => {
            Box::new(tokio_postgres::TokioPostgresBackend::new(canonical_engine)?)
        }
        "python-psycopg3" | "python" => Box::new(python_psycopg3::PythonPsycopg3Backend::new(
            canonical_engine,
        )?),
        "python-asyncpg" => Box::new(python_asyncpg::PythonAsyncpgBackend::new(canonical_engine)?),
        "python-aiomysql" => Box::new(python_aiomysql::PythonAiomysqlBackend::new(
            canonical_engine,
        )?),
        "python-aiosqlite" => Box::new(python_aiosqlite::PythonAiosqliteBackend::new(
            canonical_engine,
        )?),
        "python-duckdb" => Box::new(python_duckdb::PythonDuckdbBackend::new(canonical_engine)?),
        "typescript-postgres" | "ts" | "typescript" => Box::new(
            typescript_postgres::TypescriptPostgresBackend::new(canonical_engine)?,
        ),
        "typescript-pg" => Box::new(typescript_pg::TypescriptPgBackend::new(canonical_engine)?),
        "typescript-mysql2" => Box::new(typescript_mysql2::TypescriptMysql2Backend::new(
            canonical_engine,
        )?),
        "typescript-better-sqlite3" => Box::new(
            typescript_better_sqlite3::TypescriptBetterSqlite3Backend::new(canonical_engine)?,
        ),
        "typescript-duckdb" => Box::new(typescript_duckdb::TypescriptDuckdbBackend::new(
            canonical_engine,
        )?),
        "go-database-sql" => Box::new(go_database_sql::GoDatabaseSqlBackend::new(
            canonical_engine,
        )?),
        "go-pgx" | "go" => Box::new(go_pgx::GoPgxBackend::new(canonical_engine)?),
        "java-jdbc" | "java" => Box::new(java_jdbc::JavaJdbcBackend::new(canonical_engine)?),
        "java-r2dbc" | "r2dbc-java" => {
            Box::new(java_r2dbc::JavaR2dbcBackend::new(canonical_engine)?)
        }
        "kotlin-exposed" | "exposed" => {
            Box::new(kotlin_exposed::KotlinExposedBackend::new(canonical_engine)?)
        }
        "kotlin-jdbc" | "kotlin" | "kt" => {
            Box::new(kotlin_jdbc::KotlinJdbcBackend::new(canonical_engine)?)
        }
        "kotlin-r2dbc" | "r2dbc-kotlin" => {
            Box::new(kotlin_r2dbc::KotlinR2dbcBackend::new(canonical_engine)?)
        }
        "csharp-npgsql" | "csharp" | "c#" | "dotnet" => {
            Box::new(csharp_npgsql::CsharpNpgsqlBackend::new(canonical_engine)?)
        }
        "csharp-mysqlconnector" => Box::new(
            csharp_mysqlconnector::CsharpMysqlConnectorBackend::new(canonical_engine)?,
        ),
        "csharp-microsoft-sqlite" => Box::new(
            csharp_microsoft_sqlite::CsharpMicrosoftSqliteBackend::new(canonical_engine)?,
        ),
        "elixir-postgrex" | "elixir" | "ex" => Box::new(
            elixir_postgrex::ElixirPostgrexBackend::new(canonical_engine)?,
        ),
        "elixir-ecto" | "ecto" => Box::new(elixir_ecto::ElixirEctoBackend::new(canonical_engine)?),
        "elixir-myxql" => Box::new(elixir_myxql::ElixirMyxqlBackend::new(canonical_engine)?),
        "elixir-exqlite" => Box::new(elixir_exqlite::ElixirExqliteBackend::new(canonical_engine)?),
        "ruby-pg" | "ruby" | "rb" => Box::new(ruby_pg::RubyPgBackend::new(canonical_engine)?),
        "ruby-mysql2" => Box::new(ruby_mysql2::RubyMysql2Backend::new(canonical_engine)?),
        "ruby-sqlite3" => Box::new(ruby_sqlite3::RubySqlite3Backend::new(canonical_engine)?),
        "ruby-trilogy" | "trilogy" => {
            Box::new(ruby_trilogy::RubyTrilogyBackend::new(canonical_engine)?)
        }
        "php-pdo" | "php" => Box::new(php_pdo::PhpPdoBackend::new(canonical_engine)?),
        "php-amphp" | "amphp" => Box::new(php_amphp::PhpAmphpBackend::new(canonical_engine)?),
        // MSSQL backends
        "rust-tiberius" | "tiberius" => {
            Box::new(rust_tiberius::RustTiberiusBackend::new(canonical_engine)?)
        }
        "python-pyodbc" | "pyodbc" => {
            Box::new(python_pyodbc::PythonPyodbcBackend::new(canonical_engine)?)
        }
        "typescript-mssql" | "tedious" => Box::new(typescript_mssql::TypescriptMssqlBackend::new(
            canonical_engine,
        )?),
        "csharp-sqlclient" => Box::new(csharp_sqlclient::CsharpSqlClientBackend::new(
            canonical_engine,
        )?),
        "ruby-tiny-tds" | "tiny-tds" | "tiny_tds" => {
            Box::new(ruby_tiny_tds::RubyTinyTdsBackend::new(canonical_engine)?)
        }
        "elixir-tds" | "tds" => Box::new(elixir_tds::ElixirTdsBackend::new(canonical_engine)?),
        // Oracle backends
        "rust-sibyl" | "sibyl" => Box::new(rust_sibyl::RustSibylBackend::new(canonical_engine)?),
        "python-oracledb" | "oracledb" => Box::new(python_oracledb::PythonOracledbBackend::new(
            canonical_engine,
        )?),
        "typescript-oracledb" => Box::new(typescript_oracledb::TypescriptOracledbBackend::new(
            canonical_engine,
        )?),
        "go-godror" | "godror" => Box::new(go_godror::GoGodrorBackend::new(canonical_engine)?),
        "csharp-oracle" => Box::new(csharp_oracle::CsharpOracleBackend::new(canonical_engine)?),
        "ruby-oci8" | "oci8" => Box::new(ruby_oci8::RubyOci8Backend::new(canonical_engine)?),
        "elixir-jamdb" | "jamdb" => {
            Box::new(elixir_jamdb::ElixirJamdbBackend::new(canonical_engine)?)
        }
        // Snowflake backends
        "python-snowflake" => Box::new(python_snowflake::PythonSnowflakeBackend::new(
            canonical_engine,
        )?),
        "typescript-snowflake" => Box::new(typescript_snowflake::TypescriptSnowflakeBackend::new(
            canonical_engine,
        )?),
        "go-gosnowflake" | "gosnowflake" => {
            Box::new(go_gosnowflake::GoGosnowflakeBackend::new(canonical_engine)?)
        }
        "csharp-snowflake" => Box::new(csharp_snowflake::CsharpSnowflakeBackend::new(
            canonical_engine,
        )?),
        _ => {
            return Err(ScytheError::new(
                ErrorCode::InternalError,
                format!("unknown backend: {}", name),
            ));
        }
    };

    // Validate engine is supported by this backend
    if !backend
        .supported_engines()
        .iter()
        .any(|e| normalize_engine(e) == canonical_engine)
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
        "postgresql" | "postgres" | "pg" | "cockroachdb" | "crdb" => "postgresql",
        "mysql" => "mysql",
        "mariadb" => "mariadb",
        "sqlite" | "sqlite3" => "sqlite",
        "duckdb" => "duckdb",
        "mssql" | "sqlserver" | "tsql" => "mssql",
        "oracle" => "oracle",
        "snowflake" => "snowflake",
        "redshift" => "redshift",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, position: i64) -> AnalyzedParam {
        AnalyzedParam {
            name: name.to_string(),
            neutral_type: "string".to_string(),
            nullable: true,
            position,
        }
    }

    #[test]
    fn test_normalize_engine_cockroachdb() {
        assert_eq!(normalize_engine("cockroachdb"), "postgresql");
        assert_eq!(normalize_engine("crdb"), "postgresql");
    }

    #[test]
    fn test_get_backend_cockroachdb_with_pg_backends() {
        // CockroachDB should work with all PostgreSQL-compatible backends
        let pg_backends = [
            "rust-sqlx",
            "rust-tokio-postgres",
            "python-psycopg3",
            "python-asyncpg",
            "typescript-postgres",
            "typescript-pg",
            "go-pgx",
            "ruby-pg",
            "elixir-postgrex",
            "csharp-npgsql",
            "php-pdo",
            "php-amphp",
        ];
        for backend_name in &pg_backends {
            let result = get_backend(backend_name, "cockroachdb");
            assert!(
                result.is_ok(),
                "backend '{}' should accept cockroachdb engine, got: {:?}",
                backend_name,
                result.err()
            );
        }
    }

    #[test]
    fn test_get_backend_crdb_alias() {
        let result = get_backend("rust-sqlx", "crdb");
        assert!(
            result.is_ok(),
            "rust-sqlx should accept 'crdb' engine alias"
        );
    }

    #[test]
    fn test_normalize_engine_duckdb() {
        assert_eq!(normalize_engine("duckdb"), "duckdb");
    }

    #[test]
    fn test_get_backend_duckdb_with_compatible_backends() {
        let duckdb_backends = [
            "python-duckdb",
            "typescript-duckdb",
            "go-database-sql",
            "java-jdbc",
            "kotlin-jdbc",
        ];
        for backend_name in &duckdb_backends {
            let result = get_backend(backend_name, "duckdb");
            assert!(
                result.is_ok(),
                "backend '{}' should accept duckdb engine, got: {:?}",
                backend_name,
                result.err()
            );
        }
    }

    #[test]
    fn test_get_backend_duckdb_rejected_by_pg_only() {
        let result = get_backend("rust-sqlx", "duckdb");
        assert!(result.is_err(), "rust-sqlx should reject duckdb engine");
    }

    #[test]
    fn test_rewrite_simple_equality() {
        let sql = "SELECT * FROM users WHERE status = $1";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users WHERE ($1 IS NULL OR status = $1)"
        );
    }

    #[test]
    fn test_rewrite_qualified_column() {
        let sql = "SELECT * FROM users u WHERE u.status = $1";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users u WHERE ($1 IS NULL OR u.status = $1)"
        );
    }

    #[test]
    fn test_rewrite_multiple_optional() {
        let sql = "SELECT * FROM users WHERE status = $1 AND name = $2";
        let params = vec![param("status", 1), param("name", 2)];
        let result =
            rewrite_optional_params(sql, &["status".to_string(), "name".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users WHERE ($1 IS NULL OR status = $1) AND ($2 IS NULL OR name = $2)"
        );
    }

    #[test]
    fn test_rewrite_mixed_optional_required() {
        let sql = "SELECT * FROM users WHERE id = $1 AND status = $2";
        let params = vec![param("id", 1), param("status", 2)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users WHERE id = $1 AND ($2 IS NULL OR status = $2)"
        );
    }

    #[test]
    fn test_rewrite_like_operator() {
        let sql = "SELECT * FROM users WHERE name LIKE $1";
        let params = vec![param("name", 1)];
        let result = rewrite_optional_params(sql, &["name".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users WHERE ($1 IS NULL OR name LIKE $1)"
        );
    }

    #[test]
    fn test_rewrite_ilike_operator() {
        let sql = "SELECT * FROM users WHERE name ILIKE $1";
        let params = vec![param("name", 1)];
        let result = rewrite_optional_params(sql, &["name".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users WHERE ($1 IS NULL OR name ILIKE $1)"
        );
    }

    #[test]
    fn test_rewrite_comparison_operators() {
        let sql = "SELECT * FROM users WHERE age >= $1";
        let params = vec![param("age", 1)];
        let result = rewrite_optional_params(sql, &["age".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users WHERE ($1 IS NULL OR age >= $1)"
        );
    }

    #[test]
    fn test_rewrite_less_than() {
        let sql = "SELECT * FROM users WHERE age < $1";
        let params = vec![param("age", 1)];
        let result = rewrite_optional_params(sql, &["age".to_string()], &params);
        assert_eq!(result, "SELECT * FROM users WHERE ($1 IS NULL OR age < $1)");
    }

    #[test]
    fn test_no_rewrite_without_optional() {
        let sql = "SELECT * FROM users WHERE status = $1";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &[], &params);
        assert_eq!(result, sql);
    }

    #[test]
    fn test_rewrite_not_equal() {
        let sql = "SELECT * FROM users WHERE status <> $1";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        assert_eq!(
            result,
            "SELECT * FROM users WHERE ($1 IS NULL OR status <> $1)"
        );
    }

    #[test]
    fn test_rewrite_does_not_match_similar_placeholder() {
        // $1 should not match $10
        let sql = "SELECT * FROM users WHERE status = $10";
        let params = vec![param("status", 1)];
        let result = rewrite_optional_params(sql, &["status".to_string()], &params);
        // $1 placeholder doesn't appear, so no rewrite
        assert_eq!(result, sql);
    }

    #[test]
    fn test_normalize_engine_mariadb() {
        assert_eq!(normalize_engine("mariadb"), "mariadb");
    }

    #[test]
    fn test_get_backend_mariadb_with_mysql_backends() {
        let mariadb_backends = [
            "rust-sqlx",
            "python-aiomysql",
            "typescript-mysql2",
            "go-database-sql",
            "java-jdbc",
            "java-r2dbc",
            "kotlin-jdbc",
            "kotlin-r2dbc",
            "csharp-mysqlconnector",
            "elixir-myxql",
            "ruby-mysql2",
            "ruby-trilogy",
            "php-pdo",
            "php-amphp",
        ];
        for backend_name in &mariadb_backends {
            let result = get_backend(backend_name, "mariadb");
            assert!(
                result.is_ok(),
                "backend '{}' should accept mariadb engine, got: {:?}",
                backend_name,
                result.err()
            );
        }
    }
}
