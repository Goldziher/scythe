use std::collections::HashMap;
use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    composite_type_name, enum_type_name, enum_variant_name, fn_name, to_camel_case, to_pascal_case,
};

use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};
use crate::backends::jvm_common;

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
                    ErrorCode::InvalidConfig,
                    format!("unsupported engine '{}' for java-r2dbc backend", engine),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
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

/// The class literal to hand `Row.get(name, Class<T>)` for a column.
///
/// `Row.get` is generic in its class argument, so the only class that produces
/// a value assignable to the declared field is the declared type's own —
/// derived here rather than looked up in a parallel table. The table this
/// replaced ended in `Object.class`, which caught composites and enums and
/// made every such record constructor call a compile error; it also matched
/// `LocalDateTime` against its `contains("LocalDate")` arm first, silently
/// reading a `datetime` column as `java.time.LocalDate`.
///
/// Primitives are boxed first: `Row.get` is generic, and a generic type
/// argument cannot be a primitive (`int.class` is `Class<Integer>` at best and
/// `row.get(name, int.class)` does not type-check against `T get(String,
/// Class<T>)` the way the boxed form does).
fn r2dbc_row_class(java_type: &str) -> String {
    jvm_common::java_class_literal(box_primitive(java_type))
}

/// The boxed Java element type for an array column's `List<{T}>`. See
/// `java_jdbc.rs`'s `java_array_element_type` for the full reasoning --
/// duplicated here rather than shared because the two backends have no other
/// coupling and this is a five-line wrapper around `resolve_type`.
fn java_array_element_type(element_neutral: &str, manifest: &BackendManifest) -> String {
    resolve_type(element_neutral, manifest, false)
        .map(|t| box_primitive(&t).to_string())
        .unwrap_or_else(|_| "Object".to_string())
}

/// The Java type an array column is declared *and* cast as: `List<{T}>` with
/// `T` boxed. Not `col.lang_type`/`col.full_type` -- see
/// `java_array_element_type`.
fn java_array_list_type(element_neutral: &str, manifest: &BackendManifest) -> String {
    format!("java.util.List<{}>", java_array_element_type(element_neutral, manifest))
}

/// Resolve the display type for a Java field, boxing primitives when nullable.
fn java_field_type(col: &ResolvedColumn, manifest: &BackendManifest) -> String {
    if let Some(element) = jvm_common::array_element_neutral_type(&col.neutral_type) {
        return java_array_list_type(element, manifest);
    }
    if col.nullable {
        box_primitive(&col.lang_type).to_string()
    } else {
        col.full_type.clone()
    }
}

