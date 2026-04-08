use std::fmt::Write;
use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_camel_case, to_pascal_case,
};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/kotlin-jdbc.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/kotlin-jdbc.mysql.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/kotlin-jdbc.sqlite.toml");

pub struct KotlinJdbcBackend {
    manifest: BackendManifest,
}

impl KotlinJdbcBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            "mysql" | "mariadb" => DEFAULT_MANIFEST_MYSQL,
            "sqlite" | "sqlite3" => DEFAULT_MANIFEST_SQLITE,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("unsupported engine '{}' for kotlin-jdbc backend", engine),
                ));
            }
        };
        let manifest_path = Path::new("backends/kotlin-jdbc/manifest.toml");
        let manifest = if manifest_path.exists() {
            load_manifest(manifest_path)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        } else {
            toml::from_str(default_toml)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        };
        Ok(Self { manifest })
    }
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

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "mysql", "sqlite"]
    }

    fn file_header(&self) -> String {
        "import java.sql.Connection\n".to_string()
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "data class {}(", struct_name);
        for col in columns.iter() {
            let _ = writeln!(out, "    val {}: {},", col.field_name, col.full_type);
        }
        let _ = writeln!(out, ")");
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
        let sql = pg_to_jdbc_params(&super::clean_sql_oneline_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        // Build function params: inline for single param (conn only), multi-line for 2+
        let use_multiline_params = !params.is_empty();

        let mut out = String::new();

        // Helper: write param setters
        let write_setters = |out: &mut String, params: &[ResolvedParam]| {
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
        };

        // Helper: write function signature
        let write_fn_sig =
            |out: &mut String, name: &str, ret: &str, multiline: bool, params: &[ResolvedParam]| {
                if multiline {
                    let _ = writeln!(out, "fun {}(", name);
                    let _ = writeln!(out, "    conn: Connection,");
                    for p in params {
                        let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                    }
                    let _ = writeln!(out, "){} {{", ret);
                } else {
                    let _ = writeln!(out, "fun {}(conn: Connection){} {{", name, ret);
                }
            };

        match &analyzed.command {
            QueryCommand::Exec => {
                write_fn_sig(&mut out, &func_name, "", use_multiline_params, params);
                let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                write_setters(&mut out, params);
                let _ = writeln!(out, "        ps.executeUpdate()");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_fn_sig(&mut out, &func_name, ": Int", use_multiline_params, params);
                let _ = writeln!(
                    out,
                    "    return conn.prepareStatement(\"{}\").use {{ ps ->",
                    sql
                );
                write_setters(&mut out, params);
                let _ = writeln!(out, "        ps.executeUpdate()");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "}}");
            }
            QueryCommand::One => {
                let ret = format!(": {}?", struct_name);
                write_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params);
                let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                write_setters(&mut out, params);
                let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
                let _ = writeln!(out, "            return if (rs.next()) {{");
                let _ = writeln!(out, "                {}(", struct_name);
                for col in columns.iter() {
                    let getter = rs_getter(&col.lang_type);
                    let _ = writeln!(
                        out,
                        "                    {} = rs.{}(\"{}\"),",
                        col.field_name, getter, col.name
                    );
                }
                let _ = writeln!(out, "                )");
                let _ = writeln!(out, "            }} else {{");
                let _ = writeln!(out, "                null");
                let _ = writeln!(out, "            }}");
                let _ = writeln!(out, "        }}");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                if params.len() > 1 {
                    let params_class_name =
                        format!("{}BatchParams", to_pascal_case(&analyzed.name));
                    let _ = writeln!(out, "data class {}(", params_class_name);
                    for p in params {
                        let _ = writeln!(out, "    val {}: {},", p.field_name, p.full_type);
                    }
                    let _ = writeln!(out, ")");
                    let _ = writeln!(out);
                    let _ = writeln!(out, "fun {}(", batch_fn_name);
                    let _ = writeln!(out, "    conn: Connection,");
                    let _ = writeln!(out, "    items: List<{}>,", params_class_name);
                    let _ = writeln!(out, ") {{");
                    let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                    let _ = writeln!(out, "        for (item in items) {{");
                    for (i, param) in params.iter().enumerate() {
                        let setter = ps_setter(&param.lang_type);
                        let _ = writeln!(
                            out,
                            "            ps.{}({}, item.{})",
                            setter,
                            i + 1,
                            param.field_name
                        );
                    }
                    let _ = writeln!(out, "            ps.addBatch()");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        ps.executeBatch()");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "fun {}(", batch_fn_name);
                    let _ = writeln!(out, "    conn: Connection,");
                    let _ = writeln!(out, "    items: List<{}>,", params[0].full_type);
                    let _ = writeln!(out, ") {{");
                    let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                    let _ = writeln!(out, "        for (item in items) {{");
                    let setter = ps_setter(&params[0].lang_type);
                    let _ = writeln!(out, "            ps.{}(1, item)", setter);
                    let _ = writeln!(out, "            ps.addBatch()");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        ps.executeBatch()");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "fun {}(conn: Connection, count: Int) {{",
                        batch_fn_name
                    );
                    let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                    let _ = writeln!(out, "        repeat(count) {{");
                    let _ = writeln!(out, "            ps.addBatch()");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        ps.executeBatch()");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let ret = format!(": List<{}>", struct_name);
                write_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params);
                let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                write_setters(&mut out, params);
                let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
                let _ = writeln!(
                    out,
                    "            val result = mutableListOf<{}>()",
                    struct_name
                );
                let _ = writeln!(out, "            while (rs.next()) {{");
                let _ = writeln!(out, "                result.add(");
                let _ = writeln!(out, "                    {}(", struct_name);
                for col in columns.iter() {
                    let getter = rs_getter(&col.lang_type);
                    let _ = writeln!(
                        out,
                        "                        {} = rs.{}(\"{}\"),",
                        col.field_name, getter, col.name
                    );
                }
                let _ = writeln!(out, "                    ),");
                let _ = writeln!(out, "                )");
                let _ = writeln!(out, "            }}");
                let _ = writeln!(out, "            return result");
                let _ = writeln!(out, "        }}");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "}}");
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
        let _ = writeln!(out, "}}");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = writeln!(out, "data class {}(", name);
        for field in composite.fields.iter() {
            let field_name = to_camel_case(&field.name);
            let field_type = resolve_type(&field.neutral_type, &self.manifest, false)
                .map(|t| t.into_owned())
                .unwrap_or_else(|_| "Any".to_string());
            let _ = writeln!(out, "    val {}: {},", field_name, field_type);
        }
        let _ = writeln!(out, ")");
        Ok(out)
    }
}
