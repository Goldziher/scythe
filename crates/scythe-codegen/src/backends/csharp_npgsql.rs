use std::fmt::Write;
use std::sync::{Arc, Mutex};

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name, to_pascal_case};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/csharp-npgsql.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/csharp-npgsql.redshift.toml");

pub struct CsharpNpgsqlBackend {
    manifest: BackendManifest,
    /// Track generated enums so we can emit their extensions in post_footer
    generated_enums: Arc<Mutex<Vec<EnumInfo>>>,
}

impl CsharpNpgsqlBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_TOML,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!(
                        "csharp-npgsql only supports PostgreSQL/Redshift, got engine '{}'",
                        engine
                    ),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
        Ok(Self {
            manifest,
            generated_enums: Arc::new(Mutex::new(Vec::new())),
        })
    }
}

/// Map a neutral type to an Npgsql reader method.
///
/// `lang_type` is the manifest's own declaration for the same column (the
/// non-nullable base form). Every neutral type this table does not name falls
/// through to a typed accessor built from it -- see
/// [`super::csharp_typed_reader_method`] for why an untyped fallback cannot
/// compile.
///
/// `inet` and `interval` are deliberately absent from the `GetString` arm:
/// `csharp-npgsql.toml` declares them `System.Net.IPAddress` and `TimeSpan`,
/// and Npgsql reads both natively, so routing them through `GetString` was the
/// same declaration-vs-reader disagreement in a different disguise.
fn reader_method(neutral_type: &str, lang_type: &str) -> String {
    let mapped = match neutral_type {
        "bool" => "GetBoolean",
        "int16" => "GetInt16",
        "int32" => "GetInt32",
        "int64" => "GetInt64",
        "float32" => "GetFloat",
        "float64" => "GetDouble",
        "string" | "json" => "GetString",
        "uuid" => "GetGuid",
        "decimal" => "GetDecimal",
        "date" => "GetFieldValue<DateOnly>",
        "time" | "time_tz" => "GetFieldValue<TimeOnly>",
        "datetime" => "GetDateTime",
        "datetime_tz" => "GetFieldValue<DateTimeOffset>",
        _ => return super::csharp_typed_reader_method(lang_type),
    };
    mapped.to_string()
}

/// Build the expression to read a column from NpgsqlDataReader.
///
/// ~keep board #220: a composite column cannot go through `reader.GetFieldValue<{typ}>` --
/// Npgsql registers no binary decoder for a user-defined composite unless the caller
/// registers one with `NpgsqlDataSourceBuilder.MapComposite<T>()` on the connection *before*
/// this generated code ever runs, which this code is in no position to do on the caller's
/// behalf. Verified against a live PostgreSQL 16 through Npgsql 10: with no mapping,
/// `GetFieldValue<UserAddress>`/`GetValue`/`GetFieldValue<string>` all throw
/// `InvalidCastException` on an unmapped composite OID, even for a `NULL` value. Setting
/// `cmd.UnknownResultTypeList` (see [`unknown_result_type_list`]) forces that one column back
/// to Npgsql's text wire format, which `GetFieldValue<string>` can then read -- the same text
/// form `record_out` writes, parsed by `{Type}.FromText` below.
fn column_read_expr(col: &ResolvedColumn, ordinal: usize) -> String {
    if col.neutral_type.starts_with("enum::") {
        format!(
            "(Enum.TryParse<{typ}>(reader.GetString({ord}), true, out var enumVal{ord}) ? enumVal{ord} : throw new InvalidOperationException($\"Invalid enum value '{{reader.GetString({ord})}}' for {typ}\"))",
            typ = col.lang_type,
            ord = ordinal
        )
    } else if col.neutral_type.starts_with("composite::") {
        format!(
            "{typ}.FromText(reader.GetFieldValue<string>({ord}))!",
            typ = col.lang_type,
            ord = ordinal
        )
    } else {
        let method = reader_method(&col.neutral_type, &col.lang_type);
        format!("reader.{}({})", method, ordinal)
    }
}

