use std::fmt::Write;
use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case,
};

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str =
    include_str!("../../../../backends/csharp-npgsql/manifest.toml");

pub struct CsharpNpgsqlBackend {
    manifest: BackendManifest,
}

impl CsharpNpgsqlBackend {
    pub fn new() -> Result<Self, ScytheError> {
        let manifest_path = Path::new("backends/csharp-npgsql/manifest.toml");
        let manifest = if manifest_path.exists() {
            load_manifest(manifest_path)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        } else {
            toml::from_str(DEFAULT_MANIFEST_TOML)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        };
        Ok(Self { manifest })
    }

    pub fn manifest(&self) -> &BackendManifest {
        &self.manifest
    }
}

/// Strip SQL comments, trailing semicolons, and excess whitespace.
fn clean_sql(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Map a neutral type to an Npgsql reader method.
fn reader_method(neutral_type: &str) -> &'static str {
    match neutral_type {
        "bool" => "GetBoolean",
        "int16" => "GetInt16",
        "int32" => "GetInt32",
        "int64" => "GetInt64",
        "float32" => "GetFloat",
        "float64" => "GetDouble",
        "string" | "json" | "inet" | "interval" => "GetString",
        "uuid" => "GetGuid",
        "decimal" => "GetDecimal",
        "date" => "GetFieldValue<DateOnly>",
        "time" | "time_tz" => "GetFieldValue<TimeOnly>",
        "datetime" => "GetDateTime",
        "datetime_tz" => "GetFieldValue<DateTimeOffset>",
        _ => "GetValue",
    }
}

/// Rewrite $1, $2, ... to @p1, @p2, ...
fn rewrite_params(sql: &str) -> String {
    let mut result = sql.to_string();
    // Replace from highest number down to avoid $1 matching inside $10
    for i in (1..=99).rev() {
        let from = format!("${}", i);
        let to = format!("@p{}", i);
        result = result.replace(&from, &to);
    }
    result
}

impl CodegenBackend for CsharpNpgsqlBackend {
    fn name(&self) -> &str {
        "csharp-npgsql"
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
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

    fn generate_model_struct(
        &self,
        table_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let name = to_pascal_case(table_name);
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
        let sql = rewrite_params(&clean_sql(&analyzed.sql));
        let mut out = String::new();

        // Build C# parameter list
        let param_list = params
            .iter()
            .map(|p| format!("{} {}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        // Return type depends on command
        let return_type = match &analyzed.command {
            QueryCommand::One => format!("{}?", struct_name),
            QueryCommand::Many | QueryCommand::Batch => {
                format!("List<{}>", struct_name)
            }
            QueryCommand::Exec => "void".to_string(),
            QueryCommand::ExecResult | QueryCommand::ExecRows => "int".to_string(),
        };

        let is_async_void = return_type == "void";
        let task_type = if is_async_void {
            "Task".to_string()
        } else {
            format!("Task<{}>", return_type)
        };

        let _ = writeln!(
            out,
            "public static async {} {}(NpgsqlConnection conn{}{}) {{",
            task_type, func_name, sep, param_list
        );

        // Command setup
        let _ = writeln!(
            out,
            "    await using var cmd = new NpgsqlCommand(\"{}\", conn);",
            sql
        );
        for (i, p) in params.iter().enumerate() {
            let _ = writeln!(
                out,
                "    cmd.Parameters.AddWithValue(\"p{}\", {});",
                i + 1,
                p.field_name
            );
        }

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(
                    out,
                    "    await using var reader = await cmd.ExecuteReaderAsync();"
                );
                let _ = writeln!(out, "    if (!await reader.ReadAsync()) return null;");
                let _ = writeln!(out, "    return new {}(", struct_name);
                for (i, col) in columns.iter().enumerate() {
                    let method = reader_method(&col.neutral_type);
                    let sep = if i + 1 < columns.len() { "," } else { "" };
                    if col.nullable {
                        let _ = writeln!(
                            out,
                            "        reader.IsDBNull({i}) ? null : reader.{method}({i}){sep}"
                        );
                    } else {
                        let _ = writeln!(out, "        reader.{method}({i}){sep}");
                    }
                }
                let _ = writeln!(out, "    );");
            }
            QueryCommand::Many | QueryCommand::Batch => {
                let _ = writeln!(
                    out,
                    "    await using var reader = await cmd.ExecuteReaderAsync();"
                );
                let _ = writeln!(out, "    var results = new List<{}>();", struct_name);
                let _ = writeln!(out, "    while (await reader.ReadAsync()) {{");
                let _ = writeln!(out, "        results.Add(new {}(", struct_name);
                for (i, col) in columns.iter().enumerate() {
                    let method = reader_method(&col.neutral_type);
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
        }

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
        let _ = writeln!(out, "public record {}();", name);
        Ok(out)
    }
}
