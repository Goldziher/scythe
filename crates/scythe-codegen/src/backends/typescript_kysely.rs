use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name, to_camel_case};
use scythe_backend::types::resolve_type;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::GeneratedCode;
use crate::backend_options::reject_unknown_options;
use crate::backend_trait::GroupedQueryFn;
use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};
use crate::backends::typescript_common::{
    TsFieldCase, TsRowType, escape_ts_template_literal, generate_grouped_interface_structs,
    generate_ts_grouped_fold_body, generate_ts_interface_row_struct,
    generate_ts_json_composite_interface_with_field_case, generate_ts_nested_interface_with_field_case,
    generate_ts_union_row_struct, generate_zod_enum, generate_zod_grouped_structs, generate_zod_row_struct,
    generate_zod_union_row_struct, parse_bool_option, ts_index_access, ts_member_access, ts_property_key,
    ts_row_not_found_throw,
};

/// Board #204: whatever driver the caller's `Kysely<DB>`/`QueryExecutorProvider` wraps (`pg`
/// by default), a user-defined composite column arrives through this `sql` tag as PostgreSQL's
/// raw composite *text form* (`"(a,b,c)"`), a plain `string` -- not the generated `interface`.
/// See `typescript_pg.rs`'s identical fix for the full rationale and the escaping rules; this
/// is that same fix for Kysely's `sql<T>\`...\`.execute(db)` row shape.
fn ts_parse_composite_fields_helper(name: &str) -> String {
    format!(
        "function parse{name}Fields(text: string): (string | null)[] {{\n\
\t// ~keep Splits a PostgreSQL composite's text form (\"(a,b,c)\") into its raw field\n\
\t// tokens, honoring its escaping rules: an empty unquoted field is SQL NULL (returned as\n\
\t// null); a field needing quoting (comma, paren, quote, backslash, leading/trailing\n\
\t// space, or the empty string) is wrapped in double quotes; every other field is\n\
\t// unquoted and taken literally. Inside a quoted field `record_out` writes a literal\n\
\t// '\"' as '\"\"' and a literal '\\\\' as '\\\\\\\\' -- reading '\"\"' as a closing quote both\n\
\t// truncates the value and desynchronizes every field after it. Verified against\n\
\t// PostgreSQL 16.\n\
\tconst fields: (string | null)[] = [];\n\
\tconst inner = text.slice(1, -1);\n\
\tlet i = 0;\n\
\tconst n = inner.length;\n\
\tfor (;;) {{\n\
\t\tlet chars = \"\";\n\
\t\tlet isNull = false;\n\
\t\tif (i < n && inner[i] === '\"') {{\n\
\t\t\ti++;\n\
\t\t\twhile (i < n) {{\n\
\t\t\t\tconst c = inner[i];\n\
\t\t\t\tif (c === \"\\\\\" && i + 1 < n) {{\n\
\t\t\t\t\tchars += inner[i + 1];\n\
\t\t\t\t\ti += 2;\n\
\t\t\t\t}} else if (c === '\"' && i + 1 < n && inner[i + 1] === '\"') {{\n\
\t\t\t\t\tchars += '\"';\n\
\t\t\t\t\ti += 2;\n\
\t\t\t\t}} else if (c === '\"') {{\n\
\t\t\t\t\ti++;\n\
\t\t\t\t\tbreak;\n\
\t\t\t\t}} else {{\n\
\t\t\t\t\tchars += c;\n\
\t\t\t\t\ti++;\n\
\t\t\t\t}}\n\
\t\t\t}}\n\
\t\t}} else {{\n\
\t\t\tconst start = i;\n\
\t\t\twhile (i < n && inner[i] !== \",\") {{\n\
\t\t\t\ti++;\n\
\t\t\t}}\n\
\t\t\tchars = inner.slice(start, i);\n\
\t\t\tisNull = chars.length === 0;\n\
\t\t}}\n\
\t\tfields.push(isNull ? null : chars);\n\
\t\tif (i < n && inner[i] === \",\") {{\n\
\t\t\ti++;\n\
\t\t\tcontinue;\n\
\t\t}}\n\
\t\tbreak;\n\
\t}}\n\
\treturn fields;\n\
}}"
    )
}

fn ts_encode_composite_helper(composite: &CompositeInfo, naming: &scythe_backend::naming::NamingConfig) -> String {
    let fields = composite
        .fields
        .iter()
        .map(|field| format!("${{encode(value.{})}}", to_camel_case(&field.name)))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "function encode{name}(value: {name} | null): string | null {{\n\
\tif (value === null) return null;\n\
\tconst encode = (field: unknown): string => {{\n\
\t\tif (field === null || field === undefined) return \"\";\n\
\t\tconst text = String(field);\n\
\t\tif (text === \"\" || /[(),\\\"\\\\\\s]/.test(text)) {{\n\
\t\t\treturn `\"${{text.replaceAll(\"\\\\\", \"\\\\\\\\\").replaceAll('\\\"', '\\\"\\\"')}}\"`;\n\
\t\t}}\n\
\t\treturn text;\n\
\t}};\n\
\treturn `({fields})`;\n\
}}",
        name = composite_type_name(&composite.sql_name, naming),
    )
}

/// PostgreSQL's default `bytea` text output is hex (`"\x48656c6c6f"`); decode the digits after
/// the `\x` prefix back into a `Buffer`. Emitted only when a composite has a `bytes` field.
fn ts_parse_composite_bytes_helper(name: &str) -> String {
    format!(
        "function parse{name}Bytes(hex: string): Buffer {{\n\
\t// ~keep PostgreSQL's default bytea text output is hex: \"\\x48656c6c6f\". Decode the hex\n\
\t// digits after the \"\\x\" prefix back into a Buffer.\n\
\treturn Buffer.from(hex.slice(2), \"hex\");\n\
}}"
    )
}

/// PostgreSQL's default `timestamptz` text output uses a space instead of `T` and omits the
/// offset's minutes when they are zero; normalize both before handing the text to the `Date`
/// constructor. Emitted only when a composite has a `datetime_tz` field.
fn ts_parse_composite_offset_datetime_helper(name: &str) -> String {
    format!(
        "function parse{name}OffsetDateTime(raw: string): Date {{\n\
\t// ~keep PostgreSQL's default timestamptz text output uses a space instead of \"T\"\n\
\t// (\"2024-01-15 10:30:00+00\") and omits the offset's minutes when they are zero (\"+00\"\n\
\t// rather than \"+00:00\"); normalize both before parsing.\n\
\tlet s = raw.replace(\" \", \"T\");\n\
\tconst sign = s[s.length - 3];\n\
\tif (sign === \"+\" || sign === \"-\") {{\n\
\t\ts = s + \":00\";\n\
\t}}\n\
\treturn new Date(s);\n\
}}"
    )
}

fn ts_composite_needs_bytes_helper(composite: &CompositeInfo) -> bool {
    composite.fields.iter().any(|f| f.neutral_type == "bytes")
}

fn ts_composite_needs_offset_datetime_helper(composite: &CompositeInfo) -> bool {
    composite.fields.iter().any(|f| f.neutral_type == "datetime_tz")
}

/// The TypeScript expression converting one composite field's raw text token into its declared
/// type. Nullable fields return `null` before conversion, and `field_type` is always the
/// non-null manifest spelling used by static calls and type assertions.
fn ts_composite_field_from_text(
    neutral_type: &str,
    field_type: &str,
    raw: &str,
    nullable: bool,
    composite_name: &str,
) -> String {
    let converted = if neutral_type.strip_prefix("composite::").is_some() {
        format!("parse{field_type}({raw}) as {field_type}")
    } else if neutral_type.starts_with("enum::") {
        format!("{raw} as {field_type}")
    } else {
        match neutral_type {
            "bool" => format!("{raw} === \"t\""),
            "int16" | "int32" | "int64" | "float32" | "float64" => format!("Number({raw})"),
            "bytes" => format!("parse{composite_name}Bytes({raw} as string)"),
            "datetime" => format!("new Date(({raw} as string).replace(\" \", \"T\"))"),
            "datetime_tz" => format!("parse{composite_name}OffsetDateTime({raw} as string)"),
            _ => format!("{raw} as {field_type}"),
        }
    };
    if nullable {
        format!("{raw} === null ? null : {converted}")
    } else {
        converted
    }
}

