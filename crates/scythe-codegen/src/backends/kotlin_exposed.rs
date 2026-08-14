use std::collections::HashMap;
use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, to_camel_case, to_pascal_case};
use scythe_backend::types::resolve_type;

use scythe_core::SqlDialect;
use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};
use crate::backends::jvm_common;

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
                    ErrorCode::InvalidConfig,
                    format!("unsupported engine '{}' for kotlin-exposed backend", engine),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
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
        "String" => "text",
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

/// The Kotlin element type an array column's `List<{T}>` reader casts each
/// element to. See `kotlin_jdbc.rs`'s `element_kotlin_type` -- identical
/// reasoning, duplicated rather than shared because it is a five-line wrapper
/// and the two backends have no other coupling.
fn element_kotlin_type(element_neutral: &str, manifest: &BackendManifest) -> String {
    resolve_type(element_neutral, manifest, false)
        .map(|t| t.into_owned())
        .unwrap_or_else(|_| "Any".to_string())
}

/// The Kotlin `List<T>` expression that turns a `java.sql.Array` (already
/// known non-null) into the declared element type. See `kotlin_jdbc.rs`'s
/// `array_list_expr` for the full reasoning.
fn array_list_expr(sql_array_expr: &str, element_type: &str) -> String {
    format!("({sql_array_expr}.array as Array<*>).map {{ it as {element_type} }}")
}

/// The inline read expression for a kotlin-exposed column.
///
/// Exposed's `exec(sql) { rs -> ... }` block hands out a plain
/// `java.sql.ResultSet`, so this is the same problem `kotlin-jdbc` has and the
/// same answer: a named getter where JDBC has one, `rs.getObject(col,
/// T::class.java)` where it does not, and never the untyped
/// `rs.getObject(col)` whose `Any!` result is assignable to nothing.
///
/// Nullable columns return the preamble local written by
/// [`write_exposed_nullable_preamble`] instead of an inline read. This backend
/// had no preamble at all: a nullable `Int?` column was read with
/// `rs.getInt(col)`, which JDBC specifies to return `0` — not null — for SQL
/// NULL, so every NULL in a nullable numeric or boolean column silently became
/// `0`/`false` in the row object.
fn exposed_rs_expr(col: &ResolvedColumn, manifest: &BackendManifest) -> String {
    if col.nullable {
        return col.field_name.clone();
    }
    if let Some(element) = jvm_common::array_element_neutral_type(&col.neutral_type) {
        let element_type = element_kotlin_type(element, manifest);
        return array_list_expr(&format!("rs.getArray(\"{}\")", col.name), &element_type);
    }
    if jvm_common::is_enum_column(&col.neutral_type) {
        return format!("{}.fromValue(rs.getString(\"{}\"))", col.lang_type, col.name);
    }
    jvm_common::kotlin_jdbc_read_call(&col.name, &col.lang_type)
}

