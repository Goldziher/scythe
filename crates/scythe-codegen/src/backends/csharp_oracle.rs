use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, to_pascal_case};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/csharp-oracle.toml");

pub struct CsharpOracleBackend {
    manifest: BackendManifest,
}

impl CsharpOracleBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "oracle" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("csharp-oracle only supports Oracle, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self { manifest })
    }
}

/// Map a neutral type to an OracleDbType variant for output parameters.
fn oracle_db_type(neutral_type: &str) -> &'static str {
    match neutral_type {
        "int32" | "int64" => "OracleDbType.Int64",
        "float32" | "float64" => "OracleDbType.Double",
        "decimal" => "OracleDbType.Decimal",
        "date" | "datetime" | "datetime_tz" => "OracleDbType.Date",
        "bytes" => "OracleDbType.Blob",
        _ => "OracleDbType.Varchar2",
    }
}

/// Cast an Oracle output parameter value to the appropriate C# type.
fn oracle_out_cast(neutral_type: &str, param_expr: &str) -> String {
    match neutral_type {
        "int32" => format!(
            "((Oracle.ManagedDataAccess.Types.OracleDecimal){}).ToInt32()",
            param_expr
        ),
        "int64" => format!(
            "((Oracle.ManagedDataAccess.Types.OracleDecimal){}).ToInt64()",
            param_expr
        ),
        "float32" | "float64" => format!(
            "((Oracle.ManagedDataAccess.Types.OracleDecimal){}).ToDouble()",
            param_expr
        ),
        // ~keep OracleDecimal exposes the managed decimal through its `Value`
        // property; there is no ToDecimal(), unlike ToInt32/ToInt64/ToDouble.
        "decimal" => format!("((Oracle.ManagedDataAccess.Types.OracleDecimal){}).Value", param_expr),
        "bytes" => format!("((Oracle.ManagedDataAccess.Types.OracleBlob){}).Value", param_expr),
        "date" | "datetime" | "datetime_tz" => {
            format!("((Oracle.ManagedDataAccess.Types.OracleDate){}).Value", param_expr)
        }
        _ => format!("((Oracle.ManagedDataAccess.Types.OracleString){}).Value", param_expr),
    }
}

/// Map a neutral type to an OracleDataReader method.
///
/// `lang_type` is the manifest's own declaration for the same column (the
/// non-nullable base form). Every neutral type this table does not name --
/// `array<...>` (reachable through an `= ANY(:1)` parameter even though
/// Oracle has no array column), `range<...>`, `json_typed<...>`, a composite
/// -- falls through to a typed accessor built from it; see
/// [`super::csharp_typed_reader_method`], which generalises the `bytes` arm
/// this table already had for exactly the same reason.
fn reader_method(neutral_type: &str, lang_type: &str) -> String {
    let mapped = match neutral_type {
        "bool" => "GetBoolean",
        "int16" => "GetInt16",
        "int32" => "GetInt32",
        "int64" => "GetInt64",
        "float32" => "GetFloat",
        "float64" => "GetDouble",
        "string" | "json" | "inet" | "interval" | "uuid" => "GetString",
        "decimal" => "GetDecimal",
        "date" | "datetime" => "GetDateTime",
        "datetime_tz" => "GetFieldValue<DateTimeOffset>",
        "time" | "time_tz" => "GetFieldValue<TimeOnly>",
        _ => return super::csharp_typed_reader_method(lang_type),
    };
    mapped.to_string()
}

impl CodegenBackend for CsharpOracleBackend {
    fn name(&self) -> &str {
        "csharp-oracle"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["oracle"]
    }

