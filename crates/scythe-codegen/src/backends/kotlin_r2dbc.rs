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

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/kotlin-r2dbc.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/kotlin-r2dbc.mysql.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/kotlin-r2dbc.sqlite.toml");
const DEFAULT_MANIFEST_MARIADB: &str = include_str!("../../manifests/kotlin-r2dbc.mariadb.toml");

pub struct KotlinR2dbcBackend {
    manifest: BackendManifest,
    is_pg: bool,
    extension_functions: bool,
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
                    ErrorCode::InvalidConfig,
                    format!("unsupported engine '{}' for kotlin-r2dbc backend", engine),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
        let is_pg = matches!(engine, "postgresql" | "postgres" | "pg");
        Ok(Self {
            manifest,
            is_pg,
            extension_functions: false,
        })
    }
}

/// The class literal to hand `Row.get(name, Class<T>)` for a column.
///
/// Derived from the declared type rather than looked up in a parallel table —
/// see `r2dbc_row_class` in `java_r2dbc.rs` for the full reasoning. The table
/// this replaced ended in `Any::class.java`, whose `Any!` result Kotlin
/// refuses to pass where a composite or an enum is expected, and matched
/// `LocalDateTime` against its `contains("LocalDate")` arm first.
///
/// Kotlin's primitive-backed types resolve to `::class.javaObjectType`: the
/// unqualified `Int::class.java` is `int.class`, the *primitive* `Class`
/// object, and no driver returns a value for it.
fn r2dbc_row_class(kotlin_type: &str) -> String {
    jvm_common::kotlin_boxed_class_literal(kotlin_type)
}

