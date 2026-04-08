use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_camel_case, to_pascal_case,
};

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/java-r2dbc.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/java-r2dbc.mysql.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/java-r2dbc.sqlite.toml");
const DEFAULT_MANIFEST_MARIADB: &str = include_str!("../../manifests/java-r2dbc.mariadb.toml");

pub struct JavaR2dbcBackend {
    manifest: BackendManifest,
    is_pg: bool,
}

impl JavaR2dbcBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            "mysql" => DEFAULT_MANIFEST_MYSQL,
            "mariadb" => DEFAULT_MANIFEST_MARIADB,
            "sqlite" | "sqlite3" => DEFAULT_MANIFEST_SQLITE,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("unsupported engine '{}' for java-r2dbc backend", engine),
                ));
            }
        };
        let manifest =
            super::load_or_default_manifest("backends/java-r2dbc/manifest.toml", default_toml)?;
        let is_pg = matches!(engine, "postgresql" | "postgres" | "pg");
        Ok(Self { manifest, is_pg })
    }
}

/// Convert PostgreSQL `$1, $2, ...` placeholders for R2DBC drivers.
/// PostgreSQL R2DBC uses `$1, $2, ...` natively (no conversion needed).
/// MySQL/SQLite R2DBC drivers use `?` placeholders.
fn pg_to_r2dbc_params(sql: &str, is_pg: bool) -> String {
    if is_pg {
        return sql.to_string();
    }
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

/// Get the R2DBC Row getter class for a given Java type.
fn r2dbc_row_class(java_type: &str) -> &str {
    match java_type {
        "boolean" | "Boolean" => "Boolean.class",
        "byte" | "Byte" => "Byte.class",
        "short" | "Short" => "Short.class",
        "int" | "Integer" => "Integer.class",
        "long" | "Long" => "Long.class",
        "float" | "Float" => "Float.class",
        "double" | "Double" => "Double.class",
        "String" => "String.class",
        "byte[]" => "byte[].class",
        _ if java_type.contains("BigDecimal") => "java.math.BigDecimal.class",
        _ if java_type.contains("LocalDate") => "java.time.LocalDate.class",
        _ if java_type.contains("LocalTime") => "java.time.LocalTime.class",
        _ if java_type.contains("OffsetTime") => "java.time.OffsetTime.class",
        _ if java_type.contains("LocalDateTime") => "java.time.LocalDateTime.class",
        _ if java_type.contains("OffsetDateTime") => "java.time.OffsetDateTime.class",
        _ if java_type.contains("UUID") => "java.util.UUID.class",
        _ => "Object.class",
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

impl CodegenBackend for JavaR2dbcBackend {
    fn name(&self) -> &str {
        "java-r2dbc"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "mysql", "mariadb", "sqlite"]
    }

    fn file_header(&self) -> String {
        "// Auto-generated by scythe. Do not edit.\n\
         import io.r2dbc.spi.ConnectionFactory;\n\
         import io.r2dbc.spi.Row;\n\
         import io.r2dbc.spi.RowMetadata;\n\
         import java.math.BigDecimal;\n\
         import java.time.LocalDate;\n\
         import java.time.LocalTime;\n\
         import java.time.OffsetDateTime;\n\
         import java.util.UUID;\n\
         import javax.annotation.Nonnull;\n\
         import javax.annotation.Nullable;\n\
         import reactor.core.publisher.Flux;\n\
         import reactor.core.publisher.Mono;"
            .to_string()
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
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
        let _ = write!(out, ") {{}}");
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
        let sql = pg_to_r2dbc_params(
            &super::clean_sql_oneline_with_optional(
                &analyzed.sql,
                &analyzed.optional_params,
                &analyzed.params,
            ),
            self.is_pg,
        );

        let param_list = params
            .iter()
            .map(java_annotated_param)
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let mut out = String::new();

        // Helper: write .bind() calls for R2DBC (0-based indexing)
        let write_binds = |out: &mut String, indent: &str| {
            for (i, param) in params.iter().enumerate() {
                let _ = writeln!(out, "{}.bind({}, {})", indent, i, param.field_name);
            }
        };

        // Helper: write row mapping expression
        let write_row_map = |out: &mut String, indent: &str| {
            let _ = writeln!(out, "{}new {}(", indent, struct_name);
            for (i, col) in columns.iter().enumerate() {
                let class = r2dbc_row_class(&col.lang_type);
                let sep = if i + 1 < columns.len() { "," } else { "" };
                let _ = writeln!(
                    out,
                    "{}    row.get(\"{}\", {}){}",
                    indent, col.name, class, sep
                );
            }
            let _ = write!(out, "{})", indent);
        };

        match &analyzed.command {
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "public static Mono<Void> {}(ConnectionFactory cf{}{}) {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "    return Mono.usingWhen(");
                let _ = writeln!(out, "        Mono.from(cf.create()),");
                let _ = writeln!(out, "        conn -> {{");
                let _ = writeln!(
                    out,
                    "            var stmt = conn.createStatement(\"{}\");",
                    sql
                );
                write_binds(&mut out, "            stmt");
                let _ = writeln!(out, "            return Mono.from(stmt.execute())");
                let _ = writeln!(
                    out,
                    "                .flatMap(result -> Mono.from(result.getRowsUpdated()))"
                );
                let _ = writeln!(out, "                .then();");
                let _ = writeln!(out, "        }},");
                let _ = writeln!(out, "        conn -> Mono.from(conn.close())");
                let _ = writeln!(out, "    );");
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "public static Mono<Long> {}(ConnectionFactory cf{}{}) {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "    return Mono.usingWhen(");
                let _ = writeln!(out, "        Mono.from(cf.create()),");
                let _ = writeln!(out, "        conn -> {{");
                let _ = writeln!(
                    out,
                    "            var stmt = conn.createStatement(\"{}\");",
                    sql
                );
                write_binds(&mut out, "            stmt");
                let _ = writeln!(out, "            return Mono.from(stmt.execute())");
                let _ = writeln!(
                    out,
                    "                .flatMap(result -> Mono.from(result.getRowsUpdated()));"
                );
                let _ = writeln!(out, "        }},");
                let _ = writeln!(out, "        conn -> Mono.from(conn.close())");
                let _ = writeln!(out, "    );");
                let _ = write!(out, "}}");
            }
            QueryCommand::One => {
                let _ = writeln!(
                    out,
                    "public static Mono<{}> {}(ConnectionFactory cf{}{}) {{",
                    struct_name, func_name, sep, param_list
                );
                let _ = writeln!(out, "    return Mono.usingWhen(");
                let _ = writeln!(out, "        Mono.from(cf.create()),");
                let _ = writeln!(out, "        conn -> {{");
                let _ = writeln!(
                    out,
                    "            var stmt = conn.createStatement(\"{}\");",
                    sql
                );
                write_binds(&mut out, "            stmt");
                let _ = writeln!(out, "            return Mono.from(stmt.execute())");
                let _ = writeln!(
                    out,
                    "                .flatMap(result -> Mono.from(result.map((row, meta) ->"
                );
                write_row_map(&mut out, "                    ");
                let _ = writeln!(out, ")));");
                let _ = writeln!(out, "        }},");
                let _ = writeln!(out, "        conn -> Mono.from(conn.close())");
                let _ = writeln!(out, "    );");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "public static Flux<{}> {}(ConnectionFactory cf{}{}) {{",
                    struct_name, func_name, sep, param_list
                );
                let _ = writeln!(out, "    return Flux.usingWhen(");
                let _ = writeln!(out, "        cf.create(),");
                let _ = writeln!(out, "        conn -> {{");
                let _ = writeln!(
                    out,
                    "            var stmt = conn.createStatement(\"{}\");",
                    sql
                );
                write_binds(&mut out, "            stmt");
                let _ = writeln!(out, "            return Flux.from(stmt.execute())");
                let _ = writeln!(
                    out,
                    "                .flatMap(result -> result.map((row, meta) ->"
                );
                write_row_map(&mut out, "                    ");
                let _ = writeln!(out, "));");
                let _ = writeln!(out, "        }},");
                let _ = writeln!(out, "        conn -> Mono.from(conn.close())");
                let _ = writeln!(out, "    );");
                let _ = write!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                if params.len() > 1 {
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
                        "public static Mono<Void> {}(ConnectionFactory cf, java.util.List<{}> items) {{",
                        batch_fn_name, params_record_name
                    );
                    let _ = writeln!(out, "    return Mono.from(cf.create())");
                    let _ = writeln!(out, "        .flatMap(conn -> {{");
                    let _ = writeln!(out, "            return Mono.from(conn.beginTransaction())");
                    let _ = writeln!(out, "                .then(Mono.defer(() -> {{");
                    let _ = writeln!(
                        out,
                        "                    var stmt = conn.createStatement(\"{}\");",
                        sql
                    );
                    let _ = writeln!(out, "                    boolean first = true;");
                    let _ = writeln!(out, "                    for (var item : items) {{");
                    let _ = writeln!(out, "                        if (!first) stmt.add();");
                    for (i, param) in params.iter().enumerate() {
                        let _ = writeln!(
                            out,
                            "                        stmt.bind({}, item.{}());",
                            i, param.field_name
                        );
                    }
                    let _ = writeln!(out, "                        first = false;");
                    let _ = writeln!(out, "                    }}");
                    let _ = writeln!(
                        out,
                        "                    return Flux.from(stmt.execute()).then();"
                    );
                    let _ = writeln!(out, "                }}))");
                    let _ = writeln!(
                        out,
                        "                .then(Mono.from(conn.commitTransaction()))"
                    );
                    let _ = writeln!(
                        out,
                        "                .onErrorResume(e -> Mono.from(conn.rollbackTransaction()).then(Mono.error(e)))"
                    );
                    let _ = writeln!(
                        out,
                        "                .doFinally(s -> Mono.from(conn.close()).subscribe());"
                    );
                    let _ = writeln!(out, "        }});");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let param = &params[0];
                    let _ = writeln!(
                        out,
                        "public static Mono<Void> {}(ConnectionFactory cf, java.util.List<{}> items) {{",
                        batch_fn_name,
                        java_param_type(param)
                    );
                    let _ = writeln!(out, "    return Mono.from(cf.create())");
                    let _ = writeln!(out, "        .flatMap(conn -> {{");
                    let _ = writeln!(out, "            return Mono.from(conn.beginTransaction())");
                    let _ = writeln!(out, "                .then(Mono.defer(() -> {{");
                    let _ = writeln!(
                        out,
                        "                    var stmt = conn.createStatement(\"{}\");",
                        sql
                    );
                    let _ = writeln!(out, "                    boolean first = true;");
                    let _ = writeln!(out, "                    for (var item : items) {{");
                    let _ = writeln!(out, "                        if (!first) stmt.add();");
                    let _ = writeln!(out, "                        stmt.bind(0, item);");
                    let _ = writeln!(out, "                        first = false;");
                    let _ = writeln!(out, "                    }}");
                    let _ = writeln!(
                        out,
                        "                    return Flux.from(stmt.execute()).then();"
                    );
                    let _ = writeln!(out, "                }}))");
                    let _ = writeln!(
                        out,
                        "                .then(Mono.from(conn.commitTransaction()))"
                    );
                    let _ = writeln!(
                        out,
                        "                .onErrorResume(e -> Mono.from(conn.rollbackTransaction()).then(Mono.error(e)))"
                    );
                    let _ = writeln!(
                        out,
                        "                .doFinally(s -> Mono.from(conn.close()).subscribe());"
                    );
                    let _ = writeln!(out, "        }});");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "public static Mono<Void> {}(ConnectionFactory cf, int count) {{",
                        batch_fn_name
                    );
                    let _ = writeln!(out, "    return Mono.from(cf.create())");
                    let _ = writeln!(out, "        .flatMap(conn -> {{");
                    let _ = writeln!(out, "            return Mono.from(conn.beginTransaction())");
                    let _ = writeln!(out, "                .then(Mono.defer(() -> {{");
                    let _ = writeln!(
                        out,
                        "                    var stmt = conn.createStatement(\"{}\");",
                        sql
                    );
                    let _ = writeln!(
                        out,
                        "                    for (int i = 1; i < count; i++) {{"
                    );
                    let _ = writeln!(out, "                        stmt.add();");
                    let _ = writeln!(out, "                    }}");
                    let _ = writeln!(
                        out,
                        "                    return Flux.from(stmt.execute()).then();"
                    );
                    let _ = writeln!(out, "                }}))");
                    let _ = writeln!(
                        out,
                        "                .then(Mono.from(conn.commitTransaction()))"
                    );
                    let _ = writeln!(
                        out,
                        "                .onErrorResume(e -> Mono.from(conn.rollbackTransaction()).then(Mono.error(e)))"
                    );
                    let _ = writeln!(
                        out,
                        "                .doFinally(s -> Mono.from(conn.close()).subscribe());"
                    );
                    let _ = writeln!(out, "        }});");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Grouped => {
                // Grouped queries are not yet supported for java-r2dbc
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "grouped queries are not yet supported for java-r2dbc".to_string(),
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
