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

use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};
use crate::singularize;

use super::python_common::{PythonRowType, generate_grouped_fold_positional, generate_grouped_structs_py};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/python-duckdb.toml");

pub struct PythonDuckdbBackend {
    manifest: BackendManifest,
    row_type: PythonRowType,
}

impl PythonDuckdbBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "duckdb" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("python-duckdb only supports DuckDB, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::load_or_default_manifest("backends/python-duckdb/manifest.toml", DEFAULT_MANIFEST_TOML)?;
        Ok(Self {
            manifest,
            row_type: PythonRowType::default(),
        })
    }

    /// Emit a DuckDB `.execute(…)` call in multi-line form.
    ///
    /// All generated lines stay ≤ 88 characters (ruff E501 compliance).
    /// When the SQL is a single short line, it is placed inline:
    ///
    /// ```text
    ///     conn.execute(
    ///         """SQL""",
    ///         [args],
    ///     )
    /// ```
    ///
    /// When the first SQL line would exceed 88 characters after quoting and
    /// indentation, the triple-quote starts on its own line so the SQL content
    /// appears at column 0:
    ///
    /// ```text
    ///     conn.execute(
    ///         """
    /// SQL LINE 1
    /// SQL LINE 2
    /// """,
    ///         [args],
    ///     )
    /// ```
    ///
    /// The closing `)` of the call is always emitted; callers may chain
    /// `.fetchone()` / `.fetchall()` by appending before calling this.
    fn emit_execute_call(out: &mut String, sql: &str, args: Option<&str>) {
        let _ = writeln!(out, "    conn.execute(");
        Self::emit_sql_block(out, sql, 8);
        if let Some(a) = args {
            let _ = writeln!(out, "        {a},");
        }
        let _ = writeln!(out, "    )");
    }

    /// Emit `conn.execute(…).fetchall()` assigned to `rows`.
    fn emit_fetchall(out: &mut String, sql: &str, args: Option<&str>) {
        let _ = writeln!(out, "    rows = conn.execute(");
        Self::emit_sql_block(out, sql, 8);
        if let Some(a) = args {
            let _ = writeln!(out, "        {a},");
        }
        let _ = writeln!(out, "    ).fetchall()");
    }

    /// Emit `_res = conn.execute(…)` so the caller can chain `.fetchone()`.
    fn emit_execute_to_res(out: &mut String, sql: &str, args: Option<&str>) {
        let _ = writeln!(out, "    _res = conn.execute(");
        Self::emit_sql_block(out, sql, 8);
        if let Some(a) = args {
            let _ = writeln!(out, "        {a},");
        }
        let _ = writeln!(out, "    )");
    }

    /// Write the SQL triple-quoted string block at the given indentation level.
    ///
    /// Uses inline `"""SQL"""` when it fits within 88 characters;
    /// otherwise uses the newline-first multi-line format.
    fn emit_sql_block(out: &mut String, sql: &str, indent: usize) {
        let pad = " ".repeat(indent);
        let first_line = sql.lines().next().unwrap_or("");
        let sql_is_single = !sql.contains('\n');
        // indent + `"""` + sql + `""",`  = indent + 3 + len + 3 + 1
        let inline_fits = sql_is_single && (indent + 3 + first_line.len() + 3 + 1) <= 88;

        if inline_fits {
            let _ = writeln!(out, "{pad}\"\"\"{sql}\"\"\",");
        } else {
            let _ = writeln!(out, "{pad}\"\"\"");
            for line in sql.lines() {
                let _ = writeln!(out, "{line}");
            }
            let _ = writeln!(out, "\"\"\",");
        }
    }
}

impl CodegenBackend for PythonDuckdbBackend {
    fn name(&self) -> &str {
        "python-duckdb"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["duckdb"]
    }

    fn apply_options(&mut self, options: &HashMap<String, String>) -> Result<(), ScytheError> {
        if let Some(rt) = options.get("row_type") {
            self.row_type = PythonRowType::from_option(rt)?;
        }
        Ok(())
    }

