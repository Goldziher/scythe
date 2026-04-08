use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_camel_case, to_pascal_case,
};

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/java-jdbc.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/java-jdbc.mysql.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/java-jdbc.sqlite.toml");
const DEFAULT_MANIFEST_DUCKDB: &str = include_str!("../../manifests/java-jdbc.duckdb.toml");
const DEFAULT_MANIFEST_MARIADB: &str = include_str!("../../manifests/java-jdbc.mariadb.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/java-jdbc.redshift.toml");
const DEFAULT_MANIFEST_SNOWFLAKE: &str = include_str!("../../manifests/java-jdbc.snowflake.toml");

pub struct JavaJdbcBackend {
    manifest: BackendManifest,
}

impl JavaJdbcBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            "mysql" => DEFAULT_MANIFEST_MYSQL,
            "mariadb" => DEFAULT_MANIFEST_MARIADB,
            "sqlite" | "sqlite3" => DEFAULT_MANIFEST_SQLITE,
            "duckdb" => DEFAULT_MANIFEST_DUCKDB,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            "snowflake" => DEFAULT_MANIFEST_SNOWFLAKE,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("unsupported engine '{}' for java-jdbc backend", engine),
                ));
            }
        };
        let manifest =
            super::load_or_default_manifest("backends/java-jdbc/manifest.toml", default_toml)?;
        Ok(Self { manifest })
    }
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

/// Check whether a Java type is a primitive (not a reference type).
fn is_java_primitive(java_type: &str) -> bool {
    matches!(
        java_type,
        "boolean" | "byte" | "short" | "int" | "long" | "float" | "double" | "char"
    )
}

/// Format a Java parameter with nullability annotation.
fn java_annotated_param(param: &ResolvedParam) -> String {
    let param_type = java_param_type(param);
    if param.nullable {
        format!("@Nullable {} {}", param_type, param.field_name)
    } else if !is_java_primitive(&param.lang_type) {
        format!("@Nonnull {} {}", param_type, param.field_name)
    } else {
        format!("{} {}", param_type, param.field_name)
    }
}

impl CodegenBackend for JavaJdbcBackend {
    fn name(&self) -> &str {
        "java-jdbc"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &[
            "postgresql",
            "mysql",
            "mariadb",
            "sqlite",
            "duckdb",
            "redshift",
            "snowflake",
        ]
    }

    fn file_header(&self) -> String {
        "// Auto-generated by scythe. Do not edit.\n\
         import java.math.BigDecimal;\n\
         import java.sql.*;\n\
         import java.time.OffsetDateTime;\n\
         import java.util.ArrayList;\n\
         import java.util.List;\n\
         import javax.annotation.Nonnull;\n\
         import javax.annotation.Nullable;"
            .to_string()
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
        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(
                &analyzed.sql,
                &analyzed.optional_params,
                &analyzed.params,
            ),
            |_| "?".to_string(),
        );

        let param_list = params
            .iter()
            .map(java_annotated_param)
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
                    "public static @Nullable {} {}(Connection conn{}{}) throws SQLException {{",
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
            QueryCommand::Many => {
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
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                if params.len() > 1 {
                    // Generate params record
                    let params_record_name =
                        format!("{}BatchParams", to_pascal_case(&analyzed.name));
                    let record_fields = params
                        .iter()
                        .map(|p| format!("{} {}", java_param_type(p), p.field_name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(
                        out,
                        "public record {}({}) {{}}",
                        params_record_name, record_fields
                    );
                    let _ = writeln!(out);
                    let _ = writeln!(
                        out,
                        "public static void {}(Connection conn, java.util.List<{}> items) throws SQLException {{",
                        batch_fn_name, params_record_name
                    );
                    let _ = writeln!(out, "    conn.setAutoCommit(false);");
                    let _ = writeln!(
                        out,
                        "    try (var ps = conn.prepareStatement(\"{}\")) {{",
                        sql
                    );
                    let _ = writeln!(out, "        for (var item : items) {{");
                    for (i, param) in params.iter().enumerate() {
                        let setter = ps_setter(&param.lang_type);
                        let _ = writeln!(
                            out,
                            "            ps.{}({}, item.{}());",
                            setter,
                            i + 1,
                            param.field_name
                        );
                    }
                    let _ = writeln!(out, "            ps.addBatch();");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        ps.executeBatch();");
                    let _ = writeln!(out, "        conn.commit();");
                    let _ = writeln!(out, "    }} catch (SQLException e) {{");
                    let _ = writeln!(out, "        conn.rollback();");
                    let _ = writeln!(out, "        throw e;");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        conn.setAutoCommit(true);");
                    let _ = writeln!(out, "    }}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let param = &params[0];
                    let _ = writeln!(
                        out,
                        "public static void {}(Connection conn, java.util.List<{}> items) throws SQLException {{",
                        batch_fn_name,
                        java_param_type(param)
                    );
                    let _ = writeln!(out, "    conn.setAutoCommit(false);");
                    let _ = writeln!(
                        out,
                        "    try (var ps = conn.prepareStatement(\"{}\")) {{",
                        sql
                    );
                    let _ = writeln!(out, "        for (var item : items) {{");
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(out, "            ps.{}(1, item);", setter);
                    let _ = writeln!(out, "            ps.addBatch();");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        ps.executeBatch();");
                    let _ = writeln!(out, "        conn.commit();");
                    let _ = writeln!(out, "    }} catch (SQLException e) {{");
                    let _ = writeln!(out, "        conn.rollback();");
                    let _ = writeln!(out, "        throw e;");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        conn.setAutoCommit(true);");
                    let _ = writeln!(out, "    }}");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "public static void {}(Connection conn, int count) throws SQLException {{",
                        batch_fn_name
                    );
                    let _ = writeln!(out, "    conn.setAutoCommit(false);");
                    let _ = writeln!(
                        out,
                        "    try (var ps = conn.prepareStatement(\"{}\")) {{",
                        sql
                    );
                    let _ = writeln!(out, "        for (int i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "            ps.addBatch();");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        ps.executeBatch();");
                    let _ = writeln!(out, "        conn.commit();");
                    let _ = writeln!(out, "    }} catch (SQLException e) {{");
                    let _ = writeln!(out, "        conn.rollback();");
                    let _ = writeln!(out, "        throw e;");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        conn.setAutoCommit(true);");
                    let _ = writeln!(out, "    }}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Grouped => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "grouped queries are not yet supported for java-jdbc".to_string(),
                ));
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
