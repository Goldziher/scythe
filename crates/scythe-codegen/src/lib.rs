mod enum_gen;
mod query_fn;
mod structs;

use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery};
use scythe_core::catalog::Catalog;
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use enum_gen::generate_enum_defs;
pub use enum_gen::generate_single_enum_def;
use query_fn::generate_query_fn;
use structs::{generate_composite_defs, generate_model_struct, generate_row_struct};

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct GeneratedCode {
    pub query_fn: Option<String>,
    pub row_struct: Option<String>,
    pub model_struct: Option<String>,
    pub enum_def: Option<String>,
}

// ---------------------------------------------------------------------------
// Manifest loading
// ---------------------------------------------------------------------------

/// Default embedded manifest TOML for rust-sqlx, used as fallback.
const DEFAULT_MANIFEST_TOML: &str = include_str!("../../../backends/rust-sqlx/manifest.toml");

pub fn load_or_default_manifest() -> Result<BackendManifest, ScytheError> {
    let manifest_path = Path::new("backends/rust-sqlx/manifest.toml");
    if manifest_path.exists() {
        load_manifest(manifest_path).map_err(|e| {
            ScytheError::new(
                ErrorCode::InternalError,
                format!("failed to load manifest: {e}"),
            )
        })
    } else {
        toml::from_str(DEFAULT_MANIFEST_TOML).map_err(|e| {
            ScytheError::new(
                ErrorCode::InternalError,
                format!("failed to parse embedded manifest: {e}"),
            )
        })
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate Rust code from an analyzed query.
pub fn generate(analyzed: &AnalyzedQuery) -> Result<GeneratedCode, ScytheError> {
    let manifest = load_or_default_manifest()?;
    generate_with_manifest(analyzed, &manifest)
}

/// Stub for catalog-level codegen. Returns default for now.
pub fn generate_from_catalog(_catalog: &Catalog) -> Result<GeneratedCode, ScytheError> {
    Ok(GeneratedCode::default())
}

// ---------------------------------------------------------------------------
// Internal generation
// ---------------------------------------------------------------------------

fn generate_with_manifest(
    analyzed: &AnalyzedQuery,
    manifest: &BackendManifest,
) -> Result<GeneratedCode, ScytheError> {
    let mut result = GeneratedCode::default();

    // Generate enum definitions for any enum-typed columns
    let enum_def = generate_enum_defs(analyzed, manifest)?;
    if !enum_def.is_empty() {
        result.enum_def = Some(enum_def);
    }

    // Generate row/model struct for :one and :many commands
    let needs_row_struct = matches!(analyzed.command, QueryCommand::One | QueryCommand::Many);
    if needs_row_struct && !analyzed.columns.is_empty() {
        if analyzed.source_table.is_some() {
            // SELECT * from single table: generate model_struct with table name
            result.model_struct = Some(generate_model_struct(analyzed, manifest)?);
        } else {
            result.row_struct = Some(generate_row_struct(analyzed, manifest)?);
        }
    }

    // Generate composite type definitions
    if !analyzed.composites.is_empty() {
        let comp_defs = generate_composite_defs(&analyzed.composites, manifest)?;
        if !comp_defs.is_empty() {
            // Append to model_struct or create new
            if let Some(ref mut existing) = result.model_struct {
                existing.push_str("\n\n");
                existing.push_str(&comp_defs);
            } else {
                result.model_struct = Some(comp_defs);
            }
        }
    }

    // Generate query function
    result.query_fn = Some(generate_query_fn(analyzed, manifest)?);

    Ok(result)
}

// ---------------------------------------------------------------------------
// Type resolution helpers
// ---------------------------------------------------------------------------

fn resolve_col_type(
    col: &AnalyzedColumn,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    resolve_type(&col.neutral_type, manifest, col.nullable)
        .map(|t| t.into_owned())
        .map_err(|e| {
            ScytheError::new(
                ErrorCode::InternalError,
                format!("type resolution failed for column '{}': {}", col.name, e),
            )
        })
}

fn resolve_param_type(
    param: &AnalyzedParam,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    resolve_type(&param.neutral_type, manifest, param.nullable)
        .map(|t| t.into_owned())
        .map_err(|e| {
            ScytheError::new(
                ErrorCode::InternalError,
                format!("type resolution failed for param '{}': {}", param.name, e),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery};
    use scythe_core::parser::QueryCommand;

    fn make_query(
        name: &str,
        command: QueryCommand,
        sql: &str,
        columns: Vec<AnalyzedColumn>,
        params: Vec<AnalyzedParam>,
    ) -> AnalyzedQuery {
        AnalyzedQuery {
            name: name.to_string(),
            command,
            sql: sql.to_string(),
            columns,
            params,
            deprecated: None,
            source_table: None,
            composites: Vec::new(),
            enums: Vec::new(),
        }
    }

    #[test]
    fn test_generate_select_many() {
        let query = make_query(
            "ListUsers",
            QueryCommand::Many,
            "SELECT id, name, email FROM users",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                },
                AnalyzedColumn {
                    name: "email".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                },
            ],
            vec![],
        );

        let result = generate(&query).unwrap();

        let row_struct = result.row_struct.unwrap();
        assert!(row_struct.contains("pub struct ListUsersRow"));
        assert!(row_struct.contains("pub id: i32"));
        assert!(row_struct.contains("pub name: String"));
        assert!(row_struct.contains("pub email: Option<String>"));

        let query_fn = result.query_fn.unwrap();
        assert!(query_fn.contains("pub async fn list_users("));
        assert!(query_fn.contains("-> Result<Vec<ListUsersRow>, sqlx::Error>"));
        assert!(query_fn.contains(".fetch_all(pool)"));
    }

    #[test]
    fn test_generate_select_one_with_param() {
        let query = make_query(
            "GetUser",
            QueryCommand::One,
            "SELECT id, name FROM users WHERE id = $1",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                },
            ],
            vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
            }],
        );

        let result = generate(&query).unwrap();

        let query_fn = result.query_fn.unwrap();
        assert!(query_fn.contains("pub async fn get_user("));
        assert!(query_fn.contains("id: i32"));
        assert!(query_fn.contains("-> Result<GetUserRow, sqlx::Error>"));
        assert!(query_fn.contains(".fetch_one(pool)"));
    }

    #[test]
    fn test_generate_exec() {
        let query = make_query(
            "DeleteUser",
            QueryCommand::Exec,
            "DELETE FROM users WHERE id = $1",
            vec![],
            vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
            }],
        );

        let result = generate(&query).unwrap();

        assert!(result.row_struct.is_none());

        let query_fn = result.query_fn.unwrap();
        assert!(query_fn.contains("pub async fn delete_user("));
        assert!(query_fn.contains("-> Result<(), sqlx::Error>"));
        assert!(query_fn.contains(".execute(pool)"));
    }

    #[test]
    fn test_generate_with_enum_column() {
        let query = make_query(
            "GetUserStatus",
            QueryCommand::One,
            "SELECT id, status FROM users WHERE id = $1",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                },
                AnalyzedColumn {
                    name: "status".to_string(),
                    neutral_type: "enum::user_status".to_string(),
                    nullable: false,
                },
            ],
            vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
            }],
        );

        let result = generate(&query).unwrap();

        assert!(result.enum_def.is_some());
        let enum_def = result.enum_def.unwrap();
        assert!(enum_def.contains("pub enum UserStatus"));
        assert!(enum_def.contains("type_name = \"user_status\""));

        let row_struct = result.row_struct.unwrap();
        assert!(row_struct.contains("pub status: UserStatus"));
    }

    #[test]
    fn test_generate_from_catalog_returns_default() {
        let catalog = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER);"]).unwrap();
        let result = generate_from_catalog(&catalog).unwrap();
        assert!(result.query_fn.is_none());
        assert!(result.row_struct.is_none());
    }
}
