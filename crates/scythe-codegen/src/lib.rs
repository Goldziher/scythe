pub mod backend_trait;
pub mod backends;
pub mod overrides;
pub mod resolve;
pub mod validation;

pub use backend_trait::{
    CodegenBackend, RbsEnumInfo, RbsGenerationContext, RbsQueryInfo, ResolvedColumn, ResolvedParam,
};
pub use backends::get_backend;
pub use overrides::TypeOverride;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{row_struct_name, to_pascal_case};

use scythe_core::analyzer::{AnalyzedQuery, EnumInfo};
use scythe_core::catalog::Catalog;
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

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
// Utility (shared across backends)
// ---------------------------------------------------------------------------

/// Simple singularization: remove trailing 's'.
pub fn singularize(name: &str) -> String {
    if let Some(stem) = name.strip_suffix("ies") {
        format!("{stem}y")
    } else if name.ends_with("sses")
        || name.ends_with("shes")
        || name.ends_with("ches")
        || name.ends_with("xes")
        || name.ends_with("zes")
        || name.ends_with("ses")
    {
        name[..name.len() - 2].to_string()
    } else if name.ends_with('s') && !name.ends_with("ss") {
        name[..name.len() - 1].to_string()
    } else {
        name.to_string()
    }
}

// ---------------------------------------------------------------------------
// Manifest helpers
// ---------------------------------------------------------------------------

/// Get the manifest for a backend. Defaults to PostgreSQL engine.
pub fn get_manifest_for_backend(backend_name: &str) -> Result<BackendManifest, ScytheError> {
    let backend = get_backend(backend_name, "postgresql")?;
    Ok(backend.manifest().clone())
}

