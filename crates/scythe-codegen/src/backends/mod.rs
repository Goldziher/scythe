pub mod csharp_npgsql;
pub mod elixir_postgrex;
pub mod go_pgx;
pub mod java_jdbc;
pub mod kotlin_jdbc;
pub mod php_pdo;
pub mod python_asyncpg;
pub mod python_psycopg3;
pub mod ruby_pg;
pub mod sqlx;
pub mod tokio_postgres;
pub mod typescript_pg;
pub mod typescript_postgres;

use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::CodegenBackend;

/// Get a backend by name.
pub fn get_backend(name: &str) -> Result<Box<dyn CodegenBackend>, ScytheError> {
    match name {
        "rust-sqlx" | "sqlx" => Ok(Box::new(sqlx::SqlxBackend::new()?)),
        "rust-tokio-postgres" | "tokio-postgres" => {
            Ok(Box::new(tokio_postgres::TokioPostgresBackend::new()?))
        }
        "python-psycopg3" => Ok(Box::new(python_psycopg3::PythonPsycopg3Backend::new()?)),
        "python-asyncpg" => Ok(Box::new(python_asyncpg::PythonAsyncpgBackend::new()?)),
        "typescript-postgres" => Ok(Box::new(
            typescript_postgres::TypescriptPostgresBackend::new()?,
        )),
        "typescript-pg" => Ok(Box::new(typescript_pg::TypescriptPgBackend::new()?)),
        "go-pgx" => Ok(Box::new(go_pgx::GoPgxBackend::new()?)),
        "java-jdbc" => Ok(Box::new(java_jdbc::JavaJdbcBackend::new()?)),
        "kotlin-jdbc" => Ok(Box::new(kotlin_jdbc::KotlinJdbcBackend::new()?)),
        "csharp-npgsql" => Ok(Box::new(csharp_npgsql::CsharpNpgsqlBackend::new()?)),
        "elixir-postgrex" => Ok(Box::new(elixir_postgrex::ElixirPostgrexBackend::new()?)),
        "ruby-pg" => Ok(Box::new(ruby_pg::RubyPgBackend::new()?)),
        "php-pdo" => Ok(Box::new(php_pdo::PhpPdoBackend::new()?)),
        _ => Err(ScytheError::new(
            ErrorCode::InternalError,
            format!("unknown backend: {}", name),
        )),
    }
}