/// The `field: value` overrides a composite-column read path must splice into an otherwise
/// spread-through row object, for every `composite::` column in `columns`. Empty when the
/// query selects no composite column, so a caller can skip the spread rewrite entirely in the
/// common case.
///
/// ~keep Read raw driver keys under the default shape and generated field names when CamelCasePlugin
/// has already transformed the row. This follows the grouped-query key contract below.
fn ts_composite_field_overrides(
    columns: &[ResolvedColumn],
    var: &str,
    field_case: TsFieldCase,
) -> Vec<(String, String)> {
    columns
        .iter()
        .filter(|c| c.neutral_type.starts_with("composite::"))
        .map(|c| {
            let key = ts_property_key(&c.field_name);
            let driver_key = match field_case {
                TsFieldCase::Snake => &c.name,
                TsFieldCase::Camel => &c.field_name,
            };
            let member = ts_member_access(var, driver_key);
            (key, format!("parse{}({member}) as {}", c.lang_type, c.full_type))
        })
        .collect()
}

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/typescript-kysely.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/typescript-kysely.redshift.toml");
const DEFAULT_MANIFEST_MYSQL: &str = include_str!("../../manifests/typescript-kysely.mysql.toml");
const DEFAULT_MANIFEST_SQLITE: &str = include_str!("../../manifests/typescript-kysely.sqlite.toml");
const DEFAULT_MANIFEST_MSSQL: &str = include_str!("../../manifests/typescript-kysely.mssql.toml");

/// TypeScript codegen backend targeting [Kysely](https://kysely.dev)'s `sql`
/// template tag.
///
/// Unlike the other TypeScript backends, this one is **dialect-parameterised,
/// not driver-parameterised**: the generated functions take a `Kysely<DB>`
/// instance (generic, defaulting to `any`) and execute through
/// `sql\`...\`.execute(db)`. Kysely's query compiler renders the placeholder
/// syntax the *connected* dialect expects ($1, ?, @1, ...) at compile time,
/// from the same generated call site — so one backend covers every built-in
/// Kysely dialect (PostgreSQL, Redshift, MySQL, SQLite, MSSQL) and every
/// third-party dialect (wasm-sqlite, node:sqlite, PGlite, ...) without
/// scythe knowing they exist. The `engine` passed to
/// [`TypescriptKyselyBackend::new`] only selects the scalar type manifest
/// (matching what the underlying driver naturally returns), never the SQL
/// placeholder text: scythe's analyzer already normalizes every engine's
/// native placeholder syntax (`$N` for PostgreSQL/Redshift, bare `?` for
/// MySQL/SQLite, and `@pN` rewritten to `?` for MSSQL — see
/// `convert_mssql_placeholders` in `scythe-core`) down to positional
/// placeholders before any backend sees the SQL, so a single interpolation
/// pass handles all five engines identically. Redshift is wire-compatible
/// with PostgreSQL over `pg` (Kysely's `PostgresDialect` works against it
/// unchanged), so it only needs its own scalar manifest — `super` and
/// `geometry` — not a distinct code path.
pub struct TypescriptKyselyBackend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
    /// When true, only emit type definitions (interfaces/Zod schemas, enums,
    /// composites) — no query functions, and no `kysely` import (nothing in
    /// the file calls `sql` or takes a `Kysely`/`QueryExecutorProvider`
    /// parameter once query functions are suppressed).
    structs_only: bool,
    /// Which key the `:grouped` fold reads each column by.
    ///
    /// This backend adds no remap of its own — under `Camel` it is the
    /// caller's `CamelCasePlugin` that has already renamed the driver's keys
    /// by the time generated code runs (see `apply_options`). The fold is
    /// the one place that reads a key by name rather than handing Kysely's
    /// row straight back, so it has to follow the same declaration: raw SQL
    /// name under `Snake`, `field_name` under `Camel`.
    field_case: TsFieldCase,
}

impl TypescriptKyselyBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            "mysql" | "mariadb" => DEFAULT_MANIFEST_MYSQL,
            "sqlite" | "sqlite3" => DEFAULT_MANIFEST_SQLITE,
            "mssql" => DEFAULT_MANIFEST_MSSQL,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!(
                        "typescript-kysely only supports PostgreSQL/Redshift/MySQL/MariaDB/SQLite/MSSQL, got engine '{}'",
                        engine
                    ),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
        Ok(Self {
            manifest,
            row_type: TsRowType::default(),
            outer_join_unions: false,
            structs_only: false,
            field_case: TsFieldCase::default(),
        })
    }

    /// Build the file header, importing the `Kysely` type only when something
    /// in the file actually needs it.
    fn header_with_kysely_import(&self, needs_kysely: bool) -> String {
        let mut header = self.camel_case_plugin_note();
        if self.structs_only {
            if self.row_type == TsRowType::Zod {
                header.push_str("import { z } from \"zod\";\n");
            }
            return header;
        }
        let kysely_type = if needs_kysely { "type Kysely, " } else { "" };
        let _ = writeln!(
            header,
            "import {{ {kysely_type}type QueryExecutorProvider, sql }} from \"kysely\";"
        );
        if self.row_type == TsRowType::Zod {
            header.push_str("import { z } from \"zod\";\n");
        }
        header
    }

    /// Warn, in the generated file itself, that `field_case = "camelCase"` on
    /// this backend is a promise the caller has to keep.
    ///
    /// Unlike every other TypeScript backend, kysely does not remap the row --
    /// it indexes the driver row directly, so `field_case` here means "my
    /// Kysely instance already has `CamelCasePlugin` installed" rather than
    /// "rename the fields for me". Scythe cannot verify that at generation
    /// time, and getting it wrong is silent: the declared type says `userId`,
    /// the row carries `user_id`, every field reads back `undefined`, and
    /// `tsc` stays green because the driver row is index-signature typed.
    ///
    /// A comment in the file is the only place this warning reaches someone
    /// reading the generated code rather than the docs.
    fn camel_case_plugin_note(&self) -> String {
        if self.field_case != TsFieldCase::Camel {
            return String::new();
        }
        "// scythe: this file was generated with field_case = \"camelCase\".\n\
         // Kysely does not remap rows -- register CamelCasePlugin on your Kysely\n\
         // instance or every field below reads back undefined at runtime:\n\
         //   new Kysely({ dialect, plugins: [new CamelCasePlugin()] })\n"
            .to_string()
    }
}

/// Rewrite scythe's canonical positional placeholders (`$N` or bare `?`) to
/// Kysely `sql` tag JS interpolations (`${expr}`), where `expr` is the JS
/// expression that provides the Nth bound value.
///
/// This is the crux of the dialect-agnostic design: Kysely's `sql` tag turns
/// every `${}` interpolation into a bound parameter and lets the connected
/// dialect's query compiler decide how to render it ($1, ?, @1, ...) at
/// execution time. Emitting literal placeholder text here (as the other
/// TypeScript backends do) would hardcode one dialect's syntax into the
/// generated call site and defeat the entire point of targeting Kysely.
///
/// A single pass suffices for every supported engine: `analyzed.sql` (which
/// this receives after [`super::clean_sql_with_optional`]) always carries
/// scythe's canonical placeholder form by the time it reaches a backend —
/// MSSQL's native `@pN` is already rewritten to bare `?` by the core parser,
/// the same form MySQL/SQLite queries use natively, and PostgreSQL keeps
/// `$N`. [`super::rewrite_pg_placeholders`] recognises both.
///
/// `sql` must already have passed through [`escape_ts_template_literal`]
/// before it reaches this function. Escaping afterwards would be wrong: it
/// would also mangle the `${expr}` interpolations this function just
/// inserted (turning a live parameter binding into inert escaped text), and
/// it would still miss any `${` that was *already* live because the
/// escaping pass ran too late. Escaping first guarantees only characters
/// that came from the user's SQL are ever touched.
fn interpolate_kysely_params(sql: &str, exprs: &[String]) -> String {
    super::rewrite_pg_placeholders(sql, |n| {
        let idx = n.saturating_sub(1) as usize;
        match exprs.get(idx) {
            Some(expr) => format!("${{{}}}", expr),
            None => format!("${{p{n}}}"),
        }
    })
}

fn kysely_param_expr(
    analyzed: &AnalyzedQuery,
    param: &ResolvedParam,
    member: String,
    naming: &scythe_backend::naming::NamingConfig,
) -> String {
    analyzed
        .params
        .iter()
        .find(|candidate| candidate.name == param.name)
        .and_then(|candidate| candidate.neutral_type.strip_prefix("composite::"))
        .map_or(member.clone(), |sql_name| {
            format!("encode{}({member})", composite_type_name(sql_name, naming))
        })
}

