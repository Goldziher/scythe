use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case, to_snake_case,
};
use scythe_backend::types::resolve_type;
use std::collections::HashMap;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};
use crate::singularize;

use super::python_common::{
    PythonRowType, generate_grouped_fold_positional, generate_grouped_structs_py, type_support_imports,
};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/python-psycopg3.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/python-psycopg3.redshift.toml");

/// Build the `field=value` expression for one column read from a raw
/// positionally-indexed row (`row[i]` / `r[i]`).
///
/// psycopg3 auto-decodes `json`/`jsonb` columns to a plain `dict`/`list` --
/// it has no mechanism to construct an arbitrary dataclass/BaseModel/Struct
/// from that automatically (unlike, e.g., pgx's reflective JSON-unmarshal
/// fallback in Go). A `json_nested<...>` column therefore needs an explicit
/// constructor call here; without it, the field would hold a raw dict typed
/// as the nested class, which "compiles" (Python has no static check) and
/// breaks on first attribute access.
///
/// Keyed strictly on `json_nested`, never on `json_typed`: the latter is a
/// user's own `@json` mapping to a type scythe knows nothing about -- it may
/// be a `TypedDict`, may describe a JSON array, may not be constructible
/// from a mapping at all -- so calling a constructor on it would break code
/// that works today. Every non-nested column keeps the exact `field=row[i]`
/// form this always emitted.
fn field_assignment_expr(col: &ResolvedColumn, var: &str, index: usize) -> String {
    let raw = format!("{var}[{index}]");
    let Some(shape) = crate::nested_struct_shape(&col.neutral_type) else {
        return format!("{}={raw}", col.field_name);
    };
    let name = shape.name;
    // ~keep `_from_json` rather than `Cls(**item)`: the JSON keys are the raw SQL
    // column names, which need not match the snake_case Python field names
    // (`"createdAt"` -> `created_at`), and `**` on a mismatched key raises
    // `TypeError: unexpected keyword argument`. The classmethod holds the
    // key mapping next to the field declarations it belongs with.
    let ctor = if shape.is_array {
        let element = if shape.element_nullable {
            format!("None if item is None else {name}._from_json(item)")
        } else {
            format!("{name}._from_json(item)")
        };
        format!("[{element} for item in {raw}]")
    } else {
        format!("{name}._from_json({raw})")
    };
    if col.nullable {
        format!("{}=None if {raw} is None else {ctor}", col.field_name)
    } else {
        format!("{}={ctor}", col.field_name)
    }
}

pub struct PythonPsycopg3Backend {
    manifest: BackendManifest,
    row_type: PythonRowType,
    /// Whether this engine's manifest declares the `json_nested` container
    /// and its server actually has `json_agg`. See
    /// [`crate::backends::engine_supports_nested_aggregates`].
    nested_aggregates: bool,
}

impl PythonPsycopg3Backend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_TOML,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!(
                        "python-psycopg3 only supports PostgreSQL/Redshift, got engine '{}'",
                        engine
                    ),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
        Ok(Self {
            manifest,
            row_type: PythonRowType::default(),
            nested_aggregates: super::engine_supports_nested_aggregates(engine),
        })
    }
}

