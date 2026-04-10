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

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/kotlin-r2dbc.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/kotlin-r2dbc.mysql.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/kotlin-r2dbc.sqlite.toml");
const DEFAULT_MANIFEST_MARIADB: &str = include_str!("../../manifests/kotlin-r2dbc.mariadb.toml");

pub struct KotlinR2dbcBackend {
    manifest: BackendManifest,
    is_pg: bool,
}

impl KotlinR2dbcBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            "mysql" => DEFAULT_MANIFEST_MYSQL,
            "mariadb" => DEFAULT_MANIFEST_MARIADB,
            "sqlite" | "sqlite3" => DEFAULT_MANIFEST_SQLITE,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("unsupported engine '{}' for kotlin-r2dbc backend", engine),
                ));
            }
        };
        let manifest =
            super::load_or_default_manifest("backends/kotlin-r2dbc/manifest.toml", default_toml)?;
        let is_pg = matches!(engine, "postgresql" | "postgres" | "pg");
        Ok(Self { manifest, is_pg })
    }
}

/// Get the R2DBC Row getter class for a given Kotlin type.
fn r2dbc_row_class(kotlin_type: &str) -> &str {
    match kotlin_type {
        "Boolean" => "Boolean::class.java",
        "Byte" => "Byte::class.java",
        "Short" => "Short::class.java",
        "Int" => "Int::class.javaObjectType",
        "Long" => "Long::class.javaObjectType",
        "Float" => "Float::class.javaObjectType",
        "Double" => "Double::class.javaObjectType",
        "String" => "String::class.java",
        "ByteArray" => "ByteArray::class.java",
        _ if kotlin_type.contains("BigDecimal") => "java.math.BigDecimal::class.java",
        _ if kotlin_type.contains("LocalDate") => "java.time.LocalDate::class.java",
        _ if kotlin_type.contains("LocalTime") => "java.time.LocalTime::class.java",
        _ if kotlin_type.contains("OffsetTime") => "java.time.OffsetTime::class.java",
        _ if kotlin_type.contains("LocalDateTime") => "java.time.LocalDateTime::class.java",
        _ if kotlin_type.contains("OffsetDateTime") => "java.time.OffsetDateTime::class.java",
        _ if kotlin_type.contains("UUID") => "java.util.UUID::class.java",
        _ => "Any::class.java",
    }
}

