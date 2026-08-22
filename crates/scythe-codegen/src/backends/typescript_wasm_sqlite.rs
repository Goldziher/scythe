use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, fn_name, to_camel_case};
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

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/typescript-wasm-sqlite.toml");

/// Codegen backend for `@sqlite.org/sqlite-wasm`'s synchronous OO1 API.
///
/// Unlike `better-sqlite3`, the WASM build runs in-process (browser or Node)
/// behind a one-time async `sqlite3InitModule()` that user code performs
/// before ever calling into generated functions. The `Database` handle these
/// functions receive is already open, and every OO1 method used here
/// (`selectObject`, `selectObjects`, `exec`, `changes`, `transaction`) is
/// fully synchronous — no `async`/`await`/`Promise` appears anywhere in the
/// generated output. See <https://github.com/sqlite/sqlite-wasm>.
pub struct TypescriptWasmSqliteBackend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
    /// When true, only emit type definitions (interfaces/Zod schemas, enums,
    /// composites) — no query functions, and no `@sqlite.org/sqlite-wasm`
    /// driver import (which would otherwise be unused).
    structs_only: bool,
    /// Under `Camel`, `:one`/`:opt`/`:many` reconstruct the row field by
    /// field from the driver's raw (snake_case) keys instead of trusting a
    /// blind cast -- see [`generate_ts_one_row_remap`]/
    /// [`generate_ts_many_row_remap`]. `Snake` (the default) keeps the
    /// original blind cast, which is sound there.
    field_case: TsFieldCase,
    /// Emit plain, JSDoc-annotated `.js` instead of `.ts` -- the
    /// `javascript-wasm-sqlite` registry name (#93). See
    /// `TypescriptPgBackend::js_mode` for the shared rationale. The
    /// `sqlite3InitModule()` one-time async init this driver needs stays
    /// entirely outside generated code either way -- the `Database` handle
    /// generated functions receive is always already open, so this mode
    /// switch changes nothing about that boundary.
    js_mode: bool,
}

impl TypescriptWasmSqliteBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "sqlite" | "sqlite3" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("typescript-wasm-sqlite only supports SQLite, got engine '{}'", engine),
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

    /// As [`Self::new`], but selecting the `javascript-wasm-sqlite` JSDoc emit mode.
    pub fn new_js(engine: &str) -> Result<Self, ScytheError> {
        let mut backend = Self::new(engine)?;
        backend.js_mode = true;
        Ok(backend)
    }
}

impl CodegenBackend for TypescriptWasmSqliteBackend {
    fn name(&self) -> &str {
        if self.js_mode {
            "javascript-wasm-sqlite"
        } else {
            "typescript-wasm-sqlite"
        }
    }

    /// The manifest is shared with `typescript-wasm-sqlite` and says `ts`;
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
        // the `@param` tag as `import("@sqlite.org/sqlite-wasm").Database`,
        // and Zod is rejected in this mode.
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
        let mut header = "import type { Database } from \"@sqlite.org/sqlite-wasm\";\n".to_string();
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

        // Takes structured `(name, type)` pairs rather than a pre-joined
        // string. A naive split of a pre-joined `params_inline` on `", "`
        // would corrupt a `json` param's TS type (`Record<string, unknown>`,
        // which itself contains `", "`) into two broken parameters; deriving
        // both the one-liner and the wrapped form from the same slice avoids
        // that entirely and also keeps this closure honest about wrapping
        // whatever it was actually given -- rather than re-deriving
        // parameters from the enclosing `params` slice, which silently
        // produces the wrong signature whenever the signature isn't the
        // plain per-query parameter list (e.g. the `items: XBatchParams[]` /
        // `count: number` forms used for `:batch` queries).
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

