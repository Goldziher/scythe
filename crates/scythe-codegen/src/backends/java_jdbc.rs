use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_camel_case, to_pascal_case,
};

use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/java-jdbc.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/java-jdbc.mysql.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/java-jdbc.sqlite.toml");
const DEFAULT_MANIFEST_DUCKDB: &str = include_str!("../../manifests/java-jdbc.duckdb.toml");
const DEFAULT_MANIFEST_MARIADB: &str = include_str!("../../manifests/java-jdbc.mariadb.toml");
const DEFAULT_MANIFEST_MSSQL: &str = include_str!("../../manifests/java-jdbc.mssql.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/java-jdbc.redshift.toml");
const DEFAULT_MANIFEST_SNOWFLAKE: &str = include_str!("../../manifests/java-jdbc.snowflake.toml");
const DEFAULT_MANIFEST_ORACLE: &str = include_str!("../../manifests/java-jdbc.oracle.toml");

pub struct JavaJdbcBackend {
    manifest: BackendManifest,
    engine: String,
}

impl JavaJdbcBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            "mysql" => DEFAULT_MANIFEST_MYSQL,
            "mariadb" => DEFAULT_MANIFEST_MARIADB,
            "sqlite" | "sqlite3" => DEFAULT_MANIFEST_SQLITE,
            "duckdb" => DEFAULT_MANIFEST_DUCKDB,
            "mssql" => DEFAULT_MANIFEST_MSSQL,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            "snowflake" => DEFAULT_MANIFEST_SNOWFLAKE,
            "oracle" => DEFAULT_MANIFEST_ORACLE,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("unsupported engine '{}' for java-jdbc backend", engine),
                ));
            }
        };
        let manifest = super::load_or_default_manifest("backends/java-jdbc/manifest.toml", default_toml)?;
        Ok(Self {
            manifest,
            engine: engine.to_string(),
        })
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

/// Return the class literal for temporal types that need `rs.getObject("col", Type.class)`.
/// Returns None for non-temporal types.
fn temporal_class_literal(java_type: &str) -> Option<&str> {
    if java_type.contains("LocalDate") && !java_type.contains("LocalDateTime") {
        Some("LocalDate.class")
    } else if java_type.contains("LocalTime") && !java_type.contains("LocalDateTime") {
        Some("LocalTime.class")
    } else if java_type.contains("OffsetTime") {
        Some("OffsetTime.class")
    } else if java_type.contains("LocalDateTime") {
        Some("LocalDateTime.class")
    } else if java_type.contains("OffsetDateTime") {
        Some("OffsetDateTime.class")
    } else {
        None
    }
}

/// Map a neutral type to the java.sql.Types constant used for Oracle OUT parameters.
fn oracle_jdbc_type(neutral_type: &str) -> &'static str {
    match neutral_type {
        "int32" | "int64" | "float32" | "float64" | "decimal" => "java.sql.Types.NUMERIC",
        "date" | "datetime" => "java.sql.Types.TIMESTAMP",
        "datetime_tz" => "java.sql.Types.TIMESTAMP_WITH_TIMEZONE",
        "string" | "json" | "uuid" | "inet" | "interval" => "java.sql.Types.VARCHAR",
        _ => "java.sql.Types.VARCHAR",
    }
}

