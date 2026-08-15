use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, fn_name, to_camel_case};
use scythe_backend::types::resolve_type;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};
use crate::backends::typescript_common::{
    TsFieldCase, TsRowShape, TsRowType, escape_ts_template_literal, generate_grouped_interface_structs,
    generate_js_grouped_typedef_structs, generate_js_typedef, generate_js_typedef_row_struct, generate_jsdoc_fn_header,
    generate_ts_grouped_fold_body, generate_ts_interface_row_struct, generate_ts_many_row_remap,
    generate_ts_one_row_remap, generate_ts_union_row_struct, generate_zod_grouped_structs, generate_zod_row_struct,
    generate_zod_union_row_struct, js_fn_signature_line, js_type_cast, parse_bool_option, ts_index_access,
    ts_member_access, ts_property_key, ts_row_not_found_throw,
};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/typescript-snowflake.toml");

pub struct TypescriptSnowflakeBackend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
    /// When true, only emit type definitions (interfaces/Zod schemas, enums,
    /// composites) — no query functions, no `snowflake-sdk` driver import,
    /// and no `normalizeRow` helper (both would otherwise be unused since
    /// only query functions call them).
    structs_only: bool,
    /// Under `Camel`, `:one`/`:opt`/`:many` reconstruct the row field by
    /// field from `normalizeRow`'s lowercased (snake_case) keys instead of
    /// trusting a blind cast -- see [`generate_ts_one_row_remap`]/
    /// [`generate_ts_many_row_remap`]. `Snake` (the default) keeps that
    /// blind cast, which is sound there.
    field_case: TsFieldCase,
    /// Emit plain, JSDoc-annotated `.js` instead of `.ts` -- the
    /// `javascript-snowflake` registry name (#93). See
    /// `TypescriptPgBackend::js_mode` for the shared rationale. Unlike every
    /// other `javascript-*` backend, `file_header` here is *not* just a
    /// type-only import that JSDoc mode can drop entirely: `normalizeRow` is
    /// a runtime function the generated query bodies call, so JS mode keeps
    /// its body (re-typed via a JSDoc block, since a plain `.js` file cannot
    /// carry the TS `(row: Record<string, unknown>): Record<string,
    /// unknown>` signature) and drops only the `import type { Binds,
    /// Connection } from "snowflake-sdk";` line.
    js_mode: bool,
}

impl TypescriptSnowflakeBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "snowflake" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("typescript-snowflake only supports Snowflake, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self {
            manifest,
            row_type: TsRowType::default(),
            outer_join_unions: false,
            structs_only: false,
            field_case: TsFieldCase::default(),
            js_mode: false,
        })
    }

    /// As [`Self::new`], but selecting the `javascript-snowflake` JSDoc emit mode.
    pub fn new_js(engine: &str) -> Result<Self, ScytheError> {
        let mut backend = Self::new(engine)?;
        backend.js_mode = true;
        Ok(backend)
    }
}

/// Rewrite $1, $2, ... positional params to ? for Snowflake.
impl CodegenBackend for TypescriptSnowflakeBackend {
    fn name(&self) -> &str {
        if self.js_mode {
            "javascript-snowflake"
        } else {
            "typescript-snowflake"
        }
    }

    /// The manifest is shared with `typescript-snowflake` and says `ts`;
    /// JSDoc output is plain JavaScript and must land in a `.js` file to be
    /// runnable.
    fn output_extension(&self) -> &str {
        if self.js_mode {
            "js"
        } else {
            &self.manifest.backend.file_extension
        }
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["snowflake"]
    }

    fn file_header(&self) -> String {
        // ~keep See the `js_mode` field doc: unlike every other `javascript-*`
        // backend's `file_header` (which returns `String::new()` -- see
        // `TypescriptPgBackend::file_header`), `normalizeRow` is a runtime
        // helper the generated query bodies actually call, so it has to stay
        // even in JSDoc mode. Only the TS-only `import type { Binds,
        // Connection } from "snowflake-sdk";` line is dropped -- those become
        // `import("snowflake-sdk").Binds` / `import("snowflake-sdk").Connection`
        // JSDoc type queries at each use site instead -- and `normalizeRow`'s
        // signature moves from inline TS parameter/return annotations to a
        // JSDoc block. Zod is rejected in this mode, so `structs_only` is the
        // only other condition this branch needs: with no query function to
        // call it, `normalizeRow` would be unused, exactly like the TS path's
        // `structs_only` branch below drops both the import and the helper.
        if self.js_mode {
            if self.structs_only {
                return String::new();
            }
            return "/**\n * @param {Record<string, unknown>} row\n * @returns {Record<string, unknown>}\n \
                     */\nfunction normalizeRow(row) {\n\treturn Object.fromEntries(\n\t\tObject.entries(row)\
                     .map(([key, value]) => [key.toLowerCase(), value]),\n\t);\n}\n"
                .to_string();
        }
        if self.structs_only {
            if self.row_type == TsRowType::Zod {
                return "import { z } from \"zod\";\n".to_string();
            }
            return String::new();
        }
        let mut header = "import type { Binds, Connection } from \"snowflake-sdk\";\n\nfunction normalizeRow(row: Record<string, unknown>): Record<string, unknown> {\n\treturn Object.fromEntries(\n\t\tObject.entries(row).map(([key, value]) => [key.toLowerCase(), value]),\n\t);\n}\n"
            .to_string();
        if self.row_type == TsRowType::Zod {
            header.push_str("import { z } from \"zod\";\n");
        }
        header
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        if self.js_mode {
            return Ok(generate_js_typedef_row_struct(struct_name, query_name, columns));
        }
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
        if self.js_mode {
            return self.generate_query_fn_js(analyzed, struct_name, params);
        }
        if self.structs_only {
            return Ok(String::new());
        }

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let sql = escape_ts_template_literal(&super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        ));

