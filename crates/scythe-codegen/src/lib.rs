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

use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, EnumInfo, NestedStructInfo};
use scythe_core::catalog::Catalog;
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

#[derive(Debug, Clone, Default)]
pub struct GeneratedCode {
    pub query_fn: Option<String>,
    pub row_struct: Option<String>,
    pub model_struct: Option<String>,
    pub enum_def: Option<String>,
}

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

    // Degradation pass: must run before any resolve_columns call. A backend
    // that doesn't opt into a nested struct (generate_nested_struct_def
    // returns Ok(None)) never sees json_typed<...> referencing it -- the
    // column is rewritten to plain json first, matching this backend's
    // output from before nested-aggregate inference existed byte for byte.
    //
    // Skipped entirely when nested_structs is empty (the overwhelming
    // majority of queries) so the common path stays the zero-copy
    // `&analyzed.columns` it always was -- degrade_unsupported_nested_structs
    // would otherwise clone every column, String fields included, on every
    // call regardless of whether the feature is in use.
    let (degraded_columns, nested_struct_defs) = if analyzed.nested_structs.is_empty() {
        (None, String::new())
    } else {
        let (cols, defs) = degrade_unsupported_nested_structs(&analyzed.columns, &analyzed.nested_structs, backend)?;
        (Some(cols), defs)
    };

    let columns = resolve::resolve_columns(
        degraded_columns.as_deref().unwrap_or(&analyzed.columns),
        manifest,
        overrides,
        source_table,
    )?;
    let params = resolve::resolve_params(&analyzed.params, manifest, overrides, source_table)?;

    let mut result = GeneratedCode::default();

    let enum_def = generate_enum_defs_via_backend(analyzed, backend)?;
    if !enum_def.is_empty() {
        result.enum_def = Some(enum_def);
    }

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

    let mut extra_defs = String::new();
    if !analyzed.composites.is_empty() {
        for (i, comp) in analyzed.composites.iter().enumerate() {
            if i > 0 {
                extra_defs.push_str("\n\n");
            }
            extra_defs.push_str(&backend.generate_composite_def(comp)?);
        }
    }
    if !nested_struct_defs.is_empty() {
        if !extra_defs.is_empty() {
            extra_defs.push_str("\n\n");
        }
        extra_defs.push_str(&nested_struct_defs);
    }
    if !extra_defs.is_empty() {
        if let Some(ref mut existing) = result.model_struct {
            existing.push_str("\n\n");
            existing.push_str(&extra_defs);
        } else {
            result.model_struct = Some(extra_defs);
        }
    }

    let struct_name = determine_struct_name(analyzed, manifest);

    if analyzed.command == QueryCommand::Grouped {
        let group_by = analyzed.group_by.as_ref().ok_or_else(|| {
            ScytheError::new(
                ErrorCode::InternalError,
                format!(
                    "query '{}' is :grouped but is missing @group_by annotation",
                    analyzed.name
                ),
            )
        })?;

        let parent_struct_name = scythe_backend::naming::row_struct_name(&analyzed.name, &manifest.naming);
        let child_struct_name = {
            let suffix = &manifest.naming.row_suffix;
            let base = parent_struct_name.trim_end_matches(suffix.as_str());
            format!("{}Child{}", base, suffix)
        };

        // Same degradation pass as the flat-column path above, and the same
        // empty-nested_structs skip to keep the common case zero-copy: a
        // :grouped query's parent/child split is a distinct copy of the
        // analyzed columns, so an unsupported nested reference there needs
        // its own rewrite before resolution -- but only when there is one.
        let (degraded_parent, degraded_child) = if analyzed.nested_structs.is_empty() {
            (None, None)
        } else {
            let (dp, _) =
                degrade_unsupported_nested_structs(&group_by.parent_columns, &analyzed.nested_structs, backend)?;
            let (dc, _) =
                degrade_unsupported_nested_structs(&group_by.child_columns, &analyzed.nested_structs, backend)?;
            (Some(dp), Some(dc))
        };
        let parent_cols = resolve::resolve_columns(
            degraded_parent.as_deref().unwrap_or(&group_by.parent_columns),
            manifest,
            overrides,
            source_table,
        )?;
        let child_cols = resolve::resolve_columns(
            degraded_child.as_deref().unwrap_or(&group_by.child_columns),
            manifest,
            overrides,
            source_table,
        )?;

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
            let stub_info = EnumInfo {
                sql_name: sql_name.to_string(),
                values: vec![],
            };
            out.push_str(&backend.generate_enum_def(&stub_info)?);
        }
    }

    Ok(out)
}