    fn file_header(&self) -> String {
        let import_line = self.row_type.import_line();
        if self.row_type.is_stdlib_import() {
            format!(
                "\"\"\"Auto-generated by scythe. Do not edit.\"\"\"\n\
                 \n\
                 import datetime  # noqa: F401\n\
                 import decimal  # noqa: F401\n\
                 {import_line}\n\
                 from enum import Enum  # noqa: F401\n\
                 \n\
                 import duckdb  # noqa: F401\n\
                 \n",
            )
        } else {
            let third_party = self.row_type.sorted_third_party_imports("import duckdb  # noqa: F401");
            format!(
                "\"\"\"Auto-generated by scythe. Do not edit.\"\"\"\n\
                 \n\
                 import datetime  # noqa: F401\n\
                 import decimal  # noqa: F401\n\
                 from enum import Enum  # noqa: F401\n\
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

        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        );

        let args_list = if params.is_empty() {
            None
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            Some(format!("[{}]", args.join(", ")))
        };
        let args_ref = args_list.as_deref();

        /// Emit a function signature, wrapping to multi-line when it would exceed 88 chars.
        fn emit_sig(
            out: &mut String,
            func_name: &str,
            params: &[ResolvedParam],
            kw_sep: &str,
            param_list: &str,
            return_type: &str,
        ) {
            let sig = format!("def {func_name}(conn: duckdb.DuckDBPyConnection{kw_sep}{param_list}) -> {return_type}:");
            if sig.len() <= 88 {
                let _ = writeln!(out, "{sig}");
            } else {
                let _ = writeln!(out, "def {func_name}(");
                let _ = writeln!(out, "    conn: duckdb.DuckDBPyConnection,");
                if !params.is_empty() {
                    let _ = writeln!(out, "    *,");
                    for p in params {
                        let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                    }
                }
                let _ = writeln!(out, ") -> {return_type}:");
            }
        }

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                emit_sig(
                    &mut out,
                    &func_name,
                    params,
                    kw_sep,
                    &param_list,
                    &format!("{struct_name} | None"),
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                Self::emit_execute_to_res(&mut out, &sql, args_ref);
                let _ = writeln!(out, "    row = _res.fetchone()");
                let _ = writeln!(out, "    if row is None:");
                let _ = writeln!(out, "        return None");
                let field_assignments: Vec<String> = columns
                    .iter()
                    .enumerate()
                    .map(|(i, col)| format!("{}=row[{}]", col.field_name, i))
                    .collect();
                let oneliner = format!("    return {struct_name}({})", field_assignments.join(", "));
                if oneliner.len() <= 88 {
                    let _ = writeln!(out, "{oneliner}");
                } else {
                    let _ = writeln!(out, "    return {struct_name}(");
                    for fa in &field_assignments {
                        let _ = writeln!(out, "        {fa},");
                    }
                    let _ = writeln!(out, "    )");
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
                let param_name = if params.is_empty() { "count" } else { "items" };
                let sig = format!(
                    "def {batch_fn_name}(conn: duckdb.DuckDBPyConnection, *, {param_name}: {items_type}) -> None:"
                );
                if sig.len() <= 88 {
                    let _ = writeln!(out, "{sig}");
                } else {
                    let _ = writeln!(out, "def {batch_fn_name}(");
                    let _ = writeln!(out, "    conn: duckdb.DuckDBPyConnection,");
                    let _ = writeln!(out, "    *,");
                    let _ = writeln!(out, "    {param_name}: {items_type},");
                    let _ = writeln!(out, ") -> None:");
                }
                let _ = writeln!(
                    out,
                    "    \"\"\"Execute {} query for each item in the batch.\"\"\"",
                    analyzed.name
                );
                if params.is_empty() {
                    let _ = writeln!(out, "    for _ in range(count):");
                    let _ = writeln!(out, "        conn.execute(");
                    let _ = writeln!(out, "            \"\"\"{sql}\"\"\",");
                    let _ = writeln!(out, "        )");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "    conn.executemany(");
                    let _ = writeln!(out, "        \"\"\"{sql}\"\"\",");
                    let _ = writeln!(out, "        [[item] for item in items],");
                    let _ = writeln!(out, "    )");
                } else {
                    let _ = writeln!(out, "    conn.executemany(");
                    let _ = writeln!(out, "        \"\"\"{sql}\"\"\",");
                    let _ = writeln!(out, "        [list(item) for item in items],");
                    let _ = writeln!(out, "    )");
                }
            }
            QueryCommand::Many => {
                emit_sig(
                    &mut out,
                    &func_name,
                    params,
                    kw_sep,
                    &param_list,
                    &format!("list[{struct_name}]"),
                );
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                Self::emit_fetchall(&mut out, &sql, args_ref);
                let field_assignments: Vec<String> = columns
                    .iter()
                    .enumerate()
                    .map(|(i, col)| format!("{}=r[{}]", col.field_name, i))
                    .collect();
                let oneliner = format!(
                    "    return [{struct_name}({}) for r in rows]",
                    field_assignments.join(", ")
                );
                if oneliner.len() <= 88 {
                    let _ = writeln!(out, "{oneliner}");
                } else {
                    let _ = writeln!(out, "    return [");
                    let _ = writeln!(out, "        {struct_name}(");
                    for fa in &field_assignments {
                        let _ = writeln!(out, "            {fa},");
                    }
                    let _ = writeln!(out, "        )");
                    let _ = writeln!(out, "        for r in rows");
                    let _ = writeln!(out, "    ]");
                }
            }
            QueryCommand::Exec => {
                emit_sig(&mut out, &func_name, params, kw_sep, &param_list, "None");
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                Self::emit_execute_call(&mut out, &sql, args_ref);
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                emit_sig(&mut out, &func_name, params, kw_sep, &param_list, "int");
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                Self::emit_execute_to_res(&mut out, &sql, args_ref);
                let _ = writeln!(out, "    row = _res.fetchone()");
                let _ = writeln!(out, "    return row[0] if row else 0");
            }
        }

        Ok(out)
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

        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        );

