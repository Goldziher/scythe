use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, fn_name, to_camel_case, to_pascal_case};
use scythe_backend::types::resolve_type;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_options::reject_unknown_options;
use crate::backend_trait::GroupedQueryFn;
use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};
use crate::backends::typescript_common::{
    TsFieldCase, TsRowShape, TsRowType, escape_ts_template_literal, generate_grouped_interface_structs,
    generate_js_grouped_typedef_structs, generate_js_typedef, generate_js_typedef_row_struct, generate_jsdoc_fn_header,
    generate_ts_grouped_fold_body, generate_ts_interface_row_struct, generate_ts_many_row_remap,
    generate_ts_one_row_remap, generate_ts_union_row_struct, generate_zod_grouped_structs, generate_zod_row_struct,
    generate_zod_union_row_struct, js_fn_signature_line, js_type_cast, parse_bool_option, ts_index_access,
    ts_member_access, ts_property_key, ts_row_not_found_throw,
};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/typescript-better-sqlite3.toml");

pub struct TypescriptBetterSqlite3Backend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
    /// When true, only emit type definitions (interfaces/Zod schemas, enums,
    /// composites) — no query functions, and no `better-sqlite3` driver
    /// import (which would otherwise be unused).
    structs_only: bool,
    /// Under `Camel`, `:one`/`:opt`/`:many` reconstruct the row field by
    /// field from the driver's raw (snake_case) keys instead of trusting a
    /// blind cast -- see [`generate_ts_one_row_remap`]/
    /// [`generate_ts_many_row_remap`]. `Snake` (the default) keeps the
    /// original blind cast, which is sound there.
    field_case: TsFieldCase,
    /// Emit plain, JSDoc-annotated `.js` instead of `.ts` -- the
    /// `javascript-better-sqlite3` registry name (#81). See
    /// `TypescriptPgBackend::js_mode` for the shared rationale. The blind
    /// cast this backend's `Snake` path uses (`stmt.get() as StructName |
    /// undefined`) has no `as` in JSDoc mode; [`js_type_cast`] renders its
    /// `/** @type {T} */ (expr)` equivalent instead.
    js_mode: bool,
}

impl TypescriptBetterSqlite3Backend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "sqlite" | "sqlite3" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!(
                        "typescript-better-sqlite3 only supports SQLite, got engine '{}'",
                        engine
                    ),
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

    /// As [`Self::new`], but selecting the `javascript-better-sqlite3` JSDoc emit mode.
    pub fn new_js(engine: &str) -> Result<Self, ScytheError> {
        let mut backend = Self::new(engine)?;
        backend.js_mode = true;
        Ok(backend)
    }
}

impl CodegenBackend for TypescriptBetterSqlite3Backend {
    fn name(&self) -> &str {
        if self.js_mode {
            "javascript-better-sqlite3"
        } else {
            "typescript-better-sqlite3"
        }
    }

