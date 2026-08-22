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

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/typescript-duckdb.toml");

/// The `@duckdb/node-api` type of a connection handle.
///
/// The package exports no type called `Connection` at all -- the class is
/// `DuckDBConnection` -- so `import type { Connection } from
/// "@duckdb/node-api"` was `TS2614` on the second line of every file this
/// backend has ever produced (#217). Verified against the published
/// `@duckdb/node-api` `.d.ts`, not assumed.
const CONNECTION_TYPE: &str = "DuckDBConnection";

/// Bind `params` onto a prepared statement, then run it.
///
/// `DuckDBPreparedStatement.run()` takes **zero** arguments (again, checked
/// against the published `.d.ts`); values go on beforehand through `bind`.
/// Passing them to `run` was `TS2554: Expected 0 arguments, but got N` on
/// every query with a parameter (#217).
///
/// The `as DuckDBValue[]` assertion is the driver's own boundary type:
/// `DuckDBValue` covers `null`/`boolean`/`number`/`bigint`/`string` plus the
/// driver's wrapper classes (which is where `bytes`'s `DuckDBBlobValue`
/// lands -- see the manifest), but not `Record<string, unknown>`, which this
/// manifest maps `json` to. Without it a JSON parameter would not type-check
/// even though the driver accepts it at runtime.
fn write_bind_and_run(out: &mut String, indent: &str, args: &[String], result_binding: Option<&str>) {
    if !args.is_empty() {
        let _ = writeln!(out, "{indent}stmt.bind([{}] as DuckDBValue[]);", args.join(", "));
    }
    match result_binding {
        Some(name) => {
            let _ = writeln!(out, "{indent}const {name} = await stmt.run();");
        }
        None => {
            let _ = writeln!(out, "{indent}await stmt.run();");
        }
    }
}

pub struct TypescriptDuckdbBackend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
    /// When true, only emit type definitions (interfaces/Zod schemas, enums,
    /// composites) — no query functions, no `@duckdb/node-api` driver
    /// import, and no `firstRow`/`allRows` helpers (both would otherwise be
    /// unused since only query functions call them).
    structs_only: bool,
    /// Under `Camel`, `:one`/`:opt`/`:many` reconstruct the row field by
    /// field from the driver's raw (snake_case) keys instead of trusting
    /// `firstRow<StructName>`/`allRows<StructName>`'s blind generic cast --
    /// see [`generate_ts_one_row_remap`]/[`generate_ts_many_row_remap`].
    /// `Snake` (the default) keeps that cast, which is sound there.
    field_case: TsFieldCase,
    /// Emit plain, JSDoc-annotated `.js` instead of `.ts` -- the
    /// `javascript-duckdb` registry name (#93). See
    /// `TypescriptPgBackend::js_mode` for the shared rationale.
    js_mode: bool,
}

