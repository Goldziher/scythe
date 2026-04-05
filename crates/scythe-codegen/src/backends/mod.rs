pub mod sqlx;
pub mod tokio_postgres;

use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::CodegenBackend;

/// Get a backend by name.
pub fn get_backend(name: &str) -> Result<Box<dyn CodegenBackend>, ScytheError> {
    match name {
        "rust-sqlx" | "sqlx" => Ok(Box::new(sqlx::SqlxBackend::new()?)),
        "rust-tokio-postgres" | "tokio-postgres" => {
            Ok(Box::new(tokio_postgres::TokioPostgresBackend::new()?))
        }
        _ => Err(ScytheError::new(
            ErrorCode::InternalError,
            format!("unknown backend: {}", name),
        )),
    }
}