        let write_fn_sig = |out: &mut String, name: &str, sig_params: &[(String, String)], ret: &str| {
            let params_inline = sig_params
                .iter()
                .map(|(n, t)| format!("{n}: {t}"))
                .collect::<Vec<_>>()
                .join(", ");
            let oneliner = format!("export async function {}({}): Promise<{}> {{", name, params_inline, ret);
            if oneliner.len() <= 80 {
                let _ = writeln!(out, "{}", oneliner);
            } else {
                let _ = writeln!(out, "export async function {}(", name);
                for (n, t) in sig_params {
                    let _ = writeln!(out, "\t{}: {},", n, t);
                }
                let _ = writeln!(out, "): Promise<{}> {{", ret);
            }
        };

        let query_sig_params: Vec<(String, String)> = std::iter::once(("conn".to_string(), "Connection".to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();

        let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
        let binds = format!("[{}] as unknown as Binds", args.join(", "));

        let emit_execute = |out: &mut String, sql: &str, binds: &str, result_var: &str| {
            let _ = writeln!(
                out,
                "\tconst {} = await new Promise<unknown[]>((resolve, reject) => {{",
                result_var
            );
            let _ = writeln!(
                out,
                "\t\tconn.execute({{ sqlText: `{}`, binds: {}, complete: (err, _stmt, rows) => {{",
                sql, binds
            );
            let _ = writeln!(out, "\t\t\tif (err) reject(err);");
            let _ = writeln!(out, "\t\t\telse resolve((rows ?? []).map(normalizeRow));");
            let _ = writeln!(out, "\t\t}}}});");
            let _ = writeln!(out, "\t}});");
        };

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "/** Fetch a single {}. */", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, struct_name);
                match self.field_case {
                    TsFieldCase::Snake => {
                        emit_execute(&mut out, &sql, &binds, "rows");
                        let _ = writeln!(out, "\tif (rows.length === 0) {{");
                        let _ = writeln!(out, "\t\t{}", ts_row_not_found_throw(&analyzed.name));
                        let _ = writeln!(out, "\t}}");
                        let _ = writeln!(out, "\treturn rows[0] as {};", struct_name);
                    }
                    TsFieldCase::Camel => {
                        emit_execute(&mut out, &sql, &binds, "rawRows");
                        let _ = writeln!(
                            out,
                            "\tconst row = (rawRows[0] ?? null) as Record<string, unknown> | null;"
                        );
                        out.push_str(&generate_ts_one_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            &analyzed.command,
                            &analyzed.name,
                            |name, ty| format!("{} as {ty}", ts_index_access("row", name)),
                        ));
                    }
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Opt => {
                let _ = writeln!(out, "/** Fetch a single {} or null. */", struct_name);
                let ret = format!("{} | null", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                match self.field_case {
                    TsFieldCase::Snake => {
                        emit_execute(&mut out, &sql, &binds, "rows");
                        let _ = writeln!(out, "\treturn rows.length > 0 ? (rows[0] as {}) : null;", struct_name);
                    }
                    TsFieldCase::Camel => {
                        emit_execute(&mut out, &sql, &binds, "rawRows");
                        let _ = writeln!(
                            out,
                            "\tconst row = (rawRows[0] ?? null) as Record<string, unknown> | null;"
                        );
                        out.push_str(&generate_ts_one_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            &analyzed.command,
                            &analyzed.name,
                            |name, ty| format!("{} as {ty}", ts_index_access("row", name)),
                        ));
                    }
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                if params.len() > 1 {
                    let params_type_name = format!("{}BatchParams", struct_name);
                    let _ = writeln!(out, "/** Params for {} batch operation. */", struct_name);
                    let _ = writeln!(out, "export interface {} {{", params_type_name);
                    for p in params {
                        let _ = writeln!(out, "\t{}: {};", ts_property_key(&p.field_name), p.full_type);
                    }
                    let _ = writeln!(out, "}}");
                    let _ = writeln!(out);
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("conn".to_string(), "Connection".to_string()),
                        ("items".to_string(), format!("{}[]", params_type_name)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\tawait new Promise<void>((resolve, reject) => {{");
                    let args: Vec<String> = params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    let _ = writeln!(
                        out,
                        "\t\t\tconn.execute({{ sqlText: `{}`, binds: [{}] as unknown as Binds, complete: (err) => err ? reject(err) : resolve() }});",
                        sql,
                        args.join(", ")
                    );
                    let _ = writeln!(out, "\t\t}});");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("conn".to_string(), "Connection".to_string()),
                        ("items".to_string(), format!("{}[]", params[0].full_type)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\tawait new Promise<void>((resolve, reject) => {{");
                    let _ = writeln!(
                        out,
                        "\t\t\tconn.execute({{ sqlText: `{}`, binds: [item] as unknown as Binds, complete: (err) => err ? reject(err) : resolve() }});",
                        sql
                    );
                    let _ = writeln!(out, "\t\t}});");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("conn".to_string(), "Connection".to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tfor (let i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "\t\tawait new Promise<void>((resolve, reject) => {{");
                    let _ = writeln!(
                        out,
                        "\t\t\tconn.execute({{ sqlText: `{}`, complete: (err) => err ? reject(err) : resolve() }});",
                        sql
                    );
                    let _ = writeln!(out, "\t\t}});");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "/** Fetch all {} rows. */", struct_name);
                let ret = format!("{}[]", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                match self.field_case {
                    TsFieldCase::Snake => {
                        emit_execute(&mut out, &sql, &binds, "rows");
                        let _ = writeln!(out, "\treturn rows as {}[];", struct_name);
                    }
                    TsFieldCase::Camel => {
                        emit_execute(&mut out, &sql, &binds, "rawRows");
                        let _ = writeln!(out, "\tconst rows = rawRows as Record<string, unknown>[];");
                        out.push_str(&generate_ts_many_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            |name, ty| format!("{} as {ty}", ts_index_access("row", name)),
                        ));
                    }
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "/** Execute a query returning no rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "void");
                let _ = writeln!(out, "\tawait new Promise<void>((resolve, reject) => {{");
                let _ = writeln!(
                    out,
                    "\t\tconn.execute({{ sqlText: `{}`, binds: {}, complete: (err) => err ? reject(err) : resolve() }});",
                    sql, binds
                );
                let _ = writeln!(out, "\t}});");
                let _ = write!(out, "}}");
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "/** Execute a query and return the number of affected rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "number");
                let _ = writeln!(out, "\tconst count = await new Promise<number>((resolve, reject) => {{");
                let _ = writeln!(
                    out,
                    "\t\tconn.execute({{ sqlText: `{}`, binds: {}, complete: (err, stmt) => {{",
                    sql, binds
                );
                let _ = writeln!(out, "\t\t\tif (err) reject(err);");
                let _ = writeln!(out, "\t\t\telse resolve(stmt?.getNumUpdatedRows() ?? 0);");
                let _ = writeln!(out, "\t\t}}}});");
                let _ = writeln!(out, "\t}});");
                let _ = writeln!(out, "\treturn count;");
                let _ = write!(out, "}}");
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
        if self.js_mode {
            return Ok(generate_js_grouped_typedef_structs(
                child_struct_name,
                parent_struct_name,
                parent_columns,
                child_columns,
            ));
        }
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
        if self.js_mode {
            return self.generate_grouped_query_fn_js(request);
        }
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        if self.structs_only {
            return Ok(String::new());
        }

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql_clean = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let sql = escape_ts_template_literal(&super::rewrite_pg_placeholders(&sql_clean, |_n| "?".to_string()));

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let inline_params = if params.is_empty() {
            "conn: Connection".to_string()
        } else {
            format!("conn: Connection, {}", param_list)
        };
        let ret = format!("Promise<{parent_struct_name}[]>");

        let mut out = String::new();
        let oneliner = format!("export async function {func_name}({inline_params}): {ret} {{");
        if oneliner.len() <= 80 {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "{oneliner}");
        } else {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "export async function {func_name}(");
            let _ = writeln!(out, "\tconn: Connection,");
            for p in params {
                let _ = writeln!(out, "\t{}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): {ret} {{");
        }

        let binds_arg = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            format!(", binds: [{}] as unknown as Binds", args.join(", "))
        };
        // `Record<string, unknown>` rather than `unknown[]`: the fold body below
        // indexes each row by column name, which `unknown` does not permit.
        let _ = writeln!(
            out,
            "\tconst flatRows = await new Promise<Record<string, unknown>[]>((resolve, reject) => {{"
        );
        let _ = writeln!(out, "\t\tconn.execute({{");
        let _ = writeln!(out, "\t\t\tsqlText: `{sql}`,");
        if !binds_arg.is_empty() {
            let _ = writeln!(out, "\t\t\t{},", &binds_arg[2..]);
        }
        let _ = writeln!(out, "\t\t\tcomplete: (err, _stmt, rows) => {{");
        let _ = writeln!(out, "\t\t\t\tif (err) reject(err);");
        let _ = writeln!(out, "\t\t\t\telse resolve((rows ?? []).map(normalizeRow));");
        let _ = writeln!(out, "\t\t\t}},");
        let _ = writeln!(out, "\t\t}});");
        let _ = writeln!(out, "\t}});");

        let fold = generate_ts_grouped_fold_body(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
            false,
            |name, ty| format!("{} as {ty}", ts_index_access("row", name)),
        );
        out.push_str(&fold);
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        if self.js_mode {
            return self.generate_enum_def_js(enum_info);
        }
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        if self.row_type == TsRowType::Zod {
            return Ok(super::typescript_common::generate_zod_enum(
                &type_name,
                &enum_info.values,
                &self.manifest.naming,
            ));
        }
        let mut out = String::new();
        let variants: Vec<String> = enum_info.values.iter().map(|v| format!("\"{}\"", v)).collect();
        let _ = write!(out, "export type {} = {};", type_name, variants.join(" | "));
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        if self.js_mode {
            return self.generate_composite_def_js(composite);
        }
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "/** Composite type {}. */", composite.sql_name);
        let _ = writeln!(out, "export interface {} {{", name);
        for field in &composite.fields {
            let ts_type = resolve_type(&field.neutral_type, &self.manifest, false)
                .map(|t| t.into_owned())
                .map_err(|e| {
                    ScytheError::new(ErrorCode::InternalError, format!("composite field type error: {}", e))
                })?;
            let _ = writeln!(out, "\t{}: {};", ts_property_key(&to_camel_case(&field.name)), ts_type);
        }
        let _ = write!(out, "}}");
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
            self.field_case = TsFieldCase::from_option(value)?;
            self.manifest.naming.field_case = value.clone();
        }

        // ~keep See `TypescriptPgBackend::apply_options` for why these three are
        // rejected outright in JSDoc mode rather than silently ignored.
        if self.js_mode {
            if self.row_type == TsRowType::Zod {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-snowflake does not support row_type = \"zod\": the inferred `export type X = \
                     z.infer<...>` alias is TypeScript-only syntax a plain .js file cannot carry -- use \
                     typescript-snowflake for Zod row types"
                        .to_string(),
                ));
            }
            if self.outer_join_unions {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-snowflake does not support outer_join_unions: the discriminated union is a \
                     TypeScript `type X = A & (B | C)` alias, which plain .js cannot carry -- use \
                     typescript-snowflake"
                        .to_string(),
                ));
            }
            if self.field_case == TsFieldCase::Camel {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-snowflake does not support field_case = \"camelCase\": the field remap needs a \
                     TypeScript `as T` assertion, which plain .js cannot carry -- use typescript-snowflake"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl TypescriptSnowflakeBackend {
    /// JSDoc-mode counterpart of `generate_query_fn`. Snowflake's driver is
    /// callback-based, so every command still wraps `conn.execute(...)` in a
    /// hand-rolled `Promise`, exactly like the TS path -- but a plain `.js`
    /// file cannot spell the TS path's explicit `new Promise<Record<string,
    /// unknown>[]>(...)` / `new Promise<number>(...)` generic, so the row
    /// type here comes from TS's own inference of what `resolve(...)` is
    /// called with instead (`(rows ?? []).map(normalizeRow)` infers
    /// `Record<string, unknown>[]`, since that is `normalizeRow`'s declared
    /// JSDoc return type -- see `TypescriptSnowflakeBackend::file_header`).
    ///
    /// `resolve(undefined)`, not `resolve()`, in the `:exec` and `:batch`
    /// paths: verified against real `tsc --checkJs --strict`, a bare
    /// `resolve()` on an un-annotated `new Promise(...)` fails with TS2810
    /// ("needs a JSDoc hint to produce a `resolve` that can be called without
    /// arguments") -- inference cannot land on `Promise<void>` from zero call
    /// sites the way it lands on `Promise<Record<string, unknown>[]>` from a
    /// `.map(...)` call.
    ///
    /// `binds` routes through an explicit `unknown` hop
    /// (`/** @type {Binds} */ (/** @type {unknown} */ (expr))`), unlike the
    /// row casts below. `Binds = readonly Bind[] | InsertBinds` with `Bind =
    /// string | number | boolean | null` (copied from `snowflake-sdk@3.1.0`'s
    /// `index.d.ts`) does not admit every `ResolvedParam::full_type` this
    /// backend can emit (`Date`, `Buffer`, ...), and verified against real
    /// `tsc --checkJs --strict`, a bound array containing one of those fails
    /// to typecheck as a single-step JSDoc cast exactly as it fails as `as
    /// Binds` -- unlike the row casts below, this is not a case where the
    /// JSDoc spelling is more permissive than `as`, so it needs the same
    /// `unknown` hop the TS path's `as unknown as Binds` uses.
    fn generate_query_fn_js(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        if self.structs_only {
            return Ok(String::new());
        }

        const CONN_TYPE: &str = "import(\"snowflake-sdk\").Connection";
        const BINDS_TYPE: &str = "import(\"snowflake-sdk\").Binds";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let sql = escape_ts_template_literal(&super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |_| "?".to_string(),
        ));

        let query_sig_params: Vec<(String, String)> = std::iter::once(("conn".to_string(), CONN_TYPE.to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();

        let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
        let binds = js_type_cast(BINDS_TYPE, &js_type_cast("unknown", &format!("[{}]", args.join(", "))));

        let write_signature = |out: &mut String, description: &str, sig_params: &[(String, String)], ret: &str| {
            out.push_str(&generate_jsdoc_fn_header(description, sig_params, ret));
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", js_fn_signature_line(true, &func_name, sig_params));
        };

        let emit_execute = |out: &mut String, sql: &str, binds: &str, result_var: &str| {
            let _ = writeln!(
                out,
                "\tconst {} = await new Promise((resolve, reject) => {{",
                result_var
            );
            let _ = writeln!(
                out,
                "\t\tconn.execute({{ sqlText: `{}`, binds: {}, complete: (err, _stmt, rows) => {{",
                sql, binds
            );
            let _ = writeln!(out, "\t\t\tif (err) reject(err);");
            let _ = writeln!(out, "\t\t\telse resolve((rows ?? []).map(normalizeRow));");
            let _ = writeln!(out, "\t\t}}}});");
            let _ = writeln!(out, "\t}});");
        };

        match &analyzed.command {
            QueryCommand::One => {
                write_signature(
                    &mut out,
                    &format!("Fetch a single {}.", struct_name),
                    &query_sig_params,
                    &format!("Promise<{}>", struct_name),
                );
                emit_execute(&mut out, &sql, &binds, "rows");
                let _ = writeln!(out, "\tif (rows.length === 0) {{");
                let _ = writeln!(out, "\t\t{}", ts_row_not_found_throw(&analyzed.name));
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn {};", js_type_cast(struct_name, "rows[0]"));
                let _ = write!(out, "}}");
            }
            QueryCommand::Opt => {
                write_signature(
                    &mut out,
                    &format!("Fetch a single {} or null.", struct_name),
                    &query_sig_params,
                    &format!("Promise<{} | null>", struct_name),
                );
                emit_execute(&mut out, &sql, &binds, "rows");
                let _ = writeln!(
                    out,
                    "\treturn rows.length > 0 ? {} : null;",
                    js_type_cast(struct_name, "rows[0]")
                );
                let _ = write!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                if params.len() > 1 {
                    let params_type_name = format!("{}BatchParams", struct_name);
                    let fields: Vec<(String, String)> = params
                        .iter()
                        .map(|p| (p.field_name.clone(), p.full_type.clone()))
                        .collect();
                    out.push_str(&generate_js_typedef(
                        &params_type_name,
                        &format!("Params for {} batch operation.", struct_name),
                        &fields,
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out);
                    let batch_sig_params = vec![
                        ("conn".to_string(), CONN_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params_type_name)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!("Execute {} for each item in the batch.", analyzed.name),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\tawait new Promise((resolve, reject) => {{");
                    let item_args: Vec<String> =
                        params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    let item_binds = js_type_cast(
                        BINDS_TYPE,
                        &js_type_cast("unknown", &format!("[{}]", item_args.join(", "))),
                    );
                    let _ = writeln!(
                        out,
                        "\t\t\tconn.execute({{ sqlText: `{}`, binds: {}, complete: (err) => err ? reject(err) : \
                         resolve(undefined) }});",
                        sql, item_binds
                    );
                    let _ = writeln!(out, "\t\t}});");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let batch_sig_params = vec![
                        ("conn".to_string(), CONN_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params[0].full_type)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!("Execute {} for each item in the batch.", analyzed.name),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\tawait new Promise((resolve, reject) => {{");
                    let item_binds = js_type_cast(BINDS_TYPE, &js_type_cast("unknown", "[item]"));
                    let _ = writeln!(
                        out,
                        "\t\t\tconn.execute({{ sqlText: `{}`, binds: {}, complete: (err) => err ? reject(err) : \
                         resolve(undefined) }});",
                        sql, item_binds
                    );
                    let _ = writeln!(out, "\t\t}});");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else {
                    let batch_sig_params = vec![
                        ("conn".to_string(), CONN_TYPE.to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!("Execute {} for each item in the batch.", analyzed.name),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\tfor (let i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "\t\tawait new Promise((resolve, reject) => {{");
                    let _ = writeln!(
                        out,
                        "\t\t\tconn.execute({{ sqlText: `{}`, complete: (err) => err ? reject(err) : \
                         resolve(undefined) }});",
                        sql
                    );
                    let _ = writeln!(out, "\t\t}});");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                write_signature(
                    &mut out,
                    &format!("Fetch all {} rows.", struct_name),
                    &query_sig_params,
                    &format!("Promise<{}[]>", struct_name),
                );
                emit_execute(&mut out, &sql, &binds, "rows");
                let _ = writeln!(out, "\treturn {};", js_type_cast(&format!("{}[]", struct_name), "rows"));
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec => {
                write_signature(
                    &mut out,
                    "Execute a query returning no rows.",
                    &query_sig_params,
                    "Promise<void>",
                );
                let _ = writeln!(out, "\tawait new Promise((resolve, reject) => {{");
                let _ = writeln!(
                    out,
                    "\t\tconn.execute({{ sqlText: `{}`, binds: {}, complete: (err) => err ? reject(err) : \
                     resolve(undefined) }});",
                    sql, binds
                );
                let _ = writeln!(out, "\t}});");
                let _ = write!(out, "}}");
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_signature(
                    &mut out,
                    "Execute a query and return the number of affected rows.",
                    &query_sig_params,
                    "Promise<number>",
                );
                let _ = writeln!(out, "\tconst count = await new Promise((resolve, reject) => {{");
                let _ = writeln!(
                    out,
                    "\t\tconn.execute({{ sqlText: `{}`, binds: {}, complete: (err, stmt) => {{",
                    sql, binds
                );
                let _ = writeln!(out, "\t\t\tif (err) reject(err);");
                let _ = writeln!(out, "\t\t\telse resolve(stmt?.getNumUpdatedRows() ?? 0);");
                let _ = writeln!(out, "\t\t}}}});");
                let _ = writeln!(out, "\t}});");
                let _ = writeln!(out, "\treturn count;");
                let _ = write!(out, "}}");
            }
        }

        Ok(out)
    }

    /// JSDoc-mode counterpart of `generate_grouped_query_fn`.
    fn generate_grouped_query_fn_js(&self, request: &GroupedQueryFn<'_>) -> Result<String, ScytheError> {
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        if self.structs_only {
            return Ok(String::new());
        }

        const CONN_TYPE: &str = "import(\"snowflake-sdk\").Connection";
        const BINDS_TYPE: &str = "import(\"snowflake-sdk\").Binds";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql_clean = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let sql = escape_ts_template_literal(&super::rewrite_pg_placeholders(&sql_clean, |_n| "?".to_string()));

        let sig_params: Vec<(String, String)> = std::iter::once(("conn".to_string(), CONN_TYPE.to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();
        let ret = format!("Promise<{parent_struct_name}[]>");

        let mut out = String::new();
        out.push_str(&generate_jsdoc_fn_header(
            &format!("Fetch grouped {} rows.", analyzed.name),
            &sig_params,
            &ret,
        ));
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", js_fn_signature_line(true, &func_name, &sig_params));

        // ~keep Mirrors the TS path immediately below: `binds` is only emitted
        // when there are params, unlike `generate_query_fn_js`'s
        // `emit_execute`, which always includes it (even as an empty array).
        let binds_arg = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            js_type_cast(BINDS_TYPE, &js_type_cast("unknown", &format!("[{}]", args.join(", "))))
        };

        // ~keep No cast needed on the resolved rows themselves: TS inference
        // reads the type off `(rows ?? []).map(normalizeRow)` inside the
        // executor (`Record<string, unknown>[]`, `normalizeRow`'s declared
        // JSDoc return type), the same trick `generate_query_fn_js` uses --
        // the TS path's explicit `new Promise<Record<string, unknown>[]>(...)`
        // generic has no JS-mode spelling, so this is what stands in for it.
        let _ = writeln!(out, "\tconst flatRows = await new Promise((resolve, reject) => {{");
        let _ = writeln!(out, "\t\tconn.execute({{");
        let _ = writeln!(out, "\t\t\tsqlText: `{sql}`,");
        if !binds_arg.is_empty() {
            let _ = writeln!(out, "\t\t\tbinds: {binds_arg},");
        }
        let _ = writeln!(out, "\t\t\tcomplete: (err, _stmt, rows) => {{");
        let _ = writeln!(out, "\t\t\t\tif (err) reject(err);");
        let _ = writeln!(out, "\t\t\t\telse resolve((rows ?? []).map(normalizeRow));");
        let _ = writeln!(out, "\t\t\t}},");
        let _ = writeln!(out, "\t\t}});");
        let _ = writeln!(out, "\t}});");

        let fold = generate_ts_grouped_fold_body(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
            true,
            |name, _ty| ts_index_access("row", name),
        );
        out.push_str(&fold);
        let _ = write!(out, "}}");
        Ok(out)
    }

    /// JSDoc-mode counterpart of `generate_enum_def`.
    ///
    /// The TypeScript path's default (non-Zod) shape here is already just a
    /// string-literal union type alias with no backing runtime value, so a
    /// bare `@typedef` carries the same union directly.
    fn generate_enum_def_js(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let variants: Vec<String> = enum_info.values.iter().map(|v| format!("\"{}\"", v)).collect();
        Ok(format!("/** @typedef {{({})}} {} */", variants.join(" | "), type_name))
    }

    /// JSDoc-mode counterpart of `generate_composite_def`.
    fn generate_composite_def_js(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut fields = Vec::with_capacity(composite.fields.len());
        for field in &composite.fields {
            let ts_type = resolve_type(&field.neutral_type, &self.manifest, false)
                .map(|t| t.into_owned())
                .map_err(|e| {
                    ScytheError::new(ErrorCode::InternalError, format!("composite field type error: {}", e))
                })?;
            fields.push((to_camel_case(&field.name).into_owned(), ts_type));
        }
        Ok(generate_js_typedef(
            &name,
            &format!("Composite type {}.", composite.sql_name),
            &fields,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::TypescriptSnowflakeBackend;
    use crate::backend_trait::CodegenBackend;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    fn discriminated_join_columns() -> Vec<crate::backend_trait::ResolvedColumn> {
        use crate::backend_trait::ResolvedColumn;
        vec![
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
        ]
    }

    #[test]
    fn test_zod_row_type_combined_with_outer_join_unions_emits_a_real_union_schema() {
        let mut backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("row_type".to_string(), "zod".to_string()),
                ("outer_join_unions".to_string(), "true".to_string()),
            ]))
            .unwrap();

        let row_struct = backend
            .generate_row_struct("GetUserOrders", &discriminated_join_columns())
            .unwrap();

        assert!(
            row_struct.contains(".and(z.union(["),
            "must emit a real discriminated union; got:\n{row_struct}"
        );
    }

    #[test]
    fn test_zod_row_type_without_outer_join_unions_is_unchanged() {
        let mut backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "row_type".to_string(),
                "zod".to_string(),
            )]))
            .unwrap();

        let row_struct = backend
            .generate_row_struct("GetUserOrders", &discriminated_join_columns())
            .unwrap();

        assert_eq!(
            row_struct,
            crate::backends::typescript_common::generate_zod_row_struct(
                "GetUserOrdersRow",
                "GetUserOrders",
                &discriminated_join_columns()
            )
        );
    }

    fn make_one_query(sql: &str) -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = "GetUserById".to_string();
            aq.command = QueryCommand::One;
            aq.sql = sql.to_string();
            aq.columns = vec![AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            }];
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

    /// This must fail before the fix: a blind cast of `normalizeRow`'s
    /// output is unsound once `field_case = "camelCase"` renames the
    /// declared fields -- `normalizeRow` only lowercases Snowflake's
    /// uppercase driver keys, it does not camelCase them.
    #[test]
    fn test_one_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
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
            query_fn.contains("as Record<string, unknown> | null"),
            "must not trust a blind cast; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row['user_id'] as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_many_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .unwrap();
        let mut query = make_one_query_with_snake_case_column();
        query.command = QueryCommand::Many;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("const rows = rawRows as Record<string, unknown>[];"),
            "must not trust a blind cast; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row['user_id'] as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_one_query_fn_keeps_the_blind_cast_under_the_default_snake_case() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("rows[0] as GetSessionRow"),
            "default field_case must keep the original blind cast unchanged; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("Record<string, unknown>"),
            "must not switch to the remap path under the default; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_query_fn_escapes_user_backtick_in_sql() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE name = `oops`");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"WHERE name = \`oops\`"),
            "user backtick must be escaped; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_query_fn_escapes_user_dollar_brace_in_sql() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE name = 'literal ${evil}'");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"'literal \${evil}'"),
            "user's literal ${{}} must be escaped; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_query_fn_escapes_user_backslash_in_sql() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = make_one_query(r"SELECT id FROM users WHERE name = 'a\\b'");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"'a\\\\b'"),
            "user backslash must be doubled; got:\n{query_fn}"
        );
    }

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
    fn test_grouped_typescript_snowflake_structs() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("interface GetUsersWithOrdersChildRow"),
            "missing child; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("interface GetUsersWithOrdersRow"),
            "missing parent; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("children: GetUsersWithOrdersChildRow[]"),
            "missing children; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("interface GetUsersWithOrdersRow").unwrap();
        assert!(child_pos < parent_pos, "child must precede parent");
    }

    #[test]
    fn test_grouped_typescript_snowflake_query_fn() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("getUsersWithOrders"), "missing fn; got:\n{query_fn}");
        assert!(
            query_fn.contains("Promise<GetUsersWithOrdersRow[]>"),
            "wrong return type; got:\n{query_fn}"
        );
        // `Record<string, unknown>[]`, never `any[]`: the fold body indexes rows
        // by column name, so the element type has to admit string indexing
        // without reaching for `any`, which the project forbids and
        // `validate_structural` rejects.
        assert!(
            query_fn.contains("new Promise<Record<string, unknown>[]>"),
            "must wrap in Promise; got:\n{query_fn}"
        );
        assert!(!query_fn.contains("any["), "must not emit any[]; got:\n{query_fn}");
        assert!(
            query_fn.contains("(rows ?? []).map(normalizeRow)"),
            "must normalize Snowflake's uppercase column names; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("conn.execute"),
            "must use conn.execute; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("row['id']"),
            "must use bracket access; got:\n{query_fn}"
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

    #[test]
    fn test_typescript_snowflake_uses_scalar_binds_for_single_execution() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "DeleteUser".to_string();
            aq.command = QueryCommand::Exec;
            aq.sql = "DELETE FROM users WHERE id = $1".to_string();
            aq.columns = vec![];
            aq.params = vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            }];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = None;
            aq.custom = vec![];
        });

        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("binds: [id] as unknown as Binds"),
            "must use scalar binds; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("binds: [[id]]"),
            "must not use array binds; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_typescript_snowflake_normalizes_result_column_names() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let header = backend.file_header();

        assert!(
            header.contains("key.toLowerCase()"),
            "must normalize Snowflake column names; got:\n{header}"
        );
    }

    #[test]
    fn test_structs_only_suppresses_query_fn_import_and_normalize_row_helper() {
        let mut backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "structs_only".to_string(),
                "true".to_string(),
            )]))
            .unwrap();

        let query = make_one_query("SELECT id FROM users WHERE id = $1");
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

        let header = backend.file_header();
        assert!(
            !header.contains("snowflake-sdk"),
            "the unused snowflake-sdk driver import must be dropped; got:\n{header}"
        );
        assert!(
            !header.contains("normalizeRow"),
            "the unused normalizeRow helper must be dropped; got:\n{header}"
        );
    }

    #[test]
    fn test_structs_only_unset_leaves_query_fn_present() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = $1");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert!(
            !result.query_fn.as_deref().unwrap().is_empty(),
            "without structs_only the query function must still be generated"
        );
        let header = backend.file_header();
        assert!(header.contains("snowflake-sdk"));
        assert!(header.contains("normalizeRow"));
    }

    #[test]
    fn test_structs_only_rejects_invalid_value() {
        let mut backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    #[test]
    fn test_structs_only_combined_with_zod_row_type_keeps_zod_import_and_schema() {
        let mut backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("structs_only".to_string(), "true".to_string()),
                ("row_type".to_string(), "zod".to_string()),
            ]))
            .unwrap();

