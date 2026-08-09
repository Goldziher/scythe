use std::collections::HashMap;
use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_camel_case, to_pascal_case,
};

use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
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
                    ErrorCode::InvalidConfig,
                    format!("unsupported engine '{}' for java-jdbc backend", engine),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
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

/// Whether this engine's JDBC driver lacks `getObject(int/col, Class<T>)` support for
/// `java.time` types and needs the legacy `java.sql.{Date,Time,Timestamp}` accessors instead.
///
/// Verified by decompiling `SnowflakeBaseResultSet` in snowflake-jdbc 4.0.2: its
/// `getObject(int, Class<T>)` dispatches on exactly `Boolean, Byte, Short, Integer, Long, Float,
/// Double, String, BigDecimal, java.sql.Date, java.sql.Time, java.sql.Timestamp,
/// java.time.Duration, java.time.Period, Map, SQLData` — `LocalDate`, `LocalTime`,
/// `LocalDateTime`, and `OffsetDateTime` are not in that list, so every call throws against real
/// Snowflake. PostgreSQL, MySQL, MariaDB, SQLite, DuckDB, MSSQL, Redshift, and Oracle drivers all
/// support the `getObject(col, Type.class)` form, so this fallback is scoped to Snowflake only —
/// it is not a general "safer" rewrite for every engine.
fn engine_needs_legacy_temporal_getter(engine: &str) -> bool {
    engine == "snowflake"
}