    fn file_header(&self) -> String {
        "#nullable enable\n\nusing Oracle.ManagedDataAccess.Client;\n\npublic static class Queries {".to_string()
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
            |n| format!(":{n}"),
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
                    "public static async Task {}(OracleConnection conn, List<{}> items) {{",
                    batch_fn_name, params_record_name
                );
            } else if params.len() == 1 {
                let _ = writeln!(
                    out,
                    "public static async Task {}(OracleConnection conn, List<{}> items) {{",
                    batch_fn_name, params[0].full_type
                );
            } else {
                let _ = writeln!(
                    out,
                    "public static async Task {}(OracleConnection conn, int count) {{",
                    batch_fn_name
                );
            }
            let _ = writeln!(out, "    using var tx = conn.BeginTransaction();");
            let _ = writeln!(out, "    try {{");
            if params.is_empty() {
                let _ = writeln!(out, "        for (int i = 0; i < count; i++) {{");
            } else {
                let _ = writeln!(out, "        foreach (var item in items) {{");
            }
            let _ = writeln!(
                out,
                "            using var cmd = new OracleCommand(@\"{}\", conn);",
                sql
            );
            for (i, p) in params.iter().enumerate() {
                let value_expr = if params.len() > 1 {
                    format!("item.{}", to_pascal_case(&p.field_name))
                } else {
                    "item".to_string()
                };
                let _ = format!("{}", i);
                let _ = writeln!(
                    out,
                    "            cmd.Parameters.Add(new OracleParameter {{ Value = (object){} ?? DBNull.Value }});",
                    value_expr
                );
            }
            let _ = writeln!(out, "            await cmd.ExecuteNonQueryAsync();");
            let _ = writeln!(out, "        }}");
            let _ = writeln!(out, "        tx.Commit();");
            let _ = writeln!(out, "    }} catch {{");
            let _ = writeln!(out, "        tx.Rollback();");
            let _ = writeln!(out, "        throw;");
            let _ = writeln!(out, "    }}");
            let _ = write!(out, "}}");
            return Ok(out);
        }

        let return_type = match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => format!("{}?", struct_name),
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

        let is_one_returning = matches!(analyzed.command, QueryCommand::One | QueryCommand::Opt)
            && sql.to_uppercase().contains("RETURNING");

        let effective_sql = if is_one_returning {
            let into_clause = columns
                .iter()
                .enumerate()
                .map(|(i, _)| format!(":out{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{} INTO {}", sql, into_clause)
        } else {
            sql.clone()
        };

        let _ = writeln!(
            out,
            "public static async {} {}(OracleConnection conn{}{}) {{",
            task_type, func_name, sep, param_list
        );

        let _ = writeln!(
            out,
            "    using var cmd = new OracleCommand(@\"{}\", conn);",
            effective_sql
        );
        for p in params.iter() {
            let _ = writeln!(
                out,
                "    cmd.Parameters.Add(new OracleParameter {{ Value = (object){} ?? DBNull.Value }});",
                p.field_name
            );
        }

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                if is_one_returning {
                    for (i, col) in columns.iter().enumerate() {
                        let db_type = oracle_db_type(&col.neutral_type);
                        let size_part = if db_type == "OracleDbType.Varchar2" {
                            " Size = 4000,".to_string()
                        } else {
                            String::new()
                        };
                        let _ = writeln!(
                            out,
                            "    cmd.Parameters.Add(new OracleParameter {{ ParameterName = \"out{i}\", \
                             OracleDbType = {db_type},{size_part} Direction = System.Data.ParameterDirection.Output }});"
                        );
                    }
                    let _ = writeln!(out, "    await cmd.ExecuteNonQueryAsync();");
                    let _ = writeln!(out, "    return new {}(", struct_name);
                    for (i, col) in columns.iter().enumerate() {
                        let param_expr = format!("cmd.Parameters[\"out{i}\"].Value");
                        let cast = oracle_out_cast(&col.neutral_type, &param_expr);
                        let sep = if i + 1 < columns.len() { "," } else { "" };
                        let _ = writeln!(out, "        {cast}{sep}");
                    }
                    let _ = writeln!(out, "    );");
                } else {
                    let _ = writeln!(out, "    using var reader = await cmd.ExecuteReaderAsync();");
                    let _ = writeln!(out, "    if (!await reader.ReadAsync()) return null;");
                    let _ = writeln!(out, "    return new {}(", struct_name);
                    for (i, col) in columns.iter().enumerate() {
                        let method = reader_method(&col.neutral_type, &col.lang_type);
                        let sep = if i + 1 < columns.len() { "," } else { "" };
                        if col.nullable {
                            let _ = writeln!(out, "        reader.IsDBNull({i}) ? null : reader.{method}({i}){sep}");
                        } else {
                            let _ = writeln!(out, "        reader.{method}({i}){sep}");
                        }
                    }
                    let _ = writeln!(out, "    );");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "    using var reader = await cmd.ExecuteReaderAsync();");
                let _ = writeln!(out, "    var results = new List<{}>();", struct_name);
                let _ = writeln!(out, "    while (await reader.ReadAsync()) {{");
                let _ = writeln!(out, "        results.Add(new {}(", struct_name);
                for (i, col) in columns.iter().enumerate() {
                    let method = reader_method(&col.neutral_type, &col.lang_type);
                    let sep = if i + 1 < columns.len() { "," } else { "" };
                    if col.nullable {
                        let _ = writeln!(
                            out,
                            "            reader.IsDBNull({i}) ? null : reader.{method}({i}){sep}"
                        );
                    } else {
                        let _ = writeln!(out, "            reader.{method}({i}){sep}");
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
            |n| format!(":{n}"),
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
            "public static async Task<List<{parent_struct_name}>> {func_name}(OracleConnection conn{sep}{param_list}) {{"
        );
        let _ = writeln!(out, "    using var cmd = new OracleCommand(@\"{sql}\", conn);");
        for p in params.iter() {
            let _ = writeln!(
                out,
                "    cmd.Parameters.Add(new OracleParameter {{ Value = (object){} ?? DBNull.Value }});",
                p.field_name
            );
        }
        let _ = writeln!(out, "    using var reader = await cmd.ExecuteReaderAsync();");
        let _ = writeln!(
            out,
            "    var lookup = new Dictionary<{key_type}, {parent_struct_name}>();"
        );
        let _ = writeln!(out, "    var result = new List<{parent_struct_name}>();");
        let _ = writeln!(out, "    while (await reader.ReadAsync()) {{");

        let key_method = reader_method(&key_col.neutral_type, &key_col.lang_type);
        if key_col.nullable {
            let _ = writeln!(
                out,
                "        var key = reader.IsDBNull({key_ordinal}) ? null : (object)reader.{key_method}({key_ordinal});"
            );
        } else {
            let _ = writeln!(out, "        var key = reader.{key_method}({key_ordinal});");
        }

        let _ = writeln!(out, "        var child = new {child_struct_name}(");
        for (ci, col) in child_columns.iter().enumerate() {
            let ord = all_columns
                .iter()
                .position(|c| c.name == col.name)
                .unwrap_or(parent_columns.len() + ci);
            let method = reader_method(&col.neutral_type, &col.lang_type);
            let trailing = if ci + 1 < child_columns.len() { "," } else { "" };
            if col.nullable {
                let _ = writeln!(
                    out,
                    "            reader.IsDBNull({ord}) ? null : reader.{method}({ord}){trailing}"
                );
            } else {
                let _ = writeln!(out, "            reader.{method}({ord}){trailing}");
            }
        }
        let _ = writeln!(out, "        );");

        let _ = writeln!(out, "        if (lookup.TryGetValue(key, out var parent)) {{");
        let _ = writeln!(out, "            parent.Children.Add(child);");
        let _ = writeln!(out, "        }} else {{");
        let _ = writeln!(out, "            var newParent = new {parent_struct_name}(");
        for col in parent_columns {
            let ord = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
            let method = reader_method(&col.neutral_type, &col.lang_type);
            if col.nullable {
                let _ = writeln!(
                    out,
                    "                reader.IsDBNull({ord}) ? null : reader.{method}({ord}),"
                );
            } else {
                let _ = writeln!(out, "                reader.{method}({ord}),");
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
        let name = to_pascal_case(&composite.sql_name);
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

    use super::{oracle_db_type, oracle_out_cast, reader_method};

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
    fn test_grouped_csharp_oracle_structs() {
        let backend = crate::backends::get_backend("csharp-oracle", "oracle").unwrap();
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
    fn test_grouped_csharp_oracle_query_fn() {
        let backend = crate::backends::get_backend("csharp-oracle", "oracle").unwrap();
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
            query_fn.contains("OracleConnection conn"),
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

    /// ODP.NET's `OracleDecimal` exposes its managed value through a `Value`
    /// property. It has `ToInt32`/`ToInt64`/`ToDouble` but no `ToDecimal`, so
    /// emitting one produced C# that did not compile.
    #[test]
    fn decimal_out_param_reads_value_not_a_to_decimal_method() {
        let cast = oracle_out_cast("decimal", "p");
        assert!(cast.ends_with(").Value"), "decimal must read .Value; got: {cast}");
        assert!(
            !cast.contains("ToDecimal"),
            "OracleDecimal has no ToDecimal(); got: {cast}"
        );
    }

    /// `GetValue` returns `object`, which does not bind to a `byte[]` record
    /// field, so a BLOB column produced a CS1503 conversion error.
    #[test]
    fn bytes_column_is_read_as_a_concrete_byte_array() {
        assert_eq!(reader_method("bytes", "byte[]"), "GetFieldValue<byte[]>");
    }

    /// A BLOB bind defaulted to `Varchar2`, which is wrong for binary data.
    #[test]
    fn bytes_param_binds_as_blob() {
        assert_eq!(oracle_db_type("bytes"), "OracleDbType.Blob");
    }
}