        let query = make_one_query("SELECT id FROM users WHERE id = $1");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert_eq!(result.query_fn.as_deref(), Some(""));
        assert!(
            result.row_struct.as_deref().unwrap().contains("z.object({"),
            "zod schema must still be emitted"
        );

        let header = backend.file_header();
        assert!(header.contains("import { z } from \"zod\";"), "got:\n{header}");
        assert!(!header.contains("snowflake-sdk"), "got:\n{header}");
    }

    fn make_batch_query(name: &str, sql: &str, params: Vec<AnalyzedParam>) -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = name.to_string();
            aq.command = QueryCommand::Batch;
            aq.sql = sql.to_string();
            aq.columns = vec![];
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

    /// Regression test: `write_fn_sig`'s wrapped (>80 char) branch used to
    /// discard the `params_inline`/`sig_params` it was given and rebuild the
    /// signature from the outer per-column `params` slice instead. For a
    /// `:batch` function that slice is wrong -- the actual signature takes
    /// `items: XBatchParams[]`, not the raw columns -- so the emitted
    /// function declared parameters that didn't match its body (which
    /// references `items`). This must fail if that fallback is reinstated.
    #[test]
    fn test_batch_signature_wrap_declares_items_not_raw_columns() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = make_batch_query(
            "CreateUserAccountRecord",
            "INSERT INTO user_account_record (name, email) VALUES ($1, $2)",
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

        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("\titems: CreateUserAccountRecordRowBatchParams[],"),
            "wrapped batch signature must declare items with the batch params type; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("\tname: string,") && !query_fn.contains("\temail: string,"),
            "wrapped batch signature must not fall back to the raw per-column params; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("for (const item of items)"),
            "every identifier the body references (items) must be declared in the signature; got:\n{query_fn}"
        );
    }

    /// Regression test guarding against a naive fix that splits
    /// `params_inline` on top-level `", "` to recover individual parameters.
    /// A `json` param's TS type is `Record<string, unknown>`, which itself
    /// contains `", "` -- splitting on it would corrupt a single parameter
    /// into `payload: Record<string` and `unknown>[]`. The structured
    /// `(name, type)` pairs must survive a wrapped signature intact.
    #[test]
    fn test_batch_signature_wrap_preserves_json_param_type_intact() {
        let backend = TypescriptSnowflakeBackend::new("snowflake").unwrap();
        let query = make_batch_query(
            "CreateUserAccountRecordPayload",
            "INSERT INTO user_account_record_payload (payload) VALUES ($1)",
            vec![AnalyzedParam {
                name: "payload".to_string(),
                neutral_type: "json".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            }],
        );

        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("\titems: Record<string, unknown>[],"),
            "the json param's type must survive intact on the items line, not be split \
             on its internal ', '; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("Record<string,\n"),
            "the json type must not be split at its internal comma; got:\n{query_fn}"
        );
    }

    fn js_backend() -> TypescriptSnowflakeBackend {
        TypescriptSnowflakeBackend::new_js("snowflake").unwrap()
    }

    #[test]
    fn test_js_mode_name_is_javascript_snowflake() {
        assert_eq!(js_backend().name(), "javascript-snowflake");
    }

    #[test]
    fn test_js_mode_output_extension_is_js() {
        assert_eq!(js_backend().output_extension(), "js");
    }

    /// Regression: unlike every other `javascript-*` backend, this one's
    /// `file_header` is not empty -- `normalizeRow` is a runtime helper the
    /// generated query bodies call, not a type-only import JSDoc can inline.
    /// Only the TS-only `import type { Binds, Connection }` line drops.
    #[test]
    fn test_js_mode_file_header_keeps_normalize_row_but_drops_ts_import() {
        let header = js_backend().file_header();
        assert!(
            header.contains("function normalizeRow(row) {"),
            "must keep the normalizeRow helper, untyped; got:\n{header}"
        );
        assert!(
            header.contains("@param {Record<string, unknown>} row"),
            "normalizeRow's parameter type must move to a JSDoc block; got:\n{header}"
        );
        assert!(
            !header.contains("import type"),
            "the TS-only import type line must be dropped; got:\n{header}"
        );
        assert!(
            !header.contains("(row: Record<string, unknown>)"),
            "must not keep the inline TS parameter annotation; got:\n{header}"
        );
    }

    #[test]
    fn test_js_mode_row_struct_emits_nullable_column_as_type_or_null() {
        let backend = js_backend();
        let row_struct = backend
            .generate_row_struct("GetSession", &discriminated_join_columns())
            .unwrap();

        assert!(
            row_struct.contains(" * @property {string | null} total"),
            "nullable column must be `{{T | null}}`, never optional; got:\n{row_struct}"
        );
        assert!(row_struct.contains(" * @property {number} id"), "got:\n{row_struct}");
    }

    #[test]
    fn test_js_mode_one_query_fn_is_async_plain_js_with_jsdoc_types() {
        let backend = js_backend();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("@param {import(\"snowflake-sdk\").Connection} conn"),
            "got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("@returns {Promise<GetSessionRow>}"),
            "`:one` must not be nullable; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export async function getSession(conn) {"),
            "no type annotations on a plain .js signature; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains(" as "),
            "must not use a TS `as` assertion; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("binds: /** @type {import(\"snowflake-sdk\").Binds} */ (/** @type {unknown} */ ([])),"),
            "binds must route through the unknown hop, even with no params; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("if (rows.length === 0) {\n\t\tthrow new Error(\"no row found for query: GetSession\");"),
            "`:one` must throw on a missing row; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return /** @type {GetSessionRow} */ (rows[0]);"),
            "the row cast must be a single JSDoc step, not `unknown`-hopped like binds; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_many_query_fn_casts_rows_in_one_step() {
        let backend = js_backend();
        let mut query = make_one_query_with_snake_case_column();
        query.command = QueryCommand::Many;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("@returns {Promise<GetSessionRow[]>}"),
            "got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return /** @type {GetSessionRow[]} */ (rows);"),
            "got:\n{query_fn}"
        );
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    /// Regression: a bare `resolve()` on an un-annotated `new Promise(...)`
    /// fails real `tsc --checkJs --strict` with TS2810, so `:exec` must
    /// resolve with an explicit `undefined`.
    #[test]
    fn test_js_mode_exec_query_fn_resolves_with_explicit_undefined() {
        let backend = js_backend();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "DeleteUser".to_string();
            aq.command = QueryCommand::Exec;
            aq.sql = "DELETE FROM users WHERE id = $1".to_string();
            aq.columns = vec![];
            aq.params = vec![AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            }];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = None;
            aq.custom = vec![];
        });
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {Promise<void>}"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("resolve(undefined) });"),
            "must resolve with an explicit undefined, not a bare resolve(); got:\n{query_fn}"
        );
        assert!(!query_fn.contains("resolve() "), "got:\n{query_fn}");
        assert!(
            query_fn.contains("binds: /** @type {import(\"snowflake-sdk\").Binds} */ (/** @type {unknown} */ ([id])),"),
            "got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_exec_result_query_fn_returns_row_count() {
        let backend = js_backend();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "UpdateUserName".to_string();
            aq.command = QueryCommand::ExecResult;
            aq.sql = "UPDATE users SET name = $1 WHERE id = $2".to_string();
            aq.columns = vec![];
            aq.params = vec![
                AnalyzedParam {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    position: 1,
                    source_relation: None,
                },
                AnalyzedParam {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    position: 2,
                    source_relation: None,
                },
            ];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = None;
            aq.custom = vec![];
        });
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {Promise<number>}"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("resolve(stmt?.getNumUpdatedRows() ?? 0);"),
            "got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_batch_fn_uses_jsdoc_typedef_not_interface() {
        let backend = js_backend();
        let query = make_batch_query(
            "InsertUser",
            "INSERT INTO users (id, name) VALUES ($1, $2)",
            vec![
                AnalyzedParam {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    position: 1,
                    source_relation: None,
                },
                AnalyzedParam {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    position: 2,
                    source_relation: None,
                },
            ],
        );
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("@typedef {object} InsertUserRowBatchParams"),
            "multi-param batch needs a JSDoc typedef, not an interface; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export async function insertUserBatch(conn, items) {"),
            "got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("resolve(undefined) });"),
            "the per-item promise must also resolve with an explicit undefined; got:\n{query_fn}"
        );
        assert!(!query_fn.contains("export interface"), "got:\n{query_fn}");
    }

    #[test]
    fn test_js_mode_grouped_query_fn_has_no_ts_generics() {
        let backend = js_backend();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("export async function getUsersWithOrders(conn) {"),
            "got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("new Promise<"),
            "JSDoc mode cannot spell a Promise generic; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("new Map<"),
            "JSDoc mode cannot spell a Map generic; got:\n{query_fn}"
        );
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
        assert!(
            query_fn.contains("(rows ?? []).map(normalizeRow)"),
            "must still normalize Snowflake's uppercase column names; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("binds:"),
            "a param-less grouped query must omit binds entirely, matching the TS path; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_rejects_zod_row_type_and_camel_case_and_outer_join_unions() {
        for (key, value) in [
            ("row_type", "zod"),
            ("field_case", "camelCase"),
            ("outer_join_unions", "true"),
        ] {
            let mut backend = js_backend();
            let err = backend
                .apply_options(&std::collections::HashMap::from([(key.to_string(), value.to_string())]))
                .expect_err(&format!("javascript-snowflake must reject {key} = {value}"));
            assert!(err.to_string().contains("javascript-snowflake"), "{err}");
        }
    }

    #[test]
    fn test_js_mode_structs_only_drops_normalize_row_helper() {
        let mut backend = js_backend();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "structs_only".to_string(),
                "true".to_string(),
            )]))
            .unwrap();

        assert_eq!(backend.file_header(), "");
    }
}