/// Emit a `:batch` function body that reuses the caller's transaction instead
/// of always opening a new one.
///
/// `Transaction<DB> extends Kysely<DB>` in Kysely's type hierarchy, so
/// `db.transaction()` throws at runtime if `db` is already a transaction --
/// a caller who composes this batch helper inside their own transaction
/// would otherwise crash. Kysely exposes `db.isTransaction` precisely to
/// guard against this. `run`'s parameter is typed as the wider `Kysely<DB>`
/// (not `Transaction<DB>`) so it can be called directly with `db` when it is
/// already a transaction, or with the `Transaction<DB>` that
/// `db.transaction().execute` opens -- both satisfy `Kysely<DB>` -- with no
/// unsafe cast either way.
fn write_batch_transaction_body(out: &mut String, loop_open: &str, sql_text: &str) {
    let _ = writeln!(out, "\tconst run = async (trx: Kysely<DB>) => {{");
    let _ = writeln!(out, "\t\t{loop_open}");
    let _ = writeln!(out, "\t\t\tawait sql`{sql_text}`.execute(trx);");
    let _ = writeln!(out, "\t\t}}");
    let _ = writeln!(out, "\t}};");
    let _ = writeln!(out, "\tif (db.isTransaction) {{");
    let _ = writeln!(out, "\t\tawait run(db);");
    let _ = writeln!(out, "\t}} else {{");
    let _ = writeln!(out, "\t\tawait db.transaction().execute(run);");
    let _ = writeln!(out, "\t}}");
    let _ = write!(out, "}}");
}

