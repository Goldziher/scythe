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

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/java-jdbc.toml");

pub struct JavaJdbcBackend {
    manifest: BackendManifest,
}

impl JavaJdbcBackend {
    pub fn new() -> Result<Self, ScytheError> {
        let manifest_path = Path::new("backends/java-jdbc/manifest.toml");
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

/// Convert PostgreSQL $1, $2, ... placeholders to JDBC ? placeholders.
fn pg_to_jdbc_params(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '$' {
            // Check if followed by digits
            if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                // Consume all digits
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

/// Convert a Java primitive type to its boxed equivalent for nullable usage.
fn box_primitive(java_type: &str) -> &str {
    match java_type {
        "boolean" => "Boolean",
        "byte" => "Byte",
        "short" => "Short",
        "int" => "Integer",
        "long" => "Long",
        "float" => "Float",
        "double" => "Double",
        "char" => "Character",
        _ => java_type,
    }
}

/// Get the ResultSet getter method name for a given Java type.
fn rs_getter(java_type: &str) -> &str {
    match java_type {
        "boolean" | "Boolean" => "getBoolean",
        "byte" | "Byte" => "getByte",
        "short" | "Short" => "getShort",
        "int" | "Integer" => "getInt",
        "long" | "Long" => "getLong",
        "float" | "Float" => "getFloat",
        "double" | "Double" => "getDouble",
        "String" => "getString",
        "byte[]" => "getBytes",
        _ if java_type.contains("BigDecimal") => "getBigDecimal",
        _ if java_type.contains("LocalDate") => "getObject",
        _ if java_type.contains("LocalTime") => "getObject",
        _ if java_type.contains("OffsetTime") => "getObject",
        _ if java_type.contains("LocalDateTime") => "getObject",
        _ if java_type.contains("OffsetDateTime") => "getObject",
        _ if java_type.contains("UUID") => "getObject",
        _ => "getObject",
    }
}

/// Get the PreparedStatement setter method name for a given Java type.
fn ps_setter(java_type: &str) -> &str {
    match java_type {
        "boolean" | "Boolean" => "setBoolean",
        "byte" | "Byte" => "setByte",
        "short" | "Short" => "setShort",
        "int" | "Integer" => "setInt",
        "long" | "Long" => "setLong",
        "float" | "Float" => "setFloat",
        "double" | "Double" => "setDouble",
        "String" => "setString",
        "byte[]" => "setBytes",
        _ if java_type.contains("BigDecimal") => "setBigDecimal",
        _ => "setObject",
    }
}

/// Resolve the display type for a Java field, boxing primitives when nullable.
fn java_field_type(col: &ResolvedColumn) -> String {
    if col.nullable {
        box_primitive(&col.lang_type).to_string()
    } else {
        col.full_type.clone()
    }
}

/// Resolve the display type for a Java param, boxing primitives when nullable.
fn java_param_type(param: &ResolvedParam) -> String {
    if param.nullable {
        box_primitive(&param.lang_type).to_string()
    } else {
        param.full_type.clone()
    }
}

impl CodegenBackend for JavaJdbcBackend {
    fn name(&self) -> &str {
        "java-jdbc"
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();

        // Record declaration with fields
        let fields = columns
            .iter()
            .map(|c| {
                let field_type = java_field_type(c);
                if c.nullable {
                    format!("    @Nullable {} {}", field_type, c.field_name)
                } else {
                    format!("    {} {}", field_type, c.field_name)
                }
            })
            .collect::<Vec<_>>()
            .join(",\n");

        let _ = writeln!(out, "public record {}(", struct_name);
        let _ = writeln!(out, "{}", fields);
        let _ = writeln!(out, ") {{");

        // fromResultSet static factory method
        let _ = writeln!(
            out,
            "    public static {} fromResultSet(ResultSet rs) throws SQLException {{",
            struct_name
        );
        let _ = writeln!(out, "        return new {}(", struct_name);
        for (i, col) in columns.iter().enumerate() {
            let getter = rs_getter(&col.lang_type);
            let sep = if i + 1 < columns.len() { "," } else { "" };
            let _ = writeln!(out, "            rs.{}(\"{}\"){}", getter, col.name, sep);
        }
        let _ = writeln!(out, "        );");
        let _ = writeln!(out, "    }}");
        let _ = write!(out, "}}");
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
        _columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = pg_to_jdbc_params(&super::clean_sql_oneline(&analyzed.sql));

        let param_list = params
            .iter()
            .map(|p| {
                let param_type = java_param_type(p);
                format!("{} {}", param_type, p.field_name)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let mut out = String::new();

        match &analyzed.command {
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "public static void {}(Connection conn{}{}) throws SQLException {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(
                    out,
                    "    try (var ps = conn.prepareStatement(\"{}\")) {{",
                    sql
                );
                for (i, param) in params.iter().enumerate() {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(
                        out,
                        "        ps.{}({}, {});",
                        setter,
                        i + 1,
                        param.field_name
                    );
                }
                let _ = writeln!(out, "        ps.executeUpdate();");
                let _ = writeln!(out, "    }}");
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "public static int {}(Connection conn{}{}) throws SQLException {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(
                    out,
                    "    try (var ps = conn.prepareStatement(\"{}\")) {{",
                    sql
                );
                for (i, param) in params.iter().enumerate() {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(
                        out,
                        "        ps.{}({}, {});",
                        setter,
                        i + 1,
                        param.field_name
                    );
                }
                let _ = writeln!(out, "        return ps.executeUpdate();");
                let _ = writeln!(out, "    }}");
                let _ = write!(out, "}}");
            }
            QueryCommand::One => {
                let _ = writeln!(
                    out,
                    "public static {} {}(Connection conn{}{}) throws SQLException {{",
                    struct_name, func_name, sep, param_list
                );
                let _ = writeln!(
                    out,
                    "    try (var ps = conn.prepareStatement(\"{}\")) {{",
                    sql
                );
                for (i, param) in params.iter().enumerate() {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(
                        out,
                        "        ps.{}({}, {});",
                        setter,
                        i + 1,
                        param.field_name
                    );
                }
                let _ = writeln!(out, "        try (ResultSet rs = ps.executeQuery()) {{");
                let _ = writeln!(out, "            if (rs.next()) {{");
                let _ = writeln!(
                    out,
                    "                return {}.fromResultSet(rs);",
                    struct_name
                );
                let _ = writeln!(out, "            }}");
                let _ = writeln!(out, "            return null;");
                let _ = writeln!(out, "        }}");
                let _ = writeln!(out, "    }}");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many | QueryCommand::Batch => {
                let _ = writeln!(
                    out,
                    "public static java.util.List<{}> {}(Connection conn{}{}) throws SQLException {{",
                    struct_name, func_name, sep, param_list
                );
                let _ = writeln!(
                    out,
                    "    try (var ps = conn.prepareStatement(\"{}\")) {{",
                    sql
                );
                for (i, param) in params.iter().enumerate() {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(
                        out,
                        "        ps.{}({}, {});",
                        setter,
                        i + 1,
                        param.field_name
                    );
                }
                let _ = writeln!(out, "        try (ResultSet rs = ps.executeQuery()) {{");
                let _ = writeln!(
                    out,
                    "            java.util.List<{}> result = new java.util.ArrayList<>();",
                    struct_name
                );
                let _ = writeln!(out, "            while (rs.next()) {{");
                let _ = writeln!(
                    out,
                    "                result.add({}.fromResultSet(rs));",
                    struct_name
                );
                let _ = writeln!(out, "            }}");
                let _ = writeln!(out, "            return result;");
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
        let _ = writeln!(out, "public enum {} {{", type_name);
        for (i, value) in enum_info.values.iter().enumerate() {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let sep = if i + 1 < enum_info.values.len() {
                ","
            } else {
                ";"
            };
            let _ = writeln!(out, "    {}(\"{}\"){}", variant, value, sep);
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "    private final String value;");
        let _ = writeln!(
            out,
            "    {}(String value) {{ this.value = value; }}",
            type_name
        );
        let _ = writeln!(out, "    public String getValue() {{ return value; }}");
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        if composite.fields.is_empty() {
            let _ = writeln!(out, "public record {}() {{}}", name);
        } else {
            let fields = composite
                .fields
                .iter()
                .map(|f| format!("Object {}", to_camel_case(&f.name)))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "public record {}({}) {{}}", name, fields);
        }
        Ok(out)
    }
}
