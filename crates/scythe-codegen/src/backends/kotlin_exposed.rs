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

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/kotlin-exposed.toml");

pub struct KotlinExposedBackend {
    manifest: BackendManifest,
}

impl KotlinExposedBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("unsupported engine '{}' for kotlin-exposed backend", engine),
                ));
            }
        };
        let manifest =
            super::load_or_default_manifest("backends/kotlin-exposed/manifest.toml", default_toml)?;
        Ok(Self { manifest })
    }
}

/// Get the Exposed column type function for a given Kotlin type.
fn exposed_column_fn(kotlin_type: &str) -> &str {
    match kotlin_type {
        "Boolean" => "bool",
        "Byte" => "byte",
        "Short" => "short",
        "Int" => "integer",
        "Long" => "long",
        "Float" => "float",
        "Double" => "double",
        "String" => "varchar",
        "ByteArray" => "binary",
        _ if kotlin_type.contains("BigDecimal") => "decimal",
        _ if kotlin_type.contains("LocalDate") => "date",
        _ if kotlin_type.contains("LocalTime") => "time",
        _ if kotlin_type.contains("OffsetTime") => "time",
        _ if kotlin_type.contains("LocalDateTime") => "datetime",
        _ if kotlin_type.contains("OffsetDateTime") => "timestampWithTimeZone",
        _ if kotlin_type.contains("UUID") => "uuid",
        _ => "text",
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

/// Get the Exposed column type class for use in `exec()` parameter binding.
fn exposed_column_type_class(kotlin_type: &str) -> &str {
    match kotlin_type {
        "Boolean" => "BooleanColumnType()",
        "Byte" => "ByteColumnType()",
        "Short" => "ShortColumnType()",
        "Int" => "IntegerColumnType()",
        "Long" => "LongColumnType()",
        "Float" => "FloatColumnType()",
        "Double" => "DoubleColumnType()",
        // TODO: varchar length 255 is hardcoded; see generate_model_struct TODO.
        "String" => "VarCharColumnType(255)",
        "ByteArray" => "BinaryColumnType()",
        _ if kotlin_type.contains("BigDecimal") => "DecimalColumnType(10, 2)",
        _ if kotlin_type.contains("LocalDate") => "JavaLocalDateColumnType()",
        _ if kotlin_type.contains("LocalTime") => "JavaLocalTimeColumnType()",
        _ if kotlin_type.contains("OffsetTime") => "JavaLocalTimeColumnType()",
        _ if kotlin_type.contains("LocalDateTime") => "JavaLocalDateTimeColumnType()",
        _ if kotlin_type.contains("OffsetDateTime") => "JavaOffsetDateTimeColumnType()",
        _ if kotlin_type.contains("UUID") => "UUIDColumnType()",
        _ => "TextColumnType()",
    }
}

impl CodegenBackend for KotlinExposedBackend {
    fn name(&self) -> &str {
        "kotlin-exposed"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql"]
    }

    fn file_header(&self) -> String {
        // ktlint requires lexicographic order and no wildcard imports.
        "import org.jetbrains.exposed.dao.id.IntIdTable\n\
         import org.jetbrains.exposed.sql.BinaryColumnType\n\
         import org.jetbrains.exposed.sql.BooleanColumnType\n\
         import org.jetbrains.exposed.sql.ByteColumnType\n\
         import org.jetbrains.exposed.sql.DecimalColumnType\n\
         import org.jetbrains.exposed.sql.DoubleColumnType\n\
         import org.jetbrains.exposed.sql.FloatColumnType\n\
         import org.jetbrains.exposed.sql.IntegerColumnType\n\
         import org.jetbrains.exposed.sql.LongColumnType\n\
         import org.jetbrains.exposed.sql.ShortColumnType\n\
         import org.jetbrains.exposed.sql.TextColumnType\n\
         import org.jetbrains.exposed.sql.VarCharColumnType\n\
         import org.jetbrains.exposed.sql.javatime.JavaLocalDateColumnType\n\
         import org.jetbrains.exposed.sql.javatime.JavaLocalDateTimeColumnType\n\
         import org.jetbrains.exposed.sql.javatime.JavaLocalTimeColumnType\n\
         import org.jetbrains.exposed.sql.javatime.JavaOffsetDateTimeColumnType\n\
         import org.jetbrains.exposed.sql.transactions.transaction\n"
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
        let table_obj_name = format!("{}Table", name);
        let mut out = String::new();
        // TODO: IntIdTable is hardcoded — detecting the actual PK type (LongIdTable,
        // UUIDTable, etc.) from schema DDL requires propagating PK column info through
        // the analyzer. Follow-up: https://github.com/scythe-sql/scythe/issues/XXX
        let _ = writeln!(
            out,
            "object {} : IntIdTable(\"{}\") {{",
            table_obj_name, table_name
        );
        for col in columns.iter() {
            let col_fn = exposed_column_fn(&col.lang_type);
            let nullable_suffix = if col.nullable { ".nullable()" } else { "" };
            // TODO: varchar length is hardcoded to 255 — column lengths from schema DDL
            // are not propagated through the analyzer yet. Follow-up needed to thread
            // length/precision metadata from DDL columns to codegen.
            if col_fn == "varchar" {
                let _ = writeln!(
                    out,
                    "    val {} = varchar(\"{}\", 255){}",
                    col.field_name, col.name, nullable_suffix
                );
            } else {
                let _ = writeln!(
                    out,
                    "    val {} = {}(\"{}\"){}",
                    col.field_name, col_fn, col.name, nullable_suffix
                );
            }
        }
        let _ = writeln!(out, "}}");
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
        let sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(
                &analyzed.sql,
                &analyzed.optional_params,
                &analyzed.params,
            ),
            |_| "?".to_string(),
        );

        let mut out = String::new();

        // Helper: write function signature with expression body.
        // ktlint requires: expression body (`= expr`), and when the body is multiline
        // the expression must start on a new line after `=`.
        let write_fn_sig = |out: &mut String, name: &str, ret: &str, params: &[ResolvedParam]| {
            let inline_params: String = params
                .iter()
                .map(|p| format!("{}: {}", p.field_name, p.full_type))
                .collect::<Vec<_>>()
                .join(", ");
            let sig = format!("fun {}({}){} =", name, inline_params, ret);
            if sig.len() <= 100 {
                let _ = writeln!(out, "{}", sig);
            } else {
                let _ = writeln!(out, "fun {}(", name);
                for p in params {
                    let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                }
                let _ = writeln!(out, "){} =", ret);
            }
            let _ = writeln!(out, "    transaction {{");
        };

        // Helper: build args list for exec()
        let build_args = |params: &[ResolvedParam]| -> String {
            if params.is_empty() {
                return String::new();
            }
            let pairs: Vec<String> = params
                .iter()
                .map(|p| {
                    format!(
                        "{} to {}",
                        exposed_column_type_class(&p.lang_type),
                        p.field_name
                    )
                })
                .collect();
            format!(", listOf({})", pairs.join(", "))
        };

        match &analyzed.command {
            QueryCommand::Exec => {
                write_fn_sig(&mut out, &func_name, "", params);
                let args = build_args(params);
                let _ = writeln!(out, "        exec(\"{}\"{})", sql, args);
                let _ = writeln!(out, "    }}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_fn_sig(&mut out, &func_name, ": Int", params);
                let args = build_args(params);
                let _ = writeln!(out, "        exec(\"{}\"{}) ?: 0", sql, args);
                let _ = writeln!(out, "    }}");
            }
            QueryCommand::One | QueryCommand::Opt => {
                let ret = format!(": {}?", struct_name);
                write_fn_sig(&mut out, &func_name, &ret, params);
                let args = build_args(params);
                let _ = writeln!(out, "        exec(\"{}\"{}) {{ rs ->", sql, args);
                let _ = writeln!(out, "            if (rs.next()) {{");
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
                    let _ = writeln!(
                        out,
                        "fun {}(items: List<{}>) =",
                        batch_fn_name, params_class_name
                    );
                    let _ = writeln!(out, "    transaction {{");
                    let _ = writeln!(out, "        for (item in items) {{");
                    let args: Vec<String> = params
                        .iter()
                        .map(|p| {
                            format!(
                                "{} to item.{}",
                                exposed_column_type_class(&p.lang_type),
                                p.field_name
                            )
                        })
                        .collect();
                    let _ = writeln!(
                        out,
                        "            exec(\"{}\", listOf({}))",
                        sql,
                        args.join(", ")
                    );
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "    }}");
                } else if params.len() == 1 {
                    let _ = writeln!(
                        out,
                        "fun {}(items: List<{}>) =",
                        batch_fn_name, params[0].full_type
                    );
                    let _ = writeln!(out, "    transaction {{");
                    let _ = writeln!(out, "        for (item in items) {{");
                    let _ = writeln!(
                        out,
                        "            exec(\"{}\", listOf({} to item))",
                        sql,
                        exposed_column_type_class(&params[0].lang_type)
                    );
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "    }}");
                } else {
                    let _ = writeln!(out, "fun {}(count: Int) =", batch_fn_name);
                    let _ = writeln!(out, "    transaction {{");
                    let _ = writeln!(out, "        repeat(count) {{");
                    let _ = writeln!(out, "            exec(\"{}\")", sql);
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "    }}");
                }
            }
            QueryCommand::Grouped => {
                // Grouped queries are not yet supported by this backend.
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "kotlin-exposed backend does not yet support :grouped queries".to_string(),
                ));
            }
            QueryCommand::Many => {
                let ret = format!(": List<{}>", struct_name);
                write_fn_sig(&mut out, &func_name, &ret, params);
                let args = build_args(params);
                let _ = writeln!(out, "        val result = mutableListOf<{}>()", struct_name);
                let _ = writeln!(out, "        exec(\"{}\"{}) {{ rs ->", sql, args);
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
                let _ = writeln!(out, "        }}");
                let _ = writeln!(out, "        result");
                let _ = writeln!(out, "    }}");
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
