use std::fmt::Write;
use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::naming::{
    enum_type_name, fn_name, row_struct_name, to_pascal_case, to_snake_case,
};
use scythe_backend::types::resolve_type;

use crate::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, CompositeInfo};
use crate::catalog::Catalog;
use crate::errors::{ErrorCode, ScytheError};
use crate::parser::QueryCommand;

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
const DEFAULT_MANIFEST_TOML: &str = include_str!("../../backends/rust-sqlx/manifest.toml");

fn load_or_default_manifest() -> Result<BackendManifest, ScytheError> {
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
// Row struct generation
// ---------------------------------------------------------------------------

fn generate_row_struct(
    analyzed: &AnalyzedQuery,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    let struct_name = row_struct_name(&analyzed.name, &manifest.naming);
    let mut out = String::new();

    writeln!(out, "#[derive(Debug, sqlx::FromRow)]").unwrap();
    writeln!(out, "pub struct {} {{", struct_name).unwrap();

    for col in &analyzed.columns {
        let field_name = to_snake_case(&col.name);
        let rust_type = resolve_col_type(col, manifest)?;
        writeln!(out, "    pub {}: {},", field_name, rust_type).unwrap();
    }

    write!(out, "}}").unwrap();

    Ok(out)
}

// ---------------------------------------------------------------------------
// Model struct generation (for SELECT * from single table)
// ---------------------------------------------------------------------------

fn generate_model_struct(
    analyzed: &AnalyzedQuery,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    let table_name = analyzed.source_table.as_deref().unwrap_or(&analyzed.name);
    // Singularize table name for model struct: "users" -> "User"
    let singular = singularize(table_name);
    let struct_name = to_pascal_case(&singular).into_owned();
    let mut out = String::new();

    writeln!(out, "#[derive(Debug, sqlx::FromRow)]").unwrap();
    writeln!(out, "pub struct {} {{", struct_name).unwrap();

    for col in &analyzed.columns {
        let field_name = to_snake_case(&col.name);
        let rust_type = resolve_col_type(col, manifest)?;
        writeln!(out, "    pub {}: {},", field_name, rust_type).unwrap();
    }

    write!(out, "}}").unwrap();

    Ok(out)
}

/// Simple singularization: remove trailing 's'
fn singularize(name: &str) -> String {
    if let Some(stem) = name.strip_suffix("ies") {
        format!("{stem}y")
    } else if name.ends_with("ses")
        || name.ends_with("xes")
        || name.ends_with("shes")
        || name.ends_with("ches")
    {
        name[..name.len() - 2].to_string()
    } else if name.ends_with('s') && !name.ends_with("ss") {
        name[..name.len() - 1].to_string()
    } else {
        name.to_string()
    }
}

// ---------------------------------------------------------------------------
// Query function generation
// ---------------------------------------------------------------------------

fn generate_query_fn(
    analyzed: &AnalyzedQuery,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    let func_name = fn_name(&analyzed.name, &manifest.naming);
    let struct_name = if let Some(ref table_name) = analyzed.source_table {
        let singular = singularize(table_name);
        to_pascal_case(&singular).into_owned()
    } else {
        row_struct_name(&analyzed.name, &manifest.naming)
    };

    let mut out = String::new();

    // Deprecated annotation
    if let Some(ref msg) = analyzed.deprecated {
        writeln!(out, "#[deprecated(note = \"{}\")]", msg).unwrap();
    }

    // Build parameter list
    let mut param_parts: Vec<String> = vec!["pool: &sqlx::PgPool".to_string()];
    for param in &analyzed.params {
        let param_name = to_snake_case(&param.name);
        let rust_type = resolve_param_type(param, manifest)?;
        let rust_type = param_type_to_borrowed(&rust_type);
        param_parts.push(format!("{}: {}", param_name, rust_type));
    }

    // Return type
    let return_type = match &analyzed.command {
        QueryCommand::One => struct_name.clone(),
        QueryCommand::Many => format!("Vec<{}>", struct_name),
        QueryCommand::Exec => "()".to_string(),
        QueryCommand::ExecResult => "sqlx::postgres::PgQueryResult".to_string(),
        QueryCommand::ExecRows => "u64".to_string(),
        QueryCommand::Batch => format!("Vec<{}>", struct_name),
    };

    // Function signature - all params on one line
    writeln!(
        out,
        "pub async fn {}({}) -> Result<{}, sqlx::Error> {{",
        func_name,
        param_parts.join(", "),
        return_type
    )
    .unwrap();

    // Strip trailing semicolons from SQL
    let sql_raw = analyzed.sql.trim_end_matches(';').trim();

    // Rewrite SQL for enum columns: add "column: EnumType" aliases for sqlx
    let sql = rewrite_sql_for_enums(sql_raw, &analyzed.columns, manifest);

    // Query body
    let has_row_struct = matches!(
        analyzed.command,
        QueryCommand::One | QueryCommand::Many | QueryCommand::Batch
    );

    // Build bind params string
    let bind_params: String = analyzed
        .params
        .iter()
        .map(|p| {
            let param_name = to_snake_case(&p.name);
            if p.neutral_type.starts_with("enum::") {
                let enum_name = p.neutral_type.strip_prefix("enum::").unwrap();
                let rust_type = enum_type_name(enum_name, &manifest.naming);
                format!(", {} as &{}", param_name, rust_type)
            } else {
                format!(", {}", param_name)
            }
        })
        .collect();

    let is_exec_rows = matches!(analyzed.command, QueryCommand::ExecRows);

    if is_exec_rows {
        // ExecRows: let result = sqlx::query!(...) pattern
        if has_row_struct && !analyzed.columns.is_empty() {
            write!(
                out,
                "    let result = sqlx::query_as!({}, \"{}\"{})",
                struct_name, sql, bind_params
            )
            .unwrap();
        } else {
            write!(
                out,
                "    let result = sqlx::query!(\"{}\"{})",
                sql, bind_params
            )
            .unwrap();
        }
    } else if has_row_struct && !analyzed.columns.is_empty() {
        write!(
            out,
            "    sqlx::query_as!({}, \"{}\"{})",
            struct_name, sql, bind_params
        )
        .unwrap();
    } else {
        write!(out, "    sqlx::query!(\"{}\"{})", sql, bind_params).unwrap();
    }

    writeln!(out).unwrap();

    // Fetch method
    let fetch_method = match &analyzed.command {
        QueryCommand::One => ".fetch_one(pool)",
        QueryCommand::Many => ".fetch_all(pool)",
        QueryCommand::Exec => ".execute(pool)",
        QueryCommand::ExecResult => ".execute(pool)",
        QueryCommand::ExecRows => ".execute(pool)",
        QueryCommand::Batch => ".fetch_all(pool)",
    };

    write!(out, "        {}", fetch_method).unwrap();
    writeln!(out).unwrap();

    // Post-processing for exec variants
    match &analyzed.command {
        QueryCommand::Exec => {
            writeln!(out, "        .await?;").unwrap();
            writeln!(out, "    Ok(())").unwrap();
        }
        QueryCommand::ExecRows => {
            writeln!(out, "        .await?;").unwrap();
            writeln!(out, "    Ok(result.rows_affected())").unwrap();
        }
        _ => {
            writeln!(out, "        .await").unwrap();
        }
    }

    write!(out, "}}").unwrap();

    Ok(out)
}

// ---------------------------------------------------------------------------
// Enum definition generation
// ---------------------------------------------------------------------------

fn generate_enum_defs(
    analyzed: &AnalyzedQuery,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    use scythe_backend::naming::enum_variant_name;

    let mut out = String::new();
    let mut seen_enums: Vec<String> = Vec::new();

    // Collect enum types from columns and params
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
        if seen_enums.contains(&sql_name.to_string()) {
            continue;
        }
        seen_enums.push(sql_name.to_string());

        let type_name = enum_type_name(sql_name, &manifest.naming);

        if !out.is_empty() {
            writeln!(out).unwrap();
        }

        writeln!(out, "#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]").unwrap();
        writeln!(
            out,
            "#[sqlx(type_name = \"{}\", rename_all = \"snake_case\")]",
            sql_name
        )
        .unwrap();
        writeln!(out, "pub enum {} {{", type_name).unwrap();

        // Use actual enum values from the analyzed query
        if let Some(enum_info) = analyzed.enums.iter().find(|e| e.sql_name == sql_name) {
            for value in &enum_info.values {
                let variant = enum_variant_name(value, &manifest.naming);
                writeln!(out, "    {},", variant).unwrap();
            }
        }

        write!(out, "}}").unwrap();
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Type resolution helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Composite type definition generation
// ---------------------------------------------------------------------------

fn generate_composite_defs(
    composites: &[CompositeInfo],
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    let mut out = String::new();
    for (i, comp) in composites.iter().enumerate() {
        if i > 0 {
            writeln!(out).unwrap();
            writeln!(out).unwrap();
        }
        let struct_name = to_pascal_case(&comp.sql_name).into_owned();
        writeln!(out, "#[derive(Debug, Clone, sqlx::Type)]").unwrap();
        writeln!(out, "#[sqlx(type_name = \"{}\")]", comp.sql_name).unwrap();
        writeln!(out, "pub struct {} {{", struct_name).unwrap();
        for field in &comp.fields {
            let rust_type = resolve_type(&field.neutral_type, manifest, false)
                .map(|t| t.into_owned())
                .map_err(|e| {
                    ScytheError::new(
                        ErrorCode::InternalError,
                        format!("composite field type error: {}", e),
                    )
                })?;
            writeln!(
                out,
                "    pub {}: {},",
                to_snake_case(&field.name),
                rust_type
            )
            .unwrap();
        }
        write!(out, "}}").unwrap();
    }
    Ok(out)
}

/// Rewrite SQL to add enum type annotations for sqlx.
/// For enum columns in SELECT, adds `column AS "column: EnumType"` aliases.
fn rewrite_sql_for_enums(
    sql: &str,
    columns: &[AnalyzedColumn],
    manifest: &BackendManifest,
) -> String {
    // Find enum columns that need annotation
    let enum_cols: Vec<(&str, String)> = columns
        .iter()
        .filter_map(|col| {
            if let Some(enum_name) = col.neutral_type.strip_prefix("enum::") {
                let rust_type = enum_type_name(enum_name, &manifest.naming);
                let annotation = if col.nullable {
                    format!("Option<{}>", rust_type)
                } else {
                    rust_type
                };
                Some((col.name.as_str(), annotation))
            } else {
                None
            }
        })
        .collect();

    if enum_cols.is_empty() {
        return sql.to_string();
    }

    let mut result = sql.to_string();
    for (col_name, annotation) in &enum_cols {
        // Look for bare column reference in SELECT list and add alias
        // Try to find and replace the column name with its annotated version
        // Handle both "column" and "table.column" patterns
        let alias = format!("{} AS \\\"{}: {}\\\"", col_name, col_name, annotation);
        // Simple word-boundary replacement in the SELECT portion
        // Find the SELECT ... FROM boundary
        if let Some(from_pos) = result.to_uppercase().find(" FROM ") {
            let select_part = &result[..from_pos];
            let rest = &result[from_pos..];

            // Replace bare column name (not already aliased)
            let new_select = replace_column_in_select(select_part, col_name, &alias);
            result = format!("{}{}", new_select, rest);
        }
    }
    result
}

/// Replace a bare column name in a SELECT clause with an aliased version.
fn replace_column_in_select(select: &str, col_name: &str, replacement: &str) -> String {
    // Simple approach: find the column name as a standalone word
    let mut result = select.to_string();
    // Check for "column" as a whole word (preceded by comma/space/SELECT and followed by comma/space/FROM)
    let patterns = [format!(", {}", col_name), format!(" {}", col_name)];
    for pattern in &patterns {
        if let Some(pos) = result.rfind(pattern.as_str()) {
            let after = pos + pattern.len();
            // Check that the next char is comma, space, or end of string
            let next_char = result[after..].chars().next();
            if next_char.is_none() || matches!(next_char, Some(' ') | Some(',') | Some('\n')) {
                let prefix = &result[..pos + pattern.len() - col_name.len()];
                let suffix = &result[after..];
                result = format!("{}{}{}", prefix, replacement, suffix);
                break;
            }
        }
    }
    result
}

/// Convert a resolved Rust type to its borrowed form for function parameters.
/// Copy types (primitives) stay as-is; String becomes &str; other non-Copy types get a & prefix.
fn param_type_to_borrowed(rust_type: &str) -> String {
    // Copy types that should stay owned in function params
    let copy_types = ["bool", "i16", "i32", "i64", "f32", "f64", "u64"];
    if copy_types.contains(&rust_type) {
        return rust_type.to_string();
    }
    // String -> &str
    if rust_type == "String" {
        return "&str".to_string();
    }
    // Option<T> wrapping: Option<String> -> Option<&str>, Option<Copy> stays, Option<Other> -> Option<&Other>
    if let Some(inner) = rust_type
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let borrowed_inner = param_type_to_borrowed(inner);
        return format!("Option<{}>", borrowed_inner);
    }
    // Vec<T> -> &[T] (slice reference)
    if let Some(inner) = rust_type
        .strip_prefix("Vec<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return format!("&[{}]", inner);
    }
    // Everything else gets a & prefix
    format!("&{}", rust_type)
}

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
    use crate::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery};
    use crate::parser::QueryCommand;

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
