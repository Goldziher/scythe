use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name, to_snake_case};
use scythe_backend::types::resolve_type;
use std::collections::HashMap;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};

use super::python_common::{
    PythonRowType, generate_grouped_structs_py, no_rows_exception_def, write_def_signature, write_execute_call,
    write_missing_row_guard, write_return_call,
};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/python-snowflake.toml");

/// Maps a `DictCursor` row onto the analyzer's lower-case column names.
///
/// Snowflake upper-cases unquoted identifiers and preserves quoted ones, so
/// indexing a row with a literal projected name misses one of the two. Reading
/// by name rather than position also keeps the generated code typed against
/// snowflake-connector-python 5.x, whose cursor rows are `dict[str, Any]`.
///
/// The tuple arm exists only for the type checker: 5.1 types `Connection.cursor`
/// as returning the whole cursor union even when passed `DictCursor`, so the
/// row type keeps that union and rejects a tuple at runtime rather than
/// indexing it blindly.
const ROW_HELPER_DEF: &str = concat!(
    "\n",
    "def _scythe_row(row: dict[str, Any] | tuple[Any, ...]) -> dict[str, Any]:\n",
    "    \"\"\"Index a Snowflake row by its lower-case column name.\"\"\"\n",
    "    if isinstance(row, dict):\n",
    "        return {str(key).lower(): value for key, value in row.items()}\n",
    "    raise TypeError(\"snowflake DictCursor returned a positional row\")\n",
);

pub struct PythonSnowflakeBackend {
    manifest: BackendManifest,
    row_type: PythonRowType,
}

impl PythonSnowflakeBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "snowflake" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("python-snowflake only supports Snowflake, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self {
            manifest,
            row_type: PythonRowType::default(),
        })
    }
}

/// Rewrite $1, $2, ... positional params to ? for Snowflake.
impl CodegenBackend for PythonSnowflakeBackend {
    fn name(&self) -> &str {
        "python-snowflake"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["snowflake"]
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
        // This backend emits `?` (qmark) placeholders -- see `generate_query_fn` below --
        // but the snowflake-connector-python driver's own default `paramstyle` module
        // attribute is `pyformat`, not `qmark`. Without setting it explicitly here, every
        // bind in every generated function fails at execute time for anyone who simply
        // imports this module (the only place `paramstyle = "qmark"` existed before was a
        // hand-written compensation in the integration test harness, which does not help a
        // real caller). Setting it as a side effect of importing this module -- the same
        // place the driver import itself lives -- means it is set exactly once, regardless
        // of how many query functions or generated files import `snowflake.connector`.
        const PARAMSTYLE_LINE: &str = "snowflake.connector.paramstyle = \"qmark\"  # this module emits qmark binds";
        let header = if self.row_type.is_stdlib_import() {
            format!(
                "import datetime  # noqa: F401\n\
                 import decimal  # noqa: F401\n\
                 {import_line}\n\
                 from enum import Enum  # noqa: F401\n\
                 from typing import Any\n\
                 \n\
                 import snowflake.connector  # noqa: F401\n\
                 \n\
                 {PARAMSTYLE_LINE}\n\
                 \n",
            )
        } else {
            let third_party = self
                .row_type
                .sorted_third_party_imports("import snowflake.connector  # noqa: F401");
            format!(
                "import datetime  # noqa: F401\n\
                 import decimal  # noqa: F401\n\
                 from enum import Enum  # noqa: F401\n\
                 from typing import Any\n\
                 \n\
                 {third_party}\n\
                 \n\
                 {PARAMSTYLE_LINE}\n\
                 \n",
            )
        };
        format!("{header}{}{ROW_HELPER_DEF}", no_rows_exception_def())
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();
        let _ = write!(out, "{}", self.row_type.decorator());
        let _ = writeln!(out, "{}", self.row_type.class_def(struct_name));
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

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let kw_params: Vec<(String, String)> = params
            .iter()
            .map(|p| (p.field_name.clone(), p.full_type.clone()))
            .collect();
        const CONN_PARAM: &str = "conn: snowflake.connector.SnowflakeConnection";

        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        );

