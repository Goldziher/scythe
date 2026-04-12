use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
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
const DEFAULT_MANIFEST_DUCKDB: &str = include_str!("../../manifests/kotlin-jdbc.duckdb.toml");
const DEFAULT_MANIFEST_MARIADB: &str = include_str!("../../manifests/kotlin-jdbc.mariadb.toml");
const DEFAULT_MANIFEST_MSSQL: &str = include_str!("../../manifests/kotlin-jdbc.mssql.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/kotlin-jdbc.redshift.toml");
const DEFAULT_MANIFEST_SNOWFLAKE: &str = include_str!("../../manifests/kotlin-jdbc.snowflake.toml");
const DEFAULT_MANIFEST_ORACLE: &str = include_str!("../../manifests/kotlin-jdbc.oracle.toml");

pub struct KotlinJdbcBackend {
    manifest: BackendManifest,
    engine: String,
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
                    ErrorCode::InternalError,
                    format!("unsupported engine '{}' for kotlin-jdbc backend", engine),
                ));
            }
        };
        let manifest =
            super::load_or_default_manifest("backends/kotlin-jdbc/manifest.toml", default_toml)?;
        Ok(Self {
            manifest,
            engine: engine.to_string(),
        })
    }
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

impl CodegenBackend for KotlinJdbcBackend {
    fn name(&self) -> &str {
        "kotlin-jdbc"
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
        // Only import UUID when uuid type actually resolves to java.util.UUID.
        // Some engines (e.g. MariaDB) map uuid to String, making the import unused.
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
             {uuid_import}"
        )
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
        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(
                &analyzed.sql,
                &analyzed.optional_params,
                &analyzed.params,
            ),
            |_| "?".to_string(),
        );

        // Build function params: inline for single param (conn only), multi-line for 2+
        let use_multiline_params = !params.is_empty();

        let mut out = String::new();

        // Helper: write param setters
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
                        let _ = writeln!(
                            out,
                            "        ps.setString({}, {}.value)",
                            i + 1,
                            param.field_name
                        );
                    }
                } else {
                    let setter = ps_setter(&param.lang_type);
                    let _ = writeln!(
                        out,
                        "        ps.{}({}, {})",
                        setter,
                        i + 1,
                        param.field_name
                    );
                }
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
            QueryCommand::One | QueryCommand::Opt => {
                let ret = format!(": {}?", struct_name);
                let is_oracle_returning =
                    self.engine == "oracle" && sql.to_uppercase().contains("RETURNING");
                if is_oracle_returning {
                    // Oracle RETURNING … INTO requires a PL/SQL BEGIN…END block so that
                    // the JDBC driver correctly maps the OUT parameters from a DML statement.
                    // Plain prepareCall on a bare DML RETURNING INTO raises ORA-17173.
                    let into_placeholders =
                        columns.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
                    let full_sql = format!("BEGIN {} INTO {}; END;", sql, into_placeholders);
                    let use_multiline = !params.is_empty();
                    write_fn_sig(&mut out, &func_name, &ret, use_multiline, params);
                    let _ = writeln!(out, "    conn.prepareCall(\"{}\").use {{ cs ->", full_sql);
                    // Write setters using cs (not ps) for CallableStatement
                    for (i, param) in params.iter().enumerate() {
                        let setter = ps_setter(&param.lang_type);
                        let _ = writeln!(
                            out,
                            "        cs.{}({}, {})",
                            setter,
                            i + 1,
                            param.field_name
                        );
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
                        let getter_call =
                            oracle_cs_getter_call(&col.neutral_type, params.len() + i + 1);
                        let _ = writeln!(
                            out,
                            "            {} = cs.{},",
                            col.field_name,
                            getter_call
                        );
                    }
                    let _ = writeln!(out, "        )");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else {
                    write_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params);
                    let _ = writeln!(out, "    conn.prepareStatement(\"{}\").use {{ ps ->", sql);
                    write_setters(&mut out, params);
                    let _ = writeln!(out, "        ps.executeQuery().use {{ rs ->");
                    let _ = writeln!(out, "            return if (rs.next()) {{");
                    for col in columns.iter() {
                        if col.nullable {
                            if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
                                let _ = writeln!(
                                    out,
                                    "                val {field}Value = rs.getObject(\"{name}\", {class_lit})",
                                    field = col.field_name,
                                    name = col.name,
                                    class_lit = class_lit,
                                );
                            } else {
                                let getter = rs_getter(&col.lang_type);
                                let _ = writeln!(
                                    out,
                                    "                val {field}Value = rs.{getter}(\"{name}\")",
                                    field = col.field_name,
                                    getter = getter,
                                    name = col.name,
                                );
                            }
                            let _ = writeln!(
                                out,
                                "                val {field} = if (rs.wasNull()) null else {field}Value",
                                field = col.field_name,
                            );
                        }
                    }
                    let _ = writeln!(out, "                {}(", struct_name);
                    for col in columns.iter() {
                        if col.nullable {
                            let _ = writeln!(
                                out,
                                "                    {} = {},",
                                col.field_name, col.field_name
                            );
                        } else if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
                            let _ = writeln!(
                                out,
                                "                    {} = rs.getObject(\"{}\", {}),",
                                col.field_name, col.name, class_lit
                            );
                        } else if col.neutral_type.starts_with("enum::") {
                            let _ = writeln!(
                                out,
                                "                    {} = {}.valueOf(rs.getString(\"{}\").uppercase()),",
                                col.field_name, col.lang_type, col.name
                            );
                        } else {
                            let getter = rs_getter(&col.lang_type);
                            let _ = writeln!(
                                out,
                                "                    {} = rs.{}(\"{}\"),",
                                col.field_name, getter, col.name
                            );
                        }
                    }
                    let _ = writeln!(out, "                )");
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
                    let _ = writeln!(out, "    conn.autoCommit = false");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(
                        out,
                        "        conn.prepareStatement(\"{}\").use {{ ps ->",
                        sql
                    );
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
                    let _ = writeln!(out, "        conn.commit()");
                    let _ = writeln!(out, "    }} catch (e: Exception) {{");
                    let _ = writeln!(out, "        conn.rollback()");
                    let _ = writeln!(out, "        throw e");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        conn.autoCommit = true");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "fun {}(", batch_fn_name);
                    let _ = writeln!(out, "    conn: Connection,");
                    let _ = writeln!(out, "    items: List<{}>,", params[0].full_type);
                    let _ = writeln!(out, ") {{");
                    let _ = writeln!(out, "    conn.autoCommit = false");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(
                        out,
                        "        conn.prepareStatement(\"{}\").use {{ ps ->",
                        sql
                    );
                    let _ = writeln!(out, "            for (item in items) {{");
                    let setter = ps_setter(&params[0].lang_type);
                    let _ = writeln!(out, "                ps.{}(1, item)", setter);
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
                } else {
                    let _ = writeln!(
                        out,
                        "fun {}(conn: Connection, count: Int) {{",
                        batch_fn_name
                    );
                    let _ = writeln!(out, "    conn.autoCommit = false");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(
                        out,
                        "        conn.prepareStatement(\"{}\").use {{ ps ->",
                        sql
                    );
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
                for col in columns.iter() {
                    if col.nullable {
                        if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
                            let _ = writeln!(
                                out,
                                "                val {field}Value = rs.getObject(\"{name}\", {class_lit})",
                                field = col.field_name,
                                name = col.name,
                                class_lit = class_lit,
                            );
                        } else {
                            let getter = rs_getter(&col.lang_type);
                            let _ = writeln!(
                                out,
                                "                val {field}Value = rs.{getter}(\"{name}\")",
                                field = col.field_name,
                                getter = getter,
                                name = col.name,
                            );
                        }
                        let _ = writeln!(
                            out,
                            "                val {field} = if (rs.wasNull()) null else {field}Value",
                            field = col.field_name,
                        );
                    }
                }
                let _ = writeln!(out, "                result.add(");
                let _ = writeln!(out, "                    {}(", struct_name);
                for col in columns.iter() {
                    if col.nullable {
                        let _ = writeln!(
                            out,
                            "                        {} = {},",
                            col.field_name, col.field_name
                        );
                    } else if let Some(class_lit) = temporal_class_literal(&col.lang_type) {
                        let _ = writeln!(
                            out,
                            "                        {} = rs.getObject(\"{}\", {}),",
                            col.field_name, col.name, class_lit
                        );
                    } else if col.neutral_type.starts_with("enum::") {
                        let _ = writeln!(
                            out,
                            "                        {} = {}.valueOf(rs.getString(\"{}\").uppercase()),",
                            col.field_name, col.lang_type, col.name
                        );
                    } else {
                        let getter = rs_getter(&col.lang_type);
                        let _ = writeln!(
                            out,
                            "                        {} = rs.{}(\"{}\"),",
                            col.field_name, getter, col.name
                        );
                    }
                }
                let _ = writeln!(out, "                    ),");
                let _ = writeln!(out, "                )");
                let _ = writeln!(out, "            }}");
                let _ = writeln!(out, "            return result");
                let _ = writeln!(out, "        }}");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "}}");
            }
            QueryCommand::Grouped => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "grouped queries are not yet supported for kotlin-jdbc".to_string(),
                ));
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