/// The `NpgsqlCommand.UnknownResultTypeList` boolean mask needed before executing a query that
/// selects a composite column -- `true` at every composite ordinal, `false` elsewhere -- so
/// Npgsql hands that column back as text instead of attempting (and failing) its binary
/// decode. `None` when `columns` has no composite, so a query with none emits no extra line and
/// every other generated query stays byte-identical to before this fix.
fn unknown_result_type_list(columns: &[ResolvedColumn]) -> Option<String> {
    if !columns.iter().any(|c| c.neutral_type.starts_with("composite::")) {
        return None;
    }
    let flags = columns
        .iter()
        .map(|c| {
            if c.neutral_type.starts_with("composite::") {
                "true"
            } else {
                "false"
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("new[] {{ {flags} }}"))
}

/// Splits a PostgreSQL composite's text form (`"(a,b,c)"`) into its raw field tokens, honoring
/// its escaping rules -- an empty unquoted field is SQL NULL, and a field containing a comma,
/// paren, quote, backslash, or leading/trailing space (or the empty string) is double-quoted,
/// with an inner `"` **doubled** and an inner `\` backslash-escaped.
const CSHARP_PARSE_COMPOSITE_FIELDS_METHOD: &str = r#"    /// <summary>
    /// ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
    /// honoring its escaping rules: an empty unquoted field is SQL NULL (returned as null); a
    /// field needing quoting (containing a comma, paren, quote, backslash, or leading/trailing
    /// space, or the empty string) is wrapped in double quotes; every other field is unquoted
    /// and taken literally. A nested composite's own "(x,y)" text form always contains parens,
    /// so it always comes back quoted here, ready for that type's own FromText to parse
    /// recursively.
    ///
    /// Inside a quoted field record_out writes a literal '"' as '""' and a literal '\' as '\\'.
    /// Both spellings must be accepted: reading '""' as "closing quote, then a new field" both
    /// truncates the value and desynchronizes every field after it. Verified against
    /// PostgreSQL 16 -- ROW('he said "hi"', 'back\slash', NULL) renders as
    /// ("he said ""hi""","back\\slash",).
    /// </summary>
    private static List<string?> ParseCompositeFields(string text)
    {
        var fields = new List<string?>();
        var inner = text.Substring(1, text.Length - 2);
        int i = 0;
        int n = inner.Length;
        while (true)
        {
            var field = new System.Text.StringBuilder();
            bool isNull = false;
            if (i < n && inner[i] == '"')
            {
                i++;
                while (i < n)
                {
                    char c = inner[i];
                    if (c == '\\' && i + 1 < n)
                    {
                        field.Append(inner[i + 1]);
                        i += 2;
                    }
                    else if (c == '"' && i + 1 < n && inner[i + 1] == '"')
                    {
                        field.Append('"');
                        i += 2;
                    }
                    else if (c == '"')
                    {
                        i++;
                        break;
                    }
                    else
                    {
                        field.Append(c);
                        i++;
                    }
                }
            }
            else
            {
                int start = i;
                while (i < n && inner[i] != ',')
                {
                    i++;
                }
                field.Append(inner, start, i - start);
                isNull = field.Length == 0;
            }
            fields.Add(isNull ? null : field.ToString());
            if (i < n && inner[i] == ',')
            {
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
const CSHARP_PARSE_COMPOSITE_BYTES_METHOD: &str = r#"    /// <summary>
    /// ~keep PostgreSQL's default `bytea` text output is hex: "\x48656c6c6f". Decode the hex
    /// digits after the "\x" prefix back into bytes.
    /// </summary>
    private static byte[] ParseCompositeBytes(string hex)
    {
        return Convert.FromHexString(hex.Substring(2));
    }
"#;

/// `time_tz`'s manifest type is `TimeOnly`, which (unlike `DateTimeOffset`) has no offset
/// field, so the trailing PostgreSQL offset (`"13:22:43-05"`) has to be stripped before
/// `TimeOnly.Parse` -- it throws `FormatException` on any text it cannot fully consume.
/// Emitted only when a composite has a `time_tz` field.
const CSHARP_PARSE_COMPOSITE_TIME_ONLY_METHOD: &str = r#"    /// <summary>
    /// ~keep `time_tz`'s manifest type is `TimeOnly`, which carries no UTC offset, so the
    /// trailing PostgreSQL offset ("13:22:43-05") is dropped before parsing -- `TimeOnly.Parse`
    /// rejects any text it cannot fully consume.
    /// </summary>
    private static TimeOnly ParseCompositeTimeOnly(string raw)
    {
        int signIndex = raw.LastIndexOfAny(new[] { '+', '-' });
        var timePart = signIndex > 0 ? raw.Substring(0, signIndex) : raw;
        return TimeOnly.Parse(timePart, System.Globalization.CultureInfo.InvariantCulture);
    }
"#;

fn composite_needs_bytes_helper(composite: &CompositeInfo) -> bool {
    composite.fields.iter().any(|f| f.neutral_type == "bytes")
}

fn composite_needs_time_only_helper(composite: &CompositeInfo) -> bool {
    composite.fields.iter().any(|f| f.neutral_type == "time_tz")
}

/// The C# expression converting one composite field's raw text token (`raw`, a `string`
/// already unescaped by `ParseCompositeFields` and null-forgiven) into the field's declared C#
/// type -- the inverse of what PostgreSQL's composite output function wrote for that field.
///
/// A field's own declared type is always non-nullable (`generate_composite_def` resolves every
/// field with `nullable: false` -- composite fields carry no per-field nullability), so a
/// genuinely NULL sub-field converted through a non-string arm (`int.Parse(null)`, ...) throws
/// at runtime. That is a pre-existing gap in what `CompositeFieldInfo` tracks (matching
/// `java_jdbc.rs`'s identical note), not one this fix introduces or can close from here.
fn composite_field_from_text(neutral_type: &str, field_type: &str, raw: &str) -> String {
    if neutral_type.starts_with("composite::") {
        return format!("{field_type}.FromText({raw})!");
    }
    if neutral_type.starts_with("enum::") {
        return format!("Enum.Parse<{field_type}>({raw}, true)");
    }
    match neutral_type {
        "bool" => format!("{raw} == \"t\""),
        "int16" => format!("short.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "int32" => format!("int.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "int64" => format!("long.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "float32" => format!("float.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "float64" => format!("double.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "decimal" => format!("decimal.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "uuid" => format!("Guid.Parse({raw})"),
        "date" => format!("DateOnly.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "time" => format!("TimeOnly.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "time_tz" => format!("ParseCompositeTimeOnly({raw})"),
        // ~keep Verified against live PostgreSQL 16: unlike Java's `LocalDateTime`/`OffsetDateTime`,
        // `DateTime.Parse`/`DateTimeOffset.Parse` accept the space-separated form and an offset
        // with no minutes ("2024-01-15 10:30:00+00") natively -- no normalization needed.
        "datetime" => format!("DateTime.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "datetime_tz" => format!("DateTimeOffset.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        "bytes" => format!("ParseCompositeBytes({raw})"),
        "inet" => format!("System.Net.IPAddress.Parse({raw})"),
        "interval" => format!("TimeSpan.Parse({raw}, System.Globalization.CultureInfo.InvariantCulture)"),
        // ~keep "string"/"json" both resolve to C# `string`, so the already-parsed text needs no
        // further conversion. Any neutral type not named above (e.g. an array-typed composite
        // field) falls through here too; passing the raw text through is the least-wrong
        // fallback available at generate time rather than a hard error.
        _ => raw.to_string(),
    }
}

impl CodegenBackend for CsharpNpgsqlBackend {
    fn name(&self) -> &str {
        "csharp-npgsql"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "redshift"]
    }

    fn file_header(&self) -> String {
        "#nullable enable\n\nusing Npgsql;\n\npublic static class Queries {".to_string()
    }

    fn file_footer(&self) -> String {
        "}".to_string()
    }

    fn post_footer(&self) -> String {
        if let Ok(enums) = self.generated_enums.lock() {
            if enums.is_empty() {
                return String::new();
            }

            let mut out = String::new();
            for (i, enum_info) in enums.iter().enumerate() {
                if i > 0 {
                    out.push_str("\n\n");
                }

                let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
                let qualified_type = format!("Queries.{}", type_name);
                let _ = writeln!(out, "public static class {}Extensions {{", type_name);
                let _ = writeln!(
                    out,
                    "    public static string ToDbValue(this {} value) => value switch {{",
                    qualified_type
                );
                for value in &enum_info.values {
                    let variant = enum_variant_name(value, &self.manifest.naming);
                    let _ = writeln!(out, "        {}.{} => \"{}\",", qualified_type, variant, value);
                }
                let _ = writeln!(
                    out,
                    "        _ => throw new ArgumentOutOfRangeException(nameof(value), value, null),"
                );
                let _ = writeln!(out, "    }};");
                let _ = write!(out, "}}");
            }
            out
        } else {
            String::new()
        }
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        _query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();
        let _ = writeln!(out, "public record {}(", struct_name);
        for (i, c) in columns.iter().enumerate() {
            let field = to_pascal_case(&c.field_name);
            let sep = if i + 1 < columns.len() { "," } else { "" };
            let _ = writeln!(out, "    {} {}{}", c.full_type, field, sep);
        }
        let _ = write!(out, ");");
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
        let mut sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!("@p{n}"),
        );
        for (i, p) in params.iter().enumerate() {
            if let Some(enum_name) = p.neutral_type.strip_prefix("enum::") {
                let placeholder = format!("@p{}", i + 1);
                let casted = format!("@p{}::{}", i + 1, enum_name);
                sql = sql.replace(&placeholder, &casted);
            }
        }
        let sql = crate::sql_literal::escape_csharp_verbatim_string(&sql);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{} {}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        if matches!(analyzed.command, QueryCommand::Batch) {
            let batch_fn_name = format!("{}Batch", func_name);
            if params.len() > 1 {
                let params_record_name = format!("{}BatchParams", to_pascal_case(&analyzed.name));
                let _ = writeln!(out, "public record {}(", params_record_name);
                for (i, p) in params.iter().enumerate() {
                    let field = to_pascal_case(&p.field_name);
                    let sep = if i + 1 < params.len() { "," } else { "" };
                    let _ = writeln!(out, "    {} {}{}", p.full_type, field, sep);
                }
                let _ = writeln!(out, ");");
                let _ = writeln!(out);
                let _ = writeln!(
                    out,
                    "public static async Task {}(NpgsqlConnection conn, List<{}> items) {{",
                    batch_fn_name, params_record_name
                );
            } else if params.len() == 1 {
                let _ = writeln!(
                    out,
                    "public static async Task {}(NpgsqlConnection conn, List<{}> items) {{",
                    batch_fn_name, params[0].full_type
                );
            } else {
                let _ = writeln!(
                    out,
                    "public static async Task {}(NpgsqlConnection conn, int count) {{",
                    batch_fn_name
                );
            }
            let _ = writeln!(out, "    await using var tx = await conn.BeginTransactionAsync();");
            let _ = writeln!(out, "    try {{");
            if params.is_empty() {
                let _ = writeln!(out, "        for (int i = 0; i < count; i++) {{");
            } else {
                let _ = writeln!(out, "        foreach (var item in items) {{");
            }
            let _ = writeln!(
                out,
                "            await using var cmd = new NpgsqlCommand(@\"{}\", conn, tx);",
                sql
            );
            for (i, p) in params.iter().enumerate() {
                let value_expr = if params.len() > 1 {
                    let field = to_pascal_case(&p.field_name);
                    if p.neutral_type.starts_with("enum::") {
                        format!("item.{}.ToDbValue()", field)
                    } else {
                        format!("item.{}", field)
                    }
                } else if p.neutral_type.starts_with("enum::") {
                    "item.ToDbValue()".to_string()
                } else {
                    "item".to_string()
                };
                let _ = writeln!(
                    out,
                    "            cmd.Parameters.AddWithValue(\"p{}\", {});",
                    i + 1,
                    value_expr
                );
            }
            let _ = writeln!(out, "            await cmd.ExecuteNonQueryAsync();");
            let _ = writeln!(out, "        }}");
            let _ = writeln!(out, "        await tx.CommitAsync();");
            let _ = writeln!(out, "    }} catch {{");
            let _ = writeln!(out, "        await tx.RollbackAsync();");
            let _ = writeln!(out, "        throw;");
            let _ = writeln!(out, "    }}");
            let _ = write!(out, "}}");
            return Ok(out);
        }

        let return_type = match &analyzed.command {
            QueryCommand::One => struct_name.to_string(),
            QueryCommand::Opt => format!("{}?", struct_name),
            QueryCommand::Many => format!("List<{}>", struct_name),
            QueryCommand::Exec => "void".to_string(),
            QueryCommand::ExecResult | QueryCommand::ExecRows => "int".to_string(),
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
        };

        let is_async_void = return_type == "void";
        let task_type = if is_async_void {
            "Task".to_string()
        } else {
            format!("Task<{}>", return_type)
        };

        let _ = writeln!(
            out,
            "public static async {} {}(NpgsqlConnection conn{}{}) {{",
            task_type, func_name, sep, param_list
        );

        let _ = writeln!(out, "    await using var cmd = new NpgsqlCommand(@\"{}\", conn);", sql);
        for (i, p) in params.iter().enumerate() {
            let value_expr = if p.neutral_type.starts_with("enum::") {
                format!("{}.ToDbValue()", p.field_name)
            } else {
                p.field_name.clone()
            };
            let _ = writeln!(out, "    cmd.Parameters.AddWithValue(\"p{}\", {});", i + 1, value_expr);
        }

        if let Some(list) = unknown_result_type_list(columns) {
            let _ = writeln!(out, "    cmd.UnknownResultTypeList = {list};");
        }

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(out, "    await using var reader = await cmd.ExecuteReaderAsync();");
                if matches!(analyzed.command, QueryCommand::One) {
                    let _ = writeln!(
                        out,
                        "    if (!await reader.ReadAsync()) throw new InvalidOperationException(\"{func_name} expected exactly one row but found none\");"
                    );
                } else {
                    let _ = writeln!(out, "    if (!await reader.ReadAsync()) return null;");
                }
                let _ = writeln!(out, "    return new {}(", struct_name);
                for (i, col) in columns.iter().enumerate() {
                    let expr = column_read_expr(col, i);
                    let sep = if i + 1 < columns.len() { "," } else { "" };
                    if col.nullable {
                        let _ = writeln!(out, "        reader.IsDBNull({i}) ? null : {expr}{sep}");
                    } else {
                        let _ = writeln!(out, "        {expr}{sep}");
                    }
                }
                let _ = writeln!(out, "    );");
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "    await using var reader = await cmd.ExecuteReaderAsync();");
                let _ = writeln!(out, "    var results = new List<{}>();", struct_name);
                let _ = writeln!(out, "    while (await reader.ReadAsync()) {{");
                let _ = writeln!(out, "        results.Add(new {}(", struct_name);
                for (i, col) in columns.iter().enumerate() {
                    let expr = column_read_expr(col, i);
                    let sep = if i + 1 < columns.len() { "," } else { "" };
                    if col.nullable {
                        let _ = writeln!(out, "            reader.IsDBNull({i}) ? null : {expr}{sep}");
                    } else {
                        let _ = writeln!(out, "            {expr}{sep}");
                    }
                }
                let _ = writeln!(out, "        ));");
                let _ = writeln!(out, "    }}");
                let _ = writeln!(out, "    return results;");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "    await cmd.ExecuteNonQueryAsync();");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "    return await cmd.ExecuteNonQueryAsync();");
            }
            QueryCommand::Batch => unreachable!(),
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
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
            let field = to_pascal_case(&c.field_name);
            let sep = if i + 1 < child_columns.len() { "," } else { "" };
            let _ = writeln!(out, "    {} {}{}", c.full_type, field, sep);
        }
        let _ = writeln!(out, ");");
        let _ = writeln!(out);

        let _ = writeln!(out, "public record {}(", parent_struct_name);
        for c in parent_columns {
            let field = to_pascal_case(&c.field_name);
            let _ = writeln!(out, "    {} {},", c.full_type, field);
        }
        let _ = writeln!(out, "    List<{}> Children", child_struct_name);
        let _ = write!(out, ");");

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
        let mut sql = super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!("@p{n}"),
        );
        for (i, p) in params.iter().enumerate() {
            if let Some(enum_name) = p.neutral_type.strip_prefix("enum::") {
                let placeholder = format!("@p{}", i + 1);
                let casted = format!("@p{}::{}", i + 1, enum_name);
                sql = sql.replace(&placeholder, &casted);
            }
        }
        let sql = crate::sql_literal::escape_csharp_verbatim_string(&sql);

        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{} {}", p.full_type, p.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let key_col = parent_columns
            .iter()
            .find(|c| c.name == key_column)
            .unwrap_or(&parent_columns[0]);
        let key_type = &key_col.full_type;
        let key_ordinal = all_columns.iter().position(|c| c.name == key_column).unwrap_or(0);

        let _ = writeln!(
            out,
            "public static async Task<List<{parent_struct_name}>> {func_name}(NpgsqlConnection conn{sep}{param_list}) {{"
        );
        let _ = writeln!(out, "    await using var cmd = new NpgsqlCommand(@\"{sql}\", conn);");
        for (i, p) in params.iter().enumerate() {
            let value_expr = if p.neutral_type.starts_with("enum::") {
                format!("{}.ToDbValue()", p.field_name)
            } else {
                p.field_name.clone()
            };
            let _ = writeln!(out, "    cmd.Parameters.AddWithValue(\"p{}\", {});", i + 1, value_expr);
        }
        if let Some(list) = unknown_result_type_list(all_columns) {
            let _ = writeln!(out, "    cmd.UnknownResultTypeList = {list};");
        }
        let _ = writeln!(out, "    await using var reader = await cmd.ExecuteReaderAsync();");
        let _ = writeln!(
            out,
            "    var lookup = new Dictionary<{key_type}, {parent_struct_name}>();"
        );
        let _ = writeln!(out, "    var result = new List<{parent_struct_name}>();");
        let _ = writeln!(out, "    while (await reader.ReadAsync()) {{");

        let key_expr = column_read_expr(key_col, key_ordinal);
        let _ = writeln!(out, "        var key = {key_expr};");

        let _ = writeln!(out, "        var child = new {child_struct_name}(");
        for (ci, col) in child_columns.iter().enumerate() {
            let ord = all_columns
                .iter()
                .position(|c| c.name == col.name)
                .unwrap_or(parent_columns.len() + ci);
            let expr = column_read_expr(col, ord);
            let trailing = if ci + 1 < child_columns.len() { "," } else { "" };
            if col.nullable {
                let _ = writeln!(out, "            reader.IsDBNull({ord}) ? null : {expr}{trailing}");
            } else {
                let _ = writeln!(out, "            {expr}{trailing}");
            }
        }
        let _ = writeln!(out, "        );");

        let _ = writeln!(out, "        if (lookup.TryGetValue(key, out var parent)) {{");
        let _ = writeln!(out, "            parent.Children.Add(child);");
        let _ = writeln!(out, "        }} else {{");
        let _ = writeln!(out, "            var newParent = new {parent_struct_name}(");
        for col in parent_columns {
            let ord = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
            let expr = column_read_expr(col, ord);
            if col.nullable {
                let _ = writeln!(out, "                reader.IsDBNull({ord}) ? null : {expr},");
            } else {
                let _ = writeln!(out, "                {expr},");
            }
        }
        let _ = writeln!(out, "                new List<{child_struct_name}> {{ child }}");
        let _ = writeln!(out, "            );");
        let _ = writeln!(out, "            lookup[key] = newParent;");
        let _ = writeln!(out, "            result.Add(newParent);");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "    return result;");
        let _ = write!(out, "}}");

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "public enum {} {{", type_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "    {},", variant);
        }
        let _ = write!(out, "}}");

        if let Ok(mut enums) = self.generated_enums.lock()
            && !enums.iter().any(|e| e.sql_name == enum_info.sql_name)
        {
            enums.push(enum_info.clone());
        }

        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        // ~keep board #220: a composite with zero fields cannot exist in PostgreSQL
        // (`CREATE TYPE ... AS ()` is rejected), so there is no reachable runtime value that
        // would need `FromText` here. Left as the bare record it always was.
        if composite.fields.is_empty() {
            let _ = writeln!(out, "public record {}();", name);
            return Ok(out);
        }
        let field_types: Vec<String> = composite
            .fields
            .iter()
            .map(|f| {
                resolve_type(&f.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .unwrap_or_else(|_| "object".to_string())
            })
            .collect();
        let _ = writeln!(out, "public record {}(", name);
        for (i, field) in composite.fields.iter().enumerate() {
            let field_name = to_pascal_case(&field.name);
            let sep = if i + 1 < composite.fields.len() { "," } else { "" };
            let _ = writeln!(out, "    {} {}{}", field_types[i], field_name, sep);
        }
        let _ = writeln!(out, ") {{");
        let _ = writeln!(out, "    /// <summary>");
        let _ = writeln!(
            out,
            "    /// ~keep board #220: Npgsql has no binary decoder for this composite unless the"
        );
        let _ = writeln!(
            out,
            "    /// caller registers one with NpgsqlDataSourceBuilder.MapComposite&lt;{}&gt;() --",
            name
        );
        let _ = writeln!(
            out,
            "    /// this generated code cannot do that on the caller's behalf, so it parses the"
        );
        let _ = writeln!(out, "    /// driver's composite text form instead.");
        let _ = writeln!(out, "    /// </summary>");
        let _ = writeln!(out, "    public static {}? FromText(string? text)", name);
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        if (text is null)");
        let _ = writeln!(out, "        {{");
        let _ = writeln!(out, "            return null;");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "        var f = ParseCompositeFields(text);");
        let _ = writeln!(out, "        return new {}(", name);
        for (i, field) in composite.fields.iter().enumerate() {
            let raw = format!("f[{}]!", i);
            let value_expr = composite_field_from_text(&field.neutral_type, &field_types[i], &raw);
            let sep = if i + 1 < composite.fields.len() { "," } else { "" };
            let _ = writeln!(out, "            {}{}", value_expr, sep);
        }
        let _ = writeln!(out, "        );");
        let _ = writeln!(out, "    }}");
        let _ = writeln!(out);
        out.push_str(CSHARP_PARSE_COMPOSITE_FIELDS_METHOD);
        if composite_needs_bytes_helper(composite) {
            let _ = writeln!(out);
            out.push_str(CSHARP_PARSE_COMPOSITE_BYTES_METHOD);
        }
        if composite_needs_time_only_helper(composite) {
            let _ = writeln!(out);
            out.push_str(CSHARP_PARSE_COMPOSITE_TIME_ONLY_METHOD);
        }
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
                ..Default::default()
            },
            AnalyzedColumn {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "email".to_string(),
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
            AnalyzedColumn {
                name: "order_date".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
        AnalyzedQuery::build(|aq| {
            aq.name = "GetUsersWithOrders".to_string();
            aq.command = QueryCommand::Grouped;
            aq.sql = "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\nFROM users u\nJOIN orders o ON o.user_id = u.id".to_string();
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
    fn test_grouped_csharp_npgsql_structs() {
        let backend = crate::backends::get_backend("csharp-npgsql", "postgresql").unwrap();
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
            row_struct.contains("List<GetUsersWithOrdersChildRow> Children"),
            "parent missing Children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("public record GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("public record GetUsersWithOrdersRow(").unwrap();
        assert!(child_pos < parent_pos, "child must precede parent; got:\n{row_struct}");
        assert!(result.model_struct.is_none(), "grouped must not produce model_struct");
    }

    #[test]
    fn test_grouped_csharp_npgsql_query_fn() {
        let backend = crate::backends::get_backend("csharp-npgsql", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("Task<List<GetUsersWithOrdersRow>>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrders"),
            "missing fn name; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("NpgsqlConnection conn"),
            "missing connection param; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Dictionary<"),
            "must use Dictionary for fold lookup; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("lookup.TryGetValue"),
            "must fold with TryGetValue; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Children.Add(child)"),
            "must append child; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return result;"),
            "must return result; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_grouped_csharp_npgsql_example55_print() {
        let backend = crate::backends::get_backend("csharp-npgsql", "postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &*backend).unwrap();
        println!(
            "=== row_struct ===\n{}",
            result.row_struct.as_deref().unwrap_or("(none)")
        );
        println!("=== query_fn ===\n{}", result.query_fn.as_deref().unwrap_or("(none)"));
    }
}