impl TypescriptDuckdbBackend {
    /// The file header, with the `DuckDBValue` import included only when
    /// `needs_value_type` says the file will bind a parameter, and the
    /// `DuckDBBlobValue` import included only when `needs_blob_type` says
    /// the file references a `bytes` column -- `@duckdb/node-api` hands a
    /// `BLOB` column back as a `DuckDBBlobValue` wrapper, not a raw
    /// `Uint8Array` (`Uint8Array` is not part of the driver's own
    /// `DuckDBValue` union either -- see the comment on `write_bind_and_run`
    /// -- which is the same evidence).
    ///
    /// The short-circuit for `js_mode` lives in this helper rather than in
    /// `file_header`/`file_header_for_results` because those are the only
    /// two callers, and both funnel through here: JSDoc types are
    /// self-contained (`{import("@duckdb/node-api").DuckDBConnection}`
    /// written directly in the `@param` tag; `DuckDBValue`/`DuckDBBlobValue`
    /// referenced the same way at each cast site -- see
    /// `generate_query_fn_js`), so `.js` output never needs a driver import.
    /// It also never needs the `firstRow`/`allRows` helpers below: both are
    /// declared with a TS generic (`<T>`), which plain `.js` cannot parse --
    /// JS mode reads a row through a direct inline JSDoc cast instead.
    /// Nothing is left for this header to carry regardless of
    /// `needs_value_type`/`needs_blob_type`, which only govern the TS import
    /// list this mode skips outright.
    ///
    fn file_header_with_value_type(&self, needs_value_type: bool, needs_blob_type: bool) -> String {
        if self.js_mode {
            return String::new();
        }
        if self.structs_only {
            let mut header = String::new();
            if needs_blob_type {
                header.push_str("import type { DuckDBBlobValue } from \"@duckdb/node-api\";\n");
            }
            if self.row_type == TsRowType::Zod {
                header.push_str("import { z } from \"zod\";\n");
            }
            return header;
        }
        let mut imported = vec![CONNECTION_TYPE.to_string()];
        if needs_value_type {
            imported.push("DuckDBValue".to_string());
        }
        if needs_blob_type {
            imported.push("DuckDBBlobValue".to_string());
        }
        let mut header = format!("import type {{ {} }} from \"@duckdb/node-api\";\n", imported.join(", "));
        if self.row_type == TsRowType::Zod {
            header.push_str("import { z } from \"zod\";\n");
        }
        header.push_str(
            "\nfunction firstRow<T>(rows: readonly unknown[]): T | null {\n\
             \treturn rows.length === 0 ? null : (rows[0] as T);\n\
             }\n\n\
             function allRows<T>(rows: readonly unknown[]): T[] {\n\
             \treturn rows as T[];\n\
             }\n",
        );
        header
    }

    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "duckdb" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("typescript-duckdb only supports DuckDB, got engine '{}'", engine),
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

    /// As [`Self::new`], but selecting the `javascript-duckdb` JSDoc emit mode.
    pub fn new_js(engine: &str) -> Result<Self, ScytheError> {
        let mut backend = Self::new(engine)?;
        backend.js_mode = true;
        backend.manifest.types.scalars.insert(
            "bytes".to_string(),
            "import(\"@duckdb/node-api\").DuckDBBlobValue".to_string(),
        );
        Ok(backend)
    }
}

impl CodegenBackend for TypescriptDuckdbBackend {
    fn name(&self) -> &str {
        if self.js_mode {
            "javascript-duckdb"
        } else {
            "typescript-duckdb"
        }
    }