        let args_tuple = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            if args.len() == 1 {
                format!("({},)", args[0])
            } else {
                format!("({})", args.join(", "))
            }
        };

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let is_one = matches!(analyzed.command, QueryCommand::One);
                let ret = if is_one {
                    struct_name.to_string()
                } else {
                    format!("{struct_name} | None")
                };
                write_def_signature(&mut out, "def", &func_name, CONN_PARAM, &kw_params, &ret);
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                let _ = writeln!(out, "    cur = conn.cursor(snowflake.connector.DictCursor)");
                let args = (!params.is_empty()).then_some(args_tuple.as_str());
                write_execute_call(&mut out, "    ", "cur.execute", &sql, args);
                let _ = writeln!(out, "    row = cur.fetchone()");
                write_missing_row_guard(&mut out, "    ", "row", is_one, &analyzed.name);
                let _ = writeln!(out, "    values = _scythe_row(row)");
                let field_assignments: Vec<String> = columns
                    .iter()
                    .map(|col| format!("{}={}", col.field_name, row_value("values", col)))
                    .collect();
                write_return_call(&mut out, "    ", struct_name, &field_assignments);
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
                let batch_kw_params = vec![(items_or_count.to_string(), items_type)];
                write_def_signature(&mut out, "def", &batch_fn_name, CONN_PARAM, &batch_kw_params, "None");
                let _ = writeln!(
                    out,
                    "    \"\"\"Execute {} query for each item in the batch.\"\"\"",
                    analyzed.name
                );
                let _ = writeln!(out, "    cur = conn.cursor()");
                if params.is_empty() {
                    let _ = writeln!(out, "    for _ in range(count):");
                    write_execute_call(&mut out, "        ", "cur.execute", &sql, None);
                } else if params.len() == 1 {
                    let _ = writeln!(out, "    for item in items:");
                    write_execute_call(&mut out, "        ", "cur.execute", &sql, Some("(item,)"));
                } else {
                    let _ = writeln!(out, "    for item in items:");
                    write_execute_call(&mut out, "        ", "cur.execute", &sql, Some("item"));
                }
            }
            QueryCommand::Many => {
                let ret = format!("list[{}]", struct_name);
                write_def_signature(&mut out, "def", &func_name, CONN_PARAM, &kw_params, &ret);
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                let _ = writeln!(out, "    cur = conn.cursor(snowflake.connector.DictCursor)");
                let args = (!params.is_empty()).then_some(args_tuple.as_str());
                write_execute_call(&mut out, "    ", "cur.execute", &sql, args);
                let _ = writeln!(out, "    rows = cur.fetchall()");
                let field_assignments: Vec<String> = columns
                    .iter()
                    .map(|col| format!("{}={}", col.field_name, row_value("values", col)))
                    .collect();
                let _ = writeln!(out, "    result: {ret} = []");
                let _ = writeln!(out, "    for r in rows:");
                let _ = writeln!(out, "        values = _scythe_row(r)");
                let append = format!(
                    "        result.append({}({}))",
                    struct_name,
                    field_assignments.join(", ")
                );
                if append.len() <= 88 {
                    let _ = writeln!(out, "{append}");
                } else {
                    let _ = writeln!(out, "        result.append({}(", struct_name);
                    for fa in &field_assignments {
                        let _ = writeln!(out, "            {fa},");
                    }
                    let _ = writeln!(out, "        ))");
                }
                let _ = writeln!(out, "    return result");
            }
            QueryCommand::Exec => {
                write_def_signature(&mut out, "def", &func_name, CONN_PARAM, &kw_params, "None");
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                let _ = writeln!(out, "    cur = conn.cursor()");
                let args = (!params.is_empty()).then_some(args_tuple.as_str());
                write_execute_call(&mut out, "    ", "cur.execute", &sql, args);
            }
            QueryCommand::Grouped => unreachable!("grouped queries are routed to generate_grouped_query_fn"),
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_def_signature(&mut out, "def", &func_name, CONN_PARAM, &kw_params, "int");
                let _ = writeln!(out, "    \"\"\"Execute {} query.\"\"\"", analyzed.name);
                let _ = writeln!(out, "    cur = conn.cursor()");
                let args = (!params.is_empty()).then_some(args_tuple.as_str());
                write_execute_call(&mut out, "    ", "cur.execute", &sql, args);
                // ~keep snowflake-connector-python types `Cursor.rowcount` as `int | None`
                // (unlike every other python backend's driver, which types it plain `int`) --
                // `or 0` narrows it back to the `-> int` this fn declares (#193).
                let _ = writeln!(out, "    return cur.rowcount or 0");
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
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = write!(out, "{}", self.row_type.decorator());
        let _ = writeln!(out, "{}", self.row_type.class_def(&name));
        let _ = writeln!(out, "    \"\"\"Composite type {}.\"\"\"", composite.sql_name);
        if composite.fields.is_empty() {
            let _ = writeln!(out, "    pass");
        } else {
            let _ = writeln!(out);
            for field in &composite.fields {
                let py_type = resolve_type(&field.neutral_type, &self.manifest, field.nullable)
                    .map(|t| t.into_owned())
                    .map_err(|e| {
                        ScytheError::new(ErrorCode::InternalError, format!("composite field type error: {}", e))
                    })?;
                let _ = writeln!(out, "    {}: {}", to_snake_case(&field.name), py_type);
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

        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        );

        let args_tuple = if params.is_empty() {
            None
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            if args.len() == 1 {
                Some(format!("({},)", args[0]))
            } else {
                Some(format!("({})", args.join(", ")))
            }
        };

        let sig = format!(
            "def {func_name}(conn: snowflake.connector.SnowflakeConnection{kw_sep}{param_list}) -> list[{parent_struct_name}]:"
        );
        if sig.len() <= 88 {
            let _ = writeln!(out, "{sig}");
        } else {
            let _ = writeln!(out, "def {func_name}(");
            let _ = writeln!(out, "    conn: snowflake.connector.SnowflakeConnection,");
            if !params.is_empty() {
                let _ = writeln!(out, "    *,");
                for p in params {
                    let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                }
            }
            let _ = writeln!(out, ") -> list[{parent_struct_name}]:");
        }
        let _ = writeln!(out, "    \"\"\"Execute {} grouped query.\"\"\"", analyzed.name);
        let _ = writeln!(out, "    cur = conn.cursor(snowflake.connector.DictCursor)");
        let _ = writeln!(out, "    cur.execute(");
        let _ = writeln!(out, "        \"\"\"");
        for line in sql.lines() {
            let _ = writeln!(out, "{line}");
        }
        let _ = writeln!(out, "\"\"\",");
        if let Some(ref a) = args_tuple {
            let _ = writeln!(out, "        {a},");
        }
        let _ = writeln!(out, "    )");
        let _ = writeln!(out, "    rows = cur.fetchall()");

        generate_grouped_fold_named(
            &mut out,
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
        );

        Ok(out)
    }
}