impl CodegenBackend for PythonPsycopg3Backend {
    fn name(&self) -> &str {
        "python-psycopg3"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "redshift"]
    }

    fn apply_options(&mut self, options: &HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(&["row_type"], options)?;

        if let Some(rt) = options.get("row_type") {
            self.row_type = PythonRowType::from_option(rt)?;
        }
        Ok(())
    }

    fn file_header(&self) -> String {
        let import_line = self.row_type.import_line();
        let (needs_uuid, needs_any) = type_support_imports(&self.manifest);
        let uuid_line = if needs_uuid { "import uuid  # noqa: F401\n" } else { "" };
        let any_line = if needs_any {
            "from typing import Any  # noqa: F401\n"
        } else {
            ""
        };
        if self.row_type.is_stdlib_import() {
            format!(
                "import datetime  # noqa: F401\n\
                 import decimal  # noqa: F401\n\
                 {uuid_line}\
                 {import_line}\n\
                 from enum import Enum  # noqa: F401\n\
                 {any_line}\
                 \n\
                 from psycopg import AsyncConnection  # noqa: F401\n\
                 \n",
            )
        } else {
            let third_party = self
                .row_type
                .sorted_third_party_imports("from psycopg import AsyncConnection  # noqa: F401");
            format!(
                "import datetime  # noqa: F401\n\
                 import decimal  # noqa: F401\n\
                 {uuid_line}\
                 from enum import Enum  # noqa: F401\n\
                 {any_line}\
                 \n\
                 {third_party}\n\
                 \n",
            )
        }
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = write!(out, "{}", self.row_type.decorator());
        let _ = writeln!(out, "{}", self.row_type.class_def(&struct_name));
        let _ = writeln!(out, "    \"\"\"Row type for {} query.\"\"\"", query_name);
        if columns.is_empty() {
            let _ = writeln!(out, "    pass");
        } else {
            let _ = writeln!(out);
            for col in columns {
                let _ = writeln!(out, "    {}: {}", col.field_name, col.full_type);
            }
        }
        Ok(out)
    }

    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let singular = singularize(table_name);
        let name = to_pascal_case(&singular);
        self.generate_row_struct(&name, columns)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let kw_sep = if param_list.is_empty() { "" } else { ", *, " };

        let sql_clean = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let name_map: std::collections::HashMap<u32, String> = analyzed
            .params
            .iter()
            .map(|p| (p.position as u32, to_snake_case(&p.name).into_owned()))
            .collect();
        let sql = crate::sql_literal::escape_python_triple_double(&super::rewrite_pg_placeholders(&sql_clean, |n| {
            format!("%({})s", name_map.get(&n).map_or("?", |s| s.as_str()))
        }));

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(
                    out,
                    "async def {}(conn: AsyncConnection{}{}) -> {} | None:",
                    func_name, kw_sep, param_list, struct_name
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                if params.is_empty() {
                    let _ = writeln!(out, "    cur = await conn.execute(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "    )");
                } else {
                    let dict_entries: Vec<String> = params
                        .iter()
                        .map(|p| format!("\"{}\": {}", p.field_name, p.field_name))
                        .collect();
                    let _ = writeln!(out, "    cur = await conn.execute(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "        {{{}}},", dict_entries.join(", "));
                    let _ = writeln!(out, "    )");
                }
                let _ = writeln!(out, "    row = await cur.fetchone()");
                let _ = writeln!(out, "    if row is None:");
                let _ = writeln!(out, "        return None");
                let field_assignments: Vec<String> = columns
                    .iter()
                    .enumerate()
                    .map(|(i, col)| field_assignment_expr(col, "row", i))
                    .collect();
                let oneliner = format!("    return {}({})", struct_name, field_assignments.join(", "));
                if oneliner.len() <= 88 {
                    let _ = writeln!(out, "{}", oneliner);
                } else {
                    let _ = writeln!(out, "    return {}(", struct_name);
                    for fa in &field_assignments {
                        let _ = writeln!(out, "        {},", fa);
                    }
                    let _ = writeln!(out, "    )");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "async def {}(conn: AsyncConnection{}{}) -> list[{}]:",
                    func_name, kw_sep, param_list, struct_name
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                if params.is_empty() {
                    let _ = writeln!(out, "    cur = await conn.execute(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "    )");
                } else {
                    let dict_entries: Vec<String> = params
                        .iter()
                        .map(|p| format!("\"{}\": {}", p.field_name, p.field_name))
                        .collect();
                    let _ = writeln!(out, "    cur = await conn.execute(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "        {{{}}},", dict_entries.join(", "));
                    let _ = writeln!(out, "    )");
                }
                let _ = writeln!(out, "    rows = await cur.fetchall()");
                let field_assignments: Vec<String> = columns
                    .iter()
                    .enumerate()
                    .map(|(i, col)| field_assignment_expr(col, "r", i))
                    .collect();
                let oneliner = format!(
                    "    return [{}({}) for r in rows]",
                    struct_name,
                    field_assignments.join(", ")
                );
                if oneliner.len() <= 88 {
                    let _ = writeln!(out, "{}", oneliner);
                } else {
                    let _ = writeln!(out, "    return [");
                    let _ = writeln!(out, "        {}(", struct_name);
                    for fa in &field_assignments {
                        let _ = writeln!(out, "            {},", fa);
                    }
                    let _ = writeln!(out, "        )");
                    let _ = writeln!(out, "        for r in rows");
                    let _ = writeln!(out, "    ]");
                }
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let items_type = if params.len() > 1 {
                    let tuple_types: Vec<String> = params.iter().map(|p| p.full_type.clone()).collect();
                    format!("list[tuple[{}]]", tuple_types.join(", "))
                } else if params.len() == 1 {
                    format!("list[{}]", params[0].full_type)
                } else {
                    "int".to_string()
                };
                let items_or_count = if params.is_empty() { "count" } else { "items" };
                let _ = writeln!(
                    out,
                    "async def {}(conn: AsyncConnection, *, {}: {}) -> None:",
                    batch_fn_name, items_or_count, items_type
                );
                let _ = writeln!(
                    out,
                    "    \"\"\"Execute {} query for each item in the batch.\"\"\"",
                    analyzed.name
                );
                if params.is_empty() {
                    let _ = writeln!(out, "    for _ in range(count):");
                    let _ = writeln!(out, "        await conn.execute(");
                    let _ = writeln!(out, "            \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "        )");
                } else {
                    let dict_entries: Vec<String> = params
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            if params.len() == 1 {
                                format!("\"{}\": item", p.field_name)
                            } else {
                                format!("\"{}\": item[{}]", p.field_name, i)
                            }
                        })
                        .collect();
                    let _ = writeln!(
                        out,
                        "    params_list = [{{{dict}}} for item in items]",
                        dict = dict_entries.join(", ")
                    );
                    let _ = writeln!(out, "    cur = conn.cursor()");
                    let _ = writeln!(out, "    await cur.executemany(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "        params_list,");
                    let _ = writeln!(out, "    )");
                }
            }
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "async def {}(conn: AsyncConnection{}{}) -> None:",
                    func_name, kw_sep, param_list
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                if params.is_empty() {
                    let _ = writeln!(out, "    await conn.execute(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "    )");
                } else {
                    let dict_entries: Vec<String> = params
                        .iter()
                        .map(|p| format!("\"{}\": {}", p.field_name, p.field_name))
                        .collect();
                    let _ = writeln!(out, "    await conn.execute(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "        {{{}}},", dict_entries.join(", "));
                    let _ = writeln!(out, "    )");
                }
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "async def {}(conn: AsyncConnection{}{}) -> int:",
                    func_name, kw_sep, param_list
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                if params.is_empty() {
                    let _ = writeln!(out, "    cur = await conn.execute(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "    )");
                } else {
                    let dict_entries: Vec<String> = params
                        .iter()
                        .map(|p| format!("\"{}\": {}", p.field_name, p.field_name))
                        .collect();
                    let _ = writeln!(out, "    cur = await conn.execute(");
                    let _ = writeln!(out, "        \"\"\"{}\"\"\",", sql);
                    let _ = writeln!(out, "        {{{}}},", dict_entries.join(", "));
                    let _ = writeln!(out, "    )");
                }
                let _ = writeln!(out, "    return cur.rowcount");
            }
            QueryCommand::Grouped => {
                unreachable!("grouped queries are routed to generate_grouped_query_fn")
            }
        }

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "class {}(str, Enum):", type_name);
        let _ = writeln!(out, "    \"\"\"Database enum type {}.\"\"\"", enum_info.sql_name);
        if enum_info.values.is_empty() {
            let _ = writeln!(out, "    pass");
        } else {
            let _ = writeln!(out);
            for value in &enum_info.values {
                let variant = enum_variant_name(value, &self.manifest.naming);
                let _ = writeln!(out, "    {} = \"{}\"", variant, value);
            }
        }
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = write!(out, "{}", self.row_type.decorator());
        let _ = writeln!(out, "{}", self.row_type.class_def(&name));
        let _ = writeln!(out, "    \"\"\"Composite type {}.\"\"\"", composite.sql_name);
        if composite.fields.is_empty() {
            let _ = writeln!(out, "    pass");
        } else {
            let _ = writeln!(out);
            for field in &composite.fields {
                let py_type = resolve_type(&field.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .map_err(|e| {
                        ScytheError::new(ErrorCode::InternalError, format!("composite field type error: {}", e))
                    })?;
                let _ = writeln!(out, "    {}: {}", to_snake_case(&field.name), py_type);
            }
        }
        Ok(out)
    }

    fn generate_nested_struct_def(
        &self,
        nested: &scythe_core::analyzer::NestedStructInfo,
    ) -> Result<Option<String>, ScytheError> {
        if !self.nested_aggregates {
            return Ok(None);
        }

        // Unlike generate_composite_def (always `false` -- CompositeFieldInfo
        // has no per-field nullability), a nested-aggregate field's
        // nullability is real and comes from the source column it was
        // built from.
        let name = to_pascal_case(&nested.name);
        let mut out = String::new();
        let _ = write!(out, "{}", self.row_type.decorator());
        let _ = writeln!(out, "{}", self.row_type.class_def(&name));
        let _ = writeln!(out, "    \"\"\"Nested struct for {}.\"\"\"", nested.name);
        if nested.fields.is_empty() {
            let _ = writeln!(out, "    pass");
            return Ok(Some(out));
        }

        let _ = writeln!(out);
        let mut field_names: Vec<String> = Vec::with_capacity(nested.fields.len());
        for field in &nested.fields {
            let py_type = resolve_type(&field.neutral_type, &self.manifest, field.nullable)
                .map(|t| t.into_owned())
                .map_err(|e| {
                    ScytheError::new(
                        ErrorCode::InternalError,
                        format!("nested struct field type error: {}", e),
                    )
                })?;
            let field_name = to_snake_case(&field.name).into_owned();
            let _ = writeln!(out, "    {}: {}", field_name, py_type);
            field_names.push(field_name);
        }

        // ~keep The JSON keys json_agg/row_to_json produce are the raw SQL column
        // names, verbatim -- a quoted "createdAt" column is the key
        // "createdAt", which `Cls(**item)` would pass as an unexpected
        // keyword argument. Emitting the mapping as a classmethod keeps it
        // beside the field declarations rather than duplicated into every
        // query function that builds one, and works identically for
        // dataclass, pydantic and msgspec row types.
        let _ = writeln!(out);
        let _ = writeln!(out, "    @classmethod");
        let _ = writeln!(out, "    def _from_json(cls, obj: dict[str, Any]) -> \"{}\":", name);
        let _ = writeln!(out, "        \"\"\"Build from one decoded JSON object.\"\"\"");
        let _ = writeln!(out, "        return cls(");
        for (field, field_name) in nested.fields.iter().zip(&field_names) {
            let _ = writeln!(out, "            {}=obj[\"{}\"],", field_name, field.name);
        }
        let _ = writeln!(out, "        )");
        Ok(Some(out))
    }

    fn generate_grouped_structs(
        &self,
        parent_struct_name: &str,
        child_struct_name: &str,
        parent_columns: &[ResolvedColumn],
        child_columns: &[ResolvedColumn],
        _key_column: &str,
    ) -> Result<String, ScytheError> {
        Ok(generate_grouped_structs_py(
            self.row_type,
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
        ))
    }

    fn generate_grouped_query_fn(&self, request: &GroupedQueryFn<'_>) -> Result<String, ScytheError> {
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let all_columns = request.all_columns;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let kw_sep = if param_list.is_empty() { "" } else { ", *, " };

        let sql_clean = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let name_map: std::collections::HashMap<u32, String> = analyzed
            .params
            .iter()
            .map(|p| (p.position as u32, to_snake_case(&p.name).into_owned()))
            .collect();
        let sql = crate::sql_literal::escape_python_triple_double(&super::rewrite_pg_placeholders(&sql_clean, |n| {
            format!("%({})s", name_map.get(&n).map_or("?", |s| s.as_str()))
        }));

        let sig =
            format!("async def {func_name}(conn: AsyncConnection{kw_sep}{param_list}) -> list[{parent_struct_name}]:");
        if sig.len() <= 88 {
            let _ = writeln!(out, "{sig}");
        } else {
            let _ = writeln!(out, "async def {func_name}(");
            let _ = writeln!(out, "    conn: AsyncConnection,");
            if !params.is_empty() {
                let _ = writeln!(out, "    *,");
                for p in params {
                    let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                }
            }
            let _ = writeln!(out, ") -> list[{parent_struct_name}]:");
        }
        let _ = writeln!(out, "    \"\"\"Execute {} grouped query.\"\"\"", analyzed.name);

        let _ = writeln!(out, "    cur = await conn.execute(");
        let _ = writeln!(out, "        \"\"\"");
        for line in sql.lines() {
            let _ = writeln!(out, "{line}");
        }
        let _ = writeln!(out, "\"\"\",");
        if !params.is_empty() {
            let dict_entries: Vec<String> = params
                .iter()
                .map(|p| format!("\"{}\": {}", p.field_name, p.field_name))
                .collect();
            let _ = writeln!(out, "        {{{}}},", dict_entries.join(", "));
        }
        let _ = writeln!(out, "    )");
        let _ = writeln!(out, "    rows = await cur.fetchall()");

        generate_grouped_fold_positional(
            &mut out,
            all_columns,
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
        );

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

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
        AnalyzedQuery::build(|aq| {
            aq.name = "GetUsersWithOrders".to_string();
            aq.command = QueryCommand::Grouped;
            aq.sql = "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\nFROM users u\nJOIN orders o ON o.user_id = u.id"
                .to_string();
            aq.columns = all_cols;
            aq.params = vec![];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: parent_cols,
                child_columns: child_cols,
            });
            aq.custom = vec![];
        })
    }

    #[test]
    fn test_grouped_python_psycopg3_structs() {
        let backend = PythonPsycopg3Backend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();
        assert!(
            row_struct.contains("class GetUsersWithOrdersChildRow"),
            "missing child class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("class GetUsersWithOrdersRow"),
            "missing parent class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("children: list[GetUsersWithOrdersChildRow]"),
            "parent class missing children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("class GetUsersWithOrdersRow").unwrap();
        assert!(child_pos < parent_pos, "child class must appear before parent class");
    }

    #[test]
    fn test_grouped_python_psycopg3_query_fn() {
        let backend = PythonPsycopg3Backend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("async def get_users_with_orders"),
            "missing async fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("list[GetUsersWithOrdersRow]"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("await conn.execute("),
            "fn must use await conn.execute; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow("),
            "fn must construct child class; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersRow(**parent_kwargs, children=children)"),
            "fn must construct parent; got:\n{query_fn}"
        );
        assert!(query_fn.contains("_index"), "fn must use _index dict; got:\n{query_fn}");
        for line in query_fn.lines() {
            assert!(
                line.len() <= 88,
                "ruff E501: line exceeds 88 chars ({} chars):\n{line}",
                line.len()
            );
        }
    }

    /// #103: before this, python-psycopg3 inherited the `CodegenBackend`
    /// default `apply_options` (`Ok(())` for any map), so an unrecognized key
    /// was silently discarded here while the same typo was a hard error on
    /// every TypeScript backend. The trait default now rejects every key
    /// unless a backend declares it known.
    #[test]
    fn test_apply_options_rejects_unknown_key_with_invalid_config() {
        let mut backend = PythonPsycopg3Backend::new("postgresql").unwrap();
        let err = backend
            .apply_options(&HashMap::from([("row_typ".to_string(), "pydantic".to_string())]))
            .expect_err("row_typ is not a known python-psycopg3 option");
        assert_eq!(err.code, ErrorCode::InvalidConfig);
        assert!(err.message.contains("row_typ"), "{}", err.message);
        assert!(
            err.message.contains("row_type"),
            "error should list the real option: {}",
            err.message
        );
    }

    /// Regression guard: a real, known key must keep working -- the risk
    /// with inverting the trait default is a false positive that breaks
    /// every existing user's config.
    #[test]
    fn test_apply_options_accepts_known_key() {
        let mut backend = PythonPsycopg3Backend::new("postgresql").unwrap();
        backend
            .apply_options(&HashMap::from([("row_type".to_string(), "pydantic".to_string())]))
            .expect("row_type is a known python-psycopg3 option");
    }
}
