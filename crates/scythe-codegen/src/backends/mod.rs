pub mod csharp_microsoft_sqlite;
pub mod csharp_mysqlconnector;
pub mod csharp_npgsql;
pub mod elixir_exqlite;
pub mod elixir_myxql;
pub mod elixir_postgrex;
pub mod go_database_sql;
pub mod go_pgx;
pub mod java_jdbc;
pub mod kotlin_jdbc;
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
        "elixir-myxql" => Box::new(elixir_myxql::ElixirMyxqlBackend::new(engine)?),
        "elixir-exqlite" => Box::new(elixir_exqlite::ElixirExqliteBackend::new(engine)?),
        "ruby-pg" | "ruby" | "rb" => Box::new(ruby_pg::RubyPgBackend::new(engine)?),
        "ruby-mysql2" => Box::new(ruby_mysql2::RubyMysql2Backend::new(engine)?),
        "ruby-sqlite3" => Box::new(ruby_sqlite3::RubySqlite3Backend::new(engine)?),
        "ruby-trilogy" | "trilogy" => Box::new(ruby_trilogy::RubyTrilogyBackend::new(engine)?),
        "php-pdo" | "php" => Box::new(php_pdo::PhpPdoBackend::new(engine)?),
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
