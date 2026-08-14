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

/// Build the `ResultSet` read call for a Java column.
///
/// Delegates to [`jvm_common::java_jdbc_read_call`], which answers a named
/// getter (`rs.getInt("n")`) where JDBC has one and the class-taking
/// `rs.getObject("n", T.class)` overload where it does not. The table this
/// replaced ended in a bare `"getObject"` arm that caught `UUID`, composites,
/// and every other unrecognised type, emitting an `Object`-typed expression
/// into a `TortureAddress`/`UUID` slot.
fn rs_read_call(column: &str, java_type: &str) -> String {
    jvm_common::java_jdbc_read_call(column, java_type)
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

/// The boxed Java element type for an array column's `List<{T}>`, derived
/// from the element's own neutral type through the manifest's ordinary
/// scalar/enum/composite resolution.
///
/// Deliberately not `col.lang_type`/`col.full_type`: those come from the
/// manifest's `array` container pattern substituting `{T}` with whatever
/// `[types.scalars]` spells the element as, which for Java is deliberately
/// unboxed (`int32` -> `int`) because that is the right spelling for a plain
/// field. `List<int>` is not legal Java, so the element is re-resolved here
/// and boxed exactly like a nullable primitive *field* already is.
fn java_array_element_type(element_neutral: &str, manifest: &BackendManifest) -> String {
    resolve_type(element_neutral, manifest, false)
        .map(|t| box_primitive(&t).to_string())
        .unwrap_or_else(|_| "Object".to_string())
}

/// Build the `List<T>` expression that turns a `java.sql.Array` (already
/// known non-null) into the declared element type. `sql_array_expr` is the
/// expression producing the `java.sql.Array` -- either an inline
/// `rs.getArray(...)` call or a preamble local already null-checked by
/// [`write_jdbc_nullable_preamble`].
///
/// `getArray()` returns `Object`; every JDBC array element type (per the
/// JDBC array-mapping table) comes back as a reference-type array --
/// `Integer[]`, `String[]`, never the primitive `int[]` -- so the cast to
/// `Object[]` is always legal array covariance, and each element is then
/// cast to the declared boxed type. Never `getObject`: `Object[]` is exactly
/// the shape the untyped accessor could not produce for a non-scalar column.
fn array_list_expr(sql_array_expr: &str, boxed_element_type: &str) -> String {
    format!(
        "java.util.Arrays.stream((Object[]) {sql_array_expr}.getArray()).map(v -> ({boxed_element_type}) v).collect(java.util.stream.Collectors.toList())"
    )
}

/// Splits a PostgreSQL composite's text form (`"(a,b,c)"`) into its raw field tokens, honoring
/// its escaping rules -- an empty unquoted field is SQL NULL, and a field containing a comma,
/// paren, quote, backslash, or leading/trailing space (or the empty string) is double-quoted
/// with `"`/`\` backslash-escaped inside.
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
/// the inverse of what PostgreSQL's composite output function wrote for that field.
///
/// A field's own declared type is always non-nullable (`generate_composite_def` resolves every
/// field with `nullable: false` -- composite fields carry no per-field nullability), so a
/// genuinely NULL sub-field converted through a primitive arm (`Integer.parseInt(null)`, ...)
/// throws `NumberFormatException`/`NullPointerException`. That is a pre-existing gap in what
/// `CompositeFieldInfo` tracks, not one this fix introduces or can close from here.
fn composite_field_from_text(neutral_type: &str, field_type: &str, raw: &str) -> String {
    if let Some(sql_name) = neutral_type.strip_prefix("composite::") {
        return format!("{}.fromText({})", to_pascal_case(sql_name), raw);
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
        // text needs no further conversion. Any neutral type not named above (e.g. an array-typed
        // composite field, which this fix does not handle -- see board #196's report) falls
        // through here too; passing the raw text through is the least-wrong fallback available at
        // generate time rather than a hard error.
        _ => raw.to_string(),
    }
}

/// Resolve the display type for a Java field, boxing primitives when nullable.
fn java_field_type(col: &ResolvedColumn, manifest: &BackendManifest) -> String {
    if let Some(element) = jvm_common::array_element_neutral_type(&col.neutral_type) {
        let boxed_element = java_array_element_type(element, manifest);
        return format!("List<{boxed_element}>");
    }
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

/// Whether a column's value is produced by [`write_jdbc_nullable_preamble`]
/// into a local named after the field, rather than read inline.
///
/// Three cases: a nullable primitive (whose `getInt`/`getBoolean`/... returns
/// `0`/`false` for SQL NULL and must be paired with `wasNull()`), a nullable
/// enum (whose `fromValue(rs.getString(col))` conversion throws on a NULL
/// column and must be guarded), and a nullable array (whose
/// `rs.getArray(col)` returns `null` for SQL NULL, and calling `.getArray()`
/// on that null throws `NullPointerException`).
fn is_preamble_read(col: &ResolvedColumn) -> bool {
    col.nullable
        && (is_java_primitive(&col.lang_type)
            || col.neutral_type.starts_with("enum::")
            || jvm_common::array_element_neutral_type(&col.neutral_type).is_some())
}

/// Build the inline JDBC ResultSet expression for a column (read by column name).
/// For nullable primitives, nullable enums, nullable arrays, and, on engines from
/// [`engine_needs_legacy_temporal_getter`], nullable temporal columns, the variable name is
/// returned — the preamble has already extracted the value and performed the null check.
fn col_rs_expr(col: &ResolvedColumn, engine: &str, manifest: &BackendManifest) -> String {
    if is_preamble_read(col) {
        return col.field_name.clone();
    }
    // ~keep board #196: pgjdbc registers no type map for a user-defined composite, so
    // `rs.getObject(col, T.class)` -- correct for every other reference type below -- throws
    // `PSQLException: conversion to class T ... not supported` at runtime. `T.fromText` (emitted
    // by `generate_composite_def`) parses the driver's text form instead, and is already
    // null-safe, so no preamble entry is needed regardless of `col.nullable`.
    if col.neutral_type.starts_with("composite::") {
        return format!("{}.fromText(rs.getString(\"{}\"))", col.lang_type, col.name);
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
    if let Some(element) = jvm_common::array_element_neutral_type(&col.neutral_type) {
        let boxed_element = java_array_element_type(element, manifest);
        return array_list_expr(&format!("rs.getArray(\"{}\")", col.name), &boxed_element);
    }
    if col.neutral_type.starts_with("enum::") {
        return format!("{}.fromValue(rs.getString(\"{}\"))", col.lang_type, col.name);
    }
    rs_read_call(&col.name, &col.lang_type)
}

/// Emit nullable-primitive, nullable-enum, nullable-array, and (on engines
/// needing it) nullable-temporal preamble variable declarations for grouped
/// JDBC folding and row construction.
fn write_jdbc_nullable_preamble(
    out: &mut String,
    cols: &[ResolvedColumn],
    indent: &str,
    engine: &str,
    manifest: &BackendManifest,
) {
    for col in cols {
        if !col.nullable {
            continue;
        }
        if is_java_primitive(&col.lang_type) {
            let _ = writeln!(
                out,
                "{}var {}Raw = {};",
                indent,
                col.field_name,
                rs_read_call(&col.name, &col.lang_type)
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
        // ~keep A nullable enum column read inline would be
        // `Status.fromValue(rs.getString(col))`, which throws
        // NullPointerException the moment the column is actually NULL — the
        // one value a `@Nullable` field exists to represent. `getString`
        // already returns `null` for SQL NULL, so the guard is on the raw
        // string, not on `wasNull()`.
        if col.neutral_type.starts_with("enum::") {
            let _ = writeln!(
                out,
                "{}var {}Raw = rs.getString(\"{}\");",
                indent, col.field_name, col.name
            );
            let _ = writeln!(
                out,
                "{}{} {} = {}Raw == null ? null : {}.fromValue({}Raw);",
                indent, col.lang_type, col.field_name, col.field_name, col.lang_type, col.field_name
            );
            continue;
        }
        // ~keep A nullable array column's `rs.getArray(col)` returns `null`
        // for SQL NULL; calling `.getArray()` on that null throws
        // `NullPointerException`, so the guard is on the `java.sql.Array`
        // local, before the element conversion runs.
        if let Some(element) = jvm_common::array_element_neutral_type(&col.neutral_type) {
            let boxed_element = java_array_element_type(element, manifest);
            let _ = writeln!(
                out,
                "{}var {}SqlArray = rs.getArray(\"{}\");",
                indent, col.field_name, col.name
            );
            let list_expr = array_list_expr(&format!("{}SqlArray", col.field_name), &boxed_element);
            let _ = writeln!(
                out,
                "{}List<{}> {} = {}SqlArray == null ? null : {};",
                indent, boxed_element, col.field_name, col.field_name, list_expr
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
        let _ = writeln!(out, ") {{");

        let _ = writeln!(
            out,
            "    public static {} fromResultSet(ResultSet rs) throws SQLException {{",
            struct_name
        );
        write_jdbc_nullable_preamble(&mut out, columns, "        ", &self.engine, &self.manifest);
        let _ = writeln!(out, "        return new {}(", struct_name);
        for (i, col) in columns.iter().enumerate() {
            let sep = if i + 1 < columns.len() { "," } else { "" };
            let expr = col_rs_expr(col, &self.engine, &self.manifest);
            let _ = writeln!(out, "            {}{}", expr, sep);
        }
        let _ = writeln!(out, "        );");
        let _ = writeln!(out, "    }}");
        let _ = write!(out, "}}");
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
        let dialect = SqlDialect::from_str(&self.engine).unwrap_or_default();
        let cleaned_sql = super::clean_sql_oneline_with_optional_dialect(
            &analyzed.sql,
            dialect,
            &analyzed.optional_params,
            &analyzed.params,
        );
        let (rewritten_sql, occurrences) =
            super::rewrite_placeholders_indexed(&cleaned_sql, dialect, |_| "?".to_string());
        let sql = crate::sql_literal::escape_java_string(&rewritten_sql);

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
                for (i, position) in occurrences.iter().enumerate() {
                    let param = super::resolved_param_for_position(&analyzed.params, params, *position);
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
                for (i, position) in occurrences.iter().enumerate() {
                    let param = super::resolved_param_for_position(&analyzed.params, params, *position);
                    let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
                }
                let _ = writeln!(out, "        return ps.executeUpdate();");
                let _ = writeln!(out, "    }}");
                let _ = write!(out, "}}");
            }
            QueryCommand::One | QueryCommand::Opt => {
                // ~keep #192: :one means "exactly one row, error if absent";
                // :opt means "zero or one, null if absent". This arm used to
                // render byte-identical code for both, so :one silently
                // returned null on a missing row -- a wrong answer in the
                // caller's happy path rather than a signal. `is_one` is the
                // only difference from here down: the declared return type
                // drops `@Nullable`, and every branch's null-on-missing-row
                // tail becomes a thrown `NoSuchElementException` instead.
                let is_one = matches!(analyzed.command, QueryCommand::One);
                let return_type = if is_one {
                    struct_name.to_string()
                } else {
                    format!("@Nullable {}", struct_name)
                };
                let _ = writeln!(
                    out,
                    "public static {} {}(Connection conn{}{}) throws SQLException {{",
                    return_type, func_name, sep, param_list
                );
                let missing_row = format!(
                    "throw new java.util.NoSuchElementException(\"{}: no rows returned\");",
                    func_name
                );
                let is_oracle_returning = self.engine == "oracle" && sql.to_uppercase().contains("RETURNING");
                let is_mariadb_returning = self.engine == "mariadb" && sql.to_uppercase().contains("RETURNING");
                if is_mariadb_returning {
                    let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
                    for (i, position) in occurrences.iter().enumerate() {
                        let param = super::resolved_param_for_position(&analyzed.params, params, *position);
                        let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
                    }
                    let _ = writeln!(out, "        ps.execute();");
                    let _ = writeln!(out, "        ResultSet rs = ps.getResultSet();");
                    let _ = writeln!(out, "        if (rs != null && rs.next()) {{");
                    let _ = writeln!(out, "            return {}.fromResultSet(rs);", struct_name);
                    let _ = writeln!(out, "        }}");
                    if is_one {
                        let _ = writeln!(out, "        {}", missing_row);
                    } else {
                        let _ = writeln!(out, "        return null;");
                    }
                    let _ = writeln!(out, "    }}");
                    let _ = write!(out, "}}");
                } else if is_oracle_returning {
                    let into_placeholders = columns.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
                    let full_sql = format!("BEGIN {} INTO {}; END;", sql, into_placeholders);
                    let _ = writeln!(out, "    try (var cs = conn.prepareCall(\"{}\")) {{", full_sql);
                    for (i, position) in occurrences.iter().enumerate() {
                        let param = super::resolved_param_for_position(&analyzed.params, params, *position);
                        let setter = ps_setter(&param.lang_type);
                        let _ = writeln!(out, "        cs.{}({}, {});", setter, i + 1, param.field_name);
                    }
                    for (i, col) in columns.iter().enumerate() {
                        let jdbc_type = oracle_jdbc_type(&col.neutral_type);
                        let _ = writeln!(
                            out,
                            "        cs.registerOutParameter({}, {});",
                            occurrences.len() + i + 1,
                            jdbc_type
                        );
                    }
                    let _ = writeln!(out, "        cs.execute();");
                    let _ = writeln!(out, "        return new {}(", struct_name);
                    for (i, col) in columns.iter().enumerate() {
                        let getter_call = oracle_cs_getter_call(&col.neutral_type, occurrences.len() + i + 1);
                        let sep = if i + 1 < columns.len() { "," } else { "" };
                        let _ = writeln!(out, "            cs.{}{}", getter_call, sep);
                    }
                    let _ = writeln!(out, "        );");
                    let _ = writeln!(out, "    }}");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(out, "    try (var ps = conn.prepareStatement(\"{}\")) {{", sql);
                    for (i, position) in occurrences.iter().enumerate() {
                        let param = super::resolved_param_for_position(&analyzed.params, params, *position);
                        let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
                    }
                    let _ = writeln!(out, "        try (ResultSet rs = ps.executeQuery()) {{");
                    let _ = writeln!(out, "            if (rs.next()) {{");
                    let _ = writeln!(out, "                return {}.fromResultSet(rs);", struct_name);
                    let _ = writeln!(out, "            }}");
                    if is_one {
                        let _ = writeln!(out, "            {}", missing_row);
                    } else {
                        let _ = writeln!(out, "            return null;");
                    }
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
                for (i, position) in occurrences.iter().enumerate() {
                    let param = super::resolved_param_for_position(&analyzed.params, params, *position);
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
                    for (i, position) in occurrences.iter().enumerate() {
                        let param = super::resolved_param_for_position(&analyzed.params, params, *position);
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
                    for i in 0..occurrences.len() {
                        let _ = writeln!(out, "            ps.{}({}, item);", setter, i + 1);
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
        let _ = writeln!(out);
        // ~keep #213: the reader must decode against the same wire value the
        // bind side sends (`ps.setString(n, x.getValue())` /
        // `ps.setObject(n, x.getValue(), Types.OTHER)`). `enum_variant_name`
        // sanitises characters an identifier cannot hold, so a SQL value like
        // `in-active` becomes the variant `IN_ACTIVE` while the wire value
        // stays `"in-active"`. A reader that upper-cased the raw string and
        // called `valueOf` was matching against the *variant spelling*, not
        // the SQL value, and threw `IllegalArgumentException` on exactly the
        // value the column exists to hold. Scanning `values()` for the
        // declared `value` is the round-trip-correct answer: `fromValue(x.getValue()) == x`
        // for every variant, regardless of how the variant name was sanitised.
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
        let name = to_pascal_case(&composite.sql_name);
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
            "     * ~keep board #196: pgjdbc registers no `getObject(col, {}.class)` type map for",
            name
        );
        let _ = writeln!(
            out,
            "     * this composite -- it throws `PSQLException: conversion to class {}` at runtime.",
            name
        );
        let _ = writeln!(out, "     * Parse the driver's composite text form instead.");
        let _ = writeln!(out, "     */");
        let _ = writeln!(out, "    public static {} fromText(String text) {{", name);
        let _ = writeln!(out, "        if (text == null) {{");
        let _ = writeln!(out, "            return null;");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "        java.util.List<String> f = parseCompositeFields(text);");
        let _ = writeln!(out, "        return new {}(", name);
        for (i, (field, field_type)) in composite.fields.iter().zip(&field_types).enumerate() {
            let raw = format!("f.get({})", i);
            let value_expr = composite_field_from_text(&field.neutral_type, field_type, &raw);
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
        let dialect = SqlDialect::from_str(&self.engine).unwrap_or_default();
        let cleaned_sql = super::clean_sql_oneline_with_optional_dialect(
            &analyzed.sql,
            dialect,
            &analyzed.optional_params,
            &analyzed.params,
        );
        let (rewritten_sql, occurrences) =
            super::rewrite_placeholders_indexed(&cleaned_sql, dialect, |_| "?".to_string());
        let sql = crate::sql_literal::escape_java_string(&rewritten_sql);

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
        for (i, position) in occurrences.iter().enumerate() {
            let param = super::resolved_param_for_position(&analyzed.params, params, *position);
            let _ = writeln!(out, "        {}", ps_bind_param(param, i, &self.engine));
        }
        let _ = writeln!(out, "        try (ResultSet rs = ps.executeQuery()) {{");
        let _ = writeln!(out, "            while (rs.next()) {{");

        let key_expr = col_rs_expr(key_col, &self.engine, &self.manifest);
        let _ = writeln!(out, "                {key_type} key = {key_expr};");

        write_jdbc_nullable_preamble(
            &mut out,
            child_columns,
            "                ",
            &self.engine,
            &self.manifest,
        );

        let _ = writeln!(out, "                var child = new {child_struct_name}(");
        for (i, col) in child_columns.iter().enumerate() {
            let expr = col_rs_expr(col, &self.engine, &self.manifest);
            let sep = if i + 1 < child_columns.len() { "," } else { "" };
            let _ = writeln!(out, "                    {expr}{sep}");
        }
        let _ = writeln!(out, "                );");

        let _ = writeln!(out, "                if (lookup.containsKey(key)) {{");
        let _ = writeln!(out, "                    lookup.get(key).children().add(child);");
        let _ = writeln!(out, "                }} else {{");

        write_jdbc_nullable_preamble(
            &mut out,
            parent_columns,
            "                    ",
            &self.engine,
            &self.manifest,
        );

        let _ = writeln!(out, "                    var parent = new {parent_struct_name}(");
        for col in parent_columns {
            let expr = col_rs_expr(col, &self.engine, &self.manifest);
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

    use scythe_core::analyzer::{
        AnalyzedColumn, AnalyzedParam, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, GroupByConfig,
    };
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
                source_relation: None,
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