/// For engines that need the legacy JDBC temporal accessors (see
/// [`engine_needs_legacy_temporal_getter`]), return the `ResultSet` getter method and the
/// conversion expression to append to reach the given neutral temporal type from that getter's
/// return value. Returns `None` when there is no legacy JDBC bridge: `time_tz` has none, because
/// `java.sql.Time` carries no UTC offset and Snowflake has no `TIME WITH TIME ZONE` type to
/// produce one from.
fn legacy_temporal_getter(neutral_type: &str) -> Option<(&'static str, &'static str)> {
    match neutral_type {
        "date" => Some(("getDate", ".toLocalDate()")),
        "time" => Some(("getTime", ".toLocalTime()")),
        "datetime" => Some(("getTimestamp", ".toLocalDateTime()")),
        // java.sql.Timestamp has no offset field; bridge it as a UTC instant. This matches what
        // the driver can actually give us — the legacy JDBC contract for TIMESTAMP_TZ values.
        "datetime_tz" => Some(("getTimestamp", ".toInstant().atOffset(ZoneOffset.UTC)")),
        _ => None,
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
/// For nullable primitives and, on engines from [`engine_needs_legacy_temporal_getter`],
/// nullable temporal columns, the variable name is returned — the preamble has already
/// extracted the value and performed the wasNull() check.
fn col_rs_expr(col: &ResolvedColumn, engine: &str) -> String {
    if col.nullable && is_java_primitive(&col.lang_type) {
        return col.field_name.clone();
    }
    if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
        if engine_needs_legacy_temporal_getter(engine)
            && let Some((getter, conversion)) = legacy_temporal_getter(&col.neutral_type)
        {
            return if col.nullable {
                col.field_name.clone()
            } else {
                format!("rs.{}(\"{}\"){}", getter, col.name, conversion)
            };
        }
        return format!("rs.getObject(\"{}\", {})", col.name, class_lit);
    }
    if col.neutral_type.starts_with("enum::") {
        return format!(
            "{}.valueOf(rs.getString(\"{}\").toUpperCase())",
            col.lang_type, col.name
        );
    }
    let getter = rs_getter(&col.lang_type);
    format!("rs.{}(\"{}\")", getter, col.name)
}

/// Emit nullable-primitive and (on engines needing it) nullable-temporal preamble variable
/// declarations for grouped JDBC folding and row construction.
fn write_jdbc_nullable_preamble(out: &mut String, cols: &[ResolvedColumn], indent: &str, engine: &str) {
    for col in cols {
        if !col.nullable {
            continue;
        }
        if is_java_primitive(&col.lang_type) {
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
            continue;
        }
        if !engine_needs_legacy_temporal_getter(engine) {
            continue;
        }
        let Some(class_lit) = temporal_class_literal(&col.lang_type) else {
            continue;
        };
        let Some((getter, conversion)) = legacy_temporal_getter(&col.neutral_type) else {
            continue;
        };
        let short_name = class_lit.trim_end_matches(".class");
        let _ = writeln!(
            out,
            "{}var {}Raw = rs.{}(\"{}\");",
            indent, col.field_name, getter, col.name
        );
        let _ = writeln!(
            out,
            "{}{} {} = rs.wasNull() ? null : {}Raw{};",
            indent, short_name, col.field_name, col.field_name, conversion
        );
    }
}

impl CodegenBackend for JavaJdbcBackend {
    fn name(&self) -> &str {
        "java-jdbc"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn apply_options(&mut self, options: &HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(&["field_case"], options)?;

        if let Some(value) = options.get("field_case") {
            super::apply_field_case_option(&mut self.manifest.naming, "java-jdbc", value)?;
        }
        Ok(())
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
        "package generated;\n\
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

        let _ = writeln!(
            out,
            "    public static {} fromResultSet(ResultSet rs) throws SQLException {{",
            struct_name
        );
        write_jdbc_nullable_preamble(&mut out, columns, "        ", &self.engine);
        let _ = writeln!(out, "        return new {}(", struct_name);
        for (i, col) in columns.iter().enumerate() {
            let sep = if i + 1 < columns.len() { "," } else { "" };
            let expr = col_rs_expr(col, &self.engine);
            let _ = writeln!(out, "            {}{}", expr, sep);
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
        let sql = crate::sql_literal::escape_java_string(&super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        ));

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
        let sql = crate::sql_literal::escape_java_string(&super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        ));

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

        let key_expr = col_rs_expr(key_col, &self.engine);
        let _ = writeln!(out, "                {key_type} key = {key_expr};");

        write_jdbc_nullable_preamble(&mut out, child_columns, "                ", &self.engine);

        let _ = writeln!(out, "                var child = new {child_struct_name}(");
        for (i, col) in child_columns.iter().enumerate() {
            let expr = col_rs_expr(col, &self.engine);
            let sep = if i + 1 < child_columns.len() { "," } else { "" };
            let _ = writeln!(out, "                    {expr}{sep}");
        }
        let _ = writeln!(out, "                );");

        let _ = writeln!(out, "                if (lookup.containsKey(key)) {{");
        let _ = writeln!(out, "                    lookup.get(key).children().add(child);");
        let _ = writeln!(out, "                }} else {{");

        write_jdbc_nullable_preamble(&mut out, parent_columns, "                    ", &self.engine);

        let _ = writeln!(out, "                    var parent = new {parent_struct_name}(");
        for col in parent_columns {
            let expr = col_rs_expr(col, &self.engine);
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
    use std::collections::HashMap;

    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    use super::JavaJdbcBackend;
    use crate::backend_trait::CodegenBackend;

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
        ];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
        AnalyzedQuery::build(|aq| {
            aq.name = "GetUsersWithOrders".to_string();
            aq.command = QueryCommand::Grouped;
            aq.sql = "SELECT u.id, u.name, o.id AS order_id, o.total FROM users u JOIN orders o ON o.user_id = u.id"
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

    /// Builds a `Many` query with one non-nullable and one nullable column for each of
    /// `datetime` and `datetime_tz`, to exercise every combination of the Snowflake legacy
    /// temporal accessor fix.
    fn make_temporal_query() -> AnalyzedQuery {
        let columns = vec![
            AnalyzedColumn {
                name: "created_at".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "updated_at".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: true,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "valid_at".to_string(),
                neutral_type: "datetime_tz".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "expires_at".to_string(),
                neutral_type: "datetime_tz".to_string(),
                nullable: true,
                ..Default::default()
            },
        ];
        AnalyzedQuery::build(|aq| {
            aq.name = "ListEvents".to_string();
            aq.command = QueryCommand::Many;
            aq.sql = "SELECT created_at, updated_at, valid_at, expires_at FROM events".to_string();
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

    /// Snowflake's `getObject(col, Type.class)` dispatch does not support `java.time` classes
    /// (verified by decompiling snowflake-jdbc 4.0.2's `SnowflakeBaseResultSet`), so every
    /// `datetime`/`datetime_tz` read must go through the legacy `getTimestamp` accessor instead.
    /// This test fails if that fallback is reverted to `rs.getObject(col, LocalDateTime.class)`.
    #[test]
    fn test_snowflake_temporal_columns_use_legacy_getter() {
        let backend = crate::backends::get_backend("java-jdbc", "snowflake").unwrap();
        let query = make_temporal_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("rs.getTimestamp(\"created_at\").toLocalDateTime()"),
            "non-nullable datetime must use getTimestamp().toLocalDateTime(); got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("var updated_atRaw = rs.getTimestamp(\"updated_at\");"),
            "nullable datetime must extract via getTimestamp preamble; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("LocalDateTime updated_at = rs.wasNull() ? null : updated_atRaw.toLocalDateTime();"),
            "nullable datetime must null-check before toLocalDateTime(); got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("rs.getTimestamp(\"valid_at\").toInstant().atOffset(ZoneOffset.UTC)"),
            "non-nullable datetime_tz must use getTimestamp().toInstant().atOffset(ZoneOffset.UTC); got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("var expires_atRaw = rs.getTimestamp(\"expires_at\");"),
            "nullable datetime_tz must extract via getTimestamp preamble; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains(
                "OffsetDateTime expires_at = rs.wasNull() ? null : expires_atRaw.toInstant().atOffset(ZoneOffset.UTC);"
            ),
            "nullable datetime_tz must null-check before conversion; got:\n{row_struct}"
        );

        assert!(
            !row_struct.contains("getObject(\"created_at\", LocalDateTime.class)"),
            "must not regress to the unsupported getObject(Class) form for created_at; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains("getObject(\"valid_at\", OffsetDateTime.class)"),
            "must not regress to the unsupported getObject(Class) form for valid_at; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains(".class)"),
            "no column in this fixture should still use getObject(Class) on Snowflake; got:\n{row_struct}"
        );
    }

    /// The legacy-getter fallback is scoped to Snowflake — every other JDBC driver this backend
    /// targets supports `getObject(col, Type.class)` for `java.time` types, so their generated
    /// code must be unchanged by the fix.
    #[test]
    fn test_non_snowflake_temporal_columns_still_use_get_object() {
        let backend = crate::backends::get_backend("java-jdbc", "postgresql").unwrap();
        let query = make_temporal_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("rs.getObject(\"created_at\", LocalDateTime.class)"),
            "postgresql must keep using getObject(Class) for datetime; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("rs.getObject(\"valid_at\", OffsetDateTime.class)"),
            "postgresql must keep using getObject(Class) for datetime_tz; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains("getTimestamp"),
            "postgresql must not be affected by the Snowflake-only legacy getter; got:\n{row_struct}"
        );
    }

    fn make_one_query_with_snake_case_columns() -> AnalyzedQuery {
        AnalyzedQuery::build(|q| {
            q.name = "GetSession".to_string();
            q.command = QueryCommand::One;
            q.sql = "SELECT id, user_id FROM sessions WHERE id = $1".to_string();
            q.columns = vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "user_id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ];
            q.params = vec![];
            q.deprecated = None;
            q.source_table = None;
            q.composites = vec![];
            q.enums = vec![];
            q.optional_params = vec![];
            q.group_by = None;
            q.custom = vec![];
        })
    }

    /// The safety invariant this whole feature depends on: renaming the
    /// declared field must never touch the key the driver is asked to look
    /// up. `col_rs_expr` reads `rs.getInt(col.name)` -- the raw SQL column
    /// name -- and only the record's declared parameter (`col.field_name`)
    /// changes under `field_case = "camelCase"`. If this ever reads
    /// `rs.getInt("userId")` instead, every row decode breaks at runtime
    /// with no compile-time signal, because `ResultSet.getInt` takes a
    /// plain string.
    #[test]
    fn test_field_case_camel_case_renames_field_but_keeps_raw_lookup_key() {
        let mut backend = JavaJdbcBackend::new("postgresql").unwrap();
        backend
            .apply_options(&HashMap::from([("field_case".to_string(), "camelCase".to_string())]))
            .unwrap();
        let query = make_one_query_with_snake_case_columns();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("int userId"),
            "field_case must rename the declared record field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("rs.getInt(\"user_id\")"),
            "the ResultSet lookup key must stay the raw SQL column name; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains("rs.getInt(\"userId\")"),
            "must never look the driver up by the renamed field; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains("int user_id"),
            "must not leave the raw SQL name in the declared field; got:\n{row_struct}"
        );
    }

    #[test]
    fn test_field_case_option_rejects_invalid_value() {
        let mut backend = JavaJdbcBackend::new("postgresql").unwrap();
        let result = backend.apply_options(&HashMap::from([("field_case".to_string(), "PascalCase".to_string())]));
        assert!(result.is_err(), "expected 'PascalCase' to be rejected");
    }

    /// #103: before this, java-jdbc inherited the `CodegenBackend` default
    /// `apply_options` (`Ok(())` for any map), so a typo'd key like
    /// `field_casing` was silently discarded here while the same typo was a
    /// hard error on every TypeScript backend. The trait default now rejects
    /// every key unless a backend declares it known, so this must fail the
    /// same way `typescript-pg` already does for its own unknown keys.
    #[test]
    fn test_apply_options_rejects_unknown_key_with_invalid_config() {
        let mut backend = JavaJdbcBackend::new("postgresql").unwrap();
        let err = backend
            .apply_options(&HashMap::from([("field_casing".to_string(), "camelCase".to_string())]))
            .expect_err("field_casing is not a known java-jdbc option");
        assert_eq!(err.code, scythe_core::errors::ErrorCode::InvalidConfig);
        assert!(err.message.contains("field_casing"), "{}", err.message);
        assert!(
            err.message.contains("field_case"),
            "error should list the real option: {}",
            err.message
        );
    }

    /// Regression guard for the same change: a real, known key must keep
    /// working -- the risk with inverting the trait default is a false
    /// positive that breaks every existing user's config.
    #[test]
    fn test_apply_options_accepts_known_key() {
        let mut backend = JavaJdbcBackend::new("postgresql").unwrap();
        backend
            .apply_options(&HashMap::from([("field_case".to_string(), "camelCase".to_string())]))
            .expect("field_case is a known java-jdbc option");
    }

    fn make_one_query_with_colliding_columns() -> AnalyzedQuery {
        AnalyzedQuery::build(|q| {
            q.name = "GetUser".to_string();
            q.command = QueryCommand::One;
            q.sql = "SELECT user_id, \"userId\" FROM users WHERE user_id = $1".to_string();
            q.columns = vec![
                AnalyzedColumn {
                    name: "user_id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "userId".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ];
            q.params = vec![];
            q.deprecated = None;
            q.source_table = None;
            q.composites = vec![];
            q.enums = vec![];
            q.optional_params = vec![];
            q.group_by = None;
            q.custom = vec![];
        })
    }

    /// Collision detection lives in `resolve::resolve_columns` (see
    /// `resolve.rs`'s own thorough coverage across every spelling and both
    /// `field_case` settings) and has no backend-specific code path -- see
    /// `apply_field_case_option`'s doc comment in `backends/mod.rs`. This
    /// test only proves that wiring actually reaches java-jdbc's
    /// `generate_with_backend` pipeline, not a second copy of resolve.rs's
    /// coverage; it is deliberately not repeated in the other four
    /// Java/Kotlin backends for the same reason.
    #[test]
    fn test_field_case_collision_produces_a_clear_error_not_silent_overwrite() {
        let mut backend = JavaJdbcBackend::new("postgresql").unwrap();
        backend
            .apply_options(&HashMap::from([("field_case".to_string(), "camelCase".to_string())]))
            .unwrap();
        let query = make_one_query_with_colliding_columns();
        let err = crate::generate_with_backend(&query, &backend)
            .expect_err("user_id and userId must collide under camelCase");
        assert!(err.to_string().contains("userId"), "{err}");
        assert!(err.to_string().contains("user_id"), "{err}");
    }

    fn make_composite_with_consecutive_capitals() -> CompositeInfo {
        CompositeInfo {
            sql_name: "CreateAPIKey".to_string(),
            fields: vec![
                CompositeFieldInfo {
                    name: "HTTPSUrl".to_string(),
                    neutral_type: "string".to_string(),
                },
                CompositeFieldInfo {
                    name: "internal_id".to_string(),
                    neutral_type: "int32".to_string(),
                },
            ],
        }
    }

    /// `to_pascal_case`/`to_camel_case` now normalize consecutive capitals
    /// through `to_snake_case` (commit 6ab8994), so a composite's SQL name
    /// and field names that carry runs of capitals ("CreateAPIKey",
    /// "HTTPSUrl") changed shape here too -- this backend's
    /// `generate_composite_def` had no coverage of that change landing.
    #[test]
    fn test_composite_def_normalizes_consecutive_capitals() {
        let backend = JavaJdbcBackend::new("postgresql").unwrap();
        let composite = make_composite_with_consecutive_capitals();
        let def = backend.generate_composite_def(&composite).unwrap();

        assert!(
            def.contains("public record CreateApiKey("),
            "composite type name must normalize consecutive capitals; got:\n{def}"
        );
        assert!(
            def.contains("String httpsUrl"),
            "composite field name must normalize consecutive capitals; got:\n{def}"
        );
        assert!(
            def.contains("int internalId"),
            "composite field name must still camelCase a plain snake_case field; got:\n{def}"
        );
    }
}