/// Extract the shape of a `json_typed<...>` neutral type: whether it's
/// `array`-wrapped (`json_agg`, a list of objects) or bare (`row_to_json`, a
/// single object -- or a user `@json` mapping, which is also bare), and the
/// PascalCase struct name it references. Returns `None` for anything that
/// isn't shaped like `json_typed<...>` at all, so callers can skip ordinary
/// columns cheaply.
///
/// `"json_typed<array<GetUserPostsRowPosts>>"` -> `Some((true, "GetUserPostsRowPosts"))`
/// `"json_typed<GetPostAsJsonRowPost>"` -> `Some((false, "GetPostAsJsonRowPost"))`
/// `"json_typed<EventData>"` (a user `@json` mapping, not ours) -> `Some((false, "EventData"))`
/// `"int32"`, `"json"`, `"array<int32>"` -> `None`
///
/// `pub(crate)`: reused by backend row-construction code (e.g.
/// `python_psycopg3`) that must build a nested value from a raw
/// dict/list the driver hands back, not just the type-resolution layer.
pub(crate) fn nested_struct_shape(neutral_type: &str) -> Option<(bool, &str)> {
    let inner = neutral_type.strip_prefix("json_typed<")?.strip_suffix('>')?;
    match inner.strip_prefix("array<") {
        Some(rest) => rest.strip_suffix('>').map(|name| (true, name)),
        None => Some((false, inner)),
    }
}

/// `nested_struct_shape` without the array/bare distinction, for callers
/// (the degradation pass) that only need the name.
fn nested_struct_pascal_name(neutral_type: &str) -> Option<&str> {
    nested_struct_shape(neutral_type).map(|(_, name)| name)
}