/// Determine the struct name for a query (model struct or row struct).
fn determine_struct_name(analyzed: &AnalyzedQuery, manifest: &BackendManifest) -> String {
    if let Some(ref table_name) = analyzed.source_table {
        let singular = singularize(table_name);
        to_pascal_case(&singular).into_owned()
    } else {
        row_struct_name(&analyzed.name, &manifest.naming)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate code using a specific backend.
pub fn generate_with_backend(
    analyzed: &AnalyzedQuery,
    backend: &dyn CodegenBackend,
) -> Result<GeneratedCode, ScytheError> {
    generate_with_backend_and_overrides(analyzed, backend, &[])
}

/// Generate code using a specific backend with type overrides.
pub fn generate_with_backend_and_overrides(
    analyzed: &AnalyzedQuery,
    backend: &dyn CodegenBackend,
    overrides: &[TypeOverride],
) -> Result<GeneratedCode, ScytheError> {
    let manifest = backend.manifest();
    let source_table = analyzed.source_table.as_deref().unwrap_or("");
    let columns = resolve::resolve_columns(&analyzed.columns, manifest, overrides, source_table)?;
    let params = resolve::resolve_params(&analyzed.params, manifest, overrides, source_table)?;

    let mut result = GeneratedCode::default();

    // Generate enum definitions for any enum-typed columns
    // Use the backend-specific enum generation for proper derives
    let enum_def = generate_enum_defs_via_backend(analyzed, backend)?;
    if !enum_def.is_empty() {
        result.enum_def = Some(enum_def);
    }

    // Generate row/model struct for :one, :opt, and :many commands (not :batch or :grouped).
    // Grouped queries generate their own parent+child structs via generate_grouped_structs.
    let needs_row_struct = matches!(
        analyzed.command,
        QueryCommand::One | QueryCommand::Opt | QueryCommand::Many
    );
    if needs_row_struct && !analyzed.columns.is_empty() {
        if let Some(ref table_name) = analyzed.source_table {
            result.model_struct = Some(backend.generate_model_struct(table_name, &columns)?);
        } else {
            result.row_struct = Some(backend.generate_row_struct(&analyzed.name, &columns)?);
        }
    }

    // Generate composite type definitions
    if !analyzed.composites.is_empty() {
        let mut comp_defs = String::new();
        for (i, comp) in analyzed.composites.iter().enumerate() {
            if i > 0 {
                comp_defs.push_str("\n\n");
            }
            comp_defs.push_str(&backend.generate_composite_def(comp)?);
        }
        if !comp_defs.is_empty() {
            if let Some(ref mut existing) = result.model_struct {
                existing.push_str("\n\n");
                existing.push_str(&comp_defs);
            } else {
                result.model_struct = Some(comp_defs);
            }
        }
    }

    // Generate query function (and for :grouped, also the parent+child structs).
    let struct_name = determine_struct_name(analyzed, manifest);

    if analyzed.command == QueryCommand::Grouped {
        // Real grouped codegen: parent struct + child struct + fold-on-client query fn.
        let group_by = analyzed.group_by.as_ref().ok_or_else(|| {
            ScytheError::new(
                ErrorCode::InternalError,
                format!(
                    "query '{}' is :grouped but is missing @group_by annotation",
                    analyzed.name
                ),
            )
        })?;

        // Derive struct names from the manifest's row suffix and struct case.
        // E.g. "GetUsersWithOrders" + suffix "Row" -> parent "GetUsersWithOrdersRow",
        // child "GetUsersWithOrdersChildRow".
        let parent_struct_name = scythe_backend::naming::row_struct_name(&analyzed.name, &manifest.naming);
        let child_struct_name = {
            let suffix = &manifest.naming.row_suffix;
            let base = parent_struct_name.trim_end_matches(suffix.as_str());
            format!("{}Child{}", base, suffix)
        };

        // Resolve parent and child columns to language-specific types.
        let parent_cols = resolve::resolve_columns(&group_by.parent_columns, manifest, overrides, source_table)?;
        let child_cols = resolve::resolve_columns(&group_by.child_columns, manifest, overrides, source_table)?;

        result.row_struct = Some(backend.generate_grouped_structs(
            &parent_struct_name,
            &child_struct_name,
            &parent_cols,
            &child_cols,
            &group_by.key_column,
        )?);
        result.query_fn = Some(
            backend.generate_grouped_query_fn(&crate::backend_trait::GroupedQueryFn {
                analyzed,
                parent_struct_name: &parent_struct_name,
                child_struct_name: &child_struct_name,
                all_columns: &columns,
                parent_columns: &parent_cols,
                child_columns: &child_cols,
                params: &params,
                key_column: &group_by.key_column,
            })?,
        );
    } else {
        result.query_fn = Some(backend.generate_query_fn(analyzed, &struct_name, &columns, &params)?);
    }

    Ok(result)
}

/// Generate enum definitions via the backend trait.
fn generate_enum_defs_via_backend(
    analyzed: &AnalyzedQuery,
    backend: &dyn CodegenBackend,
) -> Result<String, ScytheError> {
    use ahash::AHashSet;
    use std::fmt::Write;

    let mut out = String::new();
    let mut seen_enums: AHashSet<String> = AHashSet::new();

    let enum_sources: Vec<&str> = analyzed
        .columns
        .iter()
        .filter_map(|col| col.neutral_type.strip_prefix("enum::"))
        .chain(
            analyzed
                .params
                .iter()
                .filter_map(|p| p.neutral_type.strip_prefix("enum::")),
        )
        .collect();

    for sql_name in enum_sources {
        if !seen_enums.insert(sql_name.to_string()) {
            continue;
        }

        if !out.is_empty() {
            let _ = writeln!(out);
        }

        if let Some(enum_info) = analyzed.enums.iter().find(|e| e.sql_name == sql_name) {
            out.push_str(&backend.generate_enum_def(enum_info)?);
        } else {
            // Generate a stub enum with no variants (for enum types referenced but
            // not fully defined in the query's EnumInfo list).
            let stub_info = EnumInfo {
                sql_name: sql_name.to_string(),
                values: vec![],
            };
            out.push_str(&backend.generate_enum_def(&stub_info)?);
        }
    }

    Ok(out)
}

/// Backward-compatible: generate code using the default sqlx backend.
pub fn generate(analyzed: &AnalyzedQuery) -> Result<GeneratedCode, ScytheError> {
    let backend = get_backend("rust-sqlx", "postgresql")?;
    generate_with_backend(analyzed, &*backend)
}

/// Stub for catalog-level codegen. Returns default for now.
pub fn generate_from_catalog(_catalog: &Catalog) -> Result<GeneratedCode, ScytheError> {
    Ok(GeneratedCode::default())
}

/// Generate a single enum definition using a specific backend.
pub fn generate_single_enum_def_with_backend(
    enum_info: &EnumInfo,
    backend: &dyn CodegenBackend,
) -> Result<String, ScytheError> {
    backend.generate_enum_def(enum_info)
}

/// Backward-compatible: generate a single enum definition (sqlx backend).
/// Uses the manifest directly for backward compatibility with existing callers.
pub fn generate_single_enum_def(enum_info: &EnumInfo, manifest: &BackendManifest) -> String {
    // Reproduce the old behavior exactly using the sqlx backend's logic
    use scythe_backend::naming::{enum_type_name, enum_variant_name};
    use std::fmt::Write;

    let mut out = String::with_capacity(256);
    let type_name = enum_type_name(&enum_info.sql_name, &manifest.naming);

    let _ = writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]");
    let _ = writeln!(
        out,
        "#[sqlx(type_name = \"{}\", rename_all = \"snake_case\")]",
        enum_info.sql_name
    );
    let _ = writeln!(out, "pub enum {type_name} {{");

    for value in &enum_info.values {
        let variant = enum_variant_name(value, &manifest.naming);
        let _ = writeln!(out, "    {variant},");
    }

    let _ = write!(out, "}}");
    out
}

/// Backward-compatible: load the default sqlx manifest.
pub fn load_or_default_manifest() -> Result<BackendManifest, ScytheError> {
    let b = backends::sqlx::SqlxBackend::new("postgresql")?;
    Ok(b.manifest().clone())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, GroupByConfig};
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
            optional_params: Vec::new(),
            group_by: None,
            custom: Vec::new(),
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

    #[test]
    fn test_singularize_basic() {
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("orders"), "order");
        assert_eq!(singularize("posts"), "post");
    }

    #[test]
    fn test_singularize_ies() {
        assert_eq!(singularize("categories"), "category");
        assert_eq!(singularize("entries"), "entry");
    }

    #[test]
    fn test_singularize_sses() {
        assert_eq!(singularize("addresses"), "address");
        assert_eq!(singularize("classes"), "class");
    }

    #[test]
    fn test_singularize_no_change() {
        assert_eq!(singularize("status"), "statu");
        assert_eq!(singularize("boss"), "boss");
        assert_eq!(singularize("address"), "address");
    }

    #[test]
    fn test_singularize_shes_ches_xes() {
        assert_eq!(singularize("batches"), "batch");
        assert_eq!(singularize("boxes"), "box");
        assert_eq!(singularize("wishes"), "wish");
    }

    // -----------------------------------------------------------------------
    // Grouped query codegen tests (issue #55 reference example)
    // Schema: users(id, name, email), orders(id, user_id, total, created_at)
    // Query: SELECT u.id, u.name, u.email, o.id AS order_id, o.total,
    //               o.created_at AS order_date FROM users u JOIN orders o ...
    // @group_by users.id
    // -----------------------------------------------------------------------

    fn make_grouped_query() -> AnalyzedQuery {
        let parent_cols = vec![
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
                nullable: false,
            },
        ];
        let child_cols = vec![
            AnalyzedColumn {
                name: "order_id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
            },
            AnalyzedColumn {
                name: "total".to_string(),
                neutral_type: "decimal".to_string(),
                nullable: true,
            },
            AnalyzedColumn {
                name: "order_date".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: false,
            },
        ];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();

        AnalyzedQuery {
            name: "GetUsersWithOrders".to_string(),
            command: QueryCommand::Grouped,
            sql: "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\n\
                  SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\n\
                  FROM users u\n\
                  JOIN orders o ON o.user_id = u.id"
                .to_string(),
            columns: all_cols,
            params: vec![],
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
            optional_params: vec![],
            group_by: Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: parent_cols,
                child_columns: child_cols,
            }),
            custom: vec![],
        }
    }

    #[test]
    fn test_grouped_sqlx_structs() {
        let backend = get_backend("rust-sqlx", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        // Both child and parent structs packed into row_struct (child first)
        let row_struct = result.row_struct.as_deref().unwrap();
        assert!(
            row_struct.contains("pub struct GetUsersWithOrdersChildRow"),
            "missing child struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub order_id: i32"),
            "child struct missing order_id field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub struct GetUsersWithOrdersRow"),
            "missing parent struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub id: i32"),
            "parent struct missing id field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub name: String"),
            "parent struct missing name field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub email: String"),
            "parent struct missing email field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("pub children: Vec<GetUsersWithOrdersChildRow>"),
            "parent struct missing children field; got:\n{row_struct}"
        );
        // Child struct must appear BEFORE parent struct (no forward references)
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("pub struct GetUsersWithOrdersRow").unwrap();
        assert!(
            child_pos < parent_pos,
            "child struct must be defined before parent struct"
        );

        // No flat struct should exist for grouped queries
        assert!(
            result.model_struct.is_none(),
            "grouped should not produce a model_struct"
        );
    }

    #[test]
    fn test_grouped_sqlx_query_fn() {
        let backend = get_backend("rust-sqlx", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("pub async fn get_users_with_orders("),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("-> Result<Vec<GetUsersWithOrdersRow>, sqlx::Error>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("sqlx::query!("),
            "grouped fn must use sqlx::query!; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow {"),
            "fn must construct child struct; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("children: vec![child]"),
            "fn must initialize children vec; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("parent.children.push(child)"),
            "fn must fold child into existing parent; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Ok(result)"),
            "fn must return result; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_grouped_python_asyncpg_structs() {
        let backend = get_backend("python-asyncpg", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();
        assert!(
            row_struct.contains("class GetUsersWithOrdersChildRow"),
            "missing child class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("order_id: int"),
            "child class missing order_id field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("class GetUsersWithOrdersRow"),
            "missing parent class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("id: int"),
            "parent class missing id field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("name: str"),
            "parent class missing name field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("children: list[GetUsersWithOrdersChildRow]"),
            "parent class missing children field; got:\n{row_struct}"
        );
        // Child class must appear before parent class
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("class GetUsersWithOrdersRow").unwrap();
        assert!(
            child_pos < parent_pos,
            "child class must be defined before parent class"
        );
    }

    #[test]
    fn test_grouped_python_asyncpg_query_fn() {
        let backend = get_backend("python-asyncpg", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("async def get_users_with_orders(conn: Connection)"),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("-> list[GetUsersWithOrdersRow]:"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("conn.fetch("),
            "fn must use conn.fetch; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow("),
            "fn must construct child class; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersRow(**parent_kwargs, children=children)"),
            "fn must construct parent with **kwargs; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("_index"),
            "fn must use index dict for O(1) lookup; got:\n{query_fn}"
        );
    }

    /// Minimal backend that implements only the required trait methods and
    /// leaves the grouped methods as their default (erroring) impl. Every
    /// shipped backend now supports grouped codegen, so the "not yet supported"
    /// path can only be exercised by a backend that has not opted in — this stub
    /// stands in for such a backend so the contract stays covered.
    struct StubBackend {
        manifest: BackendManifest,
    }

    impl CodegenBackend for StubBackend {
        fn name(&self) -> &str {
            "stub-backend"
        }
        fn manifest(&self) -> &BackendManifest {
            &self.manifest
        }
        fn generate_row_struct(&self, _query_name: &str, _columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_model_struct(&self, _table_name: &str, _columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_query_fn(
            &self,
            _analyzed: &AnalyzedQuery,
            _struct_name: &str,
            _columns: &[ResolvedColumn],
            _params: &[ResolvedParam],
        ) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_enum_def(&self, _enum_info: &scythe_core::analyzer::EnumInfo) -> Result<String, ScytheError> {
            Ok(String::new())
        }
        fn generate_composite_def(
            &self,
            _composite: &scythe_core::analyzer::CompositeInfo,
        ) -> Result<String, ScytheError> {
            Ok(String::new())
        }
    }

    #[test]
    fn test_grouped_unsupported_backend_returns_clear_error() {
        // A backend that does not override the grouped trait methods must surface
        // a clear, backend-named error rather than panicking. A stub keeps this
        // valid even though every shipped backend now implements grouped codegen.
        let manifest = get_backend("rust-sqlx", "postgresql").unwrap().manifest().clone();
        let backend = StubBackend { manifest };
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &backend);
        assert!(
            result.is_err(),
            "stub backend should return an error for grouped queries"
        );
        let err = result.unwrap_err();
        assert!(
            err.message.contains("not yet supported"),
            "error should contain 'not yet supported', got: {}",
            err.message
        );
        assert!(
            err.message.contains("stub-backend"),
            "error should name the backend, got: {}",
            err.message
        );
    }

    #[test]
    fn test_tokio_postgres_backend_basic() {
        let backend = get_backend("tokio-postgres", "postgresql").unwrap();

        let query = make_query(
            "ListUsers",
            QueryCommand::Many,
            "SELECT id, name FROM users",
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
            vec![],
        );

        let result = generate_with_backend(&query, &*backend).unwrap();

        let row_struct = result.row_struct.unwrap();
        assert!(row_struct.contains("pub struct ListUsersRow"));
        assert!(row_struct.contains("pub id: i32"));
        assert!(row_struct.contains("pub name: String"));
        assert!(row_struct.contains("from_row"));
        assert!(row_struct.contains("tokio_postgres::Row"));
        // Should NOT contain sqlx
        assert!(!row_struct.contains("sqlx"));

        let query_fn = result.query_fn.unwrap();
        assert!(query_fn.contains("pub async fn list_users("));
        assert!(query_fn.contains("tokio_postgres::GenericClient"));
        assert!(query_fn.contains("tokio_postgres::Error"));
        assert!(!query_fn.contains("sqlx"));
    }

    #[test]
    fn test_tokio_postgres_enum() {
        let backend = get_backend("tokio-postgres", "postgresql").unwrap();

        let enum_info = scythe_core::analyzer::EnumInfo {
            sql_name: "user_status".to_string(),
            values: vec!["active".to_string(), "inactive".to_string()],
        };

        let def = backend.generate_enum_def(&enum_info).unwrap();
        assert!(def.contains("pub enum UserStatus"));
        assert!(def.contains("Active"));
        assert!(def.contains("Inactive"));
        assert!(def.contains("impl std::fmt::Display"));
        assert!(def.contains("impl std::str::FromStr"));
        // Should NOT contain sqlx
        assert!(!def.contains("sqlx"));
    }

    #[test]
    fn test_sql_with_least_coalesce_sum_preserved() {
        let backend = get_backend("tokio-postgres", "postgresql").unwrap();

        let query = make_query(
            "GetBillingAggregates",
            QueryCommand::One,
            "SELECT LEAST(COALESCE(SUM(ba.free_pages_remaining), 0), 10000) as aggregated_free_pages FROM billing_aggregates ba WHERE ba.customer_id = $1",
            vec![AnalyzedColumn {
                name: "aggregated_free_pages".to_string(),
                neutral_type: "int64".to_string(),
                nullable: false,
            }],
            vec![AnalyzedParam {
                name: "customer_id".to_string(),
                neutral_type: "uuid".to_string(),
                nullable: false,
                position: 1,
            }],
        );

        let result = generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.unwrap();

        // The SQL should preserve the original casing of function names (LEAST, COALESCE, SUM)
        // and the alias keyword (as)
        assert!(
            query_fn.contains("LEAST(COALESCE(SUM(ba.free_pages_remaining), 0), 10000)"),
            "SQL should preserve original LEAST/COALESCE/SUM function names and casing"
        );
        assert!(
            query_fn.contains("as aggregated_free_pages"),
            "SQL should preserve alias keyword (as)"
        );
    }

    #[test]
    fn test_generated_rust_code_structure() {
        let backend = get_backend("tokio-postgres", "postgresql").unwrap();

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

        let result = generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.unwrap();

        // Verify basic structure is present
        assert!(query_fn.contains("pub async fn get_user("));
        assert!(query_fn.contains("tokio_postgres::GenericClient"));
        assert!(query_fn.contains("GetUserRow"));

        // Verify SQL is preserved in the code
        assert!(query_fn.contains("SELECT id, name FROM users"));
    }
}