    /// The manifest is shared with `typescript-duckdb` and says `ts`; JSDoc
    /// output is plain JavaScript and must land in a `.js` file to be
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
        &["duckdb"]
    }

    /// Without the generated queries in hand there is no way to know whether
    /// any of them binds, so this keeps the `DuckDBValue` import -- an extra
    /// import is a lint warning, a missing one is a compile error. Every
    /// caller inside this crate goes through
    /// [`CodegenBackend::file_header_for_results`], which does know.
    fn file_header(&self) -> String {
        self.file_header_with_value_type(true, false)
    }

    /// Drop the `DuckDBValue` import when nothing in the file binds a
    /// parameter, and drop the `DuckDBBlobValue` import when nothing in the
    /// file reads or declares a `bytes` column.
    ///
    /// `DuckDBValue` only appears in the `stmt.bind([...] as DuckDBValue[])`
    /// assertion (see [`write_bind_and_run`]), so a file whose every query
    /// is parameterless never mentions it -- and an unused `import type` is
    /// a lint finding on output that is supposed to be clean. `file_header`
    /// alone cannot tell: it is asked for the header without being shown the
    /// queries.
    ///
    /// `DuckDBBlobValue` can appear in a row struct (the field's declared
    /// type, or `z.custom<DuckDBBlobValue>()` under `row_type = "zod"`), a
    /// composite/model struct, a nested-aggregate struct, or -- under
    /// `field_case = "camelCase"` -- the per-field remap cast in the query
    /// function body, so every one of those has to be scanned, not just
    /// `query_fn`.
    fn file_header_for_results(&self, generated: &[crate::GeneratedCode]) -> String {
        let texts = || {
            generated.iter().flat_map(|code| {
                [
                    code.query_fn.as_deref(),
                    code.row_struct.as_deref(),
                    code.model_struct.as_deref(),
                ]
                .into_iter()
                .flatten()
                .chain(code.nested_struct_defs.iter().map(|def| def.code.as_str()))
            })
        };
        let binds = texts().any(|body| body.contains("DuckDBValue"));
        let needs_blob_type = texts().any(|body| body.contains("DuckDBBlobValue"));
        self.file_header_with_value_type(binds, needs_blob_type)
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

        let query_sig_params: Vec<(String, String)> =
            std::iter::once(("conn".to_string(), CONNECTION_TYPE.to_string()))
                .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
                .collect();

        let write_prepare = |out: &mut String, sql: &str| {
            let oneliner = format!("\tconst stmt = await conn.prepare(`{}`);", sql);
            if oneliner.len() <= 80 {
                let _ = writeln!(out, "{}", oneliner);
            } else {
                let _ = writeln!(out, "\tconst stmt = await conn.prepare(\n\t\t`{}`,\n\t);", sql);
            }
        };

        let param_args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "/** Fetch a single {}. */", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, struct_name);
                write_prepare(&mut out, &sql);
                write_bind_and_run(&mut out, "\t", &param_args, Some("result"));
                let _ = writeln!(out, "\tconst rows = await result.getRowObjects();");
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\tconst row = firstRow<{}>(rows);", struct_name);
                        let _ = writeln!(out, "\tif (row === null) {{");
                        let _ = writeln!(out, "\t\t{}", ts_row_not_found_throw(&analyzed.name));
                        let _ = writeln!(out, "\t}}");
                        let _ = writeln!(out, "\treturn row;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(out, "\tconst row = firstRow<Record<string, unknown>>(rows);");
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
                write_prepare(&mut out, &sql);
                write_bind_and_run(&mut out, "\t", &param_args, Some("result"));
                let _ = writeln!(out, "\tconst rows = await result.getRowObjects();");
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\tconst row = firstRow<{}>(rows);", struct_name);
                        let _ = writeln!(out, "\treturn row;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(out, "\tconst row = firstRow<Record<string, unknown>>(rows);");
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
                        ("conn".to_string(), CONNECTION_TYPE.to_string()),
                        ("items".to_string(), format!("{}[]", params_type_name)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    write_prepare(&mut out, &sql);
                    let _ = writeln!(out, "\tfor (const item of items) {{");
                    let args: Vec<String> = params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    write_bind_and_run(&mut out, "\t\t", &args, None);
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("conn".to_string(), CONNECTION_TYPE.to_string()),
                        ("items".to_string(), format!("{}[]", params[0].full_type)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    write_prepare(&mut out, &sql);
                    let _ = writeln!(out, "\tfor (const item of items) {{");
                    write_bind_and_run(&mut out, "\t\t", &["item".to_string()], None);
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("conn".to_string(), CONNECTION_TYPE.to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    write_prepare(&mut out, &sql);
                    let _ = writeln!(out, "\tfor (let i = 0; i < count; i++) {{");
                    write_bind_and_run(&mut out, "\t\t", &[], None);
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "/** Fetch all {} rows. */", struct_name);
                let ret = format!("{}[]", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                write_prepare(&mut out, &sql);
                write_bind_and_run(&mut out, "\t", &param_args, Some("result"));
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\treturn allRows<{}>(await result.getRowObjects());", struct_name);
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(
                            out,
                            "\tconst rows = allRows<Record<string, unknown>>(await result.getRowObjects());"
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
                write_prepare(&mut out, &sql);
                write_bind_and_run(&mut out, "\t", &param_args, None);
                let _ = write!(out, "}}");
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "/** Execute a query and return the number of affected rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "number");
                write_prepare(&mut out, &sql);
                write_bind_and_run(&mut out, "\t", &param_args, Some("result"));
                let _ = writeln!(out, "\treturn result.rowsChanged;");
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
            format!("conn: {CONNECTION_TYPE}")
        } else {
            format!("conn: {CONNECTION_TYPE}, {}", param_list)
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
            let _ = writeln!(out, "\tconn: {CONNECTION_TYPE},");
            for p in params {
                let _ = writeln!(out, "\t{}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): {ret} {{");
        }

        let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
        let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
        write_bind_and_run(&mut out, "\t", &args, Some("_result"));
        let _ = writeln!(
            out,
            "\tconst flatRows = allRows<Record<string, unknown>>(await _result.getRowObjects());"
        );

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

        // ~keep See `TypescriptWasmSqliteBackend::apply_options` for why these
        // three are rejected outright in JSDoc mode rather than silently
        // ignored.
        if self.js_mode {
            if self.row_type == TsRowType::Zod {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-duckdb does not support row_type = \"zod\": the inferred `export type X = \
                     z.infer<...>` alias is TypeScript-only syntax a plain .js file cannot carry -- use \
                     typescript-duckdb for Zod row types"
                        .to_string(),
                ));
            }
            if self.outer_join_unions {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-duckdb does not support outer_join_unions: the discriminated union is a \
                     TypeScript `type X = A & (B | C)` alias, which plain .js cannot carry -- use \
                     typescript-duckdb"
                        .to_string(),
                ));
            }
            if self.field_case == TsFieldCase::Camel {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-duckdb does not support field_case = \"camelCase\": the field remap needs a \
                     TypeScript `as T` assertion, which plain .js cannot carry -- use typescript-duckdb"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl TypescriptDuckdbBackend {
    /// JSDoc-mode counterpart of `generate_query_fn` (see
    /// `CodegenBackend::generate_query_fn`). `@duckdb/node-api` is Promise-based
    /// like the TS path, so this stays `async` throughout; the difference is
    /// entirely in how a row is cast.
    ///
    /// Every cast here is a single JSDoc `/** @type {T} */ (expr)` step,
    /// verified against the real `@duckdb/node-api@1.5.5-r.4` package (not a
    /// hand-approximated stub): `DuckDBResult.getRowObjects()` (inherited by
    /// the `DuckDBMaterializedResult` `stmt.run()` returns) declares
    /// `Promise<Record<string, DuckDBValue>[]>`, a concrete record type, not
    /// `unknown` -- so `rows[0] as GetUserRow | undefined` is a genuine
    /// TS2352 on the TypeScript path, which is why it funnels through the
    /// `firstRow<T>`/`allRows<T>` helpers above (both declared with a
    /// parameter typed `readonly unknown[]`, so the cast inside always starts
    /// from `unknown`, which is unconditionally single-step-safe). The JSDoc
    /// spelling of the identical direct assertion (no `unknown[]`-typed
    /// funnel) is accepted by real `tsc --checkJs --strict` -- confirmed
    /// against the published package -- so JS mode casts a row in one step,
    /// same as `javascript-wasm-sqlite`'s `:one`/`:many`.
    fn generate_query_fn_js(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        if self.structs_only {
            return Ok(String::new());
        }

        const DB_TYPE: &str = "import(\"@duckdb/node-api\").DuckDBConnection";
        const VALUE_ARRAY_TYPE: &str = "import(\"@duckdb/node-api\").DuckDBValue[]";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let query_sig_params: Vec<(String, String)> = std::iter::once(("conn".to_string(), DB_TYPE.to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();

        let param_args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();

        // ~keep JS-mode counterpart of the free `write_bind_and_run` above: same
        // "bind only if there are args, always await run()" shape, but the
        // bind-array assertion is a JSDoc inline cast rather than `as`.
        let write_bind_and_run_js = |out: &mut String, indent: &str, result_binding: Option<&str>| {
            if !param_args.is_empty() {
                let bind_expr = js_type_cast(VALUE_ARRAY_TYPE, &format!("[{}]", param_args.join(", ")));
                let _ = writeln!(out, "{indent}stmt.bind({bind_expr});");
            }
            match result_binding {
                Some(name) => {
                    let _ = writeln!(out, "{indent}const {name} = await stmt.run();");
                }
                None => {
                    let _ = writeln!(out, "{indent}await stmt.run();");
                }
            }
        };

        let write_signature = |out: &mut String, description: &str, sig_params: &[(String, String)], ret: &str| {
            out.push_str(&generate_jsdoc_fn_header(description, sig_params, ret));
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", js_fn_signature_line(true, &func_name, sig_params));
        };

        match &analyzed.command {
            QueryCommand::One => {
                write_signature(
                    &mut out,
                    &format!("Fetch a single {}.", struct_name),
                    &query_sig_params,
                    &format!("Promise<{}>", struct_name),
                );
                let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
                write_bind_and_run_js(&mut out, "\t", Some("result"));
                let _ = writeln!(out, "\tconst rows = await result.getRowObjects();");
                let _ = writeln!(
                    out,
                    "\tconst row = {};",
                    js_type_cast(&format!("{struct_name} | undefined"), "rows[0]")
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
                    &format!("Promise<{} | null>", struct_name),
                );
                let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
                write_bind_and_run_js(&mut out, "\t", Some("result"));
                let _ = writeln!(out, "\tconst rows = await result.getRowObjects();");
                let _ = writeln!(
                    out,
                    "\tconst row = {};",
                    js_type_cast(&format!("{struct_name} | undefined"), "rows[0]")
                );
                let _ = writeln!(out, "\treturn row ?? null;");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many => {
                write_signature(
                    &mut out,
                    &format!("Fetch all {} rows.", struct_name),
                    &query_sig_params,
                    &format!("Promise<{}[]>", struct_name),
                );
                let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
                write_bind_and_run_js(&mut out, "\t", Some("result"));
                let _ = writeln!(
                    out,
                    "\treturn {};",
                    js_type_cast(&format!("{struct_name}[]"), "await result.getRowObjects()")
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
                        ("conn".to_string(), DB_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params_type_name)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!("Execute {} for each item in the batch.", analyzed.name),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
                    let _ = writeln!(out, "\tfor (const item of items) {{");
                    let args: Vec<String> = params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    let bind_expr = js_type_cast(VALUE_ARRAY_TYPE, &format!("[{}]", args.join(", ")));
                    let _ = writeln!(out, "\t\tstmt.bind({bind_expr});");
                    let _ = writeln!(out, "\t\tawait stmt.run();");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let batch_sig_params = vec![
                        ("conn".to_string(), DB_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params[0].full_type)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!("Execute {} for each item in the batch.", analyzed.name),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
                    let _ = writeln!(out, "\tfor (const item of items) {{");
                    let bind_expr = js_type_cast(VALUE_ARRAY_TYPE, "[item]");
                    let _ = writeln!(out, "\t\tstmt.bind({bind_expr});");
                    let _ = writeln!(out, "\t\tawait stmt.run();");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else {
                    let batch_sig_params = vec![
                        ("conn".to_string(), DB_TYPE.to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!("Execute {} for each item in the batch.", analyzed.name),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
                    let _ = writeln!(out, "\tfor (let i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "\t\tawait stmt.run();");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Exec => {
                write_signature(
                    &mut out,
                    "Execute a query returning no rows.",
                    &query_sig_params,
                    "Promise<void>",
                );
                let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
                write_bind_and_run_js(&mut out, "\t", None);
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
                let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
                write_bind_and_run_js(&mut out, "\t", Some("result"));
                let _ = writeln!(out, "\treturn result.rowsChanged;");
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

        const DB_TYPE: &str = "import(\"@duckdb/node-api\").DuckDBConnection";
        const VALUE_ARRAY_TYPE: &str = "import(\"@duckdb/node-api\").DuckDBValue[]";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let sig_params: Vec<(String, String)> = std::iter::once(("conn".to_string(), DB_TYPE.to_string()))
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

        let _ = writeln!(out, "\tconst stmt = await conn.prepare(`{sql}`);");
        if !params.is_empty() {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            let bind_expr = js_type_cast(VALUE_ARRAY_TYPE, &format!("[{}]", args.join(", ")));
            let _ = writeln!(out, "\tstmt.bind({bind_expr});");
        }
        let _ = writeln!(out, "\tconst _result = await stmt.run();");
        let _ = writeln!(
            out,
            "\tconst flatRows = {};",
            js_type_cast("Array<Record<string, unknown>>", "await _result.getRowObjects()")
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

    /// JSDoc-mode counterpart of `generate_enum_def`. The TS path's default
    /// (non-Zod) shape here is already just a string-literal union type alias
    /// with no backing runtime value, so unlike `TypescriptPgBackend`'s JS
    /// enum a bare `@typedef` carries the same union directly. Matches
    /// `TypescriptDuckdbBackend::generate_enum_def`'s own TS-mode shape,
    /// which does not escape a `"` inside a variant either.
    fn generate_enum_def_js(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let variants: Vec<String> = enum_info.values.iter().map(|v| format!("\"{}\"", v)).collect();
        Ok(format!("/** @typedef {{({})}} {} */", variants.join(" | "), type_name))
    }

    /// JSDoc-mode counterpart of `generate_composite_def`. Matches the TS
    /// path's plain-interface shape: this backend has no pg-style
    /// text-wire-format parser to port, so a bare `@typedef` is a complete
    /// equivalent.
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
    use super::{TypescriptDuckdbBackend, resolve_type};
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
        let mut backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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
        let mut backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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

    /// This must fail before the fix: `firstRow<StructName>`/
    /// `allRows<StructName>` are blind generic casts of the driver's raw
    /// row, unsound once `field_case = "camelCase"` renames the declared
    /// fields -- `getRowObjects()` still returns snake_case keys.
    #[test]
    fn test_one_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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
            query_fn.contains("firstRow<Record<string, unknown>>(rows)"),
            "must not trust the blind generic cast; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row['user_id'] as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_many_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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
            query_fn.contains("allRows<Record<string, unknown>>(await result.getRowObjects())"),
            "must not trust the blind generic cast; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row['user_id'] as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_one_query_fn_keeps_the_blind_cast_under_the_default_snake_case() {
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("firstRow<GetSessionRow>(rows)"),
            "default field_case must keep the original blind cast unchanged; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("Record<string, unknown>"),
            "must not switch to the remap path under the default; got:\n{query_fn}"
        );
    }

    /// This must fail before the fix: `getRows()` returns `@duckdb/node-api`
    /// rows as positional arrays (`[[0, 10], ...]`), not objects keyed by
    /// column name. `firstRow`/`allRows` cast that blindly to the row
    /// interface type, so `tsc` accepted it and every field read back
    /// `undefined` at runtime. `getRowObjects()` is the keyed-object form the
    /// generated row types actually need.
    #[test]
    fn test_one_query_fn_uses_get_row_objects_not_get_rows() {
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = $1");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("getRowObjects()"),
            "must call getRowObjects; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("getRows()"),
            "must not call getRows -- it returns positional arrays, not keyed rows; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_many_query_fn_uses_get_row_objects_not_get_rows() {
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
        let mut query = make_one_query("SELECT id FROM users");
        query.command = QueryCommand::Many;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("getRowObjects()"),
            "must call getRowObjects; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("getRows()"),
            "must not call getRows -- it returns positional arrays, not keyed rows; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_query_fn_escapes_user_backtick_in_sql() {
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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
    fn test_grouped_typescript_duckdb_structs() {
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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
    fn test_grouped_typescript_duckdb_query_fn() {
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("getUsersWithOrders"), "missing fn; got:\n{query_fn}");
        assert!(
            query_fn.contains("Promise<GetUsersWithOrdersRow[]>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("conn.prepare"),
            "must use conn.prepare; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("getRowObjects()"),
            "must call getRowObjects, not getRows -- getRows() returns positional arrays, not \
             keyed rows, and the fold's row_access reads columns by name; got:\n{query_fn}"
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
    fn test_structs_only_suppresses_query_fn_import_and_helpers() {
        let mut backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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
            !header.contains("@duckdb/node-api"),
            "the unused duckdb driver import must be dropped; got:\n{header}"
        );
        assert!(
            !header.contains("firstRow") && !header.contains("allRows"),
            "the unused firstRow/allRows helpers must be dropped; got:\n{header}"
        );
    }

    #[test]
    fn test_structs_only_unset_leaves_query_fn_present() {
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = ?");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert!(
            !result.query_fn.as_deref().unwrap().is_empty(),
            "without structs_only the query function must still be generated"
        );
        let header = backend.file_header();
        assert!(header.contains("@duckdb/node-api"));
        assert!(header.contains("firstRow") && header.contains("allRows"));
    }

    #[test]
    fn test_structs_only_rejects_invalid_value() {
        let mut backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    #[test]
    fn test_structs_only_combined_with_zod_row_type_keeps_zod_import_and_schema() {
        let mut backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
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
        assert!(!header.contains("@duckdb/node-api"), "got:\n{header}");
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
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
        let query = make_batch_query(
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
        let backend = TypescriptDuckdbBackend::new("duckdb").unwrap();
        let query = make_batch_query(
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

    fn js_backend() -> TypescriptDuckdbBackend {
        TypescriptDuckdbBackend::new_js("duckdb").unwrap()
    }

    #[test]
    fn test_js_mode_name_is_javascript_duckdb() {
        assert_eq!(js_backend().name(), "javascript-duckdb");
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
    fn test_js_mode_qualifies_blob_type_inline_without_changing_ts_mode() {
        use crate::backend_trait::ResolvedColumn;

        let ts = TypescriptDuckdbBackend::new("duckdb").unwrap();
        let js = js_backend();
        let column = |backend: &TypescriptDuckdbBackend| {
            let resolved = resolve_type("bytes", backend.manifest(), false).unwrap().into_owned();
            ResolvedColumn {
                name: "payload".to_string(),
                field_name: "payload".to_string(),
                lang_type: resolved.clone(),
                full_type: resolved,
                neutral_type: "bytes".to_string(),
                sql_type: "BLOB".to_string(),
                nullable: false,
                join_group: None,
                nullable_before_join: false,
            }
        };

        let ts_row = ts.generate_row_struct("GetBlob", &[column(&ts)]).unwrap();
        let js_row = js.generate_row_struct("GetBlob", &[column(&js)]).unwrap();

        assert!(ts_row.contains("payload: DuckDBBlobValue;"), "got:\n{ts_row}");
        assert!(
            js_row.contains("@property {import(\"@duckdb/node-api\").DuckDBBlobValue} payload"),
            "got:\n{js_row}"
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
        assert!(!row_struct.contains("[total]"), "{row_struct}");
        assert!(!row_struct.contains("total?"), "{row_struct}");
    }

    #[test]
    fn test_js_mode_one_query_fn_is_async_with_jsdoc_types_and_a_one_step_cast() {
        let backend = js_backend();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("@param {import(\"@duckdb/node-api\").DuckDBConnection} conn"),
            "got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("@returns {Promise<GetSessionRow>}"),
            "`:one` must not be nullable; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export async function getSession(conn) {"),
            "got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains(" as "),
            "must not use a TS `as` assertion; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("const row = /** @type {GetSessionRow | undefined} */ (rows[0]);"),
            "the blind cast must be a single-step JSDoc inline cast, not funnelled through a generic \
             helper; got:\n{query_fn}"
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

    /// One cast, not the TS path's `firstRow<T>`/`allRows<T>` funnel-through-
    /// `unknown` trick: `stmt.run()` -> `result.getRowObjects()` returns the
    /// real `@duckdb/node-api` type `Promise<Record<string, DuckDBValue>[]>`,
    /// a concrete record type, so `await result.getRowObjects() as
    /// GetSessionRow[]` is a genuine TS2352 on the TypeScript path -- but the
    /// JSDoc spelling of the identical assertion is accepted by real `tsc
    /// --checkJs --strict` (verified against the published package, not a
    /// hand-approximated stub). The real-`tsc` half of this claim is
    /// `test_javascript_duckdb_grouped_and_nullable_pass_real_tools` in
    /// `tests/tool_validation.rs`, which compiles a `:many` query against the
    /// checked-in `@duckdb/node-api` stub; this assertion only pins the
    /// spelling.
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
            query_fn.contains("return /** @type {GetSessionRow[]} */ (await result.getRowObjects());"),
            "got:\n{query_fn}"
        );
        assert!(!query_fn.contains("@type {unknown}"), "got:\n{query_fn}");
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    #[test]
    fn test_js_mode_exec_query_fn_returns_promise_void() {
        let backend = js_backend();
        let mut query = make_one_query("SELECT id FROM users WHERE id = ?");
        query.command = QueryCommand::Exec;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {Promise<void>}"), "got:\n{query_fn}");
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    /// Regression: `result.rowsChanged` needs no cast to satisfy
    /// `@returns {Promise<number>}` -- it is a real `number` getter on
    /// `DuckDBResult` (inherited by `DuckDBMaterializedResult`), unlike the
    /// row reads above.
    #[test]
    fn test_js_mode_exec_result_query_fn_returns_rows_changed_uncast() {
        let backend = js_backend();
        let mut query = make_one_query("SELECT id FROM users WHERE id = ?");
        query.command = QueryCommand::ExecResult;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("return result.rowsChanged;"), "got:\n{query_fn}");
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    #[test]
    fn test_js_mode_batch_query_fn_binds_and_runs_each_item() {
        let backend = js_backend();
        let query = make_batch_query(
            "InsertUser",
            "INSERT INTO users (name) VALUES (?)",
            vec![AnalyzedParam {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                position: 1,
                source_relation: None,
            }],
        );
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("export async function insertUserBatch(conn, items) {"),
            "missing batch fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("stmt.bind(/** @type {import(\"@duckdb/node-api\").DuckDBValue[]} */ ([item]));"),
            "single-param batch must bind `item` directly; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("await stmt.run();"),
            "must call stmt.run() per item; got:\n{query_fn}"
        );
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    /// `getRowObjects()`'s real `@duckdb/node-api` return type
    /// (`Promise<Record<string, DuckDBValue>[]>`) still needs the row shape
    /// spelled out before the fold body can read arbitrary column names off
    /// it.
    #[test]
    fn test_js_mode_grouped_query_fn_casts_flat_rows() {
        let backend = js_backend();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(
                "const flatRows = /** @type {Array<Record<string, unknown>>} */ (await _result.getRowObjects());"
            ),
            "got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("@returns {Promise<GetUsersWithOrdersRow[]>}"),
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
                .expect_err(&format!("javascript-duckdb must reject {key} = {value}"));
            assert!(err.to_string().contains("javascript-duckdb"), "{err}");
        }
    }
}