        let args_list = if params.is_empty() {
            None
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            Some(format!("[{}]", args.join(", ")))
        };

        // Grouped fn: duckdb uses a synchronous API.
        // Signature is always multi-line because `duckdb.DuckDBPyConnection` is long.
        let _ = writeln!(out, "def {func_name}(");
        let _ = writeln!(out, "    conn: duckdb.DuckDBPyConnection,");
        if !params.is_empty() {
            let _ = writeln!(out, "    *,");
            for p in params {
                let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
            }
        }
        let _ = writeln!(out, ") -> list[{parent_struct_name}]:");
        let _ = writeln!(out, "    \"\"\"Execute {} grouped query.\"\"\"", analyzed.name);

        // Use the newline-first SQL format so that SQL lines appear at column 0,
        // keeping all source lines ≤ 88 characters regardless of SQL length.
        let _ = writeln!(out, "    rows = conn.execute(");
        let _ = writeln!(out, "        \"\"\"");
        for line in sql.lines() {
            let _ = writeln!(out, "{line}");
        }
        let _ = writeln!(out, "\"\"\",");
        if let Some(ref a) = args_list {
            let _ = writeln!(out, "        {a},");
        }
        let _ = writeln!(out, "    ).fetchall()");

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
            sql: "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\nFROM users u\nJOIN orders o ON o.user_id = u.id"
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
    fn test_grouped_python_duckdb_structs() {
        let backend = PythonDuckdbBackend::new("duckdb").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();
        assert!(
            row_struct.contains("class GetUsersWithOrdersChildRow"),
            "missing child class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("order_id: int"),
            "child class missing order_id; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("class GetUsersWithOrdersRow"),
            "missing parent class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("id: int"),
            "parent class missing id; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("children: list[GetUsersWithOrdersChildRow]"),
            "parent class missing children field; got:\n{row_struct}"
        );
        // Child must appear before parent
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("class GetUsersWithOrdersRow").unwrap();
        assert!(
            child_pos < parent_pos,
            "child class must be defined before parent class"
        );
    }

    #[test]
    fn test_grouped_python_duckdb_query_fn() {
        let backend = PythonDuckdbBackend::new("duckdb").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("get_users_with_orders"),
            "missing fn name; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("list[GetUsersWithOrdersRow]"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("duckdb.DuckDBPyConnection"),
            "missing conn type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("conn.execute("),
            "fn must use conn.execute; got:\n{query_fn}"
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
        assert!(
            !query_fn.contains("async def"),
            "duckdb fn must be synchronous; got:\n{query_fn}"
        );
    }
}
