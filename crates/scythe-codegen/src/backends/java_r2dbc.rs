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
        let manifest = super::load_or_default_manifest("backends/java-r2dbc/manifest.toml", default_toml)?;
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
        let _ = write!(out, ") {{}}");
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
        let sql = pg_to_r2dbc_params(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            self.is_pg,
        );

        let param_list = params.iter().map(java_annotated_param).collect::<Vec<_>>().join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let mut out = String::new();

        let write_binds = |out: &mut String, indent: &str| {
            for (i, param) in params.iter().enumerate() {
                let _ = writeln!(out, "{}.bind({}, {});", indent, i, param.field_name);
            }
        };

        let write_row_map = |out: &mut String, indent: &str| {
            let _ = writeln!(out, "{}new {}(", indent, struct_name);
            for (i, col) in columns.iter().enumerate() {
                let class = r2dbc_row_class(&col.lang_type);
                let sep = if i + 1 < columns.len() { "," } else { "" };
                let _ = writeln!(out, "{}    row.get(\"{}\", {}){}", indent, col.name, class, sep);
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
                let _ = writeln!(out, "            var stmt = conn.createStatement(\"{}\");", sql);
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
                let _ = writeln!(out, "            var stmt = conn.createStatement(\"{}\");", sql);
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
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(
                    out,
                    "public static Mono<{}> {}(ConnectionFactory cf{}{}) {{",
                    struct_name, func_name, sep, param_list
                );
                let _ = writeln!(out, "    return Mono.usingWhen(");
                let _ = writeln!(out, "        Mono.from(cf.create()),");
                let _ = writeln!(out, "        conn -> {{");
                let _ = writeln!(out, "            var stmt = conn.createStatement(\"{}\");", sql);
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
                let _ = writeln!(out, "            var stmt = conn.createStatement(\"{}\");", sql);
                write_binds(&mut out, "            stmt");
                let _ = writeln!(out, "            return Flux.from(stmt.execute())");
                let _ = writeln!(out, "                .flatMap(result -> result.map((row, meta) ->");
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
                        "public static Mono<Void> {}(ConnectionFactory cf, java.util.List<{}> items) {{",
                        batch_fn_name, params_record_name
                    );
                    let _ = writeln!(out, "    return Mono.from(cf.create())");
                    let _ = writeln!(out, "        .flatMap(conn -> {{");
                    let _ = writeln!(out, "            return Mono.from(conn.beginTransaction())");
                    let _ = writeln!(out, "                .then(Mono.defer(() -> {{");
                    let _ = writeln!(out, "                    var stmt = conn.createStatement(\"{}\");", sql);
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
                    let _ = writeln!(out, "                    return Flux.from(stmt.execute()).then();");
                    let _ = writeln!(out, "                }}))");
                    let _ = writeln!(out, "                .then(Mono.from(conn.commitTransaction()))");
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
                    let _ = writeln!(out, "                    var stmt = conn.createStatement(\"{}\");", sql);
                    let _ = writeln!(out, "                    boolean first = true;");
                    let _ = writeln!(out, "                    for (var item : items) {{");
                    let _ = writeln!(out, "                        if (!first) stmt.add();");
                    let _ = writeln!(out, "                        stmt.bind(0, item);");
                    let _ = writeln!(out, "                        first = false;");
                    let _ = writeln!(out, "                    }}");
                    let _ = writeln!(out, "                    return Flux.from(stmt.execute()).then();");
                    let _ = writeln!(out, "                }}))");
                    let _ = writeln!(out, "                .then(Mono.from(conn.commitTransaction()))");
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
                    let _ = writeln!(out, "                    var stmt = conn.createStatement(\"{}\");", sql);
                    let _ = writeln!(out, "                    for (int i = 1; i < count; i++) {{");
                    let _ = writeln!(out, "                        stmt.add();");
                    let _ = writeln!(out, "                    }}");
                    let _ = writeln!(out, "                    return Flux.from(stmt.execute()).then();");
                    let _ = writeln!(out, "                }}))");
                    let _ = writeln!(out, "                .then(Mono.from(conn.commitTransaction()))");
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
        let _ = writeln!(out, "    java.util.List<{}> children", child_struct_name);
        let _ = write!(out, ") {{}}");

        Ok(out)
    }

    fn generate_grouped_query_fn(&self, request: &GroupedQueryFn<'_>) -> Result<String, ScytheError> {
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let all_columns = request.all_columns;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = pg_to_r2dbc_params(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            self.is_pg,
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
            "public static Mono<java.util.List<{parent_struct_name}>> {func_name}(ConnectionFactory cf{sep}{param_list}) {{"
        );
        let _ = writeln!(out, "    return Flux.usingWhen(");
        let _ = writeln!(out, "        cf.create(),");
        let _ = writeln!(out, "        conn -> {{");
        let _ = writeln!(out, "            var stmt = conn.createStatement(\"{sql}\");");
        for (i, param) in params.iter().enumerate() {
            let _ = writeln!(out, "            stmt.bind({i}, {});", param.field_name);
        }
        let _ = writeln!(out, "            return Flux.from(stmt.execute())");
        let _ = writeln!(
            out,
            "                .flatMap(result -> result.map((row, meta) -> new Object[]{{"
        );
        for col in all_columns {
            let class = r2dbc_row_class(&col.lang_type);
            let _ = writeln!(out, "                    row.get(\"{}\", {}),", col.name, class);
        }
        let _ = writeln!(out, "                }});");
        let _ = writeln!(out, "        }},");
        let _ = writeln!(out, "        conn -> Mono.from(conn.close())");
        let _ = writeln!(out, "    ).collectList().map(rows -> {{");
        let _ = writeln!(
            out,
            "        var lookup = new java.util.LinkedHashMap<{key_type}, {parent_struct_name}>();"
        );
        let _ = writeln!(
            out,
            "        var result = new java.util.ArrayList<{parent_struct_name}>();"
        );
        let _ = writeln!(out, "        for (var raw : rows) {{");

        let key_ordinal = all_columns.iter().position(|c| c.name == key_column).unwrap_or(0);
        let key_cast = box_primitive(&key_col.lang_type);
        let _ = writeln!(out, "            var key = ({key_cast}) raw[{key_ordinal}];");

        let _ = writeln!(out, "            var child = new {child_struct_name}(");
        for (ci, col) in child_columns.iter().enumerate() {
            let ordinal = all_columns
                .iter()
                .position(|c| c.name == col.name)
                .unwrap_or(parent_columns.len() + ci);
            let cast_type = box_primitive(&col.lang_type);
            let sep = if ci + 1 < child_columns.len() { "," } else { "" };
            let _ = writeln!(out, "                ({cast_type}) raw[{ordinal}]{sep}");
        }
        let _ = writeln!(out, "            );");

        let _ = writeln!(out, "            if (lookup.containsKey(key)) {{");
        let _ = writeln!(out, "                lookup.get(key).children().add(child);");
        let _ = writeln!(out, "            }} else {{");
        let _ = writeln!(out, "                var parent = new {parent_struct_name}(");
        for col in parent_columns {
            let ordinal = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
            let cast_type = box_primitive(&col.lang_type);
            let _ = writeln!(out, "                    ({cast_type}) raw[{ordinal}],");
        }
        let _ = writeln!(
            out,
            "                    new java.util.ArrayList<>(java.util.List.of(child))"
        );
        let _ = writeln!(out, "                );");
        let _ = writeln!(out, "                lookup.put(key, parent);");
        let _ = writeln!(out, "                result.add(parent);");
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "        return result;");
        let _ = writeln!(out, "    }});");
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
    fn test_grouped_java_r2dbc_structs() {
        let backend = crate::backends::get_backend("java-r2dbc", "postgresql").unwrap();
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
            row_struct.contains("java.util.List<GetUsersWithOrdersChildRow> children"),
            "parent missing children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("public record GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("public record GetUsersWithOrdersRow(").unwrap();
        assert!(child_pos < parent_pos, "child must precede parent");
    }

    #[test]
    fn test_grouped_java_r2dbc_query_fn() {
        let backend = crate::backends::get_backend("java-r2dbc", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("Mono<java.util.List<GetUsersWithOrdersRow>>"),
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
            query_fn.contains("children().add(child)"),
            "must append child; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return result;"),
            "must return result; got:\n{query_fn}"
        );
    }
}