/// Splits a PostgreSQL composite's text form (`"(a,b,c)"`) into its raw field tokens. See
/// `java_jdbc.rs`'s twin constant for the full reasoning -- duplicated rather than shared
/// because the two backends have no other coupling.
const JAVA_PARSE_COMPOSITE_FIELDS_METHOD: &str = r#"    /**
     * ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
     * honoring its escaping rules: an empty unquoted field is SQL NULL (returned as `null`); a
     * field needing quoting (containing a comma, paren, quote, backslash, or leading/trailing
     * space, or the empty string) is wrapped in double quotes with `"` and `\` backslash-escaped
     * inside; every other field is unquoted and taken literally. A nested composite's own
     * "(x,y)" text form always contains parens, so it always comes back quoted here, ready for
     * that type's own `fromText` to parse recursively.
     */
    private static java.util.List<String> parseCompositeFields(String text) {
        java.util.List<String> fields = new java.util.ArrayList<>();
        String inner = text.substring(1, text.length() - 1);
        int i = 0;
        int n = inner.length();
        while (true) {
            StringBuilder field = new StringBuilder();
            boolean isNull = false;
            if (i < n && inner.charAt(i) == '"') {
                i++;
                while (i < n) {
                    char c = inner.charAt(i);
                    if (c == '\\' && i + 1 < n) {
                        field.append(inner.charAt(i + 1));
                        i += 2;
                    } else if (c == '"' && i + 1 < n && inner.charAt(i + 1) == '"') {
                        field.append('"');
                        i += 2;
                    } else if (c == '"') {
                        i++;
                        break;
                    } else {
                        field.append(c);
                        i++;
                    }
                }
            } else {
                int start = i;
                while (i < n && inner.charAt(i) != ',') {
                    i++;
                }
                field.append(inner, start, i);
                isNull = field.length() == 0;
            }
            fields.add(isNull ? null : field.toString());
            if (i < n && inner.charAt(i) == ',') {
                i++;
                continue;
            }
            break;
        }
        return fields;
    }
"#;

/// PostgreSQL's default `bytea` text output is hex (`"\x48656c6c6f"`); decode the digits after
/// the `\x` prefix back into bytes. Emitted only when a composite has a `bytes` field.
const JAVA_PARSE_COMPOSITE_BYTES_METHOD: &str = r#"    /**
     * ~keep PostgreSQL's default `bytea` text output is hex: "\x48656c6c6f". Decode the hex
     * digits after the "\x" prefix back into bytes.
     */
    private static byte[] parseCompositeBytes(String hex) {
        String digits = hex.substring(2);
        byte[] result = new byte[digits.length() / 2];
        for (int i = 0; i < result.length; i++) {
            result[i] = (byte) Integer.parseInt(digits.substring(i * 2, i * 2 + 2), 16);
        }
        return result;
    }
"#;

/// PostgreSQL's default `timestamptz` text output uses a space instead of `T` and omits the
/// offset's minutes when they are zero; normalize both before handing the text to `java.time`.
/// Emitted only when a composite has a `datetime_tz` field.
const JAVA_PARSE_COMPOSITE_OFFSET_DATETIME_METHOD: &str = r#"    /**
     * ~keep PostgreSQL's default `timestamptz` text output uses a space instead of `T`
     * ("2024-01-15 10:30:00+00") and, unlike `OffsetDateTime.parse`, omits the offset's minutes
     * when they are zero ("+00" rather than "+00:00"). Normalize both before parsing.
     */
    private static java.time.OffsetDateTime parseCompositeOffsetDateTime(String raw) {
        String s = raw.replace(' ', 'T');
        char sign = s.charAt(s.length() - 3);
        if (sign == '+' || sign == '-') {
            s = s + ":00";
        }
        return java.time.OffsetDateTime.parse(s);
    }
"#;

/// PostgreSQL's default `timetz` text output omits the offset's minutes when they are zero;
/// `OffsetTime.parse` rejects that. Emitted only when a composite has a `time_tz` field.
const JAVA_PARSE_COMPOSITE_OFFSET_TIME_METHOD: &str = r#"    /**
     * ~keep PostgreSQL's default `timetz` text output omits the offset's minutes when they are
     * zero ("13:22:43-05" rather than "13:22:43-05:00"), which `OffsetTime.parse` rejects.
     */
    private static java.time.OffsetTime parseCompositeOffsetTime(String raw) {
        String s = raw;
        char sign = s.charAt(s.length() - 3);
        if (sign == '+' || sign == '-') {
            s = s + ":00";
        }
        return java.time.OffsetTime.parse(s);
    }
"#;

fn composite_needs_bytes_helper(composite: &CompositeInfo) -> bool {
    composite.fields.iter().any(|f| f.neutral_type == "bytes")
}

fn composite_needs_offset_datetime_helper(composite: &CompositeInfo) -> bool {
    composite.fields.iter().any(|f| f.neutral_type == "datetime_tz")
}

fn composite_needs_offset_time_helper(composite: &CompositeInfo) -> bool {
    composite.fields.iter().any(|f| f.neutral_type == "time_tz")
}

/// The Java expression converting one composite field's raw text token (`raw`, a possibly-null
/// `String` already unescaped by `parseCompositeFields`) into the field's declared Java type --
/// the inverse of what PostgreSQL's composite output function wrote for that field. See
/// `java_jdbc.rs`'s twin function for the full reasoning, including why a genuinely NULL
/// sub-field converted through a primitive arm is a pre-existing, out-of-scope gap.
fn composite_field_from_text(neutral_type: &str, field_type: &str, raw: &str, manifest: &BackendManifest) -> String {
    if let Some(sql_name) = neutral_type.strip_prefix("composite::") {
        return format!("{}.fromText({})", composite_type_name(sql_name, &manifest.naming), raw);
    }
    if neutral_type.starts_with("enum::") {
        return format!("{}.fromValue({})", field_type, raw);
    }
    match neutral_type {
        "bool" => format!("\"t\".equals({})", raw),
        "int16" => format!("Short.parseShort({})", raw),
        "int32" => format!("Integer.parseInt({})", raw),
        "int64" => format!("Long.parseLong({})", raw),
        "float32" => format!("Float.parseFloat({})", raw),
        "float64" => format!("Double.parseDouble({})", raw),
        "decimal" => format!("new java.math.BigDecimal({})", raw),
        "uuid" => format!("java.util.UUID.fromString({})", raw),
        "date" => format!("java.time.LocalDate.parse({})", raw),
        "time" => format!("java.time.LocalTime.parse({})", raw),
        "datetime" => format!("java.time.LocalDateTime.parse({}.replace(' ', 'T'))", raw),
        "datetime_tz" => format!("parseCompositeOffsetDateTime({})", raw),
        "time_tz" => format!("parseCompositeOffsetTime({})", raw),
        "bytes" => format!("parseCompositeBytes({})", raw),
        // "string"/"json"/"inet"/"interval" all resolve to Java `String`, so the already-parsed
        // text needs no further conversion. Any neutral type not named above (e.g. an
        // array-typed composite field, which this fix does not handle -- see board #196's
        // report) falls through here too; passing the raw text through is the least-wrong
        // fallback available at generate time rather than a hard error.
        _ => raw.to_string(),
    }
}

/// Build the `Row.get` expression for a column, handling arrays and enums
/// specially -- everywhere else defers to [`r2dbc_row_class`].
///
/// Arrays: R2DBC hands back the driver's native array shape (`String[]`,
/// `Integer[]`, never a `List`), so the class literal asked for is an array
/// class (`String[].class`) and the result is wrapped in `Arrays.asList`.
/// `row.get` returns `null` for a NULL column, and `Arrays.asList(null)`
/// throws `NullPointerException`, hence the null check.
///
/// Enums: an R2DBC driver has no reason to know about a generated enum type,
/// and `row.get(col, WidgetStatus.class)` is driver-codec-dependent --
/// PostgreSQL's own enum codec is opt-in and, unregistered, throws at
/// runtime. Reading the wire value as `String` and decoding through
/// `fromValue` (the same conversion the JDBC family already does) has no such
/// dependency and matches the wire value the bind side sends.
///
/// board #196: a composite column is read the same way -- an unregistered composite has no
/// r2dbc-postgresql codec either, so `row.get(col, T.class)` is just as driver-codec-dependent
/// as an enum's. The wire value is read as `String` and parsed by `T.fromText` (emitted by
/// `generate_composite_def`), which is already null-safe.
fn r2dbc_col_read_expr(col: &ResolvedColumn, manifest: &BackendManifest) -> String {
    if col.neutral_type.starts_with("composite::") {
        return format!("{}.fromText(row.get(\"{}\", String.class))", col.lang_type, col.name);
    }
    if let Some(element) = jvm_common::array_element_neutral_type(&col.neutral_type) {
        let boxed_element = java_array_element_type(element, manifest);
        let get_expr = format!("row.get(\"{}\", {}[].class)", col.name, boxed_element);
        return format!("{get_expr} == null ? null : java.util.Arrays.asList({get_expr})");
    }
    if col.neutral_type.starts_with("enum::") {
        return format!("{}.fromValue(row.get(\"{}\", String.class))", col.lang_type, col.name);
    }
    let class = r2dbc_row_class(&col.lang_type);
    format!("row.get(\"{}\", {})", col.name, class)
}

/// The Java type an `:grouped` raw `Object[]` element is cast back to.
/// Mirrors [`java_field_type`]'s array branch (nullability is irrelevant to a
/// cast target -- Java generics carry no runtime nullability distinction) and
/// [`box_primitive`] otherwise, matching what [`r2dbc_col_read_expr`] actually
/// stored at that ordinal (a `fromValue`-decoded enum, a `List<T>`, or a
/// boxed scalar).
fn r2dbc_cast_type(col: &ResolvedColumn, manifest: &BackendManifest) -> String {
    if let Some(element) = jvm_common::array_element_neutral_type(&col.neutral_type) {
        return java_array_list_type(element, manifest);
    }
    box_primitive(&col.lang_type).to_string()
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

/// ~keep The expression bound at an R2DBC placeholder for `param`.
///
/// An enum parameter must be bound as its SQL spelling, not as the Java enum object:
/// r2dbc-postgresql has no codec for a user enum type and fails the whole statement with
/// "Cannot encode parameter of type generated.Queries$UserStatus (ACTIVE)". `getValue()`
/// is the accessor `generate_enum_def` already emits, and it is the exact inverse of the
/// `fromValue(...)` the read path uses, so a value round-trips even when its SQL spelling
/// is not the uppercase of its variant name.
fn r2dbc_bind_expr(param: &ResolvedParam) -> String {
    if param.neutral_type.starts_with("enum::") {
        format!("{}.getValue()", param.field_name)
    } else {
        param.field_name.clone()
    }
}

/// ~keep Append an explicit `::<enum type>` cast to each PostgreSQL placeholder whose parameter
/// is an enum.
///
/// R2DBC is stricter than JDBC here. A JDBC `setObject` sends the value untyped and lets the
/// server infer it, so `WHERE status = $1` against a `user_status` column just works. The
/// r2dbc-postgresql driver instead sends a *typed* parameter, and since the bind expression is
/// the enum's SQL spelling (a String -- see `r2dbc_bind_expr`), the server receives
/// `character varying` and refuses: "column \"status\" is of type user_status but expression is
/// of type character varying", or "operator does not exist: user_status = character varying" in
/// a predicate. Casting at the placeholder keeps the generated code self-sufficient -- the
/// alternative, registering an `EnumCodec` on the ConnectionFactory, would push the requirement
/// onto every caller and silently break the ones who did not.
///
/// Only for PostgreSQL: MySQL enums are already strings on the wire.
fn add_pg_enum_casts(sql: &str, params: &[ResolvedParam], is_pg: bool) -> String {
    if !is_pg || !params.iter().any(|p| p.neutral_type.starts_with("enum::")) {
        return sql.to_string();
    }
    let mut result = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' || !chars.peek().is_some_and(|c| c.is_ascii_digit()) {
            result.push(ch);
            continue;
        }
        let mut digits = String::new();
        while let Some(c) = chars.peek().copied() {
            if !c.is_ascii_digit() {
                break;
            }
            digits.push(c);
            chars.next();
        }
        result.push('$');
        result.push_str(&digits);
        // Placeholders are 1-based; `params` is 0-based and in placeholder order.
        let enum_type = digits
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|idx| params.get(idx))
            .and_then(|p| p.neutral_type.strip_prefix("enum::"));
        if let Some(enum_type) = enum_type {
            result.push_str("::");
            result.push_str(enum_type);
        }
    }
    result
}

impl CodegenBackend for JavaR2dbcBackend {
    fn name(&self) -> &str {
        "java-r2dbc"
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
            super::apply_field_case_option(&mut self.manifest.naming, "java-r2dbc", value)?;
        }
        Ok(())
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "mysql", "mariadb", "sqlite"]
    }

    /// The header ends by opening `public class Queries {`, matching
    /// `java-jdbc`.
    ///
    /// Without it this backend emitted top-level `public record`s, a top-level
    /// `public enum`, and bare `public static` methods into one file. None of
    /// that is legal Java: a compilation unit may hold at most one public type
    /// and cannot hold a method at all, so every generated file failed with
    /// `class X is public, should be declared in a file named X.java` followed
    /// by `<identifier> expected` on the first method. Nesting everything in
    /// one public class — records, enums, and static methods alike — is what
    /// `java-jdbc` already does and needs no name coordination with the file
    /// the CLI writes (`Queries.java`).
    fn file_header(&self) -> String {
        "package generated;\n\
         \n\
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
         import reactor.core.publisher.Mono;\n\
         \n\
         public class Queries {"
            .to_string()
    }

    fn file_footer(&self) -> String {
        "}\n".to_string()
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        _query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();

        let fields = columns
            .iter()
            .map(|c| {
                let field_type = java_field_type(c, &self.manifest);
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

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = crate::sql_literal::escape_java_string(&pg_to_r2dbc_params(
            &add_pg_enum_casts(
                &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
                params,
                self.is_pg,
            ),
            self.is_pg,
        ));

        let param_list = params.iter().map(java_annotated_param).collect::<Vec<_>>().join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let mut out = String::new();

        let write_binds = |out: &mut String, indent: &str| {
            for (i, param) in params.iter().enumerate() {
                let _ = writeln!(out, "{}.bind({}, {});", indent, i, r2dbc_bind_expr(param));
            }
        };

        let write_row_map = |out: &mut String, indent: &str| {
            let _ = writeln!(out, "{}new {}(", indent, struct_name);
            for (i, col) in columns.iter().enumerate() {
                let expr = r2dbc_col_read_expr(col, &self.manifest);
                let sep = if i + 1 < columns.len() { "," } else { "" };
                let _ = writeln!(out, "{}    {}{}", indent, expr, sep);
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
                // ~keep #192: a missing row in a reactive chain is not a
                // thrown exception at call time -- it is an error signal on
                // the publisher. `:opt`'s own shape was already correct: an
                // empty `Mono<T>` (no error) is exactly what "zero or one"
                // means to a Reactor subscriber, and `Mono<{Struct}>` never
                // changes. `:one` needs the empty case turned into an error
                // signal instead, so this appends `.switchIfEmpty(Mono.error(...))`
                // to the row-producing Mono -- never `.block()` or any other
                // synchronous idiom, which would defeat the reactive contract
                // this backend exists for.
                let is_one = matches!(analyzed.command, QueryCommand::One);
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
                if is_one {
                    let _ = writeln!(out, ")))");
                    let _ = writeln!(
                        out,
                        "                .switchIfEmpty(Mono.error(new java.util.NoSuchElementException(\"{}: no rows returned\")));",
                        func_name
                    );
                } else {
                    let _ = writeln!(out, ")));");
                }
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
        let _ = writeln!(out);
        // ~keep #213: see `java_jdbc.rs`'s `generate_enum_def` for the full
        // reasoning -- decoding against the declared `value` rather than the
        // sanitised variant spelling is what makes `fromValue(x.getValue()) ==
        // x` hold for every variant.
        let _ = writeln!(out, "    public static {} fromValue(String value) {{", type_name);
        let _ = writeln!(out, "        for ({} v : values()) {{", type_name);
        let _ = writeln!(out, "            if (v.value.equals(value)) {{");
        let _ = writeln!(out, "                return v;");
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(
            out,
            "        throw new IllegalArgumentException(\"Unknown {} value: \" + value);",
            type_name
        );
        let _ = writeln!(out, "    }}");
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        // ~keep board #196: a composite with zero fields cannot exist in PostgreSQL (`CREATE
        // TYPE ... AS ()` is rejected), so there is no reachable runtime value that would need
        // `fromText` here. Left as the bare record it always was.
        if composite.fields.is_empty() {
            let _ = writeln!(out, "public record {}() {{}}", name);
            return Ok(out);
        }
        let field_types: Vec<String> = composite
            .fields
            .iter()
            .map(|f| {
                resolve_type(&f.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .unwrap_or_else(|_| "Object".to_string())
            })
            .collect();
        let params = composite
            .fields
            .iter()
            .zip(&field_types)
            .map(|(f, field_type)| format!("{} {}", field_type, to_camel_case(&f.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "public record {}({}) {{", name, params);
        let _ = writeln!(out);
        let _ = writeln!(out, "    /**");
        let _ = writeln!(
            out,
            "     * ~keep board #196: r2dbc-postgresql has no codec for this composite -- an unregistered"
        );
        let _ = writeln!(
            out,
            "     * `row.get(col, {}.class)` is driver-codec-dependent and throws at runtime, the",
            name
        );
        let _ = writeln!(
            out,
            "     * same problem an enum column has. Parse the driver's text form instead."
        );
        let _ = writeln!(out, "     */");
        let _ = writeln!(out, "    public static {} fromText(String text) {{", name);
        let _ = writeln!(out, "        if (text == null) {{");
        let _ = writeln!(out, "            return null;");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "        java.util.List<String> f = parseCompositeFields(text);");
        let _ = writeln!(out, "        return new {}(", name);
        for (i, (field, field_type)) in composite.fields.iter().zip(&field_types).enumerate() {
            let raw = format!("f.get({})", i);
            let value_expr = composite_field_from_text(&field.neutral_type, field_type, &raw, &self.manifest);
            let sep = if i + 1 < composite.fields.len() { "," } else { "" };
            let _ = writeln!(out, "            {}{}", value_expr, sep);
        }
        let _ = writeln!(out, "        );");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out);
        out.push_str(JAVA_PARSE_COMPOSITE_FIELDS_METHOD);
        if composite_needs_bytes_helper(composite) {
            let _ = writeln!(out);
            out.push_str(JAVA_PARSE_COMPOSITE_BYTES_METHOD);
        }
        if composite_needs_offset_datetime_helper(composite) {
            let _ = writeln!(out);
            out.push_str(JAVA_PARSE_COMPOSITE_OFFSET_DATETIME_METHOD);
        }
        if composite_needs_offset_time_helper(composite) {
            let _ = writeln!(out);
            out.push_str(JAVA_PARSE_COMPOSITE_OFFSET_TIME_METHOD);
        }
        let _ = write!(out, "}}");
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
            let field_type = java_field_type(c, &self.manifest);
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
            let field_type = java_field_type(c, &self.manifest);
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
        let sql = crate::sql_literal::escape_java_string(&pg_to_r2dbc_params(
            &add_pg_enum_casts(
                &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
                params,
                self.is_pg,
            ),
            self.is_pg,
        ));

        let param_list = params.iter().map(java_annotated_param).collect::<Vec<_>>().join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let key_col = parent_columns
            .iter()
            .find(|c| c.name == key_column)
            .unwrap_or(&parent_columns[0]);
        let key_type = r2dbc_cast_type(key_col, &self.manifest);

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
            let _ = writeln!(out, "            stmt.bind({i}, {});", r2dbc_bind_expr(param));
        }
        let _ = writeln!(out, "            return Flux.from(stmt.execute())");
        let _ = writeln!(
            out,
            "                .flatMap(result -> result.map((row, meta) -> new Object[]{{"
        );
        for (i, col) in all_columns.iter().enumerate() {
            let expr = r2dbc_col_read_expr(col, &self.manifest);
            let sep = if i + 1 < all_columns.len() { "," } else { "" };
            let _ = writeln!(out, "                    {}{}", expr, sep);
        }
        // ~keep Three closers, not one: `}` ends the `new Object[]{` initializer,
        // then `)` ends `result.map(`, then `)` ends `.flatMap(`. This emitted a
        // bare `});` and left `.flatMap(` open, so every `:grouped` java-r2dbc
        // file failed with `')' or ',' expected` before any type-checking began.
        let _ = writeln!(out, "                }}));");
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
        let key_cast = r2dbc_cast_type(key_col, &self.manifest);
        let _ = writeln!(out, "            var key = ({key_cast}) raw[{key_ordinal}];");

        let _ = writeln!(out, "            var child = new {child_struct_name}(");
        for (ci, col) in child_columns.iter().enumerate() {
            let ordinal = all_columns
                .iter()
                .position(|c| c.name == col.name)
                .unwrap_or(parent_columns.len() + ci);
            let cast_type = r2dbc_cast_type(col, &self.manifest);
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
            let cast_type = r2dbc_cast_type(col, &self.manifest);
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
    use std::collections::HashMap;

    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    use super::JavaR2dbcBackend;
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
    /// up. `write_row_map` reads `row.get(col.name, class)` -- the raw SQL
    /// column name -- positionally into the record constructor; only the
    /// declared record component (`col.field_name`) changes under
    /// `field_case = "camelCase"`.
    #[test]
    fn test_field_case_camel_case_renames_field_but_keeps_raw_lookup_key() {
        let mut backend = JavaR2dbcBackend::new("postgresql").unwrap();
        backend
            .apply_options(&HashMap::from([("field_case".to_string(), "camelCase".to_string())]))
            .unwrap();
        let query = make_one_query_with_snake_case_columns();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            row_struct.contains("int userId"),
            "field_case must rename the declared record field; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains("int user_id"),
            "must not leave the raw SQL name in the declared field; got:\n{row_struct}"
        );
        assert!(
            query_fn.contains("row.get(\"user_id\", Integer.class)"),
            "the Row lookup key must stay the raw SQL column name; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("row.get(\"userId\""),
            "must never look the driver up by the renamed field; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_field_case_option_rejects_invalid_value() {
        let mut backend = JavaR2dbcBackend::new("postgresql").unwrap();
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
    /// through `to_snake_case` (commit 6ab8994), so a composite's SQL name
    /// and field names that carry runs of capitals ("CreateAPIKey",
    /// "HTTPSUrl") changed shape here too -- this backend's
    /// `generate_composite_def` had no coverage of that change landing.
    #[test]
    fn test_composite_def_normalizes_consecutive_capitals() {
        let backend = JavaR2dbcBackend::new("postgresql").unwrap();
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