    /// The manifest is shared with `typescript-better-sqlite3` and says `ts`;
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
        &["sqlite"]
    }

    fn file_header(&self) -> String {
        // ~keep See `TypescriptPgBackend::file_header`: `.js` output needs no
        // import at all in JSDoc mode -- the driver type goes straight into
        // the `@param` tag as `import("better-sqlite3").Database`, and Zod
        // is rejected in this mode.
        if self.js_mode {
            // Nothing left for this header to carry: the "do not edit"
            // notice lives in the scythe:provenance line every backend emits.
            return String::new();
        }
        if self.structs_only {
            if self.row_type == TsRowType::Zod {
                return "import { z } from \"zod\";\n".to_string();
            }
            return String::new();
        }
        let mut header = "import type Database from \"better-sqlite3\";\n".to_string();
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

        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let write_fn_sig = |out: &mut String, name: &str, sig_params: &[(String, String)], ret: &str| {
            let params_inline = sig_params
                .iter()
                .map(|(n, t)| format!("{n}: {t}"))
                .collect::<Vec<_>>()
                .join(", ");
            let oneliner = format!("export function {}({}): {} {{", name, params_inline, ret);
            if oneliner.len() <= 80 {
                let _ = writeln!(out, "{}", oneliner);
            } else {
                let _ = writeln!(out, "export function {}(", name);
                for (n, t) in sig_params {
                    let _ = writeln!(out, "\t{}: {},", n, t);
                }
                let _ = writeln!(out, "): {} {{", ret);
            }
        };

        let query_sig_params: Vec<(String, String)> =
            std::iter::once(("db".to_string(), "Database.Database".to_string()))
                .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
                .collect();

        let param_args = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            args.join(", ")
        };

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "/** Fetch a single {}. */", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, struct_name);
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                let get_call = if params.is_empty() {
                    "stmt.get()".to_string()
                } else {
                    format!("stmt.get({})", param_args)
                };
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\tconst row = {} as {} | undefined;", get_call, struct_name);
                        let _ = writeln!(out, "\tif (row === undefined) {{");
                        let _ = writeln!(out, "\t\t{}", ts_row_not_found_throw(&analyzed.name));
                        let _ = writeln!(out, "\t}}");
                        let _ = writeln!(out, "\treturn row;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(
                            out,
                            "\tconst row = {} as Record<string, unknown> | undefined;",
                            get_call
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
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                let get_call = if params.is_empty() {
                    "stmt.get()".to_string()
                } else {
                    format!("stmt.get({})", param_args)
                };
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\tconst row = {} as {} | undefined;", get_call, struct_name);
                        let _ = writeln!(out, "\treturn row ?? null;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(
                            out,
                            "\tconst row = {} as Record<string, unknown> | undefined;",
                            get_call
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
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("db".to_string(), "Database.Database".to_string()),
                        ("items".to_string(), format!("{}[]", params_type_name)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                    let _ = writeln!(
                        out,
                        "\tconst runBatch = db.transaction((items: {}[]) => {{",
                        params_type_name
                    );
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let args: Vec<String> = params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    let _ = writeln!(out, "\t\t\tstmt.run({});", args.join(", "));
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t}});");
                    let _ = writeln!(out, "\trunBatch(items);");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("db".to_string(), "Database.Database".to_string()),
                        ("items".to_string(), format!("{}[]", params[0].full_type)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                    let _ = writeln!(
                        out,
                        "\tconst runBatch = db.transaction((items: {}[]) => {{",
                        params[0].full_type
                    );
                    let _ = writeln!(out, "\t\tfor (const item of items) stmt.run(item);");
                    let _ = writeln!(out, "\t}});");
                    let _ = writeln!(out, "\trunBatch(items);");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("db".to_string(), "Database.Database".to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                    let _ = writeln!(out, "\tconst runBatch = db.transaction((n: number) => {{");
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < n; i++) stmt.run();");
                    let _ = writeln!(out, "\t}});");
                    let _ = writeln!(out, "\trunBatch(count);");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "/** Fetch all {} rows. */", struct_name);
                let ret = format!("{}[]", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                let all_call = if params.is_empty() {
                    "stmt.all()".to_string()
                } else {
                    format!("stmt.all({})", param_args)
                };
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\treturn {} as {}[];", all_call, struct_name);
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(out, "\tconst rows = {} as Record<string, unknown>[];", all_call);
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
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                if params.is_empty() {
                    let _ = writeln!(out, "\tstmt.run();");
                } else {
                    let _ = writeln!(out, "\tstmt.run({});", param_args);
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "/** Execute a query and return the number of affected rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "number");
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                if params.is_empty() {
                    let _ = writeln!(out, "\tconst result = stmt.run();");
                } else {
                    let _ = writeln!(out, "\tconst result = stmt.run({});", param_args);
                }
                let _ = writeln!(out, "\treturn result.changes;");
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
        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let inline_params = if params.is_empty() {
            "db: Database.Database".to_string()
        } else {
            format!("db: Database.Database, {}", param_list)
        };
        let ret = format!("{parent_struct_name}[]");

        let mut out = String::new();
        let oneliner = format!("export function {func_name}({inline_params}): {ret} {{");
        if oneliner.len() <= 80 {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "{oneliner}");
        } else {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "export function {func_name}(");
            let _ = writeln!(out, "\tdb: Database.Database,");
            for p in params {
                let _ = writeln!(out, "\t{}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): {ret} {{");
        }

        let _ = writeln!(out, "\tconst stmt = db.prepare(`{sql}`);");
        if params.is_empty() {
            let _ = writeln!(out, "\tconst flatRows = stmt.all() as Record<string, unknown>[];");
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            let _ = writeln!(
                out,
                "\tconst flatRows = stmt.all({}) as Record<string, unknown>[];",
                args.join(", ")
            );
        }

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
        let name = to_pascal_case(&composite.sql_name);
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
            // Keeps the central rename in `resolve.rs` (which reads this
            // string, not `TsFieldCase`) in sync with the runtime remap
            // decision above -- one option, one source of truth for both.
            self.manifest.naming.field_case = value.clone();
        }

        // ~keep See `TypescriptPgBackend::apply_options` for why these three are
        // rejected outright in JSDoc mode rather than silently ignored.
        if self.js_mode {
            if self.row_type == TsRowType::Zod {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-better-sqlite3 does not support row_type = \"zod\": the inferred `export type X \
                     = z.infer<...>` alias is TypeScript-only syntax a plain .js file cannot carry -- use \
                     typescript-better-sqlite3 for Zod row types"
                        .to_string(),
                ));
            }
            if self.outer_join_unions {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-better-sqlite3 does not support outer_join_unions: the discriminated union is a \
                     TypeScript `type X = A & (B | C)` alias, which plain .js cannot carry -- use \
                     typescript-better-sqlite3"
                        .to_string(),
                ));
            }
            if self.field_case == TsFieldCase::Camel {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-better-sqlite3 does not support field_case = \"camelCase\": the field remap needs \
                     a TypeScript `as T` assertion, which plain .js cannot carry -- use \
                     typescript-better-sqlite3"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl TypescriptBetterSqlite3Backend {
    /// JSDoc-mode counterpart of `generate_query_fn`. better-sqlite3 is
    /// synchronous, so unlike the pg/postgres.js/mysql2 JS-mode functions
    /// these are plain (non-`async`) functions with no `Promise<...>`
    /// wrapper -- exactly mirroring the TypeScript path's sync signatures.
    fn generate_query_fn_js(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        if self.structs_only {
            return Ok(String::new());
        }

        const DB_TYPE: &str = "import(\"better-sqlite3\").Database";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let query_sig_params: Vec<(String, String)> = std::iter::once(("db".to_string(), DB_TYPE.to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();

        let param_args = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            args.join(", ")
        };

        let write_signature = |out: &mut String, description: &str, sig_params: &[(String, String)], ret: &str| {
            out.push_str(&generate_jsdoc_fn_header(description, sig_params, ret));
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", js_fn_signature_line(false, &func_name, sig_params));
        };

        match &analyzed.command {
            QueryCommand::One => {
                write_signature(
                    &mut out,
                    &format!("Fetch a single {}.", struct_name),
                    &query_sig_params,
                    struct_name,
                );
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                let get_call = if params.is_empty() {
                    "stmt.get()".to_string()
                } else {
                    format!("stmt.get({})", param_args)
                };
                let _ = writeln!(
                    out,
                    "\tconst row = {};",
                    js_type_cast(&format!("{} | undefined", struct_name), &get_call)
                );
                let _ = writeln!(out, "\tif (row === undefined) {{");
                let _ = writeln!(out, "\t\t{}", ts_row_not_found_throw(&analyzed.name));
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn row;");
                let _ = write!(out, "}}");
            }
            QueryCommand::Opt => {
                write_signature(
                    &mut out,
                    &format!("Fetch a single {} or null.", struct_name),
                    &query_sig_params,
                    &format!("{} | null", struct_name),
                );
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                let get_call = if params.is_empty() {
                    "stmt.get()".to_string()
                } else {
                    format!("stmt.get({})", param_args)
                };
                let _ = writeln!(
                    out,
                    "\tconst row = {};",
                    js_type_cast(&format!("{} | undefined", struct_name), &get_call)
                );
                let _ = writeln!(out, "\treturn row ?? null;");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many => {
                write_signature(
                    &mut out,
                    &format!("Fetch all {} rows.", struct_name),
                    &query_sig_params,
                    &format!("{}[]", struct_name),
                );
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                let all_call = if params.is_empty() {
                    "stmt.all()".to_string()
                } else {
                    format!("stmt.all({})", param_args)
                };
                let _ = writeln!(
                    out,
                    "\treturn {};",
                    js_type_cast(&format!("{}[]", struct_name), &all_call)
                );
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec => {
                write_signature(
                    &mut out,
                    "Execute a query returning no rows.",
                    &query_sig_params,
                    "void",
                );
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                if params.is_empty() {
                    let _ = writeln!(out, "\tstmt.run();");
                } else {
                    let _ = writeln!(out, "\tstmt.run({});", param_args);
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_signature(
                    &mut out,
                    "Execute a query and return the number of affected rows.",
                    &query_sig_params,
                    "number",
                );
                let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                if params.is_empty() {
                    let _ = writeln!(out, "\tconst result = stmt.run();");
                } else {
                    let _ = writeln!(out, "\tconst result = stmt.run({});", param_args);
                }
                let _ = writeln!(out, "\treturn result.changes;");
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
                        ("db".to_string(), DB_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params_type_name)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!(
                            "Execute {} for each item in the batch within a transaction.",
                            analyzed.name
                        ),
                        &batch_sig_params,
                        "void",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(
                        out,
                        "{}",
                        js_fn_signature_line(false, &batch_fn_name, &batch_sig_params)
                    );
                    let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                    // The callback parameter needs its own JSDoc: `transaction` is generic over
                    // the function it is handed, so with a bare `(items)` TypeScript has nothing
                    // to infer from and resolves the parameter to `never` -- `for (const item of
                    // items)` then fails with TS2488 and the `runBatch(items)` call with TS2345.
                    // The TS emit path above says `(items: T[])` for the same reason; JSDoc mode
                    // has to say it in a comment because the signature cannot carry it.
                    let _ = writeln!(
                        out,
                        "\tconst runBatch = db.transaction(/** @param {{Array<{}>}} items */ (items) => {{",
                        params_type_name
                    );
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let args: Vec<String> = params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    let _ = writeln!(out, "\t\t\tstmt.run({});", args.join(", "));
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t}});");
                    let _ = writeln!(out, "\trunBatch(items);");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let batch_sig_params = vec![
                        ("db".to_string(), DB_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params[0].full_type)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!(
                            "Execute {} for each item in the batch within a transaction.",
                            analyzed.name
                        ),
                        &batch_sig_params,
                        "void",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(
                        out,
                        "{}",
                        js_fn_signature_line(false, &batch_fn_name, &batch_sig_params)
                    );
                    let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                    // See the multi-parameter branch above for why the callback parameter
                    // carries its own `@param`.
                    let _ = writeln!(
                        out,
                        "\tconst runBatch = db.transaction(/** @param {{Array<{}>}} items */ (items) => {{",
                        params[0].full_type
                    );
                    let _ = writeln!(out, "\t\tfor (const item of items) stmt.run(item);");
                    let _ = writeln!(out, "\t}});");
                    let _ = writeln!(out, "\trunBatch(items);");
                    let _ = write!(out, "}}");
                } else {
                    let batch_sig_params = vec![
                        ("db".to_string(), DB_TYPE.to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!(
                            "Execute {} for each item in the batch within a transaction.",
                            analyzed.name
                        ),
                        &batch_sig_params,
                        "void",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(
                        out,
                        "{}",
                        js_fn_signature_line(false, &batch_fn_name, &batch_sig_params)
                    );
                    let _ = writeln!(out, "\tconst stmt = db.prepare(`{}`);", sql);
                    // See the multi-parameter branch above; `n` needs the same treatment.
                    let _ = writeln!(
                        out,
                        "\tconst runBatch = db.transaction(/** @param {{number}} n */ (n) => {{"
                    );
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < n; i++) stmt.run();");
                    let _ = writeln!(out, "\t}});");
                    let _ = writeln!(out, "\trunBatch(count);");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
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

        const DB_TYPE: &str = "import(\"better-sqlite3\").Database";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let sig_params: Vec<(String, String)> = std::iter::once(("db".to_string(), DB_TYPE.to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();
        let ret = format!("{parent_struct_name}[]");

        let mut out = String::new();
        out.push_str(&generate_jsdoc_fn_header(
            &format!("Fetch grouped {} rows.", analyzed.name),
            &sig_params,
            &ret,
        ));
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", js_fn_signature_line(false, &func_name, &sig_params));

        let _ = writeln!(out, "\tconst stmt = db.prepare(`{sql}`);");
        // ~keep better-sqlite3's `.all()` returns `unknown[]` (it cannot know the
        // row shape without a type argument, which JS mode has no syntax
        // for) -- the TS path casts this `as Record<string, unknown>[]`;
        // the JSDoc inline cast is the equivalent here.
        let all_expr = if params.is_empty() {
            "stmt.all()".to_string()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            format!("stmt.all({})", args.join(", "))
        };
        let _ = writeln!(
            out,
            "\tconst flatRows = {};",
            js_type_cast("Array<Record<string, unknown>>", &all_expr)
        );

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
    /// string-literal union type alias (`export type X = "a" | "b";`) with
    /// no backing runtime value -- so unlike
    /// `TypescriptPgBackend::generate_enum_def_js`, there is no `as
    /// const`-guarded object to translate; a bare `@typedef` carries the
    /// same union directly.
    fn generate_enum_def_js(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let variants: Vec<String> = enum_info.values.iter().map(|v| format!("\"{}\"", v)).collect();
        Ok(format!("/** @typedef {{({})}} {} */", variants.join(" | "), type_name))
    }

    /// JSDoc-mode counterpart of `generate_composite_def`.
    fn generate_composite_def_js(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
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
    use super::TypescriptBetterSqlite3Backend;
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
        let mut backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
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
        let mut backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
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
            q.name = "GetUserById".to_string();
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

    /// This must fail before the fix: a blind `as StructName` cast of the
    /// driver's row is unsound once `field_case = "camelCase"` renames the
    /// declared fields -- better-sqlite3 still returns snake_case keys, so
    /// `tsc` reports no error while every field reads back `undefined`.
    #[test]
    fn test_one_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
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
            query_fn.contains("as Record<string, unknown> | undefined"),
            "must not trust a blind cast; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row['user_id'] as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("as GetUserByIdRow | undefined"),
            "must not use the old blind cast under camelCase; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_many_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
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
            query_fn.contains("const rows = stmt.all() as Record<string, unknown>[];"),
            "must not trust a blind cast; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return rows.map((row) => ({"),
            "must map each row; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row['user_id'] as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_one_query_fn_keeps_the_blind_cast_under_the_default_snake_case() {
        let backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("as GetUserByIdRow | undefined"),
            "default field_case must keep the original blind cast unchanged; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("Record<string, unknown>"),
            "must not switch to the remap path under the default; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_query_fn_escapes_user_backtick_in_sql() {
        let backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
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
        let backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
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
        let backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
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
    fn test_grouped_typescript_better_sqlite3_structs() {
        let backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
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
    fn test_grouped_typescript_better_sqlite3_query_fn() {
        let backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            !query_fn.contains("async function"),
            "better-sqlite3 is sync; got:\n{query_fn}"
        );
        assert!(!query_fn.contains("await"), "better-sqlite3 is sync; got:\n{query_fn}");
        assert!(
            query_fn.contains("export function getUsersWithOrders"),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersRow[]"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(query_fn.contains("stmt.all()"), "must call stmt.all; got:\n{query_fn}");
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
    fn test_structs_only_suppresses_query_fn_and_driver_import() {
        let mut backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "structs_only".to_string(),
                "true".to_string(),
            )]))
            .unwrap();

        let query = make_one_query("SELECT id FROM users WHERE id = ?");
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
            !header.contains("better-sqlite3"),
            "the unused better-sqlite3 driver import must be dropped; got:\n{header}"
        );
    }

    #[test]
    fn test_structs_only_unset_leaves_query_fn_present() {
        let backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = ?");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert!(
            !result.query_fn.as_deref().unwrap().is_empty(),
            "without structs_only the query function must still be generated"
        );
        assert!(
            backend
                .file_header()
                .contains("import type Database from \"better-sqlite3\";")
        );
    }

    #[test]
    fn test_structs_only_rejects_invalid_value() {
        let mut backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    #[test]
    fn test_structs_only_combined_with_zod_row_type_keeps_zod_import_and_schema() {
        let mut backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("structs_only".to_string(), "true".to_string()),
                ("row_type".to_string(), "zod".to_string()),
            ]))
            .unwrap();

        let query = make_one_query("SELECT id FROM users WHERE id = ?");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert_eq!(result.query_fn.as_deref(), Some(""));
        assert!(
            result.row_struct.as_deref().unwrap().contains("z.object({"),
            "zod schema must still be emitted"
        );

        let header = backend.file_header();
        assert!(header.contains("import { z } from \"zod\";"), "got:\n{header}");
        assert!(!header.contains("better-sqlite3"), "got:\n{header}");
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
        let backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
        let query = make_batch_query(
            "CreateUserAccountRecord",
            "INSERT INTO user_account_record (name, email) VALUES (?, ?)",
            vec![
                AnalyzedParam {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    position: 1,
                },
                AnalyzedParam {
                    name: "email".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    position: 2,
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
            query_fn.contains("runBatch(items);"),
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
        let backend = TypescriptBetterSqlite3Backend::new("sqlite").unwrap();
        let query = make_batch_query(
            "CreateUserAccountRecordPayload",
            "INSERT INTO user_account_record_payload (payload) VALUES (?)",
            vec![AnalyzedParam {
                name: "payload".to_string(),
                neutral_type: "json".to_string(),
                nullable: false,
                position: 1,
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

    // -- javascript-better-sqlite3 (JSDoc emit mode, #81) ---------------------

    fn js_backend() -> TypescriptBetterSqlite3Backend {
        TypescriptBetterSqlite3Backend::new_js("sqlite").unwrap()
    }

    fn resolved_columns_with_a_nullable_and_a_non_nullable_field() -> Vec<crate::backend_trait::ResolvedColumn> {
        use crate::backend_trait::ResolvedColumn;
        vec![
            ResolvedColumn {
                name: "id".to_string(),
                field_name: "id".to_string(),
                lang_type: "number".to_string(),
                full_type: "number".to_string(),
                neutral_type: "int32".to_string(),
                sql_type: "integer".to_string(),
                nullable: false,
                join_group: None,
                nullable_before_join: false,
            },
            ResolvedColumn {
                name: "bio".to_string(),
                field_name: "bio".to_string(),
                lang_type: "string".to_string(),
                full_type: "string | null".to_string(),
                neutral_type: "string".to_string(),
                sql_type: "text".to_string(),
                nullable: true,
                join_group: None,
                nullable_before_join: false,
            },
        ]
    }

    fn query_with_nullable_and_non_nullable_columns(command: QueryCommand) -> AnalyzedQuery {
        AnalyzedQuery::build(|q| {
            q.name = "GetUserById".to_string();
            q.command = command;
            q.sql = "SELECT id, bio FROM users WHERE id = $1".to_string();
            q.columns = vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "bio".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    ..Default::default()
                },
            ];
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

    #[test]
    fn test_js_mode_name_is_javascript_better_sqlite3() {
        assert_eq!(js_backend().name(), "javascript-better-sqlite3");
    }

    #[test]
    fn test_js_mode_file_header_has_no_ts_only_imports() {
        assert_eq!(js_backend().file_header(), "");
    }

    #[test]
    fn test_js_mode_row_struct_emits_nullable_column_as_type_or_null() {
        let backend = js_backend();
        let row_struct = backend
            .generate_row_struct(
                "GetUserById",
                &resolved_columns_with_a_nullable_and_a_non_nullable_field(),
            )
            .unwrap();

        assert!(
            row_struct.contains(" * @property {string | null} bio"),
            "nullable column must be `{{T | null}}`, never optional; got:\n{row_struct}"
        );
        assert!(row_struct.contains(" * @property {number} id"), "got:\n{row_struct}");
        assert!(!row_struct.contains("[bio]"), "{row_struct}");
        assert!(!row_struct.contains("bio?"), "{row_struct}");
    }

    #[test]
    fn test_js_mode_one_query_fn_is_synchronous_plain_js_with_jsdoc_types() {
        let backend = js_backend();
        let query = query_with_nullable_and_non_nullable_columns(QueryCommand::One);
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("@param {import(\"better-sqlite3\").Database} db"),
            "got:\n{query_fn}"
        );
        assert!(query_fn.contains("@param {number} id"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("@returns {GetUserByIdRow}"),
            "`:one` must not be nullable; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export function getUserById(db, id) {"),
            "better-sqlite3 is sync -- no `async`, no type annotations; got:\n{query_fn}"
        );
        assert!(!query_fn.contains("async"), "got:\n{query_fn}");
        assert!(!query_fn.contains("await"), "got:\n{query_fn}");
        assert!(
            !query_fn.contains(" as "),
            "must not use a TS `as` assertion; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("/** @type {GetUserByIdRow | undefined} */ (stmt.get(id))"),
            "the blind cast must use the JSDoc inline-cast form; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains(
                "if (row === undefined) {\n\t\tthrow new Error(\"no row found for query: GetUserById\");\n\t}\n\treturn row;"
            ),
            "`:one` must throw on a missing row, not return null; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("?? null"),
            "`:one` must not return null; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_many_query_fn_returns_array_with_jsdoc_cast() {
        let backend = js_backend();
        let query = query_with_nullable_and_non_nullable_columns(QueryCommand::Many);
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {GetUserByIdRow[]}"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("return /** @type {GetUserByIdRow[]} */ (stmt.all(id));"),
            "got:\n{query_fn}"
        );
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    #[test]
    fn test_js_mode_exec_query_fn_returns_void() {
        let backend = js_backend();
        let mut query = query_with_nullable_and_non_nullable_columns(QueryCommand::Exec);
        query.sql = "DELETE FROM users WHERE id = $1".to_string();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {void}"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("export function getUserById(db, id) {"),
            "got:\n{query_fn}"
        );
    }

    /// Regression (verified against real `tsc --checkJs --strict`):
    /// better-sqlite3's untyped `stmt.all()` returns `unknown[]`, so
    /// `for (const row of flatRows)` in the fold body leaves `row: unknown`
    /// and every property read fails (TS18046) without this cast.
    #[test]
    fn test_js_mode_grouped_query_fn_casts_flat_rows() {
        let backend = js_backend();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("const flatRows = /** @type {Array<Record<string, unknown>>} */ (stmt.all());"),
            "got:\n{query_fn}"
        );
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
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
                .expect_err(&format!("javascript-better-sqlite3 must reject {key} = {value}"));
            assert!(err.to_string().contains("javascript-better-sqlite3"), "{err}");
        }
    }
}