/// Splits a PostgreSQL composite's text form (`"(a,b,c)"`) into its raw field tokens. See
/// `java_jdbc.rs`'s twin constant for the full reasoning -- duplicated rather than shared
/// because the two backends have no other coupling.
const KT_PARSE_COMPOSITE_FIELDS_METHOD: &str = r#"        /**
         * ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
         * honoring its escaping rules: an empty unquoted field is SQL NULL (returned as `null`);
         * a field needing quoting (containing a comma, paren, quote, backslash, or
         * leading/trailing space, or the empty string) is wrapped in double quotes with `"` and
         * `\` backslash-escaped inside; every other field is unquoted and taken literally. A
         * nested composite's own "(x,y)" text form always contains parens, so it always comes
         * back quoted here, ready for that type's own `fromText` to parse recursively.
         */
        private fun parseCompositeFields(text: String): List<String?> {
            val fields = mutableListOf<String?>()
            val inner = text.substring(1, text.length - 1)
            var i = 0
            val n = inner.length
            while (true) {
                val field = StringBuilder()
                var isNull = false
                if (i < n && inner[i] == '"') {
                    i++
                    while (i < n) {
                        val c = inner[i]
                        if (c == '\\' && i + 1 < n) {
                            field.append(inner[i + 1])
                            i += 2
                        } else if (c == '"' && i + 1 < n && inner[i + 1] == '"') {
                            field.append('"')
                            i += 2
                        } else if (c == '"') {
                            i++
                            break
                        } else {
                            field.append(c)
                            i++
                        }
                    }
                } else {
                    val start = i
                    while (i < n && inner[i] != ',') {
                        i++
                    }
                    field.append(inner, start, i)
                    isNull = field.isEmpty()
                }
                fields.add(if (isNull) null else field.toString())
                if (i < n && inner[i] == ',') {
                    i++
                    continue
                }
                break
            }
            return fields
        }
"#;

/// PostgreSQL's default `bytea` text output is hex (`"\x48656c6c6f"`); decode the digits after
/// the `\x` prefix back into bytes. Emitted only when a composite has a `bytes` field.
const KT_PARSE_COMPOSITE_BYTES_METHOD: &str = r#"        /**
         * ~keep PostgreSQL's default `bytea` text output is hex: "\x48656c6c6f". Decode the hex
         * digits after the "\x" prefix back into bytes.
         */
        private fun parseCompositeBytes(hex: String): ByteArray {
            val digits = hex.substring(2)
            val result = ByteArray(digits.length / 2)
            for (i in result.indices) {
                result[i] = digits.substring(i * 2, i * 2 + 2).toInt(16).toByte()
            }
            return result
        }
"#;

/// PostgreSQL's default `timestamptz` text output uses a space instead of `T` and omits the
/// offset's minutes when they are zero; normalize both before handing the text to `java.time`.
/// Emitted only when a composite has a `datetime_tz` field.
const KT_PARSE_COMPOSITE_OFFSET_DATETIME_METHOD: &str = r#"        /**
         * ~keep PostgreSQL's default `timestamptz` text output uses a space instead of `T`
         * ("2024-01-15 10:30:00+00") and omits the offset's minutes when they are zero ("+00"
         * rather than "+00:00"). Normalize both before parsing.
         */
        private fun parseCompositeOffsetDateTime(raw: String): java.time.OffsetDateTime {
            var s = raw.replace(' ', 'T')
            val sign = s[s.length - 3]
            if (sign == '+' || sign == '-') {
                s += ":00"
            }
            return java.time.OffsetDateTime.parse(s)
        }
"#;

/// PostgreSQL's default `timetz` text output omits the offset's minutes when they are zero;
/// `OffsetTime.parse` rejects that. Emitted only when a composite has a `time_tz` field.
const KT_PARSE_COMPOSITE_OFFSET_TIME_METHOD: &str = r#"        /**
         * ~keep PostgreSQL's default `timetz` text output omits the offset's minutes when they
         * are zero ("13:22:43-05" rather than "13:22:43-05:00"), which `OffsetTime.parse` rejects.
         */
        private fun parseCompositeOffsetTime(raw: String): java.time.OffsetTime {
            var s = raw
            val sign = s[s.length - 3]
            if (sign == '+' || sign == '-') {
                s += ":00"
            }
            return java.time.OffsetTime.parse(s)
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

/// The Kotlin expression converting one composite field's raw text token (`raw`, a `String?`
/// already unescaped by `parseCompositeFields`) into the field's declared Kotlin type. See
/// `java_jdbc.rs`'s twin function for the reasoning behind the `!!` non-null assertions: a
/// composite field's declared type is always non-nullable (no per-field nullability is
/// tracked), a pre-existing gap this fix does not close.
fn composite_field_from_text_kotlin(
    neutral_type: &str,
    field_type: &str,
    raw: &str,
    manifest: &BackendManifest,
) -> String {
    if let Some(sql_name) = neutral_type.strip_prefix("composite::") {
        return format!(
            "{}.fromText({})!!",
            composite_type_name(sql_name, &manifest.naming),
            raw
        );
    }
    if neutral_type.starts_with("enum::") {
        return format!("{}.fromValue({}!!)", field_type, raw);
    }
    match neutral_type {
        "bool" => format!("{}!! == \"t\"", raw),
        "int16" => format!("{}!!.toShort()", raw),
        "int32" => format!("{}!!.toInt()", raw),
        "int64" => format!("{}!!.toLong()", raw),
        "float32" => format!("{}!!.toFloat()", raw),
        "float64" => format!("{}!!.toDouble()", raw),
        "decimal" => format!("java.math.BigDecimal({}!!)", raw),
        "uuid" => format!("java.util.UUID.fromString({}!!)", raw),
        "date" => format!("java.time.LocalDate.parse({}!!)", raw),
        "time" => format!("java.time.LocalTime.parse({}!!)", raw),
        "datetime" => format!("java.time.LocalDateTime.parse({}!!.replace(' ', 'T'))", raw),
        "datetime_tz" => format!("parseCompositeOffsetDateTime({}!!)", raw),
        "time_tz" => format!("parseCompositeOffsetTime({}!!)", raw),
        "bytes" => format!("parseCompositeBytes({}!!)", raw),
        // "string"/"json"/"inet"/"interval" all resolve to Kotlin `String`, so the already-parsed
        // text needs no further conversion beyond the non-null assertion. Any neutral type not
        // named above (e.g. an array-typed composite field, which this fix does not handle -- see
        // board #196's report) falls through here too.
        _ => format!("{}!!", raw),
    }
}

/// Build the `Row.get` expression for a column, handling arrays and enums
/// specially -- everywhere else defers to [`r2dbc_row_class`]. See
/// `java_r2dbc.rs`'s `r2dbc_col_read_expr` for the full reasoning behind both
/// cases.
///
/// Neither case null-guards: this file's existing `write_row_map` never did
/// either -- a platform-typed `row.get` result is passed straight through to
/// a declared type that may or may not be nullable, and this keeps the same
/// convention rather than introducing null-safety only for these two cases.
///
/// board #196: a composite column is read the same way -- an unregistered composite has no
/// r2dbc-postgresql codec either, so `row.get(col, T::class.java)` is just as
/// driver-codec-dependent as an enum's. The wire value is read as `String` and parsed by
/// `T.fromText` (emitted by `generate_composite_def`), which is already null-safe.
fn r2dbc_col_read_expr(col: &ResolvedColumn, manifest: &BackendManifest) -> String {
    if col.neutral_type.starts_with("composite::") {
        // ~keep `fromText` returns `T?` so a NULL column stays expressible; a NOT NULL column's
        // field is declared `T` and needs the unwrap or Kotlin rejects the argument. Same fix as
        // `kotlin_jdbc::kt_rs_expr`; Java has no compile-time nullability and so needs nothing.
        let unwrap = if col.nullable { "" } else { "!!" };
        return format!(
            "{}.fromText(row.get(\"{}\", String::class.java)){}",
            col.lang_type, col.name, unwrap
        );
    }
    if let Some(element) = jvm_common::array_element_neutral_type(&col.neutral_type) {
        let element_type = resolve_type(element, manifest, false)
            .map(|t| t.into_owned())
            .unwrap_or_else(|_| "Any".to_string());
        return format!(
            "row.get(\"{}\", Array<{}>::class.java).toList()",
            col.name, element_type
        );
    }
    if col.neutral_type.starts_with("enum::") {
        return format!(
            "{}.fromValue(row.get(\"{}\", String::class.java))",
            col.lang_type, col.name
        );
    }
    let class = r2dbc_row_class(&col.lang_type);
    format!("row.get(\"{}\", {})", col.name, class)
}

impl CodegenBackend for KotlinR2dbcBackend {
    fn name(&self) -> &str {
        "kotlin-r2dbc"
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
                            "kotlin-r2dbc: invalid value '{other}' for extension_functions (expected 'true' or 'false')"
                        ),
                    ));
                }
            };
        }
        if let Some(value) = options.get("field_case") {
            super::apply_field_case_option(&mut self.manifest.naming, "kotlin-r2dbc", value)?;
        }
        Ok(())
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "mysql", "mariadb", "sqlite"]
    }

    fn file_header(&self) -> String {
        if self.extension_functions {
            "import io.r2dbc.spi.Connection\n\
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
        } else {
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
        let cleaned =
            super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let sql = if self.is_pg {
            cleaned
        } else {
            super::rewrite_pg_placeholders(&cleaned, |_| "?".to_string())
        };
        let sql = crate::sql_literal::escape_kotlin_string(&sql);

        let use_multiline_params = !params.is_empty();
        let ext = self.extension_functions;
        let manifest = &self.manifest;

        let mut out = String::new();

        let write_binds = |out: &mut String, indent: &str| {
            for (i, param) in params.iter().enumerate() {
                let _ = writeln!(out, "{}.bind({}, {})", indent, i, param.field_name);
            }
        };

        let write_row_map = |out: &mut String, indent: &str| {
            let _ = writeln!(out, "{}{}(", indent, struct_name);
            for col in columns.iter() {
                let expr = r2dbc_col_read_expr(col, manifest);
                let _ = writeln!(out, "{}    {} = {},", indent, col.field_name, expr);
            }
            let _ = write!(out, "{})", indent);
        };

        let write_suspend_fn_sig =
            |out: &mut String, name: &str, ret: &str, multiline: bool, params: &[ResolvedParam]| {
                if ext {
                    if multiline {
                        let _ = writeln!(out, "suspend fun Connection.{}(", name);
                        for p in params {
                            let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                        }
                        let _ = writeln!(out, "){} {{", ret);
                    } else {
                        let _ = writeln!(out, "suspend fun Connection.{}(){} {{", name, ret);
                    }
                } else if multiline {
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
                if ext {
                    let _ = writeln!(out, "    val stmt = createStatement(\"{sql}\")");
                    write_binds(&mut out, "    stmt");
                    let _ = writeln!(
                        out,
                        "    Mono.from(stmt.execute()).flatMap {{ result -> Mono.from(result.rowsUpdated) }}.awaitFirstOrNull()"
                    );
                    let _ = writeln!(out, "}}");
                } else {
                    let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(out, "        val stmt = conn.createStatement(\"{sql}\")");
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
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_suspend_fn_sig(&mut out, &func_name, ": Long", use_multiline_params, params);
                if ext {
                    let _ = writeln!(out, "    val stmt = createStatement(\"{sql}\")");
                    write_binds(&mut out, "    stmt");
                    let _ = writeln!(out, "    return Mono");
                    let _ = writeln!(out, "        .from(stmt.execute())");
                    let _ = writeln!(out, "        .flatMap {{ result -> Mono.from(result.rowsUpdated) }}");
                    let _ = writeln!(out, "        .awaitFirst()");
                    let _ = writeln!(out, "}}");
                } else {
                    let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(out, "        val stmt = conn.createStatement(\"{sql}\")");
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
            }
            QueryCommand::One | QueryCommand::Opt => {
                // ~keep #192: see java-r2dbc's generate_query_fn for the full
                // reasoning -- a missing row in a reactive chain is an error
                // signal on the publisher, not a thrown exception at call
                // time. `:opt` keeps `awaitFirstOrNull()` (already correct:
                // null on an empty Mono, without an exception). `:one` chains
                // `.switchIfEmpty(Mono.error(...))` onto the row-producing
                // Mono before collecting it, then collects with
                // `awaitFirst()` (no new import: both are already imported)
                // rather than `awaitFirstOrNull()`, so the coroutine
                // propagates the error instead of returning a null the
                // now-non-nullable return type could not express.
                let is_one = matches!(analyzed.command, QueryCommand::One);
                let ret = if is_one {
                    format!(": {}", struct_name)
                } else {
                    format!(": {}?", struct_name)
                };
                write_suspend_fn_sig(&mut out, &func_name, &ret, use_multiline_params, params);
                let switch_if_empty = format!(
                    "Mono.error(java.util.NoSuchElementException(\"{}: no rows returned\"))",
                    func_name
                );
                if ext {
                    let _ = writeln!(out, "    val stmt = createStatement(\"{sql}\")");
                    write_binds(&mut out, "    stmt");
                    let _ = writeln!(out, "    return Mono");
                    let _ = writeln!(out, "        .from(stmt.execute())");
                    let _ = writeln!(out, "        .flatMap {{ result ->");
                    let _ = writeln!(out, "            Mono.from(");
                    let _ = writeln!(out, "                result.map {{ row, _ ->");
                    write_row_map(&mut out, "                    ");
                    let _ = writeln!(out);
                    let _ = writeln!(out, "                }},");
                    let _ = writeln!(out, "            )");
                    if is_one {
                        let _ = writeln!(out, "        }}");
                        let _ = writeln!(out, "        .switchIfEmpty({switch_if_empty})");
                        let _ = writeln!(out, "        .awaitFirst()");
                    } else {
                        let _ = writeln!(out, "        }}.awaitFirstOrNull()");
                    }
                    let _ = writeln!(out, "}}");
                } else {
                    let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(out, "        val stmt = conn.createStatement(\"{sql}\")");
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
                    if is_one {
                        let _ = writeln!(out, "            }}");
                        let _ = writeln!(out, "            .switchIfEmpty({switch_if_empty})");
                        let _ = writeln!(out, "            .awaitFirst()");
                    } else {
                        let _ = writeln!(out, "            }}.awaitFirstOrNull()");
                    }
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let ret = format!(": Flow<{}>", struct_name);
                if ext {
                    if use_multiline_params {
                        let _ = writeln!(out, "fun Connection.{}(", func_name);
                        for p in params {
                            let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                        }
                        let _ = writeln!(out, "){ret} =");
                    } else {
                        let _ = writeln!(out, "fun Connection.{}(){ret} =", func_name);
                    }
                    let _ = writeln!(out, "    Flux");
                    let _ = writeln!(out, "        .from(createStatement(\"{sql}\").also {{ stmt ->");
                    write_binds(&mut out, "            stmt");
                    let _ = writeln!(out, "        }}.execute())");
                    let _ = writeln!(out, "        .flatMap {{ result ->");
                    let _ = writeln!(out, "            result.map {{ row, _ ->");
                    write_row_map(&mut out, "                ");
                    let _ = writeln!(out);
                    let _ = writeln!(out, "            }}");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        .asFlow()");
                } else {
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
                    let _ = writeln!(out, "                val stmt = conn.createStatement(\"{sql}\")");
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
                        let _ = writeln!(out, "suspend fun Connection.{}(", batch_fn_name);
                        let _ = writeln!(out, "    items: List<{}>,", params_class_name);
                    } else {
                        let _ = writeln!(out, "suspend fun {}(", batch_fn_name);
                        let _ = writeln!(out, "    cf: ConnectionFactory,");
                        let _ = writeln!(out, "    items: List<{}>,", params_class_name);
                    }
                    let _ = writeln!(out, ") {{");
                    if ext {
                        let _ = writeln!(out, "    Mono.from(beginTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "    val stmt = createStatement(\"{sql}\")");
                        let _ = writeln!(out, "    var first = true");
                        let _ = writeln!(out, "    for (item in items) {{");
                        let _ = writeln!(out, "        if (!first) stmt.add()");
                        for (i, param) in params.iter().enumerate() {
                            let _ = writeln!(out, "        stmt.bind({}, item.{})", i, param.field_name);
                        }
                        let _ = writeln!(out, "        first = false");
                        let _ = writeln!(out, "    }}");
                        let _ = writeln!(out, "    Flux.from(stmt.execute()).then().awaitFirstOrNull()");
                        let _ = writeln!(out, "    Mono.from(commitTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "}}");
                    } else {
                        let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                        let _ = writeln!(out, "    try {{");
                        let _ = writeln!(out, "        Mono.from(conn.beginTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "        val stmt = conn.createStatement(\"{sql}\")");
                        let _ = writeln!(out, "        var first = true");
                        let _ = writeln!(out, "        for (item in items) {{");
                        let _ = writeln!(out, "            if (!first) stmt.add()");
                        for (i, param) in params.iter().enumerate() {
                            let _ = writeln!(out, "            stmt.bind({}, item.{})", i, param.field_name);
                        }
                        let _ = writeln!(out, "            first = false");
                        let _ = writeln!(out, "        }}");
                        let _ = writeln!(out, "        Flux.from(stmt.execute()).then().awaitFirstOrNull()");
                        let _ = writeln!(out, "        Mono.from(conn.commitTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "    }} catch (e: Exception) {{");
                        let _ = writeln!(out, "        Mono.from(conn.rollbackTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "        throw e");
                        let _ = writeln!(out, "    }} finally {{");
                        let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
                        let _ = writeln!(out, "    }}");
                        let _ = writeln!(out, "}}");
                    }
                } else if params.len() == 1 {
                    if ext {
                        let _ = writeln!(out, "suspend fun Connection.{}(", batch_fn_name);
                        let _ = writeln!(out, "    items: List<{}>,", params[0].full_type);
                    } else {
                        let _ = writeln!(out, "suspend fun {}(", batch_fn_name);
                        let _ = writeln!(out, "    cf: ConnectionFactory,");
                        let _ = writeln!(out, "    items: List<{}>,", params[0].full_type);
                    }
                    let _ = writeln!(out, ") {{");
                    if ext {
                        let _ = writeln!(out, "    Mono.from(beginTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "    val stmt = createStatement(\"{sql}\")");
                        let _ = writeln!(out, "    var first = true");
                        let _ = writeln!(out, "    for (item in items) {{");
                        let _ = writeln!(out, "        if (!first) stmt.add()");
                        let _ = writeln!(out, "        stmt.bind(0, item)");
                        let _ = writeln!(out, "        first = false");
                        let _ = writeln!(out, "    }}");
                        let _ = writeln!(out, "    Flux.from(stmt.execute()).then().awaitFirstOrNull()");
                        let _ = writeln!(out, "    Mono.from(commitTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "}}");
                    } else {
                        let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                        let _ = writeln!(out, "    try {{");
                        let _ = writeln!(out, "        Mono.from(conn.beginTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "        val stmt = conn.createStatement(\"{sql}\")");
                        let _ = writeln!(out, "        var first = true");
                        let _ = writeln!(out, "        for (item in items) {{");
                        let _ = writeln!(out, "            if (!first) stmt.add()");
                        let _ = writeln!(out, "            stmt.bind(0, item)");
                        let _ = writeln!(out, "            first = false");
                        let _ = writeln!(out, "        }}");
                        let _ = writeln!(out, "        Flux.from(stmt.execute()).then().awaitFirstOrNull()");
                        let _ = writeln!(out, "        Mono.from(conn.commitTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "    }} catch (e: Exception) {{");
                        let _ = writeln!(out, "        Mono.from(conn.rollbackTransaction()).awaitFirstOrNull()");
                        let _ = writeln!(out, "        throw e");
                        let _ = writeln!(out, "    }} finally {{");
                        let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
                        let _ = writeln!(out, "    }}");
                        let _ = writeln!(out, "}}");
                    }
                } else if ext {
                    let _ = writeln!(out, "suspend fun Connection.{}(count: Int) {{", batch_fn_name);
                    let _ = writeln!(out, "    Mono.from(beginTransaction()).awaitFirstOrNull()");
                    let _ = writeln!(out, "    val stmt = createStatement(\"{sql}\")");
                    let _ = writeln!(out, "    repeat(count - 1) {{");
                    let _ = writeln!(out, "        stmt.add()");
                    let _ = writeln!(out, "    }}");
                    let _ = writeln!(out, "    Flux.from(stmt.execute()).then().awaitFirstOrNull()");
                    let _ = writeln!(out, "    Mono.from(commitTransaction()).awaitFirstOrNull()");
                    let _ = writeln!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "suspend fun {}(cf: ConnectionFactory, count: Int) {{",
                        batch_fn_name
                    );
                    let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
                    let _ = writeln!(out, "    try {{");
                    let _ = writeln!(out, "        Mono.from(conn.beginTransaction()).awaitFirstOrNull()");
                    let _ = writeln!(out, "        val stmt = conn.createStatement(\"{sql}\")");
                    let _ = writeln!(out, "        repeat(count - 1) {{");
                    let _ = writeln!(out, "            stmt.add()");
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "        Flux.from(stmt.execute()).then().awaitFirstOrNull()");
                    let _ = writeln!(out, "        Mono.from(conn.commitTransaction()).awaitFirstOrNull()");
                    let _ = writeln!(out, "    }} catch (e: Exception) {{");
                    let _ = writeln!(out, "        Mono.from(conn.rollbackTransaction()).awaitFirstOrNull()");
                    let _ = writeln!(out, "        throw e");
                    let _ = writeln!(out, "    }} finally {{");
                    let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
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
        let _ = writeln!(out);
        // ~keep #213: see `java_jdbc.rs`'s `generate_enum_def` for the full
        // reasoning -- decoding against the declared `value` rather than the
        // sanitised variant spelling is what makes `fromValue(x.value) == x`
        // hold for every variant.
        let _ = writeln!(out, "    companion object {{");
        let _ = writeln!(out, "        fun fromValue(value: String): {} =", type_name);
        let _ = writeln!(out, "            values().firstOrNull {{ it.value == value }}");
        let _ = writeln!(
            out,
            "                ?: throw IllegalArgumentException(\"Unknown {} value: $value\")",
            type_name
        );
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let field_types: Vec<String> = composite
            .fields
            .iter()
            .map(|f| {
                resolve_type(&f.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .unwrap_or_else(|_| "Any".to_string())
            })
            .collect();
        let _ = writeln!(out, "data class {}(", name);
        for (field, field_type) in composite.fields.iter().zip(&field_types) {
            let field_name = to_camel_case(&field.name);
            let _ = writeln!(out, "    val {}: {},", field_name, field_type);
        }
        // ~keep board #196: a zero-field composite cannot exist in PostgreSQL (`CREATE TYPE ...
        // AS ()` is rejected), so -- like `kotlin_jdbc.rs`'s twin case -- there is no reachable
        // runtime value that would need a `fromText` here.
        if composite.fields.is_empty() {
            let _ = writeln!(out, ")");
            return Ok(out);
        }
        let _ = writeln!(out, ") {{");
        let _ = writeln!(out, "    companion object {{");
        let _ = writeln!(out, "        /**");
        let _ = writeln!(
            out,
            "         * ~keep board #196: r2dbc-postgresql has no codec for this composite -- an"
        );
        let _ = writeln!(
            out,
            "         * unregistered `row.get(col, {}::class.java)` is driver-codec-dependent and",
            name
        );
        let _ = writeln!(
            out,
            "         * throws at runtime. Parse the driver's composite text form instead."
        );
        let _ = writeln!(out, "         */");
        let _ = writeln!(out, "        fun fromText(text: String?): {}? {{", name);
        let _ = writeln!(out, "            if (text == null) {{");
        let _ = writeln!(out, "                return null");
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "            val f = parseCompositeFields(text)");
        let _ = writeln!(out, "            return {}(", name);
        for (i, (field, field_type)) in composite.fields.iter().zip(&field_types).enumerate() {
            let raw = format!("f[{}]", i);
            let expr = composite_field_from_text_kotlin(&field.neutral_type, field_type, &raw, &self.manifest);
            let _ = writeln!(out, "                {},", expr);
        }
        let _ = writeln!(out, "            )");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out);
        out.push_str(KT_PARSE_COMPOSITE_FIELDS_METHOD);
        if composite_needs_bytes_helper(composite) {
            let _ = writeln!(out);
            out.push_str(KT_PARSE_COMPOSITE_BYTES_METHOD);
        }
        if composite_needs_offset_datetime_helper(composite) {
            let _ = writeln!(out);
            out.push_str(KT_PARSE_COMPOSITE_OFFSET_DATETIME_METHOD);
        }
        if composite_needs_offset_time_helper(composite) {
            let _ = writeln!(out);
            out.push_str(KT_PARSE_COMPOSITE_OFFSET_TIME_METHOD);
        }
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "}}");
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
        let all_columns = request.all_columns;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let cleaned =
            super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let sql = if self.is_pg {
            cleaned
        } else {
            super::rewrite_pg_placeholders(&cleaned, |_| "?".to_string())
        };
        let sql = crate::sql_literal::escape_kotlin_string(&sql);

        let ext = self.extension_functions;
        let use_multiline_params = !params.is_empty();
        let manifest = &self.manifest;

        let key_col = parent_columns
            .iter()
            .find(|c| c.name == key_column)
            .unwrap_or(&parent_columns[0]);
        let key_type = key_col.full_type.trim_end_matches('?');
        let key_ordinal = all_columns.iter().position(|c| c.name == key_column).unwrap_or(0);

        let mut out = String::new();
        let ret = format!(": List<{parent_struct_name}>");

        if ext {
            if use_multiline_params {
                let _ = writeln!(out, "suspend fun Connection.{}(", func_name);
                for p in params {
                    let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
                }
                let _ = writeln!(out, "){ret} {{");
            } else {
                let _ = writeln!(out, "suspend fun Connection.{}(){ret} {{", func_name);
            }
            let _ = writeln!(out, "    val stmt = createStatement(\"{sql}\")");
            for (i, param) in params.iter().enumerate() {
                let _ = writeln!(out, "    stmt.bind({i}, {})", param.field_name);
            }
        } else if use_multiline_params {
            let _ = writeln!(out, "suspend fun {}(", func_name);
            let _ = writeln!(out, "    cf: ConnectionFactory,");
            for p in params {
                let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "){ret} {{");
            let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
            let _ = writeln!(out, "    try {{");
            let _ = writeln!(out, "        val stmt = conn.createStatement(\"{sql}\")");
            for (i, param) in params.iter().enumerate() {
                let _ = writeln!(out, "        stmt.bind({i}, {})", param.field_name);
            }
        } else {
            let _ = writeln!(out, "suspend fun {}(cf: ConnectionFactory){ret} {{", func_name);
            let _ = writeln!(out, "    val conn = Mono.from(cf.create()).awaitFirst()");
            let _ = writeln!(out, "    try {{");
            let _ = writeln!(out, "        val stmt = conn.createStatement(\"{sql}\")");
        }

        let stmt_indent = if ext { "    " } else { "        " };

        let _ = writeln!(out, "{stmt_indent}val rawRows = Flux.from(stmt.execute())");
        let _ = writeln!(out, "{stmt_indent}    .flatMap {{ result ->");
        let _ = writeln!(out, "{stmt_indent}        result.map {{ row, _ ->");
        let _ = writeln!(out, "{stmt_indent}            arrayOf(");
        for col in all_columns {
            let expr = r2dbc_col_read_expr(col, manifest);
            let _ = writeln!(out, "{stmt_indent}                {},", expr);
        }
        let _ = writeln!(out, "{stmt_indent}            )");
        let _ = writeln!(out, "{stmt_indent}        }}");
        let _ = writeln!(out, "{stmt_indent}    }}");
        let _ = writeln!(out, "{stmt_indent}    .asFlow().toList()");

        let _ = writeln!(
            out,
            "{stmt_indent}val lookup = LinkedHashMap<{key_type}, {parent_struct_name}>()"
        );
        let _ = writeln!(out, "{stmt_indent}val result = mutableListOf<{parent_struct_name}>()");
        let _ = writeln!(out, "{stmt_indent}for (raw in rawRows) {{");
        let _ = writeln!(out, "{stmt_indent}    val key = raw[{key_ordinal}] as {key_type}");

        let _ = writeln!(out, "{stmt_indent}    val child = {child_struct_name}(");
        for (ci, col) in child_columns.iter().enumerate() {
            let ordinal = all_columns
                .iter()
                .position(|c| c.name == col.name)
                .unwrap_or(parent_columns.len() + ci);
            let cast = &col.full_type;
            let _ = writeln!(
                out,
                "{stmt_indent}        {} = raw[{ordinal}] as {cast},",
                col.field_name
            );
        }
        let _ = writeln!(out, "{stmt_indent}    )");

        let _ = writeln!(out, "{stmt_indent}    if (lookup.containsKey(key)) {{");
        let _ = writeln!(out, "{stmt_indent}        lookup[key]!!.children.add(child)");
        let _ = writeln!(out, "{stmt_indent}    }} else {{");
        let _ = writeln!(out, "{stmt_indent}        val parent = {parent_struct_name}(");
        for col in parent_columns {
            let ordinal = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
            let cast = &col.full_type;
            let _ = writeln!(
                out,
                "{stmt_indent}            {} = raw[{ordinal}] as {cast},",
                col.field_name
            );
        }
        let _ = writeln!(out, "{stmt_indent}            children = mutableListOf(child),");
        let _ = writeln!(out, "{stmt_indent}        )");
        let _ = writeln!(out, "{stmt_indent}        lookup[key] = parent");
        let _ = writeln!(out, "{stmt_indent}        result.add(parent)");
        let _ = writeln!(out, "{stmt_indent}    }}");
        let _ = writeln!(out, "{stmt_indent}}}");
        let _ = writeln!(out, "{stmt_indent}result");

        if ext {
            let _ = write!(out, "}}");
        } else {
            let _ = writeln!(out, "    }} finally {{");
            let _ = writeln!(out, "        Mono.from(conn.close()).awaitFirstOrNull()");
            let _ = writeln!(out, "    }}");
            let _ = write!(out, "}}");
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    use super::KotlinR2dbcBackend;
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
    fn test_grouped_kotlin_r2dbc_structs() {
        let backend = crate::backends::get_backend("kotlin-r2dbc", "postgresql").unwrap();
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
    fn test_grouped_kotlin_r2dbc_query_fn() {
        let backend = crate::backends::get_backend("kotlin-r2dbc", "postgresql").unwrap();
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
            query_fn.contains("asFlow().toList()"),
            "must collect rows with asFlow().toList(); got:\n{query_fn}"
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
    /// column name -- and only the data class' declared property
    /// (`col.field_name`) changes under `field_case = "camelCase"`.
    #[test]
    fn test_field_case_camel_case_renames_field_but_keeps_raw_lookup_key() {
        let mut backend = KotlinR2dbcBackend::new("postgresql").unwrap();
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
            query_fn.contains("userId = row.get(\"user_id\", Int::class.javaObjectType)"),
            "the Row lookup key must stay the raw SQL column name; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("row.get(\"userId\""),
            "must never look the driver up by the renamed field; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_field_case_option_rejects_invalid_value() {
        let mut backend = KotlinR2dbcBackend::new("postgresql").unwrap();
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
        let backend = KotlinR2dbcBackend::new("postgresql").unwrap();
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
