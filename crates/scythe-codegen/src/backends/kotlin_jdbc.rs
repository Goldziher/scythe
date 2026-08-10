use std::collections::HashMap;
use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, to_camel_case, to_pascal_case};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};
use crate::backends::jvm_common;

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/kotlin-jdbc.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/kotlin-jdbc.mysql.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/kotlin-jdbc.sqlite.toml");
const DEFAULT_MANIFEST_DUCKDB: &str = include_str!("../../manifests/kotlin-jdbc.duckdb.toml");
const DEFAULT_MANIFEST_MARIADB: &str = include_str!("../../manifests/kotlin-jdbc.mariadb.toml");
const DEFAULT_MANIFEST_MSSQL: &str = include_str!("../../manifests/kotlin-jdbc.mssql.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/kotlin-jdbc.redshift.toml");
const DEFAULT_MANIFEST_SNOWFLAKE: &str = include_str!("../../manifests/kotlin-jdbc.snowflake.toml");
const DEFAULT_MANIFEST_ORACLE: &str = include_str!("../../manifests/kotlin-jdbc.oracle.toml");

pub struct KotlinJdbcBackend {
    manifest: BackendManifest,
    engine: String,
    extension_functions: bool,
}

impl KotlinJdbcBackend {
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
                    format!("unsupported engine '{}' for kotlin-jdbc backend", engine),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
        Ok(Self {
            manifest,
            engine: engine.to_string(),
            extension_functions: false,
        })
    }
}

/// Build the `ResultSet` read call for a Kotlin column.
///
/// Delegates to [`jvm_common::kotlin_jdbc_read_call`], which answers a named
/// getter (`rs.getInt("n")`) where JDBC has one and the class-taking
/// `rs.getObject("n", T::class.java)` overload where it does not. The table
/// this replaced ended in a bare `"getObject"` arm that caught `UUID`,
/// composites, and every other unrecognised type; its static type is `Any!`,
/// which Kotlin refuses to pass where a `UUID`/`TortureAddress` is expected.
fn rs_read_call(column: &str, kotlin_type: &str) -> String {
    jvm_common::kotlin_jdbc_read_call(column, kotlin_type)
}