        let query_sig_params: Vec<(String, String)> = std::iter::once(("db".to_string(), "Database".to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();

        let bind_array = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            format!("[{}]", args.join(", "))
        };

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "/** Fetch a single {}. */", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, struct_name);
                let select_call = if params.is_empty() {
                    format!("db.selectObject(`{}`)", sql)
                } else {
                    format!("db.selectObject(`{}`, {})", sql, bind_array)
                };
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(
                            out,
                            "\tconst row = {} as unknown as {} | undefined;",
                            select_call, struct_name
                        );
                        let _ = writeln!(out, "\tif (row === undefined) {{");
                        let _ = writeln!(out, "\t\t{}", ts_row_not_found_throw(&analyzed.name));
                        let _ = writeln!(out, "\t}}");
                        let _ = writeln!(out, "\treturn row;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(
                            out,
                            "\tconst row = {} as unknown as Record<string, unknown> | undefined;",
                            select_call
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
                let select_call = if params.is_empty() {
                    format!("db.selectObject(`{}`)", sql)
                } else {
                    format!("db.selectObject(`{}`, {})", sql, bind_array)
                };
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(
                            out,
                            "\tconst row = {} as unknown as {} | undefined;",
                            select_call, struct_name
                        );
                        let _ = writeln!(out, "\treturn row ?? null;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(
                            out,
                            "\tconst row = {} as unknown as Record<string, unknown> | undefined;",
                            select_call
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
                        ("db".to_string(), "Database".to_string()),
                        ("items".to_string(), format!("{}[]", params_type_name)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tdb.transaction(() => {{");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let args: Vec<String> = params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    let _ = writeln!(out, "\t\t\tdb.exec({{ sql: `{}`, bind: [{}] }});", sql, args.join(", "));
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t}});");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("db".to_string(), "Database".to_string()),
                        ("items".to_string(), format!("{}[]", params[0].full_type)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tdb.transaction(() => {{");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tdb.exec({{ sql: `{}`, bind: [item] }});", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t}});");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("db".to_string(), "Database".to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tdb.transaction(() => {{");
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < count; i++) db.exec(`{}`);", sql);
                    let _ = writeln!(out, "\t}});");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "/** Fetch all {} rows. */", struct_name);
                let ret = format!("{}[]", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                let select_call = if params.is_empty() {
                    format!("db.selectObjects(`{}`)", sql)
                } else {
                    format!("db.selectObjects(`{}`, {})", sql, bind_array)
                };
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\treturn {} as unknown as {}[];", select_call, struct_name);
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(
                            out,
                            "\tconst rows = {} as unknown as Record<string, unknown>[];",
                            select_call
                        );
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
                if params.is_empty() {
                    let _ = writeln!(out, "\tdb.exec(`{}`);", sql);
                } else {
                    let _ = writeln!(out, "\tdb.exec({{ sql: `{}`, bind: {} }});", sql, bind_array);
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "/** Execute a query and return the number of affected rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "number");
                if params.is_empty() {
                    let _ = writeln!(out, "\tdb.exec(`{}`);", sql);
                } else {
                    let _ = writeln!(out, "\tdb.exec({{ sql: `{}`, bind: {} }});", sql, bind_array);
                }
                let _ = writeln!(out, "\treturn db.changes();");
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
            "db: Database".to_string()
        } else {
            format!("db: Database, {}", param_list)
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
            let _ = writeln!(out, "\tdb: Database,");
            for p in params {
                let _ = writeln!(out, "\t{}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): {ret} {{");
        }

        if params.is_empty() {
            let _ = writeln!(
                out,
                "\tconst flatRows = db.selectObjects(`{sql}`) as Record<string, unknown>[];"
            );
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            let _ = writeln!(
                out,
                "\tconst flatRows = db.selectObjects(`{sql}`, [{}]) as Record<string, unknown>[];",
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
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "/** Composite type {}. */", composite.sql_name);
        let _ = writeln!(out, "export interface {} {{", name);
        for field in &composite.fields {
            let ts_type = resolve_type(&field.neutral_type, &self.manifest, field.nullable)
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
                    "javascript-wasm-sqlite does not support row_type = \"zod\": the inferred `export type X = \
                     z.infer<...>` alias is TypeScript-only syntax a plain .js file cannot carry -- use \
                     typescript-wasm-sqlite for Zod row types"
                        .to_string(),
                ));
            }
            if self.outer_join_unions {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-wasm-sqlite does not support outer_join_unions: the discriminated union is a \
                     TypeScript `type X = A & (B | C)` alias, which plain .js cannot carry -- use \
                     typescript-wasm-sqlite"
                        .to_string(),
                ));
            }
            if self.field_case == TsFieldCase::Camel {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-wasm-sqlite does not support field_case = \"camelCase\": the field remap needs a \
                     TypeScript `as T` assertion, which plain .js cannot carry -- use typescript-wasm-sqlite"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl TypescriptWasmSqliteBackend {
    /// JSDoc-mode counterpart of `generate_query_fn`. `@sqlite.org/sqlite-wasm`'s
    /// OO1 API is synchronous, so -- like the TS path -- these are plain
    /// (non-`async`) functions with no `Promise<...>` wrapper, and `:batch`
    /// uses `db.transaction(...)` exactly like the TS path (the driver has the
    /// helper, unlike `node:sqlite`'s `DatabaseSync`).
    ///
    /// Every cast here is a single JSDoc `/** @type {T} */ (expr)` step, unlike
    /// the TS path a few hundred lines up, which routes the same expressions
    /// through `as unknown as`. `db.selectObject(...) as Row | undefined` and
    /// `db.selectObjects(...) as Row[]` are both genuine TS2352s ("neither type
    /// sufficiently overlaps") against the real `@sqlite.org/sqlite-wasm`
    /// declarations (`selectObject`/`selectObjects` return `Record<string,
    /// SqlValue> | undefined` / `Record<string, SqlValue>[]`, copied from
    /// `@sqlite.org/sqlite-wasm@3.53.0-build1`'s `src/index.d.ts`) -- but the
    /// JSDoc spelling of the same assertion is accepted for both, verified
    /// against real `tsc --checkJs --strict`, so JS mode never needs the
    /// `unknown` hop the TS path uses for either the single-row or the array
    /// case.
    fn generate_query_fn_js(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        if self.structs_only {
            return Ok(String::new());
        }

        const DB_TYPE: &str = "import(\"@sqlite.org/sqlite-wasm\").Database";

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

        let bind_array = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            format!("[{}]", args.join(", "))
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
                let select_call = if params.is_empty() {
                    format!("db.selectObject(`{}`)", sql)
                } else {
                    format!("db.selectObject(`{}`, {})", sql, bind_array)
                };
                let _ = writeln!(
                    out,
                    "\tconst row = {};",
                    js_type_cast(&format!("{} | undefined", struct_name), &select_call)
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
                let select_call = if params.is_empty() {
                    format!("db.selectObject(`{}`)", sql)
                } else {
                    format!("db.selectObject(`{}`, {})", sql, bind_array)
                };
                let _ = writeln!(
                    out,
                    "\tconst row = {};",
                    js_type_cast(&format!("{} | undefined", struct_name), &select_call)
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
                let select_call = if params.is_empty() {
                    format!("db.selectObjects(`{}`)", sql)
                } else {
                    format!("db.selectObjects(`{}`, {})", sql, bind_array)
                };
                let _ = writeln!(
                    out,
                    "\treturn {};",
                    js_type_cast(&format!("{}[]", struct_name), &select_call)
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
                if params.is_empty() {
                    let _ = writeln!(out, "\tdb.exec(`{}`);", sql);
                } else {
                    let _ = writeln!(out, "\tdb.exec({{ sql: `{}`, bind: {} }});", sql, bind_array);
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
                if params.is_empty() {
                    let _ = writeln!(out, "\tdb.exec(`{}`);", sql);
                } else {
                    let _ = writeln!(out, "\tdb.exec({{ sql: `{}`, bind: {} }});", sql, bind_array);
                }
                let _ = writeln!(out, "\treturn db.changes();");
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
                    let _ = writeln!(out, "\tdb.transaction(() => {{");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let args: Vec<String> = params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    let _ = writeln!(out, "\t\t\tdb.exec({{ sql: `{}`, bind: [{}] }});", sql, args.join(", "));
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t}});");
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
                    let _ = writeln!(out, "\tdb.transaction(() => {{");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tdb.exec({{ sql: `{}`, bind: [item] }});", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t}});");
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
                    let _ = writeln!(out, "\tdb.transaction(() => {{");
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < count; i++) db.exec(`{}`);", sql);
                    let _ = writeln!(out, "\t}});");
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

        const DB_TYPE: &str = "import(\"@sqlite.org/sqlite-wasm\").Database";

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

        // ~keep `selectObjects` cannot know the row shape without a type
        // argument, which JS mode has no syntax for -- the TS path casts this
        // `as Record<string, unknown>[]` directly (that widening assertion is
        // fine even though the real return type is `Record<string,
        // SqlValue>[]`, unlike the row-interface-array case in
        // `generate_query_fn_js`'s `:many` branch above); the JSDoc inline
        // cast is the equivalent here.
        let select_expr = if params.is_empty() {
            format!("db.selectObjects(`{sql}`)")
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            format!("db.selectObjects(`{sql}`, [{}])", args.join(", "))
        };
        let _ = writeln!(
            out,
            "\tconst flatRows = {};",
            js_type_cast("Array<Record<string, unknown>>", &select_expr)
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
    /// string-literal union type alias with no backing runtime value, so
    /// unlike `TypescriptPgBackend::generate_enum_def_js`, there is no `as
    /// const`-guarded object to translate; a bare `@typedef` carries the same
    /// union directly.
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
            let ts_type = resolve_type(&field.neutral_type, &self.manifest, field.nullable)
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
    use super::TypescriptWasmSqliteBackend;
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
    fn test_engine_rejection_for_non_sqlite() {
        let result = TypescriptWasmSqliteBackend::new("postgresql");
        assert!(result.is_err(), "typescript-wasm-sqlite must reject non-sqlite engines");
    }

    #[test]
    fn test_engine_acceptance_for_sqlite_aliases() {
        assert!(TypescriptWasmSqliteBackend::new("sqlite").is_ok());
        assert!(TypescriptWasmSqliteBackend::new("sqlite3").is_ok());
    }

    #[test]
    fn test_file_header_imports_database_type_from_sqlite_wasm() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let header = backend.file_header();
        assert!(
            header.contains("import type { Database } from \"@sqlite.org/sqlite-wasm\";"),
            "must import the Database type from @sqlite.org/sqlite-wasm; got:\n{header}"
        );
        assert!(!header.contains("zod"), "zod import must be opt-in; got:\n{header}");
    }

    #[test]
    fn test_zod_row_type_combined_with_outer_join_unions_emits_a_real_union_schema() {
        let mut backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
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
        let mut backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
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
            q.sql = "SELECT id, user_id FROM sessions WHERE id = ?".to_string();
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

    /// This must fail before the fix: a blind cast of the driver's row is
    /// unsound once `field_case = "camelCase"` renames the declared fields
    /// -- sqlite-wasm still returns snake_case keys.
    #[test]
    fn test_one_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
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
            query_fn.contains("as unknown as Record<string, unknown> | undefined"),
            "must not trust a blind cast; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row['user_id'] as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_many_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
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
            query_fn.contains("as unknown as Record<string, unknown>[];"),
            "must not trust a blind cast; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row['user_id'] as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_one_query_fn_keeps_the_blind_cast_under_the_default_snake_case() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("as unknown as GetSessionRow | undefined"),
            "default field_case must keep the original blind cast unchanged; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("Record<string, unknown>"),
            "must not switch to the remap path under the default; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_query_fn_escapes_user_backtick_in_sql() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
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
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
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
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_one_query(r"SELECT id FROM users WHERE name = 'a\\b'");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"'a\\\\b'"),
            "user backslash must be doubled; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_one_query_fn_is_synchronous_and_uses_select_object() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = ?");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(!query_fn.contains("async"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("await"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("Promise"), "must be sync; got:\n{query_fn}");
        assert!(
            query_fn.contains("db.selectObject(`"),
            "must use selectObject; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("as unknown as GetUserByIdRow | undefined"),
            "must cast through unknown; got:\n{query_fn}"
        );
    }

    fn make_many_query() -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = "ListUsers".to_string();
            aq.command = QueryCommand::Many;
            aq.sql = "SELECT id FROM users".to_string();
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

    #[test]
    fn test_many_query_fn_is_synchronous_and_uses_select_objects() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_many_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(!query_fn.contains("async"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("await"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("Promise"), "must be sync; got:\n{query_fn}");
        assert!(
            query_fn.contains("db.selectObjects(`"),
            "must use selectObjects; got:\n{query_fn}"
        );
    }

    fn make_exec_query() -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = "DeleteUser".to_string();
            aq.command = QueryCommand::Exec;
            aq.sql = "DELETE FROM users WHERE id = ?".to_string();
            aq.columns = vec![];
            aq.params = vec![scythe_core::analyzer::AnalyzedParam {
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
        })
    }

    #[test]
    fn test_exec_query_fn_is_synchronous_and_uses_exec() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_exec_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(!query_fn.contains("async"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("await"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("Promise"), "must be sync; got:\n{query_fn}");
        assert!(
            query_fn.contains("db.exec({ sql: `"),
            "must use db.exec with bind; got:\n{query_fn}"
        );
        assert!(query_fn.contains(": void"), "Exec returns void; got:\n{query_fn}");
    }

    fn make_exec_result_query() -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = "UpdateUserName".to_string();
            aq.command = QueryCommand::ExecResult;
            aq.sql = "UPDATE users SET name = ? WHERE id = ?".to_string();
            aq.columns = vec![];
            aq.params = vec![
                scythe_core::analyzer::AnalyzedParam {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: false,
                    position: 1,
                    source_relation: None,
                },
                scythe_core::analyzer::AnalyzedParam {
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
        })
    }

    #[test]
    fn test_exec_result_query_fn_is_synchronous_and_returns_changes() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_exec_result_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(!query_fn.contains("async"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("await"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("Promise"), "must be sync; got:\n{query_fn}");
        assert!(
            query_fn.contains("return db.changes();"),
            "must return db.changes(); got:\n{query_fn}"
        );
        assert!(
            query_fn.contains(": number"),
            "ExecResult returns number; got:\n{query_fn}"
        );
    }

    fn make_batch_query() -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = "InsertUser".to_string();
            aq.command = QueryCommand::Batch;
            aq.sql = "INSERT INTO users (id, name) VALUES (?, ?)".to_string();
            aq.columns = vec![];
            aq.params = vec![
                scythe_core::analyzer::AnalyzedParam {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    position: 1,
                    source_relation: None,
                },
                scythe_core::analyzer::AnalyzedParam {
                    name: "name".to_string(),
                    neutral_type: "string".to_string(),
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
        })
    }

    #[test]
    fn test_batch_query_fn_is_synchronous_and_uses_db_transaction() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_batch_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(!query_fn.contains("async"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("await"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("Promise"), "must be sync; got:\n{query_fn}");
        assert!(
            query_fn.contains("db.transaction(() => {"),
            "must use db.transaction; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export interface InsertUserRowBatchParams"),
            "multi-param batch needs a params interface; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export function insertUserBatch"),
            "missing batch fn; got:\n{query_fn}"
        );
        // Regression coverage: a wrapped (>80 char) signature must render the
        // `items: XBatchParams[]` parameter it was actually given, not fall back to
        // the query's own per-column parameter list.
        assert!(
            query_fn.contains("\titems: InsertUserRowBatchParams[],"),
            "wrapped signature must use the items array param; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("\tid: number,\n\tname: string,"),
            "wrapped signature must not fall back to per-column params; got:\n{query_fn}"
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
    fn test_grouped_typescript_wasm_sqlite_structs() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
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
    fn test_grouped_typescript_wasm_sqlite_query_fn() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(!query_fn.contains("async function"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("await"), "must be sync; got:\n{query_fn}");
        assert!(!query_fn.contains("Promise"), "must be sync; got:\n{query_fn}");
        assert!(
            query_fn.contains("export function getUsersWithOrders"),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersRow[]"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("db.selectObjects(`"),
            "must call db.selectObjects; got:\n{query_fn}"
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

    fn make_named_batch_query(name: &str, sql: &str, params: Vec<AnalyzedParam>) -> AnalyzedQuery {
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
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_named_batch_query(
            "CreateUserAccountRecord",
            "INSERT INTO user_account_record (name, email) VALUES (?, ?)",
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
            query_fn.contains("db.transaction(() => {"),
            "every identifier the body references (items) must be declared in the signature; got:\n{query_fn}"
        );
    }

    /// The critical regression test for this backend specifically: it used
    /// to split a pre-joined `params_inline` string on top-level `", "` to
    /// recover individual parameters for the wrapped form. A `json` param's
    /// TS type is `Record<string, unknown>`, which itself contains `", "` --
    /// splitting on it corrupted a single parameter into
    /// `payload: Record<string` and `unknown>[]`. The structured
    /// `(name, type)` pairs must survive a wrapped signature intact.
    #[test]
    fn test_batch_signature_wrap_preserves_json_param_type_intact() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_named_batch_query(
            "CreateUserAccountRecordPayload",
            "INSERT INTO user_account_record_payload (payload) VALUES (?)",
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

    #[test]
    fn test_structs_only_suppresses_query_fn_and_driver_import() {
        let mut backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
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
            !header.contains("@sqlite.org/sqlite-wasm"),
            "the unused sqlite-wasm driver import must be dropped; got:\n{header}"
        );
    }

    #[test]
    fn test_structs_only_unset_leaves_query_fn_present() {
        let backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = ?");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert!(
            !result.query_fn.as_deref().unwrap().is_empty(),
            "without structs_only the query function must still be generated"
        );
        assert!(
            backend
                .file_header()
                .contains("import type { Database } from \"@sqlite.org/sqlite-wasm\";")
        );
    }

    #[test]
    fn test_structs_only_rejects_invalid_value() {
        let mut backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    #[test]
    fn test_structs_only_combined_with_zod_row_type_keeps_zod_import_and_schema() {
        let mut backend = TypescriptWasmSqliteBackend::new("sqlite").unwrap();
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
        assert!(!header.contains("@sqlite.org/sqlite-wasm"), "got:\n{header}");
    }

    fn js_backend() -> TypescriptWasmSqliteBackend {
        TypescriptWasmSqliteBackend::new_js("sqlite").unwrap()
    }

    #[test]
    fn test_js_mode_name_is_javascript_wasm_sqlite() {
        assert_eq!(js_backend().name(), "javascript-wasm-sqlite");
    }

    #[test]
    fn test_js_mode_output_extension_is_js() {
        assert_eq!(js_backend().output_extension(), "js");
    }

    #[test]
    fn test_js_mode_file_header_has_no_ts_only_imports() {
        assert_eq!(js_backend().file_header(), "");
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
        assert!(!row_struct.contains("[total]"), "{row_struct}");
        assert!(!row_struct.contains("total?"), "{row_struct}");
    }

    #[test]
    fn test_js_mode_one_query_fn_is_synchronous_plain_js_with_jsdoc_types() {
        let backend = js_backend();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("@param {import(\"@sqlite.org/sqlite-wasm\").Database} db"),
            "got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("@returns {GetSessionRow}"),
            "`:one` must not be nullable; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export function getSession(db) {"),
            "sqlite-wasm is sync -- no `async`, no type annotations; got:\n{query_fn}"
        );
        assert!(!query_fn.contains("async"), "got:\n{query_fn}");
        assert!(!query_fn.contains("await"), "got:\n{query_fn}");
        assert!(
            !query_fn.contains(" as "),
            "must not use a TS `as` assertion; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains(
                "/** @type {GetSessionRow | undefined} */ (db.selectObject(`SELECT id, user_id FROM \
                 sessions WHERE id = ?`))"
            ),
            "the blind cast must use the JSDoc inline-cast form; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains(
                "if (row === undefined) {\n\t\tthrow new Error(\"no row found for query: GetSession\");\n\t}\n\t\
                 return row;"
            ),
            "`:one` must throw on a missing row, not return null; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("?? null"),
            "`:one` must not return null; got:\n{query_fn}"
        );
    }

    /// One cast, not the TS path's `as unknown as`. `db.selectObjects(...) as
    /// GetSessionRow[]` is a real TS2352 against the real `@sqlite.org/sqlite-wasm`
    /// declaration (`Record<string, SqlValue>[]`) -- but the JSDoc spelling of
    /// the same assertion is accepted by `tsc --checkJs --strict`, so mirroring
    /// the TS shape here would put a second cast in every generated `:many` to
    /// dodge a diagnostic that never fires. The real-`tsc` half of this is
    /// `test_javascript_wasm_sqlite_grouped_and_nullable_pass_real_tools`, which
    /// compiles a `:many` query against the checked-in `@sqlite.org/sqlite-wasm`
    /// stub; this assertion only pins the spelling.
    #[test]
    fn test_js_mode_many_query_fn_casts_rows_in_one_step() {
        let backend = js_backend();
        let mut query = make_one_query_with_snake_case_column();
        query.command = QueryCommand::Many;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {GetSessionRow[]}"), "got:\n{query_fn}");
        assert!(
            query_fn.contains(
                "return /** @type {GetSessionRow[]} */ (db.selectObjects(`SELECT id, user_id FROM \
                 sessions WHERE id = ?`));"
            ),
            "got:\n{query_fn}"
        );
        assert!(!query_fn.contains("@type {unknown}"), "got:\n{query_fn}");
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    #[test]
    fn test_js_mode_exec_query_fn_returns_void() {
        let backend = js_backend();
        let query = make_exec_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {void}"), "got:\n{query_fn}");
        assert!(!query_fn.contains("async"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("db.exec({ sql: `"),
            "must use db.exec with bind; got:\n{query_fn}"
        );
    }

    /// Regression: `db.changes()` needs no cast to satisfy `@returns {number}`
    /// -- `Sqlite3Result` (the real declared return type) is a union of
    /// numeric-literal CAPI constants, which widens to `number` on return
    /// without a diagnostic, same as the TS path.
    #[test]
    fn test_js_mode_exec_result_query_fn_returns_changes_uncast() {
        let backend = js_backend();
        let query = make_exec_result_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("return db.changes();"), "got:\n{query_fn}");
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    /// Regression: `@sqlite.org/sqlite-wasm`'s `Database` has a `.transaction()`
    /// helper (unlike `node:sqlite`'s `DatabaseSync`), so the JS-mode `:batch`
    /// path must use it, exactly like the TS path does -- not explicit
    /// `BEGIN`/`COMMIT`/`ROLLBACK` statements.
    #[test]
    fn test_js_mode_batch_query_fn_uses_db_transaction() {
        let backend = js_backend();
        let query = make_batch_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(!query_fn.contains("async"), "got:\n{query_fn}");
        assert!(!query_fn.contains("await"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("db.transaction(() => {"),
            "missing db.transaction; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("@typedef {object} InsertUserRowBatchParams"),
            "multi-param batch needs a JSDoc typedef, not an interface; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export function insertUserBatch(db, items) {"),
            "missing batch fn; got:\n{query_fn}"
        );
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    /// `selectObjects`'s real `@sqlite.org/sqlite-wasm` return type
    /// (`Record<string, SqlValue>[]`) still needs the row shape spelled out
    /// before the fold body can read arbitrary column names off it.
    #[test]
    fn test_js_mode_grouped_query_fn_casts_flat_rows() {
        let backend = js_backend();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(
                "const flatRows = /** @type {Array<Record<string, unknown>>} */ (db.selectObjects(`SELECT \
                 u.id, u.name, o.id AS order_id, o.total FROM users u JOIN orders o ON o.user_id = u.id`));"
            ),
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
                .expect_err(&format!("javascript-wasm-sqlite must reject {key} = {value}"));
            assert!(err.to_string().contains("javascript-wasm-sqlite"), "{err}");
        }
    }
}
