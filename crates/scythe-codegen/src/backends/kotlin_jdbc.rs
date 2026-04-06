use std::fmt::Write;
use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_camel_case, to_pascal_case,
};

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../../../backends/kotlin-jdbc/manifest.toml");

pub struct KotlinJdbcBackend {
    manifest: BackendManifest,
}

impl KotlinJdbcBackend {
    pub fn new() -> Result<Self, ScytheError> {
        let manifest_path = Path::new("backends/kotlin-jdbc/manifest.toml");
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
        .join(" ")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Convert PostgreSQL $1, $2, ... placeholders to JDBC ? placeholders.
fn pg_to_jdbc_params(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    chars.next();
                }
                result.push('?');
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Get the ResultSet getter method name for a given Kotlin type.
fn rs_getter(kotlin_type: &str) -> &str {
    match kotlin_type {
        "Boolean" => "getBoolean",
        "Byte" => "getByte",
        "Short" => "getShort",
        "Int" => "getInt",
        "Long" => "getLong",
        "Float" => "getFloat",
        "Double" => "getDouble",
        "String" => "getString",
        "ByteArray" => "getBytes",
        _ if kotlin_type.contains("BigDecimal") => "getBigDecimal",
        _ if kotlin_type.contains("LocalDate") => "getObject",
        _ if kotlin_type.contains("LocalTime") => "getObject",
        _ if kotlin_type.contains("OffsetTime") => "getObject",
        _ if kotlin_type.contains("LocalDateTime") => "getObject",
        _ if kotlin_type.contains("OffsetDateTime") => "getObject",
        _ if kotlin_type.contains("UUID") => "getObject",
        _ => "getObject",
    }
}

/// Get the PreparedStatement setter method name for a given Kotlin type.
fn ps_setter(kotlin_type: &str) -> &str {
    match kotlin_type {
        "Boolean" => "setBoolean",
        "Byte" => "setByte",
        "Short" => "setShort",
        "Int" => "setInt",
        "Long" => "setLong",
        "Float" => "setFloat",
        "Double" => "setDouble",
        "String" => "setString",
        "ByteArray" => "setBytes",
        _ if kotlin_type.contains("BigDecimal") => "setBigDecimal",
        _ => "setObject",
    }
}

impl CodegenBackend for KotlinJdbcBackend {
    fn name(&self) -> &str {
        "kotlin-jdbc"
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "data class {}(", struct_name);
        for (i, col) in columns.iter().enumerate() {
            let sep = if i + 1 < columns.len() { "," } else { "" };
            let _ = writeln!(out, "    val {}: {}{}", col.field_name, col.full_type, sep);
        }
        let _ = write!(out, ")");
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
        let sql = pg_to_jdbc_params(&clean_sql(&analyzed.sql));

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let mut out = String::new();

        match &analyzed.command {
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "fun {}(conn: Connection{}{}) {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                for (i, param) in params.iter().enumerate() {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(
                        out,
                        "        ps.{}({}, {})",
                        setter,
                        i + 1,
                        param.field_name
                    );
                }
                let _ = writeln!(out, "        ps.executeUpdate()");
                let _ = writeln!(out, "    }}");
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "fun {}(conn: Connection{}{}): Int {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(
                    out,
                    "    return conn.prepareStatement(\"{}\").use {{ ps ->",
                    sql
                );
                for (i, param) in params.iter().enumerate() {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(
                        out,
                        "        ps.{}({}, {})",
                        setter,
                        i + 1,
                        param.field_name
                    );
                }
                let _ = writeln!(out, "        ps.executeUpdate()");
                let _ = writeln!(out, "    }}");
                let _ = write!(out, "}}");
            }
            QueryCommand::One => {
                let _ = writeln!(
                    out,
                    "fun {}(conn: Connection{}{}): {}? {{",
                    func_name, sep, param_list, struct_name
                );
                let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                for (i, param) in params.iter().enumerate() {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(
                        out,
                        "        ps.{}({}, {})",
                        setter,
                        i + 1,
                        param.field_name
                    );
                }
                let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
                let _ = writeln!(out, "            return if (rs.next()) {}(", struct_name);
                for (i, col) in columns.iter().enumerate() {
                    let getter = rs_getter(&col.lang_type);
                    let sep = if i + 1 < columns.len() { "," } else { "" };
                    let _ = writeln!(
                        out,
                        "                {} = rs.{}(\"{}\"){}",
                        col.field_name, getter, col.name, sep
                    );
                }
                let _ = writeln!(out, "            ) else null");
                let _ = writeln!(out, "        }}");
                let _ = writeln!(out, "    }}");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many | QueryCommand::Batch => {
                let _ = writeln!(
                    out,
                    "fun {}(conn: Connection{}{}): List<{}> {{",
                    func_name, sep, param_list, struct_name
                );
                let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                for (i, param) in params.iter().enumerate() {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(
                        out,
                        "        ps.{}({}, {})",
                        setter,
                        i + 1,
                        param.field_name
                    );
                }
                let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
                let _ = writeln!(
                    out,
                    "            val result = mutableListOf<{}>()",
                    struct_name
                );
                let _ = writeln!(out, "            while (rs.next()) {{");
                let _ = writeln!(out, "                result.add({}(", struct_name);
                for (i, col) in columns.iter().enumerate() {
                    let getter = rs_getter(&col.lang_type);
                    let sep = if i + 1 < columns.len() { "," } else { "" };
                    let _ = writeln!(
                        out,
                        "                    {} = rs.{}(\"{}\"){}",
                        col.field_name, getter, col.name, sep
                    );
                }
                let _ = writeln!(out, "                ))");
                let _ = writeln!(out, "            }}");
                let _ = writeln!(out, "            return result");
                let _ = writeln!(out, "        }}");
                let _ = writeln!(out, "    }}");
                let _ = write!(out, "}}");
            }
        }

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "enum class {}(val value: String) {{", type_name);
        for (i, value) in enum_info.values.iter().enumerate() {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let sep = if i + 1 < enum_info.values.len() {
                ","
            } else {
                ";"
            };
            let _ = writeln!(out, "    {}(\"{}\"){}", variant, value, sep);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = writeln!(out, "data class {}(", name);
        if composite.fields.is_empty() {
            let _ = writeln!(out, "    // TODO: fields");
        } else {
            for (i, field) in composite.fields.iter().enumerate() {
                let field_name = to_camel_case(&field.name);
                let sep = if i + 1 < composite.fields.len() {
                    ","
                } else {
                    ""
                };
                let _ = writeln!(out, "    val {}: Any?{}", field_name, sep);
            }
        }
        let _ = write!(out, ")");
        Ok(out)
    }
}