impl CodegenBackend for TypescriptKyselyBackend {
    fn name(&self) -> &str {
        "typescript-kysely"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "redshift", "mysql", "mariadb", "sqlite", "mssql"]
    }

    fn file_header(&self) -> String {
        self.header_with_kysely_import(true)
    }

    /// Only `:batch` needs the `Kysely` type -- every other command takes the
    /// narrower `QueryExecutorProvider`. Importing `Kysely` regardless leaves an
    /// unused import in the common case where a project has no batch query.
    fn file_header_for_results(&self, generated: &[GeneratedCode]) -> String {
        let needs_kysely = generated
            .iter()
            .filter_map(|code| code.query_fn.as_deref())
            .any(|fragment| fragment.contains("Kysely<"));
        self.header_with_kysely_import(needs_kysely)
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        if self.row_type == TsRowType::Zod {
            if self.outer_join_unions {
                return Ok(generate_zod_union_row_struct(struct_name, query_name, columns));
            }
            return Ok(generate_zod_row_struct(struct_name, query_name, columns));
        }
        if self.outer_join_unions {
            return Ok(generate_ts_union_row_struct(struct_name, query_name, columns, None));
        }
        Ok(generate_ts_interface_row_struct(struct_name, query_name, columns))
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        if self.structs_only {
            return Ok(String::new());
        }

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let cleaned = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        // Escape the user's SQL before any of scythe's own `${}` bindings
        // are interpolated into it (see `interpolate_kysely_params` doc).
        let escaped = escape_ts_template_literal(&cleaned);
        let exprs: Vec<String> = params
            .iter()
            .map(|param| kysely_param_expr(analyzed, param, param.field_name.clone(), &self.manifest.naming))
            .collect();
        let sql_text = interpolate_kysely_params(&escaped, &exprs);

        // ~keep Query/exec commands only ever call `.execute(db)` through the `sql`
        // tag, which needs nothing more than `QueryExecutorProvider` (see
        // Kysely's `raw-builder.d.ts`): any object exposing `getExecutor()`
        // works, whether that's a `Kysely` instance, an open `Transaction`,
        // a `ControlledTransaction`, or a caller's own adapter. That's wider
        // than `Kysely<DB>` -- and since nothing here depends on `DB`, these
        // signatures drop the `<DB = any>` generic entirely rather than
        // carry a type parameter that does nothing.
        //
        // ~keep `:batch` is the one exception: it calls `db.transaction()` and
        // reads `db.isTransaction`, both `Kysely`/`Transaction`-specific, so
        // it keeps the narrower `Kysely<DB>` parameter and `<DB = any>`.
        let write_fn_sig = |out: &mut String, name: &str, generic: &str, sig_params: &[(String, String)], ret: &str| {
            let params_inline = sig_params
                .iter()
                .map(|(n, t)| format!("{n}: {t}"))
                .collect::<Vec<_>>()
                .join(", ");
            let oneliner = format!("export async function {name}{generic}({params_inline}): {ret} {{");
            if oneliner.len() <= 80 {
                let _ = writeln!(out, "{oneliner}");
            } else {
                let _ = writeln!(out, "export async function {name}{generic}(");
                for (n, t) in sig_params {
                    let _ = writeln!(out, "\t{n}: {t},");
                }
                let _ = writeln!(out, "): {ret} {{");
            }
        };

        let query_sig_params: Vec<(String, String)> =
            std::iter::once(("db".to_string(), "QueryExecutorProvider".to_string()))
                .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
                .collect();

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "/** Fetch a single {}. */", struct_name);
                let ret = format!("Promise<{}>", struct_name);
                write_fn_sig(&mut out, &func_name, "", &query_sig_params, &ret);
                let _ = writeln!(
                    out,
                    "\tconst result = await sql<{}>`{}`.execute(db);",
                    struct_name, sql_text
                );
                let _ = writeln!(out, "\tconst row = result.rows[0];");
                let _ = writeln!(out, "\tif (row === undefined) {{");
                let _ = writeln!(out, "\t\t{}", ts_row_not_found_throw(&analyzed.name));
                let _ = writeln!(out, "\t}}");
                let overrides = ts_composite_field_overrides(columns, "row", self.field_case);
                if overrides.is_empty() {
                    let _ = writeln!(out, "\treturn row;");
                } else {
                    let _ = writeln!(out, "\treturn {{");
                    let _ = writeln!(out, "\t\t...row,");
                    for (key, expr) in &overrides {
                        let _ = writeln!(out, "\t\t{key}: {expr},");
                    }
                    let _ = writeln!(out, "\t}};");
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Opt => {
                let _ = writeln!(out, "/** Fetch a single {} or null. */", struct_name);
                let ret = format!("Promise<{} | null>", struct_name);
                write_fn_sig(&mut out, &func_name, "", &query_sig_params, &ret);
                let _ = writeln!(
                    out,
                    "\tconst result = await sql<{}>`{}`.execute(db);",
                    struct_name, sql_text
                );
                let overrides = ts_composite_field_overrides(columns, "row", self.field_case);
                if overrides.is_empty() {
                    let _ = writeln!(out, "\treturn result.rows[0] ?? null;");
                } else {
                    let _ = writeln!(out, "\tconst row = result.rows[0];");
                    let _ = writeln!(out, "\tif (row === undefined) {{");
                    let _ = writeln!(out, "\t\treturn null;");
                    let _ = writeln!(out, "\t}}");
                    let _ = writeln!(out, "\treturn {{");
                    let _ = writeln!(out, "\t\t...row,");
                    for (key, expr) in &overrides {
                        let _ = writeln!(out, "\t\t{key}: {expr},");
                    }
                    let _ = writeln!(out, "\t}};");
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "/** Fetch all {} rows. */", struct_name);
                let ret = format!("Promise<{}[]>", struct_name);
                write_fn_sig(&mut out, &func_name, "", &query_sig_params, &ret);
                let _ = writeln!(
                    out,
                    "\tconst result = await sql<{}>`{}`.execute(db);",
                    struct_name, sql_text
                );
                let overrides = ts_composite_field_overrides(columns, "row", self.field_case);
                if overrides.is_empty() {
                    let _ = writeln!(out, "\treturn result.rows;");
                } else {
                    let _ = writeln!(out, "\treturn result.rows.map((row) => ({{");
                    let _ = writeln!(out, "\t\t...row,");
                    for (key, expr) in &overrides {
                        let _ = writeln!(out, "\t\t{key}: {expr},");
                    }
                    let _ = writeln!(out, "\t}}));");
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                if params.len() > 1 {
                    let params_type_name = format!("{}BatchParams", struct_name);
                    let item_exprs: Vec<String> = params
                        .iter()
                        .map(|param| {
                            kysely_param_expr(
                                analyzed,
                                param,
                                ts_member_access("item", &param.field_name),
                                &self.manifest.naming,
                            )
                        })
                        .collect();
                    let batch_sql = interpolate_kysely_params(&escaped, &item_exprs);

                    let _ = writeln!(out, "/** Params for {} batch operation. */", struct_name);
                    let _ = writeln!(out, "export interface {} {{", params_type_name);
                    for p in params {
                        let _ = writeln!(out, "\t{}: {};", ts_property_key(&p.field_name), p.full_type);
                    }
                    let _ = writeln!(out, "}}");
                    let _ = writeln!(out);
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("db".to_string(), "Kysely<DB>".to_string()),
                        ("items".to_string(), format!("{params_type_name}[]")),
                    ];
                    write_fn_sig(
                        &mut out,
                        &batch_fn_name,
                        "<DB = any>",
                        &batch_sig_params,
                        "Promise<void>",
                    );
                    write_batch_transaction_body(&mut out, "for (const item of items) {", &batch_sql);
                } else if params.len() == 1 {
                    let item = kysely_param_expr(analyzed, &params[0], "item".to_string(), &self.manifest.naming);
                    let batch_sql = interpolate_kysely_params(&escaped, &[item]);

                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("db".to_string(), "Kysely<DB>".to_string()),
                        ("items".to_string(), format!("{}[]", params[0].full_type)),
                    ];
                    write_fn_sig(
                        &mut out,
                        &batch_fn_name,
                        "<DB = any>",
                        &batch_sig_params,
                        "Promise<void>",
                    );
                    write_batch_transaction_body(&mut out, "for (const item of items) {", &batch_sql);
                } else {
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("db".to_string(), "Kysely<DB>".to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    write_fn_sig(
                        &mut out,
                        &batch_fn_name,
                        "<DB = any>",
                        &batch_sig_params,
                        "Promise<void>",
                    );
                    write_batch_transaction_body(&mut out, "for (let i = 0; i < count; i++) {", &sql_text);
                }
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "/** Execute a query returning no rows. */");
                write_fn_sig(&mut out, &func_name, "", &query_sig_params, "Promise<void>");
                let _ = writeln!(out, "\tawait sql`{}`.execute(db);", sql_text);
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "/** Execute a query and return the number of affected rows. */");
                write_fn_sig(&mut out, &func_name, "", &query_sig_params, "Promise<number>");
                let _ = writeln!(out, "\tconst result = await sql`{}`.execute(db);", sql_text);
                let _ = writeln!(out, "\treturn Number(result.numAffectedRows ?? 0n);");
                let _ = write!(out, "}}");
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
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
        if self.row_type == TsRowType::Zod {
            return Ok(generate_zod_grouped_structs(
                child_struct_name,
                parent_struct_name,
                parent_columns,
                child_columns,
            ));
        }
        Ok(generate_grouped_interface_structs(
            child_struct_name,
            parent_struct_name,
            parent_columns,
            child_columns,
        ))
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

        if self.structs_only {
            return Ok(String::new());
        }

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let cleaned = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let escaped = escape_ts_template_literal(&cleaned);
        let exprs: Vec<String> = params
            .iter()
            .map(|param| kysely_param_expr(analyzed, param, param.field_name.clone(), &self.manifest.naming))
            .collect();
        let sql_text = interpolate_kysely_params(&escaped, &exprs);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        // ~keep Same rationale as `generate_query_fn`: grouped fetches only ever
        // `.execute(db)` through the `sql` tag, so `db` takes the minimal
        // `QueryExecutorProvider` interface instead of `Kysely<DB>`, and the
        // now-unused `<DB = any>` generic is dropped.
        let inline_params = if params.is_empty() {
            "db: QueryExecutorProvider".to_string()
        } else {
            format!("db: QueryExecutorProvider, {}", param_list)
        };
        let ret = format!("Promise<{parent_struct_name}[]>");

        let oneliner = format!("export async function {func_name}({inline_params}): {ret} {{");
        if oneliner.len() <= 80 {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "{oneliner}");
        } else {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "export async function {func_name}(");
            let _ = writeln!(out, "\tdb: QueryExecutorProvider,");
            for p in params {
                let _ = writeln!(out, "\t{}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): {ret} {{");
        }

        let _ = writeln!(
            out,
            "\tconst {{ rows: flatRows }} = await sql<Record<string, unknown>>`{sql_text}`.execute(db);"
        );

        // The flat row carries both parent and child columns with no
        // generated struct describing that shape, so each field read casts
        // through the `unknown` index-signature value the sql tag's fallback
        // row type supplies (matches the mssql/duckdb bracket+cast pattern).
        //
        // The key this reads by is the key the *driver* produced, which is
        // not the same thing as the field name generated code declares:
        //
        // - Default (`Snake`): nothing has touched the result, so `flatRows`
        //   is keyed by the raw SQL column name, exactly as written in the
        //   SELECT. It must be read by `col.name`. `col.field_name` is
        //   ~keep *not* a safe stand-in: it is `to_snake_case(name)`, which is
        //   only the identity for already-snake_case SQL. Kysely ships mssql
        //   and mysql manifests where PascalCase columns are idiomatic, so
        //   `SELECT o.OrderId` would be read as `row['order_id']` -- a key
        //   the row does not have, `undefined` at runtime, and no `tsc`
        //   error because `flatRows` is index-signature typed.
        // - `field_case = "camelCase"`: the caller asserts CamelCasePlugin
        //   is registered on their `Kysely` instance. That plugin transforms
        //   the result of every query executed through the instance --
        //   including this raw `sql` tag query, not just the query builder
        //   -- so `flatRows` already has camelCase keys before this code
        //   runs, and the read must follow with `col.field_name`.
        //
        // Either way the write side stays `col.field_name`, which
        // `generate_ts_row_object_literal` supplies, so the fold's output
        // matches the declared parent/child structs in both modes.
        let driver_key_for = |raw_name: &str| -> String {
            match self.field_case {
                TsFieldCase::Snake => raw_name.to_string(),
                TsFieldCase::Camel => all_columns
                    .iter()
                    .find(|c| c.name == raw_name)
                    .map_or_else(|| raw_name.to_string(), |c| c.field_name.clone()),
            }
        };
        let fold = generate_ts_grouped_fold_body(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
            false,
            |name, ty| {
                let key = driver_key_for(name);
                let member = ts_index_access("row", &key);
                if let Some(col) = all_columns.iter().find(|c| c.name == name)
                    && col.neutral_type.starts_with("composite::")
                {
                    return format!("parse{}({member}) as {ty}", col.lang_type);
                }
                format!("{member} as {ty}")
            },
        );
        out.push_str(&fold);
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        if self.row_type == TsRowType::Zod {
            return Ok(generate_zod_enum(&type_name, &enum_info.values, &self.manifest.naming));
        }
        let mut out = String::new();
        let values_name = format!("{}Values", type_name);
        let _ = writeln!(out, "export const {} = {{", values_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "\t{}: \"{}\",", variant, value);
        }
        let _ = writeln!(out, "}} as const;");
        let _ = writeln!(out);
        let _ = write!(
            out,
            "export type {} = typeof {}[keyof typeof {}];",
            type_name, values_name, values_name
        );
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "/** Composite type {}. */", composite.sql_name);
        let _ = writeln!(out, "export interface {} {{", name);
        // ~keep board #204: a composite with zero fields cannot exist in PostgreSQL
        // (`CREATE TYPE ... AS ()` is rejected), so there is no reachable runtime value that
        // would need `parse{name}` here.
        if composite.fields.is_empty() {
            let _ = write!(out, "}}");
            return Ok(out);
        }
        let mut field_types: Vec<String> = Vec::with_capacity(composite.fields.len());
        for field in &composite.fields {
            let ts_type = resolve_type(&field.neutral_type, &self.manifest, field.nullable)
                .map(|t| t.into_owned())
                .map_err(|e| {
                    ScytheError::new(ErrorCode::InternalError, format!("composite field type error: {}", e))
                })?;
            let _ = writeln!(out, "\t{}: {};", ts_property_key(&to_camel_case(&field.name)), ts_type);
            field_types.push(ts_type);
        }
        let _ = writeln!(out, "}}");

        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "// ~keep board #204: whatever driver this wraps has no adapter for a user-defined"
        );
        let _ = writeln!(
            out,
            "// composite -- it hands back the raw text form as a plain string."
        );
        let _ = writeln!(out, "export function parse{name}(raw: unknown): {name} | null {{");
        let _ = writeln!(out, "\tif (raw === null || raw === undefined) {{");
        let _ = writeln!(out, "\t\treturn null;");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tconst f = parse{name}Fields(raw as string);");
        let _ = writeln!(out, "\treturn {{");
        for (i, field) in composite.fields.iter().enumerate() {
            let raw = format!("f[{i}]");
            let conversion_type = resolve_type(&field.neutral_type, &self.manifest, false)
                .map(|t| t.into_owned())
                .map_err(|e| {
                    ScytheError::new(ErrorCode::InternalError, format!("composite field type error: {}", e))
                })?;
            let value_expr =
                ts_composite_field_from_text(&field.neutral_type, &conversion_type, &raw, field.nullable, &name);
            let _ = writeln!(
                out,
                "\t\t{}: {value_expr},",
                ts_property_key(&to_camel_case(&field.name))
            );
        }
        let _ = writeln!(out, "\t}};");
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);
        out.push_str(&ts_parse_composite_fields_helper(&name));
        if ts_composite_needs_bytes_helper(composite) {
            let _ = writeln!(out);
            let _ = writeln!(out);
            out.push_str(&ts_parse_composite_bytes_helper(&name));
        }
        if ts_composite_needs_offset_datetime_helper(composite) {
            let _ = writeln!(out);
            let _ = writeln!(out);
            out.push_str(&ts_parse_composite_offset_datetime_helper(&name));
        }
        let _ = writeln!(out);
        let _ = writeln!(out);
        out.push_str(&ts_encode_composite_helper(composite, &self.manifest.naming));
        Ok(out)
    }

    fn generate_nested_struct_def(
        &self,
        nested: &scythe_core::analyzer::NestedStructInfo,
    ) -> Result<Option<String>, ScytheError> {
        if !self.manifest.types.containers.contains_key("json_nested") {
            return Ok(None);
        }
        Ok(generate_ts_nested_interface_with_field_case(nested, &self.manifest, self.field_case).ok())
    }

    fn generate_composite_def_for_nested(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let mut out = self.generate_composite_def(composite)?;
        if self.manifest.types.containers.contains_key("json_nested") {
            out.push_str("\n\n");
            out.push_str(&generate_ts_json_composite_interface_with_field_case(
                composite,
                &self.manifest,
                self.field_case,
            )?);
        }
        Ok(out)
    }

    fn apply_options(&mut self, options: &std::collections::HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(
            &["row_type", "outer_join_unions", "structs_only", "field_case"],
            options,
        )?;

        if let Some(value) = options.get("row_type") {
            self.row_type = TsRowType::from_option(value)?;
        }
        if let Some(value) = options.get("outer_join_unions") {
            self.outer_join_unions = parse_bool_option("outer_join_unions", value)?;
        }
        if let Some(value) = options.get("structs_only") {
            self.structs_only = parse_bool_option("structs_only", value)?;
        }
        if let Some(value) = options.get("field_case") {
            // No runtime remap here, unlike the driver backends: this one
            // always hands back Kysely's own result rows for `:one`/`:many`.
            // `field_case` on kysely means "the caller's Kysely instance
            // already has CamelCasePlugin installed" -- the driver, via the
            // plugin, has done the remap before any generated code runs.
            //
            // Two things still have to be written. `naming.field_case` is
            // what makes the central rename in `resolve.rs` produce
            // camelCase field names in the declared row struct at all; and
            // `self.field_case` is what tells the `:grouped` fold which key
            // the plugin left on the driver's row (see
            // `generate_grouped_query_fn`). Accepting the option via
            // `reject_unknown_options` above and writing neither would make
            // it a certified no-op -- exactly the trap `field_case` was
            // already deleted once for being.
            self.field_case = TsFieldCase::from_option(value)?;
            self.manifest.naming.field_case = value.clone();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{TsFieldCase, TypescriptKyselyBackend, ts_composite_field_overrides};
    use crate::GeneratedCode;
    use crate::backend_trait::CodegenBackend;
    use scythe_core::analyzer::{
        AnalyzedColumn, AnalyzedParam, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, GroupByConfig,
        NestedFieldInfo, NestedStructInfo,
    };
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

    fn make_one_query(sql: &str, params: Vec<AnalyzedParam>) -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = "GetUserById".to_string();
            aq.command = QueryCommand::One;
            aq.sql = sql.to_string();
            aq.columns = vec![
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
            aq.params = params;
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = None;
            aq.custom = vec![];
        })
    }

    /// Only `:batch` emits a `Kysely<DB>` parameter; every other command takes
    /// `QueryExecutorProvider`. A project with no batch query used to get an
    /// unused `type Kysely` import in every generated file.
    #[test]
    fn test_header_omits_the_kysely_import_when_nothing_needs_it() {
        let backend = TypescriptKyselyBackend::new("postgresql").expect("backend");
        let generated = vec![GeneratedCode {
            query_fn: Some("export async function f(db: QueryExecutorProvider) {}".to_string()),
            row_struct: None,
            model_struct: None,
            enum_def: None,
            nested_struct_defs: Vec::new(),
            ..Default::default()
        }];

        let header = backend.file_header_for_results(&generated);

        assert!(
            !header.contains("type Kysely"),
            "nothing in the file needs Kysely; got:\n{header}"
        );
        assert!(header.contains("type QueryExecutorProvider"), "got:\n{header}");
    }

    #[test]
    fn test_header_imports_kysely_when_a_batch_fn_needs_it() {
        let backend = TypescriptKyselyBackend::new("postgresql").expect("backend");
        let generated = vec![GeneratedCode {
            query_fn: Some("export async function fBatch<DB = any>(db: Kysely<DB>) {}".to_string()),
            row_struct: None,
            model_struct: None,
            enum_def: None,
            nested_struct_defs: Vec::new(),
            ..Default::default()
        }];

        let header = backend.file_header_for_results(&generated);

        assert!(
            header.contains("type Kysely,"),
            "a batch fn needs the Kysely type; got:\n{header}"
        );
    }

    #[test]
    fn test_engine_selects_manifest_and_rejects_unsupported() {
        assert!(TypescriptKyselyBackend::new("postgresql").is_ok());
        assert!(TypescriptKyselyBackend::new("redshift").is_ok());
        assert!(TypescriptKyselyBackend::new("mysql").is_ok());
        assert!(TypescriptKyselyBackend::new("mariadb").is_ok());
        assert!(TypescriptKyselyBackend::new("sqlite").is_ok());
        assert!(TypescriptKyselyBackend::new("mssql").is_ok());
        assert!(TypescriptKyselyBackend::new("oracle").is_err());
    }

    /// Redshift is wire-compatible with PostgreSQL over `pg`, so the
    /// backend must accept it via `supported_engines()` too (used by
    /// `get_backend`'s engine-compatibility check), and its manifest must
    /// carry the Redshift-only scalars (`super`, `geometry`) that the
    /// PostgreSQL manifest doesn't have.
    #[test]
    fn test_redshift_engine_is_supported_and_uses_redshift_scalars() {
        use crate::backend_trait::CodegenBackend;

        let backend = TypescriptKyselyBackend::new("redshift").unwrap();
        assert!(backend.supported_engines().contains(&"redshift"));

        let manifest = backend.manifest();
        assert_eq!(
            manifest.types.scalars.get("super").map(String::as_str),
            Some("Record<string, unknown>"),
            "redshift manifest must map the SUPER scalar"
        );
        assert_eq!(
            manifest.types.scalars.get("geometry").map(String::as_str),
            Some("string"),
            "redshift manifest must map the GEOMETRY scalar"
        );

        let postgres_manifest = TypescriptKyselyBackend::new("postgresql").unwrap();
        assert!(
            !postgres_manifest.manifest().types.scalars.contains_key("super"),
            "the plain postgresql manifest must not carry redshift-only scalars"
        );
    }

    #[test]
    fn test_query_fn_uses_sql_tag_with_js_interpolation_not_dialect_placeholders() {
        let backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let query = make_one_query(
            "SELECT id, name FROM users WHERE id = $1",
            vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            }],
        );
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("db: QueryExecutorProvider"),
            "must accept the minimal QueryExecutorProvider Kysely's sql tag actually needs \
             (Kysely, Transaction, ControlledTransaction, or a caller's own adapter all satisfy it), \
             not the narrower Kysely<DB>; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("<DB = any>") && !query_fn.contains("Kysely<DB>"),
            "the DB generic is unused once db is QueryExecutorProvider and must be dropped, not left vestigial; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("sql<GetUserByIdRow>`SELECT id, name FROM users WHERE id = ${id}`.execute(db)"),
            "must interpolate params through the sql tag, not emit dialect placeholder text; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("$1"),
            "must not leak the postgres placeholder into the sql tag; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains(
                "const row = result.rows[0];\n\tif (row === undefined) {\n\t\tthrow new Error(\"no row found for query: GetUserById\");\n\t}\n\treturn row;"
            ),
            "`:one` must throw on a missing row, not return null; got:\n{query_fn}"
        );
    }

    /// Bare `?` covers MySQL and SQLite queries natively, and MSSQL too: the
    /// core parser rewrites `@pN` down to bare `?` before any backend sees
    /// the SQL (see `convert_mssql_placeholders` in scythe-core), so the
    /// same interpolation pass must handle all three.
    #[test]
    fn test_query_fn_interpolates_bare_placeholders_for_mysql_sqlite_and_mssql() {
        // (engine, bool, decimal, datetime, json) -- the manifests genuinely
        // diverge on these scalars per driver (e.g. sqlite has no native
        // boolean or Date, mssql's driver returns JSON as a raw string), so
        // asserting only the placeholder substring (as this test used to)
        // left the whole per-engine manifest selection unverified.
        let expected_scalars = [
            ("mysql", "number", "string", "Date", "Record<string, unknown>"),
            ("sqlite", "number", "number", "string", "Record<string, unknown>"),
            ("mssql", "boolean", "string", "Date", "string"),
        ];

        for (engine, bool_ty, decimal_ty, datetime_ty, json_ty) in expected_scalars {
            let backend = TypescriptKyselyBackend::new(engine).unwrap();

            let manifest = backend.manifest();
            assert_eq!(
                manifest.types.scalars.get("bool").map(String::as_str),
                Some(bool_ty),
                "engine {engine} bool scalar mismatch"
            );
            assert_eq!(
                manifest.types.scalars.get("decimal").map(String::as_str),
                Some(decimal_ty),
                "engine {engine} decimal scalar mismatch"
            );
            assert_eq!(
                manifest.types.scalars.get("datetime").map(String::as_str),
                Some(datetime_ty),
                "engine {engine} datetime scalar mismatch"
            );
            assert_eq!(
                manifest.types.scalars.get("json").map(String::as_str),
                Some(json_ty),
                "engine {engine} json scalar mismatch"
            );

            let query = make_one_query(
                "SELECT id, name FROM users WHERE id = ?",
                vec![AnalyzedParam {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    position: 1,
                    source_relation: None,
                }],
            );
            let result = crate::generate_with_backend(&query, &backend).unwrap();
            let query_fn = result.query_fn.as_deref().unwrap();
            assert!(
                query_fn.contains("WHERE id = ${id}`.execute(db)"),
                "engine {engine} must interpolate bare '?' placeholders too; got:\n{query_fn}"
            );
        }
    }

    /// Mysql/MariaDB idiomatically backtick-quotes identifiers
    /// (`` `users`.`id` ``). Unescaped, that backtick would terminate the
    /// `sql` tag's template literal early and corrupt the generated file.
    #[test]
    fn test_query_fn_escapes_user_backtick_in_sql() {
        let backend = TypescriptKyselyBackend::new("mysql").unwrap();
        let query = make_one_query("SELECT `id`, `name` FROM `users`", vec![]);
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"SELECT \`id\`, \`name\` FROM \`users\`"),
            "user backticks must be escaped; got:\n{query_fn}"
        );
    }

    /// A literal `${` in the user's SQL (e.g. inside a string literal) must
    /// not become a live JS interpolation — and inside the Kysely `sql` tag
    /// specifically, a live `${}` is a *parameter binding*, so an unescaped
    /// literal `${` would silently corrupt bind positions.
    #[test]
    fn test_query_fn_escapes_user_dollar_brace_but_keeps_own_param_bindings_live() {
        let backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let query = make_one_query(
            "SELECT id FROM users WHERE name = 'literal ${not_a_binding}' AND id = $1",
            vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            }],
        );
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"'literal \${not_a_binding}'"),
            "user's literal ${{}} must be escaped inert text; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("AND id = ${id}"),
            "scythe's own param binding must stay a live interpolation; got:\n{query_fn}"
        );
    }

    /// A backslash in the user's SQL (e.g. a Postgres escape string) must be
    /// doubled so it stays a single literal backslash in the generated JS
    /// string, rather than escaping whatever character follows it.
    #[test]
    fn test_query_fn_escapes_user_backslash_in_sql() {
        let backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let query = make_one_query(r"SELECT id FROM users WHERE name = E'a\\b'", vec![]);
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"E'a\\\\b'"),
            "user backslash must be doubled; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_exec_result_reads_num_affected_rows() {
        let backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let mut query = make_one_query(
            "DELETE FROM orders WHERE user_id = $1",
            vec![AnalyzedParam {
                name: "user_id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            }],
        );
        query.command = QueryCommand::ExecRows;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("Number(result.numAffectedRows ?? 0n)"),
            "got:\n{query_fn}"
        );
    }

    #[test]
    fn test_batch_uses_kysely_transaction() {
        let backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let mut query = make_one_query(
            "INSERT INTO users (name) VALUES ($1)",
            vec![AnalyzedParam {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            }],
        );
        query.command = QueryCommand::Batch;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("db: Kysely<DB>") && query_fn.contains("<DB = any>"),
            ":batch calls db.transaction()/db.isTransaction, both Kysely/Transaction-specific, so it must \
             keep the narrower Kysely<DB> parameter (unlike the widened QueryExecutorProvider commands); got:\n{query_fn}"
        );
        assert!(query_fn.contains("db.transaction().execute(run)"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("sql`INSERT INTO users (name) VALUES (${item}"),
            "got:\n{query_fn}"
        );
        assert!(query_fn.contains(".execute(trx)"), "got:\n{query_fn}");
    }

    /// `Transaction<DB> extends Kysely<DB>`, so a caller who already holds a
    /// transaction and passes it as `db` would hit a runtime throw on
    /// `db.transaction()` (Kysely forbids nesting transactions this way).
    /// The generated batch function must check `db.isTransaction` and reuse
    /// the caller's transaction directly instead of always opening a new
    /// one -- for every arity of the batch body (multi-param struct,
    /// single-param, and the zero-param `count` loop).
    #[test]
    fn test_batch_reuses_existing_transaction_instead_of_nesting() {
        let backend = TypescriptKyselyBackend::new("postgresql").unwrap();

        let mut multi_query = make_one_query(
            "INSERT INTO users (name, email) VALUES ($1, $2)",
            vec![
                AnalyzedParam {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    position: 1,
                    source_relation: None,
                },
                AnalyzedParam {
                    name: "email".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    position: 2,
                    source_relation: None,
                },
            ],
        );
        multi_query.command = QueryCommand::Batch;
        let multi_result = crate::generate_with_backend(&multi_query, &backend).unwrap();
        let multi_fn = multi_result.query_fn.as_deref().unwrap();
        assert!(
            multi_fn.contains("if (db.isTransaction) {")
                && multi_fn.contains("await run(db);")
                && multi_fn.contains("} else {")
                && multi_fn.contains("await db.transaction().execute(run);"),
            "multi-param batch must reuse an existing transaction via db.isTransaction; got:\n{multi_fn}"
        );
        assert!(
            !multi_fn.contains("db.transaction().execute(async (trx) => {"),
            "must not unconditionally open a nested transaction; got:\n{multi_fn}"
        );

        let mut single_query = make_one_query(
            "INSERT INTO users (name) VALUES ($1)",
            vec![AnalyzedParam {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            }],
        );
        single_query.command = QueryCommand::Batch;
        let single_result = crate::generate_with_backend(&single_query, &backend).unwrap();
        let single_fn = single_result.query_fn.as_deref().unwrap();
        assert!(
            single_fn.contains("if (db.isTransaction) {") && single_fn.contains("await run(db);"),
            "single-param batch must reuse an existing transaction via db.isTransaction; got:\n{single_fn}"
        );

        // Zero-param batch (count-based loop).
        let mut count_query = make_one_query("DELETE FROM sessions WHERE expired = true", vec![]);
        count_query.command = QueryCommand::Batch;
        let count_result = crate::generate_with_backend(&count_query, &backend).unwrap();
        let count_fn = count_result.query_fn.as_deref().unwrap();
        assert!(
            count_fn.contains("if (db.isTransaction) {") && count_fn.contains("await run(db);"),
            "count-based batch must reuse an existing transaction via db.isTransaction; got:\n{count_fn}"
        );
        assert!(
            count_fn.contains("for (let i = 0; i < count; i++) {"),
            "got:\n{count_fn}"
        );
    }

    #[test]
    fn test_grouped_typescript_kysely_structs() {
        let backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("interface GetUsersWithOrdersChildRow"),
            "missing child; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("order_id: number"),
            "child missing order_id; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("interface GetUsersWithOrdersRow"),
            "missing parent; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("children: GetUsersWithOrdersChildRow[]"),
            "missing children; got:\n{row_struct}"
        );
    }

    #[test]
    fn test_grouped_typescript_kysely_query_fn() {
        let backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("getUsersWithOrders"),
            "missing fn name; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Promise<GetUsersWithOrdersRow[]>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("sql<Record<string, unknown>>`") && query_fn.contains(".execute(db)"),
            "must use the sql tag; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("row['id'] as number"),
            "must cast through the unknown row shape; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("new Map<unknown, GetUsersWithOrdersRow>()"),
            "must use Map; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("parent.children.push"),
            "must fold children; got:\n{query_fn}"
        );
    }

    /// The whole point of the note is that scythe cannot check the caller
    /// actually registered the plugin, so the warning has to ride along in
    /// the generated file. If it only appeared under `field_case`, and only
    /// when structs are not suppressed, a `structs_only` project would lose
    /// it -- so both shapes are pinned.
    #[test]
    fn should_emit_camel_case_plugin_note_only_under_camel_case() {
        let plain = TypescriptKyselyBackend::new("postgresql").unwrap();
        assert!(
            !plain.file_header().contains("CamelCasePlugin"),
            "the default snake_case config needs no plugin, so it must not warn about one"
        );

        for structs_only in [false, true] {
            let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
            let mut options = std::collections::HashMap::from([("field_case".to_string(), "camelCase".to_string())]);
            if structs_only {
                options.insert("structs_only".to_string(), "true".to_string());
            }
            backend.apply_options(&options).unwrap();

            let header = backend.file_header();
            assert!(
                header.contains("CamelCasePlugin"),
                "field_case = camelCase must warn (structs_only = {structs_only}); got:\n{header}"
            );
            assert!(
                header.contains("undefined"),
                "the warning has to state the consequence, not just name the plugin; got:\n{header}"
            );
        }
    }

    /// CamelCasePlugin, when registered on the caller's `Kysely` instance,
    /// transforms the result of every query executed through it --
    /// including this raw `sql` tag query, not just the query builder -- so
    /// with the plugin installed, `flatRows` already has camelCase keys
    /// before this generated code ever runs. Reading a field by its raw SQL
    /// name (`row['order_id']`) would look up a key the transformed row
    /// doesn't have and silently read `undefined`; reading by `field_name`
    /// (`row['orderId']`) is what the plugin's declared contract
    /// (`field_case = "camelCase"`) actually requires.
    #[test]
    fn test_grouped_fold_reads_by_field_name_under_camel_case() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        // Through `apply_options`, not a direct field poke: this is what
        // proves a real manifest setting `field_case = "camelCase"` can
        // actually reach the fold, not just that the fold logic works in
        // isolation once `naming.field_case` happens to be set some other
        // way.
        backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("row['orderId']"),
            "must read the camelCase field name; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("row['orderDate']"),
            "must read the camelCase field name; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("row['order_id']") && !query_fn.contains("row['order_date']"),
            "must not read the raw SQL name once field_case renamed it; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_composite_row_override_reads_camel_case_driver_key() {
        use crate::backend_trait::ResolvedColumn;

        let columns = vec![ResolvedColumn {
            name: "home_address".to_string(),
            field_name: "homeAddress".to_string(),
            lang_type: "WidgetAddress".to_string(),
            full_type: "WidgetAddress".to_string(),
            neutral_type: "composite::widget_address".to_string(),
            sql_type: "widget_address".to_string(),
            nullable: false,
            join_group: None,
            nullable_before_join: false,
        }];

        assert_eq!(
            ts_composite_field_overrides(&columns, "row", TsFieldCase::Camel),
            vec![(
                "homeAddress".to_string(),
                "parseWidgetAddress(row.homeAddress) as WidgetAddress".to_string(),
            )]
        );
    }

    fn make_grouped_query_with_pascal_case_columns() -> AnalyzedQuery {
        let mut query = make_grouped_query();
        let rename = |name: &str| match name {
            "order_id" => "OrderId".to_string(),
            "order_date" => "OrderDate".to_string(),
            other => other.to_string(),
        };
        for column in &mut query.columns {
            column.name = rename(&column.name);
        }
        if let Some(group_by) = query.group_by.as_mut() {
            for column in &mut group_by.child_columns {
                column.name = rename(&column.name);
            }
        }
        query
    }

    /// This must fail before the fix: the fold read every column by
    /// `col.field_name`, which under the default `field_case` is
    /// `to_snake_case(name)` -- the identity only for already-snake_case
    /// SQL. Kysely ships mssql and mysql manifests where PascalCase columns
    /// are idiomatic, so with no CamelCasePlugin registered the driver hands
    /// back `OrderId` while the fold looked up `row['order_id']`:
    /// `undefined` at every field, silently, with `tsc` green because
    /// `flatRows` is index-signature typed.
    #[test]
    fn test_grouped_fold_reads_the_raw_sql_name_by_default() {
        let backend = TypescriptKyselyBackend::new("mssql").unwrap();
        let query = make_grouped_query_with_pascal_case_columns();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("row['OrderId']") && query_fn.contains("row['OrderDate']"),
            "must read the key the driver actually returns; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("row['order_id']") && !query_fn.contains("row['order_date']"),
            "must not read a snake_cased key the driver never produced; got:\n{query_fn}"
        );
        // The write side still uses the declared field name, so the object
        // built here matches the generated parent/child structs.
        assert!(
            query_fn.contains("order_id: row['OrderId']"),
            "must still write the declared field name; got:\n{query_fn}"
        );
    }

    fn make_one_query_with_snake_case_column() -> AnalyzedQuery {
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

    /// This must fail before the fix: `field_case` was accepted by
    /// `reject_unknown_options` but never written to
    /// `self.manifest.naming.field_case`, so a real manifest setting
    /// `field_case = "camelCase"` had no effect at all -- the declared row
    /// struct stayed snake_case. Exactly the "declared, read by nothing"
    /// trap the option was already deleted once for being.
    #[test]
    fn test_field_case_option_renames_declared_struct_fields() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("userId: number;"),
            "field_case must rename the declared struct field; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains("user_id"),
            "must not leave the raw SQL name in the declaration; got:\n{row_struct}"
        );
    }

    #[test]
    fn test_camel_case_plugin_shapes_nested_json_interface_keys() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .unwrap();
        let nested = NestedStructInfo {
            name: "get_users_as_json_row_payload".to_string(),
            fields: vec![NestedFieldInfo {
                name: "secondary_status".to_string(),
                neutral_type: "string".to_string(),
                nullable: true,
            }],
        };

        let generated = backend
            .generate_nested_struct_def(&nested)
            .unwrap()
            .expect("nested JSON interface");

        assert!(
            generated.contains("secondaryStatus: string | null;"),
            "got:\n{generated}"
        );
        assert!(!generated.contains("secondary_status:"), "got:\n{generated}");
    }

    #[test]
    fn test_camel_case_plugin_shapes_json_composite_interface_keys() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .unwrap();
        let composite = CompositeInfo {
            sql_name: "user_address".to_string(),
            fields: vec![CompositeFieldInfo {
                name: "postal_code".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            }],
        };

        let generated = backend.generate_composite_def_for_nested(&composite).unwrap();

        assert!(generated.contains("postalCode: string;"), "got:\n{generated}");
        assert!(!generated.contains("postal_code: string;"), "got:\n{generated}");
    }

    /// Kysely gets the declared rename only, no runtime remap: `:one`/
    /// `:many` still return Kysely's own result rows directly, trusting the
    /// caller's CamelCasePlugin (asserted by `field_case = "camelCase"`) to
    /// have already produced camelCase keys at runtime.
    #[test]
    fn test_field_case_option_does_not_add_a_runtime_remap_to_one_query() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("sql<GetSessionRow>`"),
            "must still trust Kysely's own generic, no remap; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("const row = result.rows[0];"),
            "must read Kysely's row directly, unmodified; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("throw new Error(\"no row found for query: GetSession\");"),
            "`:one` must throw on a missing row; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_field_case_option_rejects_invalid_value() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "field_case".to_string(),
            "PascalCase".to_string(),
        )]));
        assert!(result.is_err(), "expected 'PascalCase' to be rejected");
    }

    #[test]
    fn test_outer_join_unions_option_applies_to_kysely_row_struct() {
        use crate::backend_trait::ResolvedColumn;

        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "outer_join_unions".to_string(),
                "true".to_string(),
            )]))
            .unwrap();
        assert!(backend.outer_join_unions);

        let columns = vec![
            ResolvedColumn {
                name: "id".to_string(),
                field_name: "id".to_string(),
                lang_type: "number".to_string(),
                full_type: "number".to_string(),
                neutral_type: "int32".to_string(),
                sql_type: "int4".to_string(),
                nullable: false,
                join_group: None,
                nullable_before_join: false,
            },
            ResolvedColumn {
                name: "total".to_string(),
                field_name: "total".to_string(),
                lang_type: "string".to_string(),
                full_type: "string | null".to_string(),
                neutral_type: "decimal".to_string(),
                sql_type: "numeric".to_string(),
                nullable: true,
                join_group: Some("o".to_string()),
                nullable_before_join: false,
            },
            ResolvedColumn {
                name: "notes".to_string(),
                field_name: "notes".to_string(),
                lang_type: "string".to_string(),
                full_type: "string | null".to_string(),
                neutral_type: "string".to_string(),
                sql_type: "text".to_string(),
                nullable: true,
                join_group: Some("o".to_string()),
                nullable_before_join: true,
            },
        ];

        // The independently-nullable flat interface a plain TS backend
        // would emit for the join group.
        let flat = crate::backends::typescript_common::generate_ts_interface_row_struct(
            "GetUserOrdersRow",
            "GetUserOrders",
            &columns,
        );
        assert!(
            flat.contains("total: string | null;") && flat.contains("notes: string | null;"),
            "sanity check on the flat baseline; got:\n{flat}"
        );

        let row_struct = backend.generate_row_struct("GetUserOrders", &columns).unwrap();

        assert_ne!(
            row_struct, flat,
            "outer_join_unions must change the emitted shape, not silently fall through to the flat interface"
        );
        assert!(
            row_struct.contains("export type GetUserOrdersRow = {"),
            "must emit a type alias (interfaces can't express a union), not `export interface`; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("\t| { total: string; notes: string | null }"),
            "matched branch: total ignores its own nullability (lang_type) since the join matched, \
             notes keeps its own nullable full_type because it's nullable even on a match; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("\t| { total: null; notes: null }"),
            "unmatched branch: every projected column in the join group is null together; got:\n{row_struct}"
        );
    }

    /// Before the fix, `row_type = "zod"` returned before the
    /// `outer_join_unions` branch was ever reached, so combining both
    /// options silently discarded the discriminated union and fell back to
    /// the flat Zod schema. This must produce a real `z.union([...])`.
    #[test]
    fn test_zod_row_type_combined_with_outer_join_unions_emits_a_real_union_schema() {
        use crate::backend_trait::ResolvedColumn;

        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("row_type".to_string(), "zod".to_string()),
                ("outer_join_unions".to_string(), "true".to_string()),
            ]))
            .unwrap();

        let columns = vec![
            ResolvedColumn {
                name: "id".to_string(),
                field_name: "id".to_string(),
                lang_type: "number".to_string(),
                full_type: "number".to_string(),
                neutral_type: "int32".to_string(),
                sql_type: "int4".to_string(),
                nullable: false,
                join_group: None,
                nullable_before_join: false,
            },
            ResolvedColumn {
                name: "total".to_string(),
                field_name: "total".to_string(),
                lang_type: "string".to_string(),
                full_type: "string | null".to_string(),
                neutral_type: "decimal".to_string(),
                sql_type: "numeric".to_string(),
                nullable: true,
                join_group: Some("o".to_string()),
                nullable_before_join: false,
            },
            ResolvedColumn {
                name: "notes".to_string(),
                field_name: "notes".to_string(),
                lang_type: "string".to_string(),
                full_type: "string | null".to_string(),
                neutral_type: "string".to_string(),
                sql_type: "text".to_string(),
                nullable: true,
                join_group: Some("o".to_string()),
                nullable_before_join: true,
            },
        ];

        let row_struct = backend.generate_row_struct("GetUserOrders", &columns).unwrap();

        assert!(
            row_struct.contains(".and(z.union(["),
            "must emit a real discriminated union, not the flat schema; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("z.object({ total: z.string(), notes: z.string().nullable() })"),
            "got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("z.object({ total: z.null(), notes: z.null() })"),
            "got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("export type GetUserOrdersRow = z.infer<typeof GetUserOrdersRowSchema>;"),
            "must still infer the row type from the schema; got:\n{row_struct}"
        );
    }

    /// Without a discriminant, Zod + `outer_join_unions` must still fall
    /// back to the flat schema.
    #[test]
    fn test_zod_row_type_combined_with_outer_join_unions_falls_back_without_a_discriminant() {
        use crate::backend_trait::ResolvedColumn;

        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("row_type".to_string(), "zod".to_string()),
                ("outer_join_unions".to_string(), "true".to_string()),
            ]))
            .unwrap();

        let columns = vec![ResolvedColumn {
            name: "id".to_string(),
            field_name: "id".to_string(),
            lang_type: "number".to_string(),
            full_type: "number".to_string(),
            neutral_type: "int32".to_string(),
            sql_type: "int4".to_string(),
            nullable: false,
            join_group: None,
            nullable_before_join: false,
        }];

        let row_struct = backend.generate_row_struct("GetUsers", &columns).unwrap();

        assert!(
            !row_struct.contains(".and("),
            "no union without a discriminant; got:\n{row_struct}"
        );
        assert!(row_struct.contains("z.object({"), "got:\n{row_struct}");
    }

    /// Zod without `outer_join_unions` must be byte-identical to before this
    /// change — a plain flat schema.
    #[test]
    fn test_zod_row_type_without_outer_join_unions_is_unchanged() {
        use crate::backend_trait::ResolvedColumn;

        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "row_type".to_string(),
                "zod".to_string(),
            )]))
            .unwrap();

        let columns = vec![ResolvedColumn {
            name: "id".to_string(),
            field_name: "id".to_string(),
            lang_type: "number".to_string(),
            full_type: "number".to_string(),
            neutral_type: "int32".to_string(),
            sql_type: "int4".to_string(),
            nullable: false,
            join_group: None,
            nullable_before_join: false,
        }];

        let row_struct = backend.generate_row_struct("GetUsers", &columns).unwrap();

        assert_eq!(
            row_struct,
            crate::backends::typescript_common::generate_zod_row_struct("GetUsersRow", "GetUsers", &columns)
        );
    }

    /// An unrecognized value must be reported, not silently treated as
    /// disabling the feature.
    #[test]
    fn test_outer_join_unions_option_rejects_invalid_value() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "outer_join_unions".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    #[test]
    fn test_structs_only_suppresses_query_fn_and_kysely_import() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "structs_only".to_string(),
                "true".to_string(),
            )]))
            .unwrap();

        let query = make_one_query("SELECT id, name FROM users WHERE id = $1", vec![]);
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert_eq!(
            result.query_fn.as_deref(),
            Some(""),
            "structs_only must suppress the query function"
        );
        assert!(
            result
                .row_struct
                .as_deref()
                .unwrap()
                .contains("interface GetUserByIdRow"),
            "row struct must still be emitted"
        );

        // ~keep Neither file_header() nor file_header_for_results() must import
        // anything from "kysely" once nothing in the file needs it.
        assert!(
            !backend.file_header().contains("kysely"),
            "got:\n{}",
            backend.file_header()
        );
        let header_for_results = backend.file_header_for_results(&[GeneratedCode {
            query_fn: Some(String::new()),
            row_struct: result.row_struct.clone(),
            model_struct: None,
            enum_def: None,
            nested_struct_defs: Vec::new(),
            ..Default::default()
        }]);
        assert!(!header_for_results.contains("kysely"), "got:\n{header_for_results}");
    }

    #[test]
    fn test_structs_only_unset_leaves_query_fn_present() {
        let backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let query = make_one_query("SELECT id, name FROM users WHERE id = $1", vec![]);
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert!(
            !result.query_fn.as_deref().unwrap().is_empty(),
            "without structs_only the query function must still be generated"
        );
        assert!(backend.file_header().contains("from \"kysely\""));
    }

    #[test]
    fn test_structs_only_rejects_invalid_value() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    #[test]
    fn test_structs_only_combined_with_zod_row_type_keeps_zod_import_and_schema() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("structs_only".to_string(), "true".to_string()),
                ("row_type".to_string(), "zod".to_string()),
            ]))
            .unwrap();

        let query = make_one_query("SELECT id, name FROM users WHERE id = $1", vec![]);
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert_eq!(result.query_fn.as_deref(), Some(""));
        assert!(
            result.row_struct.as_deref().unwrap().contains("z.object({"),
            "zod schema must still be emitted"
        );

        let header = backend.file_header();
        assert!(header.contains("import { z } from \"zod\";"), "got:\n{header}");
        assert!(!header.contains("kysely"), "got:\n{header}");
    }
}