/// Return the Kotlin class literal for temporal types that need
/// `rs.getObject("col", Type::class.java)`. Returns None for non-temporal types.
fn temporal_class_literal(kotlin_type: &str) -> Option<&str> {
    if kotlin_type.contains("LocalDate") && !kotlin_type.contains("LocalDateTime") {
        Some("LocalDate::class.java")
    } else if kotlin_type.contains("LocalTime") && !kotlin_type.contains("LocalDateTime") {
        Some("LocalTime::class.java")
    } else if kotlin_type.contains("OffsetTime") {
        Some("OffsetTime::class.java")
    } else if kotlin_type.contains("LocalDateTime") {
        Some("LocalDateTime::class.java")
    } else if kotlin_type.contains("OffsetDateTime") {
        Some("OffsetDateTime::class.java")
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
/// support the `getObject(col, Type::class.java)` form, so this fallback is scoped to Snowflake
/// only — it is not a general "safer" rewrite for every engine.
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
/// Returns the complete expression like `getLong(3)` or `getObject(3, LocalDateTime::class.java)`.
fn oracle_cs_getter_call(neutral_type: &str, index: usize) -> String {
    match neutral_type {
        "int32" => format!("getInt({})", index),
        "int64" => format!("getLong({})", index),
        "float32" => format!("getFloat({})", index),
        "float64" => format!("getDouble({})", index),
        "decimal" => format!("getBigDecimal({})", index),
        "date" | "datetime" => format!("getObject({}, LocalDateTime::class.java)", index),
        "datetime_tz" => format!("getObject({}, OffsetDateTime::class.java)", index),
        _ => format!("getString({})", index),
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

/// Build the inline ResultSet read expression for a Kotlin JDBC column (by name).
/// For nullable columns, the preamble has already extracted the value with wasNull().
fn kt_rs_expr(col: &ResolvedColumn, engine: &str) -> String {
    if col.nullable {
        return col.field_name.clone();
    }
    if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
        if engine_needs_legacy_temporal_getter(engine)
            && let Some((getter, conversion)) = legacy_temporal_getter(&col.neutral_type)
        {
            return format!("rs.{}(\"{}\"){}", getter, col.name, conversion);
        }
        return format!("rs.getObject(\"{}\", {})", col.name, class_lit);
    }
    if col.neutral_type.starts_with("enum::") {
        return format!("{}.valueOf(rs.getString(\"{}\").uppercase())", col.lang_type, col.name);
    }
    rs_read_call(&col.name, &col.lang_type)
}

/// Emit nullable-column preamble for Kotlin JDBC grouped folding and row construction.
fn write_kt_nullable_preamble(out: &mut String, cols: &[ResolvedColumn], indent: &str, engine: &str) {
    for col in cols {
        if !col.nullable {
            continue;
        }
        if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
            if engine_needs_legacy_temporal_getter(engine)
                && let Some((getter, conversion)) = legacy_temporal_getter(&col.neutral_type)
            {
                let _ = writeln!(
                    out,
                    "{}val {}Value = rs.{}(\"{}\")",
                    indent, col.field_name, getter, col.name
                );
                let _ = writeln!(
                    out,
                    "{}val {} = if (rs.wasNull()) null else {}Value{}",
                    indent, col.field_name, col.field_name, conversion
                );
                continue;
            }
            let _ = writeln!(
                out,
                "{}val {}Value = rs.getObject(\"{}\", {})",
                indent, col.field_name, col.name, class_lit
            );
        } else if col.neutral_type.starts_with("enum::") {
            // ~keep A nullable enum column read inline would be
            // `Status.valueOf(rs.getString(col).uppercase())`, which throws on
            // a NULL column: `getString` returns `null` and `uppercase()` is
            // called on it. Guard on the raw string rather than `wasNull()`,
            // and return early — the shared tail below cannot express the
            // conversion.
            let _ = writeln!(
                out,
                "{}val {}Value = rs.getString(\"{}\")",
                indent, col.field_name, col.name
            );
            let _ = writeln!(
                out,
                "{}val {} = if ({}Value == null) null else {}.valueOf({}Value.uppercase())",
                indent, col.field_name, col.field_name, col.lang_type, col.field_name
            );
            continue;
        } else {
            let _ = writeln!(
                out,
                "{}val {}Value = {}",
                indent,
                col.field_name,
                rs_read_call(&col.name, &col.lang_type)
            );
        }
        let _ = writeln!(
            out,
            "{}val {} = if (rs.wasNull()) null else {}Value",
            indent, col.field_name, col.field_name
        );
    }
}

/// Emit `StructName(\n    field = expr,\n    ...\n){suffix}` reading each column from `rs`.
/// Assumes any nullable-column preamble locals have already been written via
/// [`write_kt_nullable_preamble`] using the same `engine`.
fn write_kt_struct_literal(
    out: &mut String,
    struct_name: &str,
    columns: &[ResolvedColumn],
    engine: &str,
    outer_indent: &str,
    field_indent: &str,
    closing_suffix: &str,
) {
    let _ = writeln!(out, "{}{}(", outer_indent, struct_name);
    for col in columns {
        let expr = kt_rs_expr(col, engine);
        let _ = writeln!(out, "{}{} = {},", field_indent, col.field_name, expr);
    }
    let _ = writeln!(out, "{}){}", outer_indent, closing_suffix);
}

impl CodegenBackend for KotlinJdbcBackend {
    fn name(&self) -> &str {
        "kotlin-jdbc"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn apply_options(&mut self, options: &HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(&["extension_functions", "field_case"], options)?;

        if let Some(v) = options.get("extension_functions") {
            self.extension_functions = match v.as_str() {
                "true" => true,
                "false" => false,
                other => {
                    return Err(ScytheError::new(
                        ErrorCode::InvalidConfig,
                        format!(
                            "kotlin-jdbc: invalid value '{other}' for extension_functions (expected 'true' or 'false')"
                        ),
                    ));
                }
            };
        }
        if let Some(value) = options.get("field_case") {
            super::apply_field_case_option(&mut self.manifest.naming, "kotlin-jdbc", value)?;
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
        let uuid_type = self
            .manifest
            .types
            .scalars
            .get("uuid")
            .map(String::as_str)
            .unwrap_or("java.util.UUID");
        let uuid_import = if uuid_type.contains("UUID") {
            "import java.util.UUID\n"
        } else {
            ""
        };
        // ~keep Snowflake can't bridge `datetime_tz` via `getObject(col, OffsetDateTime::class.java)` —
        // see `engine_needs_legacy_temporal_getter` — so reads go through `getTimestamp` +
        // `.toInstant().atOffset(ZoneOffset.UTC)` instead, which needs this import.
        let zone_offset_import = if engine_needs_legacy_temporal_getter(&self.engine) {
            "import java.time.ZoneOffset\n"
        } else {
            ""
        };
        format!(
            "package generated\n\
             \n\
             import java.math.BigDecimal\n\
             import java.sql.Connection\n\
             import java.time.LocalDate\n\
             import java.time.LocalDateTime\n\
             import java.time.LocalTime\n\
             import java.time.OffsetDateTime\n\
             import java.time.OffsetTime\n\
             {zone_offset_import}{uuid_import}"
        )
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        _query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();
        let _ = writeln!(out, "data class {}(", struct_name);
        for col in columns.iter() {
            let _ = writeln!(out, "    val {}: {},", col.field_name, col.full_type);
        }
        let _ = writeln!(out, ")");
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
        let sql = crate::sql_literal::escape_kotlin_string(&super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        ));

        let use_multiline_params = !params.is_empty();
        let ext = self.extension_functions;
        let receiver = if ext { "this" } else { "conn" };

        let mut out = String::new();

        let engine = &self.engine;
        let write_setters = |out: &mut String, params: &[ResolvedParam]| {
            for (i, param) in params.iter().enumerate() {
                if param.neutral_type.starts_with("enum::") {
                    if engine == "postgresql" {
                        let _ = writeln!(
                            out,
                            "        ps.setObject({}, {}.value, java.sql.Types.OTHER)",
                            i + 1,
                            param.field_name
                        );
                    } else {
                        let _ = writeln!(out, "        ps.setString({}, {}.value)", i + 1, param.field_name);
                    }
                } else {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(out, "        ps.{}({}, {})", setter, i + 1, param.field_name);
                }
            }
        };

        let write_fn_sig =
            |out: &mut String, name: &str, ret: &str, multiline: bool, params: &[ResolvedParam], expr: bool| {
                if ext {
                    if multiline {
                        let _ = writeln!(out, "fun Connection.{}(", name);
                        for p in params {
                            let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                        }
                        if expr {
                            let _ = writeln!(out, "){} =", ret);
                        } else {
                            let _ = writeln!(out, "){} {{", ret);
                        }
                    } else if expr {
                        let _ = writeln!(out, "fun Connection.{}(){} =", name, ret);
                    } else {
                        let _ = writeln!(out, "fun Connection.{}(){} {{", name, ret);
                    }
                } else if multiline {
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
                write_fn_sig(&mut out, &func_name, "", use_multiline_params, params, false);
                let _ = writeln!(out, "    {receiver}.prepareStatement(\"{sql}\").use {{ ps ->",);
                write_setters(&mut out, params);
                let _ = writeln!(out, "        ps.executeUpdate()");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                if ext {
                    write_fn_sig(&mut out, &func_name, ": Int", use_multiline_params, params, true);
                    let _ = writeln!(out, "    {receiver}.prepareStatement(\"{sql}\").use {{ ps ->",);
                    write_setters(&mut out, params);
                    let _ = writeln!(out, "        ps.executeUpdate()");
                    let _ = writeln!(out, "    }}");
                } else {
                    write_fn_sig(&mut out, &func_name, ": Int", use_multiline_params, params, false);
                    let _ = writeln!(out, "    return conn.prepareStatement(\"{sql}\").use {{ ps ->",);
                    write_setters(&mut out, params);
                    let _ = writeln!(out, "        ps.executeUpdate()");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                }
            }
            QueryCommand::One | QueryCommand::Opt => {
                let ret = format!(": {}?", struct_name);
                let is_oracle_returning = self.engine == "oracle" && sql.to_uppercase().contains("RETURNING");
                let is_mariadb_returning = self.engine == "mariadb" && sql.to_uppercase().contains("RETURNING");
                if is_mariadb_returning {
                    write_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params, false);
                    let _ = writeln!(out, "    {receiver}.prepareStatement(\"{sql}\").use {{ ps ->",);
                    write_setters(&mut out, params);
                    let _ = writeln!(out, "        ps.execute()");
                    let _ = writeln!(out, "        val rs = ps.resultSet");
                    let _ = writeln!(out, "        if (rs != null && rs.next()) {{");
                    let _ = writeln!(out, "            return {}(", struct_name);
                    for col in columns.iter() {
                        if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
                            let _ = writeln!(
                                out,
                                "                {} = rs.getObject(\"{}\", {}),",
                                col.field_name, col.name, class_lit
                            );
                        } else if col.neutral_type.starts_with("enum::") {
                            let _ = writeln!(
                                out,
                                "                {} = {}.valueOf(rs.getString(\"{}\").uppercase()),",
                                col.field_name, col.lang_type, col.name
                            );
                        } else {
                            let _ = writeln!(
                                out,
                                "                {} = {},",
                                col.field_name,
                                rs_read_call(&col.name, &col.lang_type)
                            );
                        }
                    }
                    let _ = writeln!(out, "            )");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        return null");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else if is_oracle_returning {
                    let into_placeholders = columns.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
                    let full_sql = format!("BEGIN {sql} INTO {into_placeholders}; END;");
                    let use_multiline = !params.is_empty();
                    write_fn_sig(&mut out, &func_name, &ret, use_multiline, params, false);
                    let _ = writeln!(out, "    {receiver}.prepareCall(\"{full_sql}\").use {{ cs ->");
                    for (i, param) in params.iter().enumerate() {
                        let setter = ps_setter(&param.lang_type);
                        let _ = writeln!(out, "        cs.{}({}, {})", setter, i + 1, param.field_name);
                    }
                    for (i, col) in columns.iter().enumerate() {
                        let jdbc_type = oracle_jdbc_type(&col.neutral_type);
                        let _ = writeln!(
                            out,
                            "        cs.registerOutParameter({}, {})",
                            params.len() + i + 1,
                            jdbc_type
                        );
                    }
                    let _ = writeln!(out, "        cs.execute()");
                    let _ = writeln!(out, "        return {}(", struct_name);
                    for (i, col) in columns.iter().enumerate() {
                        let getter_call = oracle_cs_getter_call(&col.neutral_type, params.len() + i + 1);
                        let _ = writeln!(out, "            {} = cs.{},", col.field_name, getter_call);
                    }
                    let _ = writeln!(out, "        )");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else if ext {
                    write_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params, true);
                    let _ = writeln!(out, "    {receiver}.prepareStatement(\"{sql}\").use {{ ps ->",);
                    write_setters(&mut out, params);
                    let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
                    let _ = writeln!(out, "            if (rs.next()) {{");
                    write_kt_nullable_preamble(&mut out, columns, "                ", engine);
                    write_kt_struct_literal(
                        &mut out,
                        struct_name,
                        columns,
                        engine,
                        "                ",
                        "                    ",
                        "",
                    );
                    let _ = writeln!(out, "            }} else {{");
                    let _ = writeln!(out, "                null");
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "    }}");
                } else {
                    write_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params, false);
                    let _ = writeln!(out, "    conn.prepareStatement(\"{sql}\").use {{ ps ->",);
                    write_setters(&mut out, params);
                    let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
                    let _ = writeln!(out, "            return if (rs.next()) {{");
                    write_kt_nullable_preamble(&mut out, columns, "                ", engine);
                    write_kt_struct_literal(
                        &mut out,
                        struct_name,
                        columns,
                        engine,
                        "                ",
                        "                    ",
                        "",
                    );
                    let _ = writeln!(out, "            }} else {{");
                    let _ = writeln!(out, "                null");
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                }
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                if params.len() > 1 {
                    let params_class_name = format!("{}BatchParams", to_pascal_case(&analyzed.name));
                    let _ = writeln!(out, "data class {}(", params_class_name);
                    for p in params {
                        let _ = writeln!(out, "    val {}: {},", p.field_name, p.full_type);
                    }
                    let _ = writeln!(out, ")");
                    let _ = writeln!(out);
                    if ext {
                        let _ = writeln!(out, "fun Connection.{}(", batch_fn_name);
                        let _ = writeln!(out, "    items: List<{}>,", params_class_name);
                    } else {
                        let _ = writeln!(out, "fun {}(", batch_fn_name);
                        let _ = writeln!(out, "    conn: Connection,");
                        let _ = writeln!(out, "    items: List<{}>,", params_class_name);
                    }
                    let _ = writeln!(out, ") {{");
                    let _ = writeln!(out, "    {receiver}.autoCommit = false");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(out, "        {receiver}.prepareStatement(\"{sql}\").use {{ ps ->",);
                    let _ = writeln!(out, "            for (item in items) {{");
                    for (i, param) in params.iter().enumerate() {
                        let setter = ps_setter(&param.lang_type);
                        let _ = writeln!(
                            out,
                            "                ps.{}({}, item.{})",
                            setter,
                            i + 1,
                            param.field_name
                        );
                    }
                    let _ = writeln!(out, "                ps.addBatch()");
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "            ps.executeBatch()");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        {receiver}.commit()");
                    let _ = writeln!(out, "    }} catch (e: Exception) {{");
                    let _ = writeln!(out, "        {receiver}.rollback()");
                    let _ = writeln!(out, "        throw e");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        {receiver}.autoCommit = true");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else if params.len() == 1 {
                    if ext {
                        let _ = writeln!(out, "fun Connection.{}(", batch_fn_name);
                        let _ = writeln!(out, "    items: List<{}>,", params[0].full_type);
                    } else {
                        let _ = writeln!(out, "fun {}(", batch_fn_name);
                        let _ = writeln!(out, "    conn: Connection,");
                        let _ = writeln!(out, "    items: List<{}>,", params[0].full_type);
                    }
                    let _ = writeln!(out, ") {{");
                    let _ = writeln!(out, "    {receiver}.autoCommit = false");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(out, "        {receiver}.prepareStatement(\"{sql}\").use {{ ps ->",);
                    let _ = writeln!(out, "            for (item in items) {{");
                    let setter = ps_setter(&params[0].lang_type);
                    let _ = writeln!(out, "                ps.{}(1, item)", setter);
                    let _ = writeln!(out, "                ps.addBatch()");
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "            ps.executeBatch()");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        {receiver}.commit()");
                    let _ = writeln!(out, "    }} catch (e: Exception) {{");
                    let _ = writeln!(out, "        {receiver}.rollback()");
                    let _ = writeln!(out, "        throw e");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        {receiver}.autoCommit = true");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else if ext {
                    let _ = writeln!(out, "fun Connection.{}(count: Int) {{", batch_fn_name);
                    let _ = writeln!(out, "    {receiver}.autoCommit = false");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(out, "        {receiver}.prepareStatement(\"{sql}\").use {{ ps ->",);
                    let _ = writeln!(out, "            repeat(count) {{");
                    let _ = writeln!(out, "                ps.addBatch()");
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "            ps.executeBatch()");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        {receiver}.commit()");
                    let _ = writeln!(out, "    }} catch (e: Exception) {{");
                    let _ = writeln!(out, "        {receiver}.rollback()");
                    let _ = writeln!(out, "        throw e");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        {receiver}.autoCommit = true");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else {
                    let _ = writeln!(out, "fun {}(conn: Connection, count: Int) {{", batch_fn_name);
                    let _ = writeln!(out, "    conn.autoCommit = false");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(out, "        conn.prepareStatement(\"{sql}\").use {{ ps ->",);
                    let _ = writeln!(out, "            repeat(count) {{");
                    let _ = writeln!(out, "                ps.addBatch()");
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "            ps.executeBatch()");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        conn.commit()");
                    let _ = writeln!(out, "    }} catch (e: Exception) {{");
                    let _ = writeln!(out, "        conn.rollback()");
                    let _ = writeln!(out, "        throw e");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        conn.autoCommit = true");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let ret = format!(": List<{}>", struct_name);
                if ext {
                    write_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params, true);
                    let _ = writeln!(out, "    {receiver}.prepareStatement(\"{sql}\").use {{ ps ->",);
                    write_setters(&mut out, params);
                    let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
                    let _ = writeln!(out, "            val result = mutableListOf<{struct_name}>()",);
                    let _ = writeln!(out, "            while (rs.next()) {{");
                    write_kt_nullable_preamble(&mut out, columns, "                ", engine);
                    let _ = writeln!(out, "                result.add(");
                    write_kt_struct_literal(
                        &mut out,
                        struct_name,
                        columns,
                        engine,
                        "                    ",
                        "                        ",
                        ",",
                    );
                    let _ = writeln!(out, "                )");
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "            result");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "    }}");
                } else {
                    write_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params, false);
                    let _ = writeln!(out, "    conn.prepareStatement(\"{sql}\").use {{ ps ->",);
                    write_setters(&mut out, params);
                    let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
                    let _ = writeln!(out, "            val result = mutableListOf<{struct_name}>()",);
                    let _ = writeln!(out, "            while (rs.next()) {{");
                    write_kt_nullable_preamble(&mut out, columns, "                ", engine);
                    let _ = writeln!(out, "                result.add(");
                    write_kt_struct_literal(
                        &mut out,
                        struct_name,
                        columns,
                        engine,
                        "                    ",
                        "                        ",
                        ",",
                    );
                    let _ = writeln!(out, "                )");
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "            return result");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
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
        let _ = writeln!(out, "enum class {}(val value: String) {{", type_name);
        for (i, value) in enum_info.values.iter().enumerate() {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let sep = if i + 1 < enum_info.values.len() { "," } else { ";" };
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

    fn generate_grouped_structs(
        &self,
        parent_struct_name: &str,
        child_struct_name: &str,
        parent_columns: &[ResolvedColumn],
        child_columns: &[ResolvedColumn],
        _key_column: &str,
    ) -> Result<String, ScytheError> {
        let mut out = String::new();

        let _ = writeln!(out, "data class {}(", child_struct_name);
        for col in child_columns {
            let _ = writeln!(out, "    val {}: {},", col.field_name, col.full_type);
        }
        let _ = writeln!(out, ")");
        let _ = writeln!(out);

        let _ = writeln!(out, "data class {}(", parent_struct_name);
        for col in parent_columns {
            let _ = writeln!(out, "    val {}: {},", col.field_name, col.full_type);
        }
        let _ = writeln!(out, "    val children: MutableList<{}>,", child_struct_name);
        let _ = write!(out, ")");

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
        let sql = crate::sql_literal::escape_kotlin_string(&super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        ));

        let ext = self.extension_functions;
        let receiver = if ext { "this" } else { "conn" };
        let use_multiline_params = !params.is_empty();

        let key_col = parent_columns
            .iter()
            .find(|c| c.name == key_column)
            .unwrap_or(&parent_columns[0]);
        let key_type = key_col.full_type.trim_end_matches('?');

        let mut out = String::new();
        let ret = format!(": List<{parent_struct_name}>");

        let engine = &self.engine;
        let write_setters = |out: &mut String| {
            for (i, param) in params.iter().enumerate() {
                if param.neutral_type.starts_with("enum::") {
                    if engine == "postgresql" {
                        let _ = writeln!(
                            out,
                            "        ps.setObject({}, {}.value, java.sql.Types.OTHER)",
                            i + 1,
                            param.field_name
                        );
                    } else {
                        let _ = writeln!(out, "        ps.setString({}, {}.value)", i + 1, param.field_name);
                    }
                } else {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(out, "        ps.{}({}, {})", setter, i + 1, param.field_name);
                }
            }
        };

        if ext {
            if use_multiline_params {
                let _ = writeln!(out, "fun Connection.{}(", func_name);
                for p in params {
                    let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                }
                let _ = writeln!(out, "){ret} {{");
            } else {
                let _ = writeln!(out, "fun Connection.{}(){ret} {{", func_name);
            }
        } else if use_multiline_params {
            let _ = writeln!(out, "fun {}(", func_name);
            let _ = writeln!(out, "    conn: Connection,");
            for p in params {
                let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "){ret} {{");
        } else {
            let _ = writeln!(out, "fun {}(conn: Connection){ret} {{", func_name);
        }

        let _ = writeln!(
            out,
            "    val lookup = LinkedHashMap<{key_type}, {parent_struct_name}>()"
        );
        let _ = writeln!(out, "    val result = mutableListOf<{parent_struct_name}>()");
        let _ = writeln!(out, "    {receiver}.prepareStatement(\"{sql}\").use {{ ps ->");
        write_setters(&mut out);
        let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
        let _ = writeln!(out, "            while (rs.next()) {{");

        write_kt_nullable_preamble(&mut out, child_columns, "                ", engine);
        write_kt_nullable_preamble(&mut out, parent_columns, "                ", engine);

        let key_expr = kt_rs_expr(key_col, engine);
        let _ = writeln!(out, "                val key = {key_expr}");

        let _ = writeln!(out, "                val child = {child_struct_name}(");
        for col in child_columns {
            let expr = kt_rs_expr(col, engine);
            let _ = writeln!(out, "                    {} = {},", col.field_name, expr);
        }
        let _ = writeln!(out, "                )");

        let _ = writeln!(out, "                if (lookup.containsKey(key)) {{");
        let _ = writeln!(out, "                    lookup[key]!!.children.add(child)");
        let _ = writeln!(out, "                }} else {{");
        let _ = writeln!(out, "                    val parent = {parent_struct_name}(");
        for col in parent_columns {
            let expr = kt_rs_expr(col, engine);
            let _ = writeln!(out, "                        {} = {},", col.field_name, expr);
        }
        let _ = writeln!(out, "                        children = mutableListOf(child),");
        let _ = writeln!(out, "                    )");
        let _ = writeln!(out, "                    lookup[key] = parent");
        let _ = writeln!(out, "                    result.add(parent)");
        let _ = writeln!(out, "                }}");
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
        if ext {
            let _ = writeln!(out, "    result");
        } else {
            let _ = writeln!(out, "    return result");
        }
        let _ = write!(out, "}}");

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    use super::KotlinJdbcBackend;
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
    fn test_grouped_kotlin_jdbc_structs() {
        let backend = crate::backends::get_backend("kotlin-jdbc", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("data class GetUsersWithOrdersChildRow"),
            "missing child data class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("data class GetUsersWithOrdersRow"),
            "missing parent data class; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("val children: MutableList<GetUsersWithOrdersChildRow>"),
            "parent missing children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("data class GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("data class GetUsersWithOrdersRow(").unwrap();
        assert!(child_pos < parent_pos, "child must precede parent");
    }

    #[test]
    fn test_grouped_kotlin_jdbc_query_fn() {
        let backend = crate::backends::get_backend("kotlin-jdbc", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("List<GetUsersWithOrdersRow>"),
            "wrong return type; got:\n{query_fn}"
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
            query_fn.contains("children.add(child)"),
            "must append child; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return result") || query_fn.contains("    result\n"),
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

    /// Snowflake's `getObject(col, Type::class.java)` dispatch does not support `java.time`
    /// classes (verified by decompiling snowflake-jdbc 4.0.2's `SnowflakeBaseResultSet`), so
    /// every `datetime`/`datetime_tz` read must go through the legacy `getTimestamp` accessor
    /// instead. This test fails if that fallback is reverted to
    /// `rs.getObject(col, LocalDateTime::class.java)`.
    #[test]
    fn test_snowflake_temporal_columns_use_legacy_getter() {
        let backend = crate::backends::get_backend("kotlin-jdbc", "snowflake").unwrap();
        let query = make_temporal_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("created_at = rs.getTimestamp(\"created_at\").toLocalDateTime()"),
            "non-nullable datetime must use getTimestamp().toLocalDateTime(); got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("val updated_atValue = rs.getTimestamp(\"updated_at\")"),
            "nullable datetime must extract via getTimestamp preamble; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("val updated_at = if (rs.wasNull()) null else updated_atValue.toLocalDateTime()"),
            "nullable datetime must null-check before toLocalDateTime(); got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("valid_at = rs.getTimestamp(\"valid_at\").toInstant().atOffset(ZoneOffset.UTC)"),
            "non-nullable datetime_tz must use getTimestamp().toInstant().atOffset(ZoneOffset.UTC); got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("val expires_atValue = rs.getTimestamp(\"expires_at\")"),
            "nullable datetime_tz must extract via getTimestamp preamble; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains(
                "val expires_at = if (rs.wasNull()) null else expires_atValue.toInstant().atOffset(ZoneOffset.UTC)"
            ),
            "nullable datetime_tz must null-check before conversion; got:\n{query_fn}"
        );

        assert!(
            !query_fn.contains("getObject(\"created_at\", LocalDateTime::class.java)"),
            "must not regress to the unsupported getObject(Class) form for created_at; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("getObject(\"valid_at\", OffsetDateTime::class.java)"),
            "must not regress to the unsupported getObject(Class) form for valid_at; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("::class.java"),
            "no column in this fixture should still use getObject(Class) on Snowflake; got:\n{query_fn}"
        );

        let file_header = backend.file_header();
        assert!(
            file_header.contains("import java.time.ZoneOffset"),
            "snowflake file header must import ZoneOffset for the datetime_tz conversion; got:\n{file_header}"
        );
    }

    /// The legacy-getter fallback is scoped to Snowflake — every other JDBC driver this backend
    /// targets supports `getObject(col, Type::class.java)` for `java.time` types, so their
    /// generated code must be unchanged by the fix.
    #[test]
    fn test_non_snowflake_temporal_columns_still_use_get_object() {
        let backend = crate::backends::get_backend("kotlin-jdbc", "postgresql").unwrap();
        let query = make_temporal_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("created_at = rs.getObject(\"created_at\", LocalDateTime::class.java)"),
            "postgresql must keep using getObject(Class) for datetime; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("valid_at = rs.getObject(\"valid_at\", OffsetDateTime::class.java)"),
            "postgresql must keep using getObject(Class) for datetime_tz; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("getTimestamp"),
            "postgresql must not be affected by the Snowflake-only legacy getter; got:\n{query_fn}"
        );

        let file_header = backend.file_header();
        assert!(
            !file_header.contains("ZoneOffset"),
            "non-snowflake file header must not import ZoneOffset; got:\n{file_header}"
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
    /// up. `kt_rs_expr` reads `rs.getInt(col.name)` -- the raw SQL column
    /// name -- and only the data class' declared property (`col.field_name`)
    /// changes under `field_case = "camelCase"`.
    #[test]
    fn test_field_case_camel_case_renames_field_but_keeps_raw_lookup_key() {
        let mut backend = KotlinJdbcBackend::new("postgresql").unwrap();
        backend
            .apply_options(&HashMap::from([("field_case".to_string(), "camelCase".to_string())]))
            .unwrap();
        let query = make_one_query_with_snake_case_columns();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            row_struct.contains("val userId: Int"),
            "field_case must rename the declared data class property; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains("val user_id"),
            "must not leave the raw SQL name in the declared property; got:\n{row_struct}"
        );
        assert!(
            query_fn.contains("userId = rs.getInt(\"user_id\")"),
            "the ResultSet lookup key must stay the raw SQL column name; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("rs.getInt(\"userId\")"),
            "must never look the driver up by the renamed field; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_field_case_option_rejects_invalid_value() {
        let mut backend = KotlinJdbcBackend::new("postgresql").unwrap();
        let result = backend.apply_options(&HashMap::from([("field_case".to_string(), "PascalCase".to_string())]));
        assert!(result.is_err(), "expected 'PascalCase' to be rejected");
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
    /// through `to_snake_case` (commit 6ab8994), so a composite's field
    /// names that carry runs of capitals ("HTTPSUrl") changed shape here too
    /// -- this backend's `generate_composite_def` had no coverage of that
    /// change landing.
    #[test]
    fn test_composite_def_normalizes_consecutive_capitals() {
        let backend = KotlinJdbcBackend::new("postgresql").unwrap();
        let composite = make_composite_with_consecutive_capitals();
        let def = backend.generate_composite_def(&composite).unwrap();

        assert!(
            def.contains("data class CreateApiKey("),
            "composite type name must normalize consecutive capitals; got:\n{def}"
        );
        assert!(
            def.contains("val httpsUrl: String"),
            "composite field name must normalize consecutive capitals; got:\n{def}"
        );
        assert!(
            def.contains("val internalId: Int"),
            "composite field name must still camelCase a plain snake_case field; got:\n{def}"
        );
    }
}
