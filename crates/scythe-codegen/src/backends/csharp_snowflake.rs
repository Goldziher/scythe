use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name, to_pascal_case};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/csharp-snowflake.toml");

pub struct CsharpSnowflakeBackend {
    manifest: BackendManifest,
}

impl CsharpSnowflakeBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "snowflake" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("csharp-snowflake only supports Snowflake, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self { manifest })
    }
}

/// Map a neutral type to a SnowflakeDbDataReader method.
///
/// `lang_type` is the manifest's own declaration for the same column (the
/// non-nullable base form). Every neutral type this table does not name --
/// `bytes`, `array<...>`, `json_typed<...>` -- falls through to a typed
/// accessor built from it; see [`super::csharp_typed_reader_method`].
fn reader_method(neutral_type: &str, lang_type: &str) -> String {
    let mapped = match neutral_type {
        "bool" => "GetBoolean",
        "int16" => "GetInt16",
        "int32" => "GetInt32",
        "int64" => "GetInt64",
        "float32" => "GetFloat",
        "float64" => "GetDouble",
        "string" | "json" | "inet" | "interval" | "uuid" | "time" | "time_tz" => "GetString",
        "decimal" => "GetDecimal",
        "date" | "datetime" => "GetDateTime",
        "datetime_tz" => "GetFieldValue<DateTimeOffset>",
        _ => return super::csharp_typed_reader_method(lang_type),
    };
    mapped.to_string()
}

/// Map a neutral type to the `System.Data.DbType` a `SnowflakeDbParameter`
/// must carry.
///
/// `DbParameter.DbType` defaults to `AnsiString`, and `Snowflake.Data`'s
/// `SFDataConverter.csharpTypeValToSfTypeVal` has no mapping for it, so an
/// unset `DbType` makes every parameterized query throw
/// `SnowflakeDbException: No corresponding Snowflake type for type AnsiString`
/// on its first execution — against real Snowflake as much as against a fake.
fn parameter_db_type(neutral_type: &str) -> &'static str {
    match neutral_type {
        "bool" => "System.Data.DbType.Boolean",
        "int16" => "System.Data.DbType.Int16",
        "int32" => "System.Data.DbType.Int32",
        "int64" => "System.Data.DbType.Int64",
        "float32" => "System.Data.DbType.Single",
        "float64" => "System.Data.DbType.Double",
        "decimal" => "System.Data.DbType.Decimal",
        "date" => "System.Data.DbType.Date",
        "datetime" => "System.Data.DbType.DateTime",
        "datetime_tz" => "System.Data.DbType.DateTimeOffset",
        "bytes" => "System.Data.DbType.Binary",
        _ => "System.Data.DbType.String",
    }
}

/// Rewrite $1, $2, ... to ?
/// Build the expression to read a column from SnowflakeDbDataReader.
fn column_read_expr(col: &ResolvedColumn, ordinal: usize) -> String {
    let method = reader_method(&col.neutral_type, &col.lang_type);
    format!("reader.{}({})", method, ordinal)
}