/// Emit the nullable-column preamble locals a following row construction reads
/// through [`exposed_rs_expr`].
fn write_exposed_nullable_preamble(
    out: &mut String,
    cols: &[ResolvedColumn],
    indent: &str,
    manifest: &BackendManifest,
) {
    for col in cols {
        if !col.nullable {
            continue;
        }
        if let Some(element) = jvm_common::array_element_neutral_type(&col.neutral_type) {
            // ~keep A nullable array column's `rs.getArray(col)` returns
            // `null` for SQL NULL; calling `.array` on that null throws
            // `NullPointerException`, so the guard is on the `java.sql.Array`
            // local, before the element conversion runs.
            let element_type = element_kotlin_type(element, manifest);
            let _ = writeln!(
                out,
                "{}val {}SqlArray = rs.getArray(\"{}\")",
                indent, col.field_name, col.name
            );
            let list_expr = array_list_expr(&format!("{}SqlArray", col.field_name), &element_type);
            let _ = writeln!(
                out,
                "{}val {} = if ({}SqlArray == null) null else {}",
                indent, col.field_name, col.field_name, list_expr
            );
            continue;
        }
        if jvm_common::is_enum_column(&col.neutral_type) {
            // ~keep `getString` returns null for SQL NULL, so the guard is on
            // the raw string: `fromValue(rs.getString(col))` would throw on
            // exactly the value a nullable enum column exists to hold.
            let _ = writeln!(
                out,
                "{}val {}Value = rs.getString(\"{}\")",
                indent, col.field_name, col.name
            );
            let _ = writeln!(
                out,
                "{}val {} = if ({}Value == null) null else {}.fromValue({}Value)",
                indent, col.field_name, col.field_name, col.lang_type, col.field_name
            );
            continue;
        }
        let _ = writeln!(
            out,
            "{}val {}Value = {}",
            indent,
            col.field_name,
            jvm_common::kotlin_jdbc_read_call(&col.name, &col.lang_type)
        );
        let _ = writeln!(
            out,
            "{}val {} = if (rs.wasNull()) null else {}Value",
            indent, col.field_name, col.field_name
        );
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
        "String" => "TextColumnType()",
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

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn apply_options(&mut self, options: &HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(&["field_case"], options)?;

        if let Some(value) = options.get("field_case") {
            super::apply_field_case_option(&mut self.manifest.naming, "kotlin-exposed", value)?;
        }
        Ok(())
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql"]
    }

    fn file_header(&self) -> String {
        "import org.jetbrains.exposed.dao.id.IntIdTable\n\
         import org.jetbrains.exposed.dao.id.LongIdTable\n\
         import org.jetbrains.exposed.dao.id.UUIDTable\n\
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
         import org.jetbrains.exposed.sql.javatime.JavaLocalDateColumnType\n\
         import org.jetbrains.exposed.sql.javatime.JavaLocalDateTimeColumnType\n\
         import org.jetbrains.exposed.sql.javatime.JavaLocalTimeColumnType\n\
         import org.jetbrains.exposed.sql.javatime.JavaOffsetDateTimeColumnType\n\
         import org.jetbrains.exposed.sql.transactions.transaction\n"
            .to_string()
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

    /// #214: this used to declare only `object EvTable : IntIdTable("ev")`
    /// and no row type at all, while the query function `determine_struct_name`
    /// pairs it with constructs `Ev(...)` -- a type declared nowhere. Exposed's
    /// idiom genuinely is a table object (there is no idiomatic way to drop
    /// it), so the fix emits *both*: the table object, for callers who want to
    /// build Exposed DSL queries against it, and the `data class` the query
    /// function already references.
    ///
    /// `struct_name` is derived through [`crate::model_struct_name`] -- the
    /// same function the base trait's default `generate_model_struct` and
    /// `determine_struct_name` both use -- not `to_pascal_case(table_name)`,
    /// whose un-singularized spelling (`"events"` -> `Events`, not `Event`) is
    /// exactly the #164 divergence this repo already fixed for every other
    /// backend.
    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = crate::model_struct_name(table_name, &self.manifest.naming);
        let table_obj_name = format!("{}Table", struct_name);
        let mut out = String::new();
        let table_base = exposed_id_table_type(columns);
        let _ = writeln!(out, "object {} : {}(\"{}\") {{", table_obj_name, table_base, table_name);
        for col in columns.iter() {
            let col_fn = exposed_column_fn(&col.lang_type);
            let nullable_suffix = if col.nullable { ".nullable()" } else { "" };
            let _ = writeln!(
                out,
                "    val {} = {}(\"{}\"){}",
                col.field_name, col_fn, col.name, nullable_suffix
            );
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        let row_decl = self.generate_struct_decl(&struct_name, &struct_name, columns)?;
        out.push_str(&row_decl);
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
        let dialect = SqlDialect::PostgreSQL;
        let cleaned_sql = super::clean_sql_oneline_with_optional_dialect(
            &analyzed.sql,
            dialect,
            &analyzed.optional_params,
            &analyzed.params,
        );
        let (rewritten_sql, occurrences) =
            super::rewrite_placeholders_indexed(&cleaned_sql, dialect, |_| "?".to_string());
        let sql = crate::sql_literal::escape_kotlin_string(&rewritten_sql);

        let mut out = String::new();

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

        let build_args = |occurrences: &[u32]| -> String {
            if occurrences.is_empty() {
                return String::new();
            }
            let pairs: Vec<String> = occurrences
                .iter()
                .map(|&position| {
                    let p = super::resolved_param_for_position(&analyzed.params, params, position);
                    format!("{} to {}", exposed_column_type_class(&p.lang_type), p.field_name)
                })
                .collect();
            format!(", listOf({})", pairs.join(", "))
        };

        match &analyzed.command {
            QueryCommand::Exec => {
                write_fn_sig(&mut out, &func_name, "", params);
                let args = build_args(&occurrences);
                let _ = writeln!(out, "        exec(\"{}\"{})", sql, args);
                let _ = writeln!(out, "    }}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_fn_sig(&mut out, &func_name, ": Int", params);
                let args = build_args(&occurrences);
                let _ = writeln!(out, "        exec(\"{}\"{}) ?: 0", sql, args);
                let _ = writeln!(out, "    }}");
            }
            QueryCommand::One | QueryCommand::Opt => {
                // ~keep #192: see java-jdbc's generate_query_fn for the full
                // reasoning -- this shared arm used to render byte-identical
                // code for :one and :opt, so :one silently returned null on a
                // missing row instead of erroring. `is_one` is the only
                // difference from here down. The `rs.next()` if/else itself
                // -- including its `null` branch -- stays identical between
                // :one and :opt: `Transaction.exec(sql) { rs -> T }` returns
                // `T?` regardless of what the lambda's branches do, so a
                // throw *inside* the lambda would not change the static
                // nullability of the `exec(...)` call it's nested in, and
                // `: {Struct}` (no `?`) would not type-check against it. The
                // fallback instead sits on the *outer* call, as `exec(...) {
                // ... } ?: throw NoSuchElementException(...)` -- turning the
                // nullable result non-null the same way it would for a
                // driver-level absent-ResultSet null, not only the
                // rs.next()-false case (`kotlin.NoSuchElementException` is in
                // Kotlin's default imports, so no import is needed).
                let is_one = matches!(analyzed.command, QueryCommand::One);
                let ret = if is_one {
                    format!(": {}", struct_name)
                } else {
                    format!(": {}?", struct_name)
                };
                write_fn_sig(&mut out, &func_name, &ret, params);
                let args = build_args(&occurrences);
                let _ = writeln!(out, "        exec(\"{}\"{}) {{ rs ->", sql, args);
                let _ = writeln!(out, "            if (rs.next()) {{");
                write_exposed_nullable_preamble(&mut out, columns, "                ", &self.manifest);
                let _ = writeln!(out, "                {}(", struct_name);
                for col in columns.iter() {
                    let _ = writeln!(
                        out,
                        "                    {} = {},",
                        col.field_name,
                        exposed_rs_expr(col, &self.manifest)
                    );
                }
                let _ = writeln!(out, "                )");
                let _ = writeln!(out, "            }} else {{");
                let _ = writeln!(out, "                null");
                let _ = writeln!(out, "            }}");
                if is_one {
                    let _ = writeln!(
                        out,
                        "        }} ?: throw NoSuchElementException(\"{}: no rows returned\")",
                        func_name
                    );
                } else {
                    let _ = writeln!(out, "        }}");
                }
                let _ = writeln!(out, "    }}");
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
                    let _ = writeln!(out, "fun {}(items: List<{}>) =", batch_fn_name, params_class_name);
                    let _ = writeln!(out, "    transaction {{");
                    let _ = writeln!(out, "        for (item in items) {{");
                    let args: Vec<String> = occurrences
                        .iter()
                        .map(|&position| {
                            let p = super::resolved_param_for_position(&analyzed.params, params, position);
                            format!("{} to item.{}", exposed_column_type_class(&p.lang_type), p.field_name)
                        })
                        .collect();
                    let _ = writeln!(out, "            exec(\"{}\", listOf({}))", sql, args.join(", "));
                    let _ = writeln!(out, "        }}");
                    let _ = writeln!(out, "    }}");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "fun {}(items: List<{}>) =", batch_fn_name, params[0].full_type);
                    let _ = writeln!(out, "    transaction {{");
                    let _ = writeln!(out, "        for (item in items) {{");
                    let args: Vec<String> = occurrences
                        .iter()
                        .map(|_| format!("{} to item", exposed_column_type_class(&params[0].lang_type)))
                        .collect();
                    let _ = writeln!(out, "            exec(\"{}\", listOf({}))", sql, args.join(", "));
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
                unreachable!("routed to generate_grouped_query_fn")
            }
            QueryCommand::Many => {
                let ret = format!(": List<{}>", struct_name);
                write_fn_sig(&mut out, &func_name, &ret, params);
                let args = build_args(&occurrences);
                let _ = writeln!(out, "        val result = mutableListOf<{}>()", struct_name);
                let _ = writeln!(out, "        exec(\"{}\"{}) {{ rs ->", sql, args);
                let _ = writeln!(out, "            while (rs.next()) {{");
                write_exposed_nullable_preamble(&mut out, columns, "                ", &self.manifest);
                let _ = writeln!(out, "                result.add(");
                let _ = writeln!(out, "                    {}(", struct_name);
                for col in columns.iter() {
                    let _ = writeln!(
                        out,
                        "                        {} = {},",
                        col.field_name,
                        exposed_rs_expr(col, &self.manifest)
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
        let dialect = SqlDialect::PostgreSQL;
        let cleaned_sql = super::clean_sql_oneline_with_optional_dialect(
            &analyzed.sql,
            dialect,
            &analyzed.optional_params,
            &analyzed.params,
        );
        let (rewritten_sql, occurrences) =
            super::rewrite_placeholders_indexed(&cleaned_sql, dialect, |_| "?".to_string());
        let sql = crate::sql_literal::escape_kotlin_string(&rewritten_sql);

        let key_col = parent_columns
            .iter()
            .find(|c| c.name == key_column)
            .unwrap_or(&parent_columns[0]);
        let key_type = key_col.full_type.trim_end_matches('?');

        let args = if occurrences.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = occurrences
                .iter()
                .map(|&position| {
                    let p = super::resolved_param_for_position(&analyzed.params, params, position);
                    format!("{} to {}", exposed_column_type_class(&p.lang_type), p.field_name)
                })
                .collect();
            format!(", listOf({})", pairs.join(", "))
        };

        let inline_params: String = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sig = format!("fun {}({}): List<{parent_struct_name}> =", func_name, inline_params);
        let mut out = String::new();
        if sig.len() <= 100 {
            let _ = writeln!(out, "{sig}");
        } else {
            let _ = writeln!(out, "fun {}(", func_name);
            for p in params {
                let _ = writeln!(out, "    {}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): List<{parent_struct_name}> =");
        }
        let _ = writeln!(out, "    transaction {{");
        let _ = writeln!(
            out,
            "        val lookup = LinkedHashMap<{key_type}, {parent_struct_name}>()"
        );
        let _ = writeln!(out, "        val result = mutableListOf<{parent_struct_name}>()");
        let _ = writeln!(out, "        exec(\"{sql}\"{args}) {{ rs ->");
        let _ = writeln!(out, "            while (rs.next()) {{");

        write_exposed_nullable_preamble(&mut out, child_columns, "                ", &self.manifest);
        write_exposed_nullable_preamble(&mut out, parent_columns, "                ", &self.manifest);

        let key_expr = exposed_rs_expr(key_col, &self.manifest);
        let _ = writeln!(out, "                val key = {key_expr}");

        let _ = writeln!(out, "                val child = {child_struct_name}(");
        for col in child_columns {
            let _ = writeln!(
                out,
                "                    {} = {},",
                col.field_name,
                exposed_rs_expr(col, &self.manifest)
            );
        }
        let _ = writeln!(out, "                )");

        let _ = writeln!(out, "                if (lookup.containsKey(key)) {{");
        let _ = writeln!(out, "                    lookup[key]!!.children.add(child)");
        let _ = writeln!(out, "                }} else {{");
        let _ = writeln!(out, "                    val parent = {parent_struct_name}(");
        for col in parent_columns {
            let _ = writeln!(
                out,
                "                        {} = {},",
                col.field_name,
                exposed_rs_expr(col, &self.manifest)
            );
        }
        let _ = writeln!(out, "                        children = mutableListOf(child),");
        let _ = writeln!(out, "                    )");
        let _ = writeln!(out, "                    lookup[key] = parent");
        let _ = writeln!(out, "                    result.add(parent)");
        let _ = writeln!(out, "                }}");
        let _ = writeln!(out, "            }}");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "        result");
        let _ = write!(out, "    }}");

        Ok(out)
    }
}

fn exposed_id_table_type(columns: &[ResolvedColumn]) -> &str {
    let Some(id_column) = columns.iter().find(|column| column.name.eq_ignore_ascii_case("id")) else {
        return "IntIdTable";
    };
    match id_column.lang_type.as_str() {
        "Long" => "LongIdTable",
        ty if ty.contains("UUID") => "UUIDTable",
        _ => "IntIdTable",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use scythe_core::analyzer::{
        AnalyzedColumn, AnalyzedParam, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, GroupByConfig,
    };
    use scythe_core::parser::QueryCommand;

    use super::KotlinExposedBackend;
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
    fn test_grouped_kotlin_exposed_structs() {
        let backend = crate::backends::get_backend("kotlin-exposed", "postgresql").unwrap();
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
    fn test_grouped_kotlin_exposed_query_fn() {
        let backend = crate::backends::get_backend("kotlin-exposed", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("List<GetUsersWithOrdersRow>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("transaction {"),
            "must use transaction block; got:\n{query_fn}"
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
            // ~keep The SQL text declares `$1`; the analyzer would always
            // register a matching `AnalyzedParam` for a placeholder it sees,
            // so an empty list here (as this fixture had before #149) is a
            // combination the real pipeline never produces and panics
            // `resolved_param_for_position` (#149's occurrence-based bind
            // lookup has no position to resolve `$1` against).
            q.params = vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
            }];
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
    /// up. `generate_query_fn`'s `One`/`Opt` branch reads `rs.getInt(col.name)`
    /// -- the raw SQL column name -- and only the data class' declared
    /// property (`col.field_name`) changes under `field_case = "camelCase"`.
    #[test]
    fn test_field_case_camel_case_renames_field_but_keeps_raw_lookup_key() {
        let mut backend = KotlinExposedBackend::new("postgresql").unwrap();
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

    fn make_select_star_query() -> AnalyzedQuery {
        AnalyzedQuery::build(|q| {
            q.name = "GetAllUsers".to_string();
            q.command = QueryCommand::Many;
            q.sql = "SELECT * FROM users".to_string();
            q.columns = vec![
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
            q.params = vec![];
            q.deprecated = None;
            q.source_table = Some("users".to_string());
            q.composites = vec![];
            q.enums = vec![];
            q.optional_params = vec![];
            q.group_by = None;
            q.custom = vec![];
        })
    }

    /// #214: `generate_model_struct` used to emit only the Exposed table
    /// object (`object UserTable : IntIdTable("users")`), never a row type,
    /// while `determine_struct_name` -- shared naming code this backend does
    /// not control -- decided the query function would construct `User(...)`,
    /// a type declared nowhere. `test_select_star_declares_and_references_the_same_struct_name_across_all_backends`
    /// in `lib.rs` checks this invariant generically across every backend;
    /// this test exercises the same fix directly so it lives with the code it
    /// covers.
    #[test]
    fn test_select_star_declares_both_the_table_object_and_the_row_data_class() {
        let backend = KotlinExposedBackend::new("postgresql").unwrap();
        let query = make_select_star_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let model_struct = result.model_struct.as_deref().unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            model_struct.contains("object UserTable : IntIdTable(\"users\") {"),
            "must still declare the Exposed table object; got:\n{model_struct}"
        );
        assert!(
            model_struct.contains("data class User("),
            "must also declare the row type the query function references; got:\n{model_struct}"
        );
        assert!(
            query_fn.contains("User("),
            "the query function must construct the declared row type; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_field_case_option_rejects_invalid_value() {
        let mut backend = KotlinExposedBackend::new("postgresql").unwrap();
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
        let backend = KotlinExposedBackend::new("postgresql").unwrap();
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