impl CodegenBackend for KotlinR2dbcBackend {
    fn name(&self) -> &str {
        "kotlin-r2dbc"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "mysql", "mariadb", "sqlite"]
    }

    fn file_header(&self) -> String {
        // ktlint requires lexicographic order with java.* imports at the end.
        "import io.r2dbc.spi.ConnectionFactory\n\
         import kotlinx.coroutines.flow.Flow\n\
         import kotlinx.coroutines.reactive.asFlow\n\
         import kotlinx.coroutines.reactive.awaitFirst\n\
         import kotlinx.coroutines.reactive.awaitFirstOrNull\n\
         import reactor.core.publisher.Flux\n\
         import reactor.core.publisher.Mono\n\
         import java.math.BigDecimal\n\
         import java.time.LocalDate\n\
         import java.time.LocalDateTime\n\
         import java.time.LocalTime\n\
         import java.time.OffsetDateTime\n\
         import java.time.OffsetTime\n\
         import java.util.UUID\n"
            .to_string()
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
        let cleaned = super::clean_sql_oneline_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        );
        let sql = if self.is_pg {
            cleaned
        } else {
            super::rewrite_pg_placeholders(&cleaned, |_| "?".to_string())
        };

        let use_multiline_params = !params.is_empty();

        let mut out = String::new();

        // Helper: write .bind() calls for R2DBC (0-based indexing)
        let write_binds = |out: &mut String, indent: &str| {
            for (i, param) in params.iter().enumerate() {
                let _ = writeln!(out, "{}.bind({}, {})", indent, i, param.field_name);
            }
        };

        // Helper: write row mapping expression for Kotlin
        let write_row_map = |out: &mut String, indent: &str| {
            let _ = writeln!(out, "{}{}(", indent, struct_name);
            for col in columns.iter() {
                let class = r2dbc_row_class(&col.lang_type);
                let _ = writeln!(
                    out,
                    "{}    {} = row.get(\"{}\", {}),",
                    indent, col.field_name, col.name, class
                );
            }
            let _ = write!(out, "{})", indent);
        };

        // Helper: write suspend function signature
        let write_suspend_fn_sig =
            |out: &mut String, name: &str, ret: &str, multiline: bool, params: &[ResolvedParam]| {
                if multiline {
                    let _ = writeln!(out, "suspend fun {}(", name);
                    let _ = writeln!(out, "    cf: ConnectionFactory,");
                    for p in params {
                        let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                    }
                    let _ = writeln!(out, "){} {{", ret);
                } else {
                    let _ = writeln!(out, "suspend fun {}(cf: ConnectionFactory){} {{", name, ret);
                }
            };

        match &analyzed.command {
            QueryCommand::Exec => {
                write_suspend_fn_sig(&mut out, &func_name, "", use_multiline_params, params);
                let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                let _ = writeln!(out, "    try {{");
                let _ = writeln!(out, "        val stmt = conn.createStatement(\"{}\")", sql);
                write_binds(&mut out, "        stmt");
                let _ = writeln!(
                    out,
                    "        Mono.from(stmt.execute()).flatMap {{ result -> Mono.from(result.rowsUpdated) }}.awaitFirstOrNull()"
                );
                let _ = writeln!(out, "    }} finally {{");
                let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_suspend_fn_sig(&mut out, &func_name, ": Long", use_multiline_params, params);
                let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                let _ = writeln!(out, "    try {{");
                let _ = writeln!(out, "        val stmt = conn.createStatement(\"{}\")", sql);
                write_binds(&mut out, "        stmt");
                let _ = writeln!(out, "        return Mono");
                let _ = writeln!(out, "            .from(stmt.execute())");
                let _ = writeln!(
                    out,
                    "            .flatMap {{ result -> Mono.from(result.rowsUpdated) }}"
                );
                let _ = writeln!(out, "            .awaitFirst()");
                let _ = writeln!(out, "    }} finally {{");
                let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "}}");
            }
            QueryCommand::One | QueryCommand::Opt => {
                let ret = format!(": {}?", struct_name);
                write_suspend_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params);
                let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                let _ = writeln!(out, "    try {{");
                let _ = writeln!(out, "        val stmt = conn.createStatement(\"{}\")", sql);
                write_binds(&mut out, "        stmt");
                let _ = writeln!(out, "        return Mono");
                let _ = writeln!(out, "            .from(stmt.execute())");
                let _ = writeln!(out, "            .flatMap {{ result ->");
                let _ = writeln!(out, "                Mono.from(");
                let _ = writeln!(out, "                    result.map {{ row, _ ->");
                write_row_map(&mut out, "                        ");
                let _ = writeln!(out);
                let _ = writeln!(out, "                    }},");
                let _ = writeln!(out, "                )");
                let _ = writeln!(out, "            }}.awaitFirstOrNull()");
                let _ = writeln!(out, "    }} finally {{");
                let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "}}");
            }
            QueryCommand::Many => {
                // :many returns Flow<T> (non-suspend function, expression body)
                let ret = format!(": Flow<{}>", struct_name);
                if use_multiline_params {
                    let _ = writeln!(out, "fun {}(", func_name);
                    let _ = writeln!(out, "    cf: ConnectionFactory,");
                    for p in params {
                        let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                    }
                    let _ = writeln!(out, "){} =", ret);
                } else {
                    let _ = writeln!(out, "fun {}(cf: ConnectionFactory){} =", func_name, ret);
                }
                let _ = writeln!(out, "    Flux");
                let _ = writeln!(out, "        .usingWhen(");
                let _ = writeln!(out, "            cf.create(),");
                let _ = writeln!(out, "            {{ conn ->");
                let _ = writeln!(
                    out,
                    "                val stmt = conn.createStatement(\"{}\")",
                    sql
                );
                write_binds(&mut out, "                stmt");
                let _ = writeln!(out, "                Flux");
                let _ = writeln!(out, "                    .from(stmt.execute())");
                let _ = writeln!(out, "                    .flatMap {{ result ->");
                let _ = writeln!(out, "                        result.map {{ row, _ ->");
                write_row_map(&mut out, "                            ");
                let _ = writeln!(out);
                let _ = writeln!(out, "                        }}");
                let _ = writeln!(out, "                    }}");
                let _ = writeln!(out, "            }},");
                let _ = writeln!(out, "            {{ conn -> Mono.from(conn.close()) }},");
                let _ = writeln!(out, "        ).asFlow()");
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
                    let _ = writeln!(out, "suspend fun {}(", batch_fn_name);
                    let _ = writeln!(out, "    cf: ConnectionFactory,");
                    let _ = writeln!(out, "    items: List<{}>,", params_class_name);
                    let _ = writeln!(out, ") {{");
                    let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(
                        out,
                        "        Mono.from(conn.beginTransaction()).awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "        val stmt = conn.createStatement(\"{}\")", sql);
                    let _ = writeln!(out, "        var first = true");
                    let _ = writeln!(out, "        for (item in items) {{");
                    let _ = writeln!(out, "            if (!first) stmt.add()");
                    for (i, param) in params.iter().enumerate() {
                        let _ = writeln!(
                            out,
                            "            stmt.bind({}, item.{})",
                            i, param.field_name
                        );
                    }
                    let _ = writeln!(out, "            first = false");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(
                        out,
                        "        Flux.from(stmt.execute()).then().awaitFirstOrNull()"
                    );
                    let _ = writeln!(
                        out,
                        "        Mono.from(conn.commitTransaction()).awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "    }} catch (e: Exception) {{");
                    let _ = writeln!(
                        out,
                        "        Mono.from(conn.rollbackTransaction()).awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "        throw e");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "suspend fun {}(", batch_fn_name);
                    let _ = writeln!(out, "    cf: ConnectionFactory,");
                    let _ = writeln!(out, "    items: List<{}>,", params[0].full_type);
                    let _ = writeln!(out, ") {{");
                    let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(
                        out,
                        "        Mono.from(conn.beginTransaction()).awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "        val stmt = conn.createStatement(\"{}\")", sql);
                    let _ = writeln!(out, "        var first = true");
                    let _ = writeln!(out, "        for (item in items) {{");
                    let _ = writeln!(out, "            if (!first) stmt.add()");
                    let _ = writeln!(out, "            stmt.bind(0, item)");
                    let _ = writeln!(out, "            first = false");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(
                        out,
                        "        Flux.from(stmt.execute()).then().awaitFirstOrNull()"
                    );
                    let _ = writeln!(
                        out,
                        "        Mono.from(conn.commitTransaction()).awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "    }} catch (e: Exception) {{");
                    let _ = writeln!(
                        out,
                        "        Mono.from(conn.rollbackTransaction()).awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "        throw e");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "suspend fun {}(cf: ConnectionFactory, count: Int) {{",
                        batch_fn_name
                    );
                    let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(
                        out,
                        "        Mono.from(conn.beginTransaction()).awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "        val stmt = conn.createStatement(\"{}\")", sql);
                    let _ = writeln!(out, "        repeat(count - 1) {{");
                    let _ = writeln!(out, "            stmt.add()");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(
                        out,
                        "        Flux.from(stmt.execute()).then().awaitFirstOrNull()"
                    );
                    let _ = writeln!(
                        out,
                        "        Mono.from(conn.commitTransaction()).awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "    }} catch (e: Exception) {{");
                    let _ = writeln!(
                        out,
                        "        Mono.from(conn.rollbackTransaction()).awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "        throw e");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                }
            }
            QueryCommand::Grouped => {
                // Grouped queries are not yet supported for kotlin-r2dbc
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "grouped queries are not yet supported for kotlin-r2dbc".to_string(),
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