impl CodegenBackend for CsharpSnowflakeBackend {
    fn name(&self) -> &str {
        "csharp-snowflake"
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

    fn file_header(&self) -> String {
        "#nullable enable\n\nusing System.Data;\nusing Snowflake.Data.Client;\n\npublic static class Queries {"
            .to_string()
    }

    fn file_footer(&self) -> String {
        "}".to_string()
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        _query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();
        let _ = writeln!(out, "public record {}(", struct_name);
        for (i, c) in columns.iter().enumerate() {
            let field = to_pascal_case(&c.field_name);
            let sep = if i + 1 < columns.len() { "," } else { "" };
            let _ = writeln!(out, "    {} {}{}", c.full_type, field, sep);
        }
        let _ = write!(out, ");");
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
        let sql = crate::sql_literal::escape_csharp_verbatim_string(&super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        ));
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{} {}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        if matches!(analyzed.command, QueryCommand::Batch) {
            let batch_fn_name = format!("{}Batch", func_name);
            if params.len() > 1 {
                let params_record_name = format!("{}BatchParams", to_pascal_case(&analyzed.name));
                let _ = writeln!(out, "public record {}(", params_record_name);
                for (i, p) in params.iter().enumerate() {
                    let field = to_pascal_case(&p.field_name);
                    let sep = if i + 1 < params.len() { "," } else { "" };
                    let _ = writeln!(out, "    {} {}{}", p.full_type, field, sep);
                }
                let _ = writeln!(out, ");");
                let _ = writeln!(out);
                let _ = writeln!(
                    out,
                    "public static async Task {}(SnowflakeDbConnection conn, List<{}> items) {{",
                    batch_fn_name, params_record_name
                );
            } else if params.len() == 1 {
                let _ = writeln!(
                    out,
                    "public static async Task {}(SnowflakeDbConnection conn, List<{}> items) {{",
                    batch_fn_name, params[0].full_type
                );
            } else {
                let _ = writeln!(
                    out,
                    "public static async Task {}(SnowflakeDbConnection conn, int count) {{",
                    batch_fn_name
                );
            }
            let _ = writeln!(
                out,
                "    await using var tx = (System.Data.Common.DbTransaction)await conn.BeginTransactionAsync();"
            );
            let _ = writeln!(out, "    try {{");
            if params.is_empty() {
                let _ = writeln!(out, "        for (int i = 0; i < count; i++) {{");
            } else {
                let _ = writeln!(out, "        foreach (var item in items) {{");
            }
            let _ = writeln!(out, "            await using var cmd = new SnowflakeDbCommand(conn);");
            let _ = writeln!(out, "            cmd.CommandText = @\"{}\";", sql);
            for (i, p) in params.iter().enumerate() {
                let value_expr = if params.len() > 1 {
                    let field = to_pascal_case(&p.field_name);
                    format!("item.{}", field)
                } else {
                    "item".to_string()
                };
                let _ = writeln!(
                    out,
                    "            cmd.Parameters.Add(new SnowflakeDbParameter {{ ParameterName = \"{}\", DbType = {}, Value = {} }});",
                    i + 1,
                    parameter_db_type(&p.neutral_type),
                    value_expr
                );
            }
            let _ = writeln!(out, "            await cmd.ExecuteNonQueryAsync();");
            let _ = writeln!(out, "        }}");
            let _ = writeln!(out, "        await tx.CommitAsync();");
            let _ = writeln!(out, "    }} catch {{");
            let _ = writeln!(out, "        await tx.RollbackAsync();");
            let _ = writeln!(out, "        throw;");
            let _ = writeln!(out, "    }}");
            let _ = write!(out, "}}");
            return Ok(out);
        }

        let return_type = match &analyzed.command {
            QueryCommand::One => struct_name.to_string(),
            QueryCommand::Opt => format!("{}?", struct_name),
            QueryCommand::Many => format!("List<{}>", struct_name),
            QueryCommand::Exec => "void".to_string(),
            QueryCommand::ExecResult | QueryCommand::ExecRows => "int".to_string(),
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        };

        let is_async_void = return_type == "void";
        let task_type = if is_async_void {
            "Task".to_string()
        } else {
            format!("Task<{}>", return_type)
        };

        let _ = writeln!(
            out,
            "public static async {} {}(SnowflakeDbConnection conn{}{}) {{",
            task_type, func_name, sep, param_list
        );

        let _ = writeln!(out, "    await using var cmd = new SnowflakeDbCommand(conn);");
        let _ = writeln!(out, "    cmd.CommandText = @\"{}\";", sql);
        for (i, p) in params.iter().enumerate() {
            let _ = writeln!(
                out,
                "    cmd.Parameters.Add(new SnowflakeDbParameter {{ ParameterName = \"{}\", DbType = {}, Value = {} }});",
                i + 1,
                parameter_db_type(&p.neutral_type),
                p.field_name
            );
        }

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(out, "    await using var reader = await cmd.ExecuteReaderAsync();");
                if matches!(analyzed.command, QueryCommand::One) {
                    let _ = writeln!(
                        out,
                        "    if (!await reader.ReadAsync()) throw new InvalidOperationException(\"{func_name} expected exactly one row but found none\");"
                    );
                } else {
                    let _ = writeln!(out, "    if (!await reader.ReadAsync()) return null;");
                }
                let _ = writeln!(out, "    return new {}(", struct_name);
                for (i, col) in columns.iter().enumerate() {
                    let expr = column_read_expr(col, i);
                    let sep = if i + 1 < columns.len() { "," } else { "" };
                    if col.nullable {
                        let _ = writeln!(out, "        reader.IsDBNull({i}) ? null : {expr}{sep}");
                    } else {
                        let _ = writeln!(out, "        {expr}{sep}");
                    }
                }
                let _ = writeln!(out, "    );");
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "    await using var reader = await cmd.ExecuteReaderAsync();");
                let _ = writeln!(out, "    var results = new List<{}>();", struct_name);
                let _ = writeln!(out, "    while (await reader.ReadAsync()) {{");
                let _ = writeln!(out, "        results.Add(new {}(", struct_name);
                for (i, col) in columns.iter().enumerate() {
                    let expr = column_read_expr(col, i);
                    let sep = if i + 1 < columns.len() { "," } else { "" };
                    if col.nullable {
                        let _ = writeln!(out, "            reader.IsDBNull({i}) ? null : {expr}{sep}");
                    } else {
                        let _ = writeln!(out, "            {expr}{sep}");
                    }
                }
                let _ = writeln!(out, "        ));");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    return results;");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "    await cmd.ExecuteNonQueryAsync();");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "    return await cmd.ExecuteNonQueryAsync();");
            }
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        }

        let _ = write!(out, "}}");
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
        let mut out = String::new();

        let _ = writeln!(out, "public record {}(", child_struct_name);
        for (i, c) in child_columns.iter().enumerate() {
            let field = to_pascal_case(&c.field_name);
            let sep = if i + 1 < child_columns.len() { "," } else { "" };
            let _ = writeln!(out, "    {} {}{}", c.full_type, field, sep);
        }
        let _ = writeln!(out, ");");
        let _ = writeln!(out);

        let _ = writeln!(out, "public record {}(", parent_struct_name);
        for c in parent_columns {
            let field = to_pascal_case(&c.field_name);
            let _ = writeln!(out, "    {} {},", c.full_type, field);
        }
        let _ = writeln!(out, "    List<{}> Children", child_struct_name);
        let _ = write!(out, ");");

        Ok(out)
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
        let sql = crate::sql_literal::escape_csharp_verbatim_string(&super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        ));

        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{} {}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let key_col = parent_columns
            .iter()
            .find(|c| c.name == key_column)
            .unwrap_or(&parent_columns[0]);
        let key_type = &key_col.full_type;
        let key_ordinal = all_columns.iter().position(|c| c.name == key_column).unwrap_or(0);

        let _ = writeln!(
            out,
            "public static async Task<List<{parent_struct_name}>> {func_name}(SnowflakeDbConnection conn{sep}{param_list}) {{"
        );
        let _ = writeln!(out, "    await using var cmd = new SnowflakeDbCommand(conn);");
        let _ = writeln!(out, "    cmd.CommandText = @\"{sql}\";");
        for (i, p) in params.iter().enumerate() {
            let _ = writeln!(
                out,
                "    cmd.Parameters.Add(new SnowflakeDbParameter {{ ParameterName = \"{}\", DbType = {}, Value = {} }});",
                i + 1,
                parameter_db_type(&p.neutral_type),
                p.field_name
            );
        }
        let _ = writeln!(out, "    await using var reader = await cmd.ExecuteReaderAsync();");
        let _ = writeln!(
            out,
            "    var lookup = new Dictionary<{key_type}, {parent_struct_name}>();"
        );
        let _ = writeln!(out, "    var result = new List<{parent_struct_name}>();");
        let _ = writeln!(out, "    while (await reader.ReadAsync()) {{");

        let key_expr = column_read_expr(key_col, key_ordinal);
        let _ = writeln!(out, "        var key = {key_expr};");

        let _ = writeln!(out, "        var child = new {child_struct_name}(");
        for (ci, col) in child_columns.iter().enumerate() {
            let ord = all_columns
                .iter()
                .position(|c| c.name == col.name)
                .unwrap_or(parent_columns.len() + ci);
            let expr = column_read_expr(col, ord);
            let trailing = if ci + 1 < child_columns.len() { "," } else { "" };
            if col.nullable {
                let _ = writeln!(out, "            reader.IsDBNull({ord}) ? null : {expr}{trailing}");
            } else {
                let _ = writeln!(out, "            {expr}{trailing}");
            }
        }
        let _ = writeln!(out, "        );");

        let _ = writeln!(out, "        if (lookup.TryGetValue(key, out var parent)) {{");
        let _ = writeln!(out, "            parent.Children.Add(child);");
        let _ = writeln!(out, "        }} else {{");
        let _ = writeln!(out, "            var newParent = new {parent_struct_name}(");
        for col in parent_columns {
            let ord = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
            let expr = column_read_expr(col, ord);
            if col.nullable {
                let _ = writeln!(out, "                reader.IsDBNull({ord}) ? null : {expr},");
            } else {
                let _ = writeln!(out, "                {expr},");
            }
        }
        let _ = writeln!(out, "                new List<{child_struct_name}> {{ child }}");
        let _ = writeln!(out, "            );");
        let _ = writeln!(out, "            lookup[key] = newParent;");
        let _ = writeln!(out, "            result.Add(newParent);");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "    return result;");
        let _ = write!(out, "}}");

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "public enum {} {{", type_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "    {},", variant);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        if composite.fields.is_empty() {
            let _ = writeln!(out, "public record {}();", name);
        } else {
            let _ = writeln!(out, "public record {}(", name);
            for (i, field) in composite.fields.iter().enumerate() {
                let cs_type = resolve_type(&field.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .unwrap_or_else(|_| "object".to_string());
                let field_name = to_pascal_case(&field.name);
                let sep = if i + 1 < composite.fields.len() { "," } else { "" };
                let _ = writeln!(out, "    {} {}{}", cs_type, field_name, sep);
            }
            let _ = write!(out, ");");
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
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
            aq.sql = "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\nFROM users u\nJOIN orders o ON o.user_id = u.id".to_string();
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
    fn test_grouped_csharp_snowflake_structs() {
        let backend = crate::backends::get_backend("csharp-snowflake", "snowflake").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("public record GetUsersWithOrdersChildRow"),
            "missing child record; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("public record GetUsersWithOrdersRow"),
            "missing parent record; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("List<GetUsersWithOrdersChildRow> Children"),
            "parent missing Children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("public record GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("public record GetUsersWithOrdersRow(").unwrap();
        assert!(child_pos < parent_pos, "child must precede parent; got:\n{row_struct}");
        assert!(result.model_struct.is_none(), "grouped must not produce model_struct");
    }

    #[test]
    fn test_grouped_csharp_snowflake_query_fn() {
        let backend = crate::backends::get_backend("csharp-snowflake", "snowflake").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("Task<List<GetUsersWithOrdersRow>>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrders"),
            "missing fn name; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("SnowflakeDbConnection conn"),
            "missing connection param; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Dictionary<"),
            "must use Dictionary; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("lookup.TryGetValue"),
            "must fold with TryGetValue; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Children.Add(child)"),
            "must append child; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return result;"),
            "must return result; got:\n{query_fn}"
        );
    }

    /// `DbParameter.DbType` defaults to `AnsiString`, for which
    /// `Snowflake.Data` has no Snowflake-type mapping — so a parameter left
    /// without an explicit `DbType` throws
    /// `No corresponding Snowflake type for type AnsiString` on the first
    /// execution, against real Snowflake as much as against a fake.
    #[test]
    fn test_query_fn_sets_an_explicit_db_type_on_every_parameter() {
        use super::CsharpSnowflakeBackend;

        let backend = CsharpSnowflakeBackend::new("snowflake").unwrap();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "CreateOrder".to_string();
            aq.command = QueryCommand::Exec;
            aq.sql = "INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3)".to_string();
            aq.columns = vec![];
            aq.params = vec![
                scythe_core::analyzer::AnalyzedParam {
                    name: "user_id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    position: 1,
                    source_relation: None,
                },
                scythe_core::analyzer::AnalyzedParam {
                    name: "total".to_string(),
                    neutral_type: "decimal".to_string(),
                    nullable: false,
                    position: 2,
                    source_relation: None,
                },
                scythe_core::analyzer::AnalyzedParam {
                    name: "notes".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    position: 3,
                    source_relation: None,
                },
            ];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = None;
            aq.custom = vec![];
        });

        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert_eq!(
            query_fn.matches("SnowflakeDbParameter").count(),
            3,
            "expected one parameter per placeholder; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("SnowflakeDbParameter { ParameterName = \"1\", Value ="),
            "every parameter must carry an explicit DbType; got:\n{query_fn}"
        );
        // Snowflake's REST protocol keys bindings by ordinal for `?` placeholders,
        // so the name must be the bare 1-based index. A `p1`-style name is read as
        // a *named* binding and the query fails server-side -- which is what kept
        // csharp-snowflake out of CI since v0.6.0, misfiled as an emulator
        // limitation. Nothing had ever run this backend against a real Snowflake
        // or an emulator, so the spelling was never falsified.
        assert!(
            query_fn.contains("ParameterName = \"1\"") && query_fn.contains("ParameterName = \"3\""),
            "positional bindings must be named by bare ordinal; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("ParameterName = \"p"),
            "no parameter may carry a `p`-prefixed name; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("DbType = System.Data.DbType.Int32"),
            "int32 param must map to DbType.Int32; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("DbType = System.Data.DbType.Decimal"),
            "decimal param must map to DbType.Decimal; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("DbType = System.Data.DbType.String"),
            "string param must map to DbType.String; got:\n{query_fn}"
        );
    }
}