/// The lower-case row key a projected column is read by.
///
/// `_scythe_row` lower-cases the driver's key, and Snowflake's unquoted
/// identifiers arrive upper-cased, so the analyzer's own spelling is lowered
/// to match.
fn column_key(column: &ResolvedColumn) -> String {
    column.name.to_lowercase().replace('\\', "\\\\").replace('"', "\\\"")
}

/// A `values["column"]` read for the normalized row mapping `values`.
fn row_value(values_var: &str, column: &ResolvedColumn) -> String {
    format!("{values_var}[\"{}\"]", column_key(column))
}

/// Emits the `:grouped` client-side fold using name-keyed row access.
///
/// The positional variant in `python_common` cannot be used here:
/// snowflake-connector-python 5.x types `fetchall()` rows as `dict[str, Any]`,
/// so indexing a row by position does not type-check against it.
fn generate_grouped_fold_named(
    out: &mut String,
    parent_struct_name: &str,
    child_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
    key_column: &str,
) {
    let _ = writeln!(out, "    _index: dict = {{}}");
    let _ = writeln!(out, "    _entries: list = []");
    let _ = writeln!(out, "    for row in rows:");
    let _ = writeln!(out, "        values = _scythe_row(row)");
    let _ = writeln!(out, "        key = values[\"{}\"]", key_column.to_lowercase());
    let _ = writeln!(out, "        if key not in _index:");
    let _ = writeln!(out, "            _index[key] = len(_entries)");
    let _ = writeln!(out, "            _entries.append((");
    let _ = writeln!(out, "                {{");
    for col in parent_columns {
        let _ = writeln!(
            out,
            "                    \"{}\": {},",
            col.field_name,
            row_value("values", col)
        );
    }
    let _ = writeln!(out, "                }},");
    let _ = writeln!(out, "                [],");
    let _ = writeln!(out, "            ))");
    let _ = writeln!(out, "        _entries[_index[key]][1].append({child_struct_name}(");
    for col in child_columns {
        let _ = writeln!(out, "            {}={},", col.field_name, row_value("values", col));
    }
    let _ = writeln!(out, "        ))");
    let _ = writeln!(out, "    return [");
    let _ = writeln!(out, "        {parent_struct_name}(**parent_kwargs, children=children)");
    let _ = writeln!(out, "        for parent_kwargs, children in _entries");
    let _ = write!(out, "    ]");
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
    fn test_grouped_python_snowflake_structs() {
        let backend = PythonSnowflakeBackend::new("snowflake").unwrap();
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
    fn test_grouped_python_snowflake_query_fn() {
        let backend = PythonSnowflakeBackend::new("snowflake").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("def get_users_with_orders"),
            "missing fn name; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("list[GetUsersWithOrdersRow]"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("snowflake.connector.SnowflakeConnection"),
            "missing conn type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("cur.execute("),
            "fn must use cur.execute; got:\n{query_fn}"
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
        assert!(
            !query_fn.contains("async def"),
            "snowflake fn must be synchronous; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("values[\"id\"]"),
            "grouped fn must read the key by name; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("row[") && !query_fn.contains("r[0]"),
            "grouped fn must not emit positional row access; got:\n{query_fn}"
        );
    }

    fn make_flat_query(command: QueryCommand) -> AnalyzedQuery {
        let columns = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int64".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "total_sum".to_string(),
                neutral_type: "decimal".to_string(),
                nullable: true,
                ..Default::default()
            },
        ];
        AnalyzedQuery::build(|aq| {
            aq.name = "GetOrderTotal".to_string();
            aq.command = command;
            aq.sql = "SELECT id, total_sum FROM orders".to_string();
            aq.columns = columns;
            aq.params = vec![];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = None;
            aq.custom = vec![];
        })
    }

    /// snowflake-connector-python 5.x types `fetchall()` rows as `dict[str, Any]`,
    /// so positional access fails `pyrefly check -p strict`; every row-returning
    /// command must read by projected column name through the `DictCursor`.
    #[test]
    fn test_snowflake_reads_rows_by_name_not_position() {
        let backend = PythonSnowflakeBackend::new("snowflake").unwrap();
        for command in [QueryCommand::One, QueryCommand::Opt, QueryCommand::Many] {
            let query = make_flat_query(command);
            let result = crate::generate_with_backend(&query, &backend).unwrap();
            let query_fn = result.query_fn.as_deref().unwrap();
            assert!(
                query_fn.contains("conn.cursor(snowflake.connector.DictCursor)"),
                "fn must use DictCursor; got:\n{query_fn}"
            );
            assert!(
                query_fn.contains("_scythe_row("),
                "fn must normalize the row; got:\n{query_fn}"
            );
            assert!(
                query_fn.contains("values[\"id\"]") && query_fn.contains("values[\"total_sum\"]"),
                "fn must read by column name; got:\n{query_fn}"
            );
            assert!(
                !query_fn.contains("row[0]") && !query_fn.contains("r[0]"),
                "fn must not emit positional row access; got:\n{query_fn}"
            );
        }
    }
}