/// Build the full CallableStatement getter call expression for an Oracle OUT parameter.
/// Returns the complete expression like `getLong(3)` or `getObject(3, LocalDateTime.class)`.
fn oracle_cs_getter_call(neutral_type: &str, index: usize) -> String {
    match neutral_type {
        "int32" => format!("getInt({})", index),
        "int64" => format!("getLong({})", index),
        "float32" => format!("getFloat({})", index),
        "float64" => format!("getDouble({})", index),
        "decimal" => format!("getBigDecimal({})", index),
        "date" | "datetime" => format!("getObject({}, LocalDateTime.class)", index),
        "datetime_tz" => format!("getObject({}, OffsetDateTime.class)", index),
        _ => format!("getString({})", index),
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

/// Get the PreparedStatement setter call for a parameter, handling enums specially.
/// PostgreSQL requires `setObject(n, val, Types.OTHER)` for custom enum types.
/// MySQL/MariaDB/Oracle use `setString(n, val.getValue())`.
fn ps_bind_param(param: &ResolvedParam, index: usize, engine: &str) -> String {
    if param.neutral_type.starts_with("enum::") {
        if engine == "postgresql" {
            format!(
                "ps.setObject({}, {}.getValue(), java.sql.Types.OTHER);",
                index + 1,
                param.field_name
            )
        } else {
            format!("ps.setString({}, {}.getValue());", index + 1, param.field_name)
        }
    } else {
        let setter = ps_setter(&param.lang_type);
        format!("ps.{}({}, {});", setter, index + 1, param.field_name)
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

/// Build the inline JDBC ResultSet expression for a column (read by column name).
/// For nullable primitives, the variable name is returned — the preamble has already
/// extracted the value and performed the wasNull() check.
fn col_rs_expr(col: &ResolvedColumn) -> String {
    if col.nullable && is_java_primitive(&col.lang_type) {
        col.field_name.clone()
    } else if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
        format!("rs.getObject(\"{}\", {})", col.name, class_lit)
    } else if col.neutral_type.starts_with("enum::") {
        format!(
            "{}.valueOf(rs.getString(\"{}\").toUpperCase())",
            col.lang_type, col.name
        )
    } else {
        let getter = rs_getter(&col.lang_type);
        format!("rs.{}(\"{}\")", getter, col.name)
    }
}

/// Emit nullable-primitive preamble variable declarations for grouped JDBC folding.
fn write_jdbc_nullable_preamble(out: &mut String, cols: &[ResolvedColumn], indent: &str) {
    for col in cols {
        if col.nullable && is_java_primitive(&col.lang_type) {
            let getter = rs_getter(&col.lang_type);
            let _ = writeln!(
                out,
                "{}var {}Raw = rs.{}(\"{}\");",
                indent, col.field_name, getter, col.name
            );
            let _ = writeln!(
                out,
                "{}{} {} = rs.wasNull() ? null : {}Raw;",
                indent,
                box_primitive(&col.lang_type),
                col.field_name,
                col.field_name
            );
        }
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
            "mssql",
            "redshift",
            "snowflake",
            "oracle",
        ]
    }

    fn file_header(&self) -> String {
        "// Auto-generated by scythe. Do not edit.\n\
         package generated;\n\
         \n\
         import java.math.BigDecimal;\n\
         import java.sql.*;\n\
         import java.time.*;\n\
         import java.util.ArrayList;\n\
         import java.util.List;\n\
         import javax.annotation.Nonnull;\n\
         import javax.annotation.Nullable;\n\
         \n\
         public class Queries {"
            .to_string()
    }

    fn file_footer(&self) -> String {
        "}\n".to_string()
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
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
        // Check if any nullable primitives need wasNull() handling
        let needs_preamble = columns.iter().any(|c| c.nullable && is_java_primitive(&c.lang_type));
        if needs_preamble {
            for col in columns.iter() {
                if col.nullable && is_java_primitive(&col.lang_type) {
                    let getter = rs_getter(&col.lang_type);
                    let _ = writeln!(
                        out,
                        "        var {}Raw = rs.{}(\"{}\");",
                        col.field_name, getter, col.name
                    );
                    let _ = writeln!(
                        out,
                        "        {} {} = rs.wasNull() ? null : {}Raw;",
                        box_primitive(&col.lang_type),
                        col.field_name,
                        col.field_name
                    );
                }
            }
        }
        let _ = writeln!(out, "        return new {}(", struct_name);
        for (i, col) in columns.iter().enumerate() {
            let sep = if i + 1 < columns.len() { "," } else { "" };
            if col.nullable && is_java_primitive(&col.lang_type) {
                // Already extracted above with wasNull() check
                let _ = writeln!(out, "            {}{}", col.field_name, sep);
            } else if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
                let _ = writeln!(out, "            rs.getObject(\"{}\", {}){}", col.name, class_lit, sep);
            } else if col.neutral_type.starts_with("enum::") {
                // Enum columns: convert DB string to enum via valueOf(UPPER_CASE)
                let _ = writeln!(
                    out,
                    "            {}.valueOf(rs.getString(\"{}\").toUpperCase()){}",
                    col.lang_type, col.name, sep
                );
            } else {
                let getter = rs_getter(&col.lang_type);
                let _ = writeln!(out, "            rs.{}(\"{}\"){}", getter, col.name, sep);
            }
        }
        let _ = writeln!(out, "        );");
        let _ = writeln!(out, "    }}");
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
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
        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        );

        let param_list = params.iter().map(java_annotated_param).collect::<Vec<_>>().join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let mut out = String::new();

        match &analyzed.command {
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "public static void {}(Connection conn{}{}) throws SQLException {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
                for (i, param) in params.iter().enumerate() {
                    let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
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
                let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
                for (i, param) in params.iter().enumerate() {
                    let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
                }
                let _ = writeln!(out, "        return ps.executeUpdate();");
                let _ = writeln!(out, "    }}");
                let _ = write!(out, "}}");
            }
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(
                    out,
                    "public static @Nullable {} {}(Connection conn{}{}) throws SQLException {{",
                    struct_name, func_name, sep, param_list
                );
                let is_oracle_returning = self.engine == "oracle" && sql.to_uppercase().contains("RETURNING");
                let is_mariadb_returning = self.engine == "mariadb" && sql.to_uppercase().contains("RETURNING");
                if is_mariadb_returning {
                    // MySQL Connector/J rejects executeQuery() for DML statements.
                    // MariaDB RETURNING works via execute() + getResultSet() instead.
                    let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
                    for (i, param) in params.iter().enumerate() {
                        let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
                    }
                    let _ = writeln!(out, "        ps.execute();");
                    let _ = writeln!(out, "        ResultSet rs = ps.getResultSet();");
                    let _ = writeln!(out, "        if (rs != null && rs.next()) {{");
                    let _ = writeln!(out, "            return {}.fromResultSet(rs);", struct_name);
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        return null;");
                    let _ = writeln!(out, "    }}");
                    let _ = write!(out, "}}");
                } else if is_oracle_returning {
                    // Oracle RETURNING … INTO requires a PL/SQL BEGIN…END block so that
                    // the JDBC driver correctly maps the OUT parameters from a DML statement.
                    // Plain prepareCall on a bare DML RETURNING INTO raises ORA-17173.
                    let into_placeholders = columns.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
                    let full_sql = format!("BEGIN {} INTO {}; END;", sql, into_placeholders);
                    let _ = writeln!(out, "    try (var cs = conn.prepareCall(\"{}\")) {{", full_sql);
                    for (i, param) in params.iter().enumerate() {
                        let setter = ps_setter(&param.lang_type);
                        let _ = writeln!(out, "        cs.{}({}, {});", setter, i + 1, param.field_name);
                    }
                    for (i, col) in columns.iter().enumerate() {
                        let jdbc_type = oracle_jdbc_type(&col.neutral_type);
                        let _ = writeln!(
                            out,
                            "        cs.registerOutParameter({}, {});",
                            params.len() + i + 1,
                            jdbc_type
                        );
                    }
                    let _ = writeln!(out, "        cs.execute();");
                    let _ = writeln!(out, "        return new {}(", struct_name);
                    for (i, col) in columns.iter().enumerate() {
                        let getter_call = oracle_cs_getter_call(&col.neutral_type, params.len() + i + 1);
                        let sep = if i + 1 < columns.len() { "," } else { "" };
                        let _ = writeln!(out, "            cs.{}{}", getter_call, sep);
                    }
                    let _ = writeln!(out, "        );");
                    let _ = writeln!(out, "    }}");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
                    for (i, param) in params.iter().enumerate() {
                        let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
                    }
                    let _ = writeln!(out, "        try (ResultSet rs = ps.executeQuery()) {{");
                    let _ = writeln!(out, "            if (rs.next()) {{");
                    let _ = writeln!(out, "                return {}.fromResultSet(rs);", struct_name);
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "            return null;");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "    }}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "public static List<{}> {}(Connection conn{}{}) throws SQLException {{",
                    struct_name, func_name, sep, param_list
                );
                let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
                for (i, param) in params.iter().enumerate() {
                    let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
                }
                let _ = writeln!(out, "        try (ResultSet rs = ps.executeQuery()) {{");
                let _ = writeln!(out, "            List<{}> result = new ArrayList<>();", struct_name);
                let _ = writeln!(out, "            while (rs.next()) {{");
                let _ = writeln!(out, "                result.add({}.fromResultSet(rs));", struct_name);
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
                    let params_record_name = format!("{}BatchParams", to_pascal_case(&analyzed.name));
                    let record_fields = params
                        .iter()
                        .map(|p| format!("{} {}", java_param_type(p), p.field_name))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "public record {}({}) {{}}", params_record_name, record_fields);
                    let _ = writeln!(out);
                    let _ = writeln!(
                        out,
                        "public static void {}(Connection conn, List<{}> items) throws SQLException {{",
                        batch_fn_name, params_record_name
                    );
                    let _ = writeln!(out, "    conn.setAutoCommit(false);");
                    let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
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
                        "public static void {}(Connection conn, List<{}> items) throws SQLException {{",
                        batch_fn_name,
                        java_param_type(param)
                    );
                    let _ = writeln!(out, "    conn.setAutoCommit(false);");
                    let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
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
                    let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
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
                unreachable!("routed to generate_grouped_query_fn")
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
            let sep = if i + 1 < enum_info.values.len() { "," } else { ";" };
            let _ = writeln!(out, "    {}(\"{}\"){}", variant, value, sep);
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "    private final String value;");
        let _ = writeln!(out, "    {}(String value) {{ this.value = value; }}", type_name);
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
                .map(|f| {
                    let field_type = resolve_type(&f.neutral_type, &self.manifest, false)
                        .map(|t| t.into_owned())
                        .unwrap_or_else(|_| "Object".to_string());
                    format!("{} {}", field_type, to_camel_case(&f.name))
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "public record {}({}) {{}}", name, fields);
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
        let mut out = String::new();

        // Child record first — no forward reference in the parent.
        let _ = writeln!(out, "public record {}(", child_struct_name);
        for (i, c) in child_columns.iter().enumerate() {
            let field_type = java_field_type(c);
            let sep = if i + 1 < child_columns.len() { "," } else { "" };
            if c.nullable {
                let _ = writeln!(out, "    @Nullable {} {}{}", field_type, c.field_name, sep);
            } else {
                let _ = writeln!(out, "    {} {}{}", field_type, c.field_name, sep);
            }
        }
        let _ = writeln!(out, ") {{}}");
        let _ = writeln!(out);

        // Parent record — parent columns then the children collection.
        let _ = writeln!(out, "public record {}(", parent_struct_name);
        for c in parent_columns {
            let field_type = java_field_type(c);
            if c.nullable {
                let _ = writeln!(out, "    @Nullable {} {},", field_type, c.field_name);
            } else {
                let _ = writeln!(out, "    {} {},", field_type, c.field_name);
            }
        }
        let _ = writeln!(out, "    List<{}> children", child_struct_name);
        let _ = write!(out, ") {{}}");

        Ok(out)
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
        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        );

        let param_list = params.iter().map(java_annotated_param).collect::<Vec<_>>().join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let key_col = parent_columns
            .iter()
            .find(|c| c.name == key_column)
            .unwrap_or(&parent_columns[0]);
        let key_type = box_primitive(&key_col.lang_type).to_string();

        let mut out = String::new();
        let _ = writeln!(
            out,
            "public static List<{parent_struct_name}> {func_name}(Connection conn{sep}{param_list}) throws SQLException {{"
        );
        let _ = writeln!(
            out,
            "    var lookup = new java.util.LinkedHashMap<{key_type}, {parent_struct_name}>();"
        );
        let _ = writeln!(out, "    var result = new ArrayList<{parent_struct_name}>();");
        let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{sql}\")) {{");
        for (i, param) in params.iter().enumerate() {
            let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
        }
        let _ = writeln!(out, "        try (ResultSet rs = ps.executeQuery()) {{");
        let _ = writeln!(out, "            while (rs.next()) {{");

        // Extract key (always non-null for grouping key)
        let key_expr = col_rs_expr(key_col);
        let _ = writeln!(out, "                {key_type} key = {key_expr};");

        // Nullable-primitive preamble for child columns
        write_jdbc_nullable_preamble(&mut out, child_columns, "                ");

        // Build child row
        let _ = writeln!(out, "                var child = new {child_struct_name}(");
        for (i, col) in child_columns.iter().enumerate() {
            let expr = col_rs_expr(col);
            let sep = if i + 1 < child_columns.len() { "," } else { "" };
            let _ = writeln!(out, "                    {expr}{sep}");
        }
        let _ = writeln!(out, "                );");

        // Fold into lookup
        let _ = writeln!(out, "                if (lookup.containsKey(key)) {{");
        let _ = writeln!(out, "                    lookup.get(key).children().add(child);");
        let _ = writeln!(out, "                }} else {{");

        // Nullable-primitive preamble for parent columns
        write_jdbc_nullable_preamble(&mut out, parent_columns, "                    ");

        let _ = writeln!(out, "                    var parent = new {parent_struct_name}(");
        for col in parent_columns {
            let expr = col_rs_expr(col);
            let _ = writeln!(out, "                        {expr},");
        }
        let _ = writeln!(out, "                        new ArrayList<>(List.of(child))");
        let _ = writeln!(out, "                    );");
        let _ = writeln!(out, "                    lookup.put(key, parent);");
        let _ = writeln!(out, "                    result.add(parent);");
        let _ = writeln!(out, "                }}");
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "    return result;");
        let _ = write!(out, "}}");

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
            },
            AnalyzedColumn {
                name: "name".to_string(),
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
        ];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
        AnalyzedQuery {
            name: "GetUsersWithOrders".to_string(),
            command: QueryCommand::Grouped,
            sql: "SELECT u.id, u.name, o.id AS order_id, o.total FROM users u JOIN orders o ON o.user_id = u.id"
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
    fn test_grouped_java_jdbc_structs() {
        let backend = crate::backends::get_backend("java-jdbc", "postgresql").unwrap();
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
            row_struct.contains("List<GetUsersWithOrdersChildRow> children"),
            "parent missing children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("public record GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("public record GetUsersWithOrdersRow(").unwrap();
        assert!(child_pos < parent_pos, "child must precede parent");
    }

    #[test]
    fn test_grouped_java_jdbc_query_fn() {
        let backend = crate::backends::get_backend("java-jdbc", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("List<GetUsersWithOrdersRow> getUsersWithOrders"),
            "wrong signature; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("LinkedHashMap"),
            "must use LinkedHashMap for fold lookup; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("lookup.containsKey(key)"),
            "must fold with containsKey; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("children().add(child)"),
            "must append child; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return result;"),
            "must return result; got:\n{query_fn}"
        );
    }
}