/// Degradation pass for nested-aggregate columns (`json_agg(o.*)`,
/// `row_to_json(u.*)`). Must run before `resolve::resolve_columns` -- every
/// caller that resolves `AnalyzedColumn`s into a backend's types needs this,
/// not just [`generate_with_backend_and_overrides`]: `scythe-cli`'s RBS
/// signature generation (`generate_rbs_if_supported`) calls
/// `resolve::resolve_columns` directly on a second, independent path, so it
/// must run this pass too or it silently bypasses the byte-identity
/// guarantee for any backend that hasn't opted in.
///
/// For each entry in `nested_structs`, asks the backend whether it opts in
/// via [`CodegenBackend::generate_nested_struct_def`]:
/// - `Ok(Some(def))`: the struct is supported. Its definition is collected
///   to append to the backend's output (the same channel
///   `generate_composite_def` output goes through), and columns
///   referencing it are left as `json_typed<...>`.
/// - `Ok(None)` (the default): not supported. Every column referencing it
///   is rewritten to plain `json` -- byte-identical to this backend's
///   output before nested-aggregate inference existed, since that is
///   exactly what the pre-existing `json_agg`/`row_to_json` arms already
///   produced.
/// - `Err(_)`: a genuine failure, propagated rather than degraded.
///
/// Returns the (possibly rewritten) columns and the concatenated
/// definitions for every struct the backend supports. Skip calling this
/// entirely when `nested_structs` is empty (the common case) to keep that
/// path zero-copy -- see the callers in this file for the pattern.
pub fn degrade_unsupported_nested_structs(
    columns: &[AnalyzedColumn],
    nested_structs: &[NestedStructInfo],
    backend: &dyn CodegenBackend,
) -> Result<(Vec<AnalyzedColumn>, String), ScytheError> {
    if nested_structs.is_empty() {
        return Ok((columns.to_vec(), String::new()));
    }

    use ahash::AHashSet;

    let mut unsupported: AHashSet<String> = AHashSet::new();
    let mut defs = String::new();
    for nested in nested_structs {
        match backend.generate_nested_struct_def(nested)? {
            Some(def) => {
                if !defs.is_empty() {
                    defs.push_str("\n\n");
                }
                defs.push_str(&def);
            }
            None => {
                unsupported.insert(to_pascal_case(&nested.name).into_owned());
            }
        }
    }

    if unsupported.is_empty() {
        return Ok((columns.to_vec(), defs));
    }

    let degraded = columns
        .iter()
        .cloned()
        .map(|mut col| {
            if let Some(name) = nested_struct_pascal_name(&col.neutral_type)
                && unsupported.contains(name)
            {
                col.neutral_type = "json".to_string();
            }
            col
        })
        .collect();

    Ok((degraded, defs))
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

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, GroupByConfig, NestedFieldInfo};
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
            ..Default::default()
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
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "email".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    ..Default::default()
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
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
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
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "status".to_string(),
                    neutral_type: "enum::user_status".to_string(),
                    nullable: false,
                    ..Default::default()
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

    fn make_grouped_query() -> AnalyzedQuery {
        let parent_cols = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "email".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
        let child_cols = vec![
            AnalyzedColumn {
                name: "order_id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "total".to_string(),
                neutral_type: "decimal".to_string(),
                nullable: true,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "order_date".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: false,
                ..Default::default()
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
            ..Default::default()
        }
    }

    #[test]
    fn test_grouped_sqlx_structs() {
        let backend = get_backend("rust-sqlx", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

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
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("pub struct GetUsersWithOrdersRow").unwrap();
        assert!(
            child_pos < parent_pos,
            "child struct must be defined before parent struct"
        );

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
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
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
                ..Default::default()
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
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    ..Default::default()
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

        assert!(query_fn.contains("pub async fn get_user("));
        assert!(query_fn.contains("tokio_postgres::GenericClient"));
        assert!(query_fn.contains("GetUserRow"));

        assert!(query_fn.contains("SELECT id, name FROM users"));
    }

    // -----------------------------------------------------------------
    // Nested-aggregate degradation pass (degrade_unsupported_nested_structs).
    // -----------------------------------------------------------------

    fn a_nested_struct() -> NestedStructInfo {
        NestedStructInfo {
            name: "get_user_posts_row_posts".to_string(),
            fields: vec![NestedFieldInfo {
                name: "title".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            }],
        }
    }

    #[test]
    fn test_nested_struct_pascal_name_unwraps_array() {
        assert_eq!(
            nested_struct_pascal_name("json_typed<array<GetUserPostsRowPosts>>"),
            Some("GetUserPostsRowPosts")
        );
    }

    #[test]
    fn test_nested_struct_pascal_name_bare() {
        assert_eq!(
            nested_struct_pascal_name("json_typed<GetPostAsJsonRowPost>"),
            Some("GetPostAsJsonRowPost")
        );
    }

    #[test]
    fn test_nested_struct_pascal_name_user_json_mapping_still_matches() {
        // A user's own @json rust_type mapping (json_mappings in the
        // analyzer) produces the exact same json_typed<Name> shape -- the
        // parser can't and doesn't need to distinguish the two, since a
        // user-declared type is never in AnalyzedQuery::nested_structs and
        // so is never a member of the `unsupported` set either.
        assert_eq!(nested_struct_pascal_name("json_typed<EventData>"), Some("EventData"));
    }

    #[test]
    fn test_nested_struct_pascal_name_none_for_ordinary_types() {
        assert_eq!(nested_struct_pascal_name("int32"), None);
        assert_eq!(nested_struct_pascal_name("json"), None);
        assert_eq!(nested_struct_pascal_name("array<int32>"), None);
    }

    #[test]
    fn test_degrade_unsupported_nested_structs_rewrites_to_plain_json() {
        let manifest = get_backend("rust-sqlx", "postgresql").unwrap().manifest().clone();
        let backend = StubBackend { manifest };
        let nested = a_nested_struct();
        let columns = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "posts".to_string(),
                neutral_type: "json_typed<array<GetUserPostsRowPosts>>".to_string(),
                nullable: true,
                ..Default::default()
            },
        ];

        let (degraded, defs) = degrade_unsupported_nested_structs(&columns, &[nested], &backend).unwrap();

        assert_eq!(
            degraded[0].neutral_type, "int32",
            "an unrelated column must be untouched"
        );
        assert_eq!(
            degraded[1].neutral_type, "json",
            "StubBackend does not override generate_nested_struct_def, so it must degrade to plain json"
        );
        assert!(
            defs.is_empty(),
            "an unsupported backend must not emit a struct definition"
        );
    }

    /// Minimal backend that opts into nested-struct support, to prove the
    /// degradation pass leaves a supported column and definition alone.
    struct NestedSupportingBackend {
        manifest: BackendManifest,
    }

    impl CodegenBackend for NestedSupportingBackend {
        fn name(&self) -> &str {
            "nested-supporting-backend"
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
        fn generate_nested_struct_def(&self, nested: &NestedStructInfo) -> Result<Option<String>, ScytheError> {
            Ok(Some(format!("struct {} {{}}", to_pascal_case(&nested.name))))
        }
    }

    #[test]
    fn test_degrade_unsupported_nested_structs_keeps_supported_column_and_collects_def() {
        let manifest = get_backend("rust-sqlx", "postgresql").unwrap().manifest().clone();
        let backend = NestedSupportingBackend { manifest };
        let nested = a_nested_struct();
        let columns = vec![AnalyzedColumn {
            name: "posts".to_string(),
            neutral_type: "json_typed<array<GetUserPostsRowPosts>>".to_string(),
            nullable: true,
            ..Default::default()
        }];

        let (degraded, defs) = degrade_unsupported_nested_structs(&columns, &[nested], &backend).unwrap();

        assert_eq!(
            degraded[0].neutral_type, "json_typed<array<GetUserPostsRowPosts>>",
            "a supported backend must leave the nested reference untouched"
        );
        assert_eq!(defs, "struct GetUserPostsRowPosts {}");
    }

    /// The review check for this batch: for a backend that does not opt in,
    /// generated output for a nested-aggregate column must be byte-identical
    /// to what the same backend produces for an ordinary plain-`json` column
    /// -- proving the degradation pass, not a per-backend special case,
    /// is what keeps ~44 non-opted-in backends safe.
    #[test]
    fn test_unopted_backend_output_is_byte_identical_to_plain_json_baseline() {
        let backend = get_backend("java-jdbc", "postgresql").unwrap();

        let baseline = make_query(
            "GetUserPosts",
            QueryCommand::Many,
            "SELECT json_agg(p.*) AS posts FROM users u JOIN posts p ON u.id = p.user_id",
            vec![AnalyzedColumn {
                name: "posts".to_string(),
                neutral_type: "json".to_string(),
                nullable: true,
                ..Default::default()
            }],
            vec![],
        );

        let mut nested_query = baseline.clone();
        nested_query.columns[0].neutral_type = "json_typed<array<GetUserPostsRowPosts>>".to_string();
        nested_query.nested_structs = vec![a_nested_struct()];

        let baseline_result = generate_with_backend(&baseline, &*backend).unwrap();
        let nested_result = generate_with_backend(&nested_query, &*backend).unwrap();

        assert_eq!(
            baseline_result.row_struct, nested_result.row_struct,
            "row struct output must match byte for byte"
        );
        assert_eq!(
            baseline_result.query_fn, nested_result.query_fn,
            "query fn output must match byte for byte"
        );
        assert_eq!(baseline_result.model_struct, nested_result.model_struct);
    }
}
