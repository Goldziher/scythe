use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name, to_camel_case};
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
    generate_ts_grouped_fold_body, generate_ts_interface_row_struct_with_base, generate_ts_many_row_remap,
    generate_ts_one_row_remap, generate_ts_union_row_struct, generate_zod_enum, generate_zod_grouped_structs,
    generate_zod_row_struct, generate_zod_union_row_struct, js_fn_signature_line, js_type_cast, parse_bool_option,
    ts_member_access, ts_property_key, ts_row_not_found_throw,
};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/typescript-mysql2.toml");

pub struct TypescriptMysql2Backend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
    /// When true, only emit type definitions (interfaces/Zod schemas, enums,
    /// composites) — no query functions. `Pool` is dropped from the driver
    /// import since nothing names it any more. `RowDataPacket` survives only
    /// under `field_case = "snake_case"`, where every generated row type
    /// still extends/intersects it; under `Camel` the row types drop that
    /// base and the only remaining references are in the query functions
    /// `structs_only` suppresses, so the driver import goes away entirely.
    /// See [`CodegenBackend::file_header`].
    structs_only: bool,
    /// Under `Camel`, declared row types stop extending/intersecting
    /// `RowDataPacket` (its `[column: string]: any` index signature would
    /// keep accepting the driver's raw snake_case keys structurally, even
    /// on a type that declares only camelCase fields -- defeating the point
    /// of renaming them), and `:one`/`:opt`/`:many` fetch through bare
    /// `RowDataPacket` and reconstruct the row field by field instead of
    /// trusting a `pool.execute<StructName[]>` generic -- see
    /// [`generate_ts_one_row_remap`]/[`generate_ts_many_row_remap`]. `Snake`
    /// (the default) is unchanged.
    field_case: TsFieldCase,
    /// Emit plain, JSDoc-annotated `.js` instead of `.ts` -- the
    /// `javascript-mysql2` registry name (#81). See
    /// `TypescriptPgBackend::js_mode` for the shared rationale. In this mode
    /// row types are plain JSDoc `@typedef`s with no `RowDataPacket`
    /// base/intersection at all: JSDoc typedefs are structural, so there is
    /// no equivalent of TypeScript's `extends`/`&` needed to satisfy
    /// `pool.execute`'s generic constraint (which JS mode does not use
    /// either -- see `generate_query_fn_js`).
    js_mode: bool,
}

impl TypescriptMysql2Backend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "mysql" | "mariadb" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("typescript-mysql2 only supports MySQL/MariaDB, got engine '{}'", engine),
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

    /// As [`Self::new`], but selecting the `javascript-mysql2` JSDoc emit mode.
    pub fn new_js(engine: &str) -> Result<Self, ScytheError> {
        let mut backend = Self::new(engine)?;
        backend.js_mode = true;
        Ok(backend)
    }
}

impl CodegenBackend for TypescriptMysql2Backend {
    fn name(&self) -> &str {
        if self.js_mode {
            "javascript-mysql2"
        } else {
            "typescript-mysql2"
        }
    }

    /// The manifest is shared with `typescript-mysql2` and says `ts`; JSDoc
    /// output is plain JavaScript and must land in a `.js` file to be runnable.
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
        &["mysql", "mariadb"]
    }

    fn file_header(&self) -> String {
        // ~keep See `TypescriptPgBackend::file_header`: `.js` output needs no
        // import at all in JSDoc mode -- the driver type goes straight into
        // the `@param` tag as `import("mysql2/promise").Pool`, and Zod is
        // rejected in this mode.
        if self.js_mode {
            // Nothing left for this header to carry: the "do not edit"
            // notice lives in the scythe:provenance line every backend emits.
            return String::new();
        }
        let mut types: Vec<&str> = Vec::new();
        // `Pool` is only ever named by a query function signature.
        if !self.structs_only {
            types.push("Pool");
        }
        // `RowDataPacket` has two independent referrers, and `noUnusedLocals`
        // fails the generated file if neither is present. Under `Snake` the
        // declared row types extend/intersect it, so it is always used.
        // Under `Camel` they drop that base (see `generate_row_struct`),
        // leaving the `pool.execute<RowDataPacket[]>` fetch in the query
        // functions as the only reference — which `structs_only` removes.
        if self.field_case == TsFieldCase::Snake || !self.structs_only {
            types.push("RowDataPacket");
        }

        let mut header = String::new();
        if !types.is_empty() {
            let _ = writeln!(
                header,
                "import type {{ {} }} from \"mysql2/promise\";",
                types.join(", ")
            );
        }
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
        // Under Camel, the declared row type must not extend/intersect
        // RowDataPacket -- see the `field_case` field doc for why -- so
        // there is also no bridging `{struct_name}Packet` type to generate:
        // `:one`/`:opt`/`:many` fetch through bare `RowDataPacket` directly
        // in that mode (mirroring what the `:grouped` fold already does
        // unconditionally), so nothing needs a per-query Packet alias.
        let base = match self.field_case {
            TsFieldCase::Snake => Some("RowDataPacket"),
            TsFieldCase::Camel => None,
        };
        if self.row_type == TsRowType::Zod {
            if self.outer_join_unions {
                let mut out = generate_zod_union_row_struct(struct_name, query_name, columns);
                let Some(base) = base else {
                    return Ok(out);
                };
                let _ = writeln!(out);
                let _ = writeln!(out);
                // `interface extends` cannot extend a union, so use an
                // intersection type alias instead; it stays valid even
                // when there's no discriminant and the schema collapses
                // back to a plain object.
                let _ = write!(out, "export type {struct_name}Packet = {base} & {struct_name};");
                return Ok(out);
            }
            let mut out = generate_zod_row_struct(struct_name, query_name, columns);
            let Some(base) = base else {
                return Ok(out);
            };
            let _ = writeln!(out);
            let _ = writeln!(out);
            let _ = write!(
                out,
                "export interface {struct_name}Packet extends {base}, {struct_name} {{}}"
            );
            return Ok(out);
        }
        if self.outer_join_unions {
            return Ok(generate_ts_union_row_struct(struct_name, query_name, columns, base));
        }
        Ok(generate_ts_interface_row_struct_with_base(
            struct_name,
            query_name,
            columns,
            base,
        ))
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
            let oneliner = format!("export async function {}({}): {} {{", name, params_inline, ret);
            if oneliner.len() <= 80 {
                let _ = writeln!(out, "{}", oneliner);
            } else {
                let _ = writeln!(out, "export async function {}(", name);
                for (n, t) in sig_params {
                    let _ = writeln!(out, "\t{}: {},", n, t);
                }
                let _ = writeln!(out, "): {} {{", ret);
            }
        };

        let query_sig_params: Vec<(String, String)> = std::iter::once(("pool".to_string(), "Pool".to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();

        let param_array = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            format!(", [{}]", args.join(", "))
        };

        // Under Camel there is no Packet type to name (see generate_row_struct):
        // the fetch always trusts bare RowDataPacket instead, then remaps.
        let query_type = match self.field_case {
            TsFieldCase::Snake if self.row_type == TsRowType::Zod => format!("{struct_name}Packet"),
            TsFieldCase::Snake => struct_name.to_string(),
            TsFieldCase::Camel => "RowDataPacket".to_string(),
        };

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "/** Fetch a single {}. */", struct_name);
                let ret = format!("Promise<{}>", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                let _ = writeln!(out, "\tconst [rows] = await pool.execute<{}[]>(", query_type);
                let _ = writeln!(out, "\t\t`{}`{},", sql, param_array);
                let _ = writeln!(out, "\t);");
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\tconst row = rows[0];");
                        let _ = writeln!(out, "\tif (row === undefined) {{");
                        let _ = writeln!(out, "\t\t{}", ts_row_not_found_throw(&analyzed.name));
                        let _ = writeln!(out, "\t}}");
                        let _ = writeln!(out, "\treturn row;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(out, "\tconst row = rows[0];");
                        out.push_str(&generate_ts_one_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            &analyzed.command,
                            &analyzed.name,
                            |name, ty| format!("{} as {ty}", ts_member_access("row", name)),
                        ));
                    }
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Opt => {
                let _ = writeln!(out, "/** Fetch a single {} or null. */", struct_name);
                let ret = format!("Promise<{} | null>", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                let _ = writeln!(out, "\tconst [rows] = await pool.execute<{}[]>(", query_type);
                let _ = writeln!(out, "\t\t`{}`{},", sql, param_array);
                let _ = writeln!(out, "\t);");
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\treturn rows[0] ?? null;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(out, "\tconst row = rows[0];");
                        out.push_str(&generate_ts_one_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            &analyzed.command,
                            &analyzed.name,
                            |name, ty| format!("{} as {ty}", ts_member_access("row", name)),
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
                        ("pool".to_string(), "Pool".to_string()),
                        ("items".to_string(), format!("{}[]", params_type_name)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "Promise<void>");
                    let _ = writeln!(out, "\tconst conn = await pool.getConnection();");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait conn.beginTransaction();");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let args: Vec<String> = params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    let _ = writeln!(out, "\t\t\tawait conn.execute(");
                    let args_str = args.join(", ");
                    let _ = writeln!(out, "\t\t\t\t`{}`, [{}],", sql, args_str);
                    let _ = writeln!(out, "\t\t\t);");
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait conn.commit();");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait conn.rollback();");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}} finally {{");
                    let _ = writeln!(out, "\t\tconn.release();");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("pool".to_string(), "Pool".to_string()),
                        ("items".to_string(), format!("{}[]", params[0].full_type)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "Promise<void>");
                    let _ = writeln!(out, "\tconst conn = await pool.getConnection();");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait conn.beginTransaction();");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tawait conn.execute(`{}`, [item]);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait conn.commit();");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait conn.rollback();");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}} finally {{");
                    let _ = writeln!(out, "\t\tconn.release();");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("pool".to_string(), "Pool".to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "Promise<void>");
                    let _ = writeln!(out, "\tconst conn = await pool.getConnection();");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait conn.beginTransaction();");
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "\t\t\tawait conn.execute(`{}`);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait conn.commit();");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait conn.rollback();");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}} finally {{");
                    let _ = writeln!(out, "\t\tconn.release();");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "/** Fetch all {} rows. */", struct_name);
                let ret = format!("Promise<{}[]>", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                let _ = writeln!(out, "\tconst [rows] = await pool.execute<{}[]>(", query_type);
                let _ = writeln!(out, "\t\t`{}`{},", sql, param_array);
                let _ = writeln!(out, "\t);");
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\treturn rows;");
                    }
                    TsFieldCase::Camel => {
                        out.push_str(&generate_ts_many_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            |name, ty| format!("{} as {ty}", ts_member_access("row", name)),
                        ));
                    }
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "/** Execute a query returning no rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "Promise<void>");
                let _ = writeln!(out, "\tawait pool.execute(");
                let _ = writeln!(out, "\t\t`{}`{},", sql, param_array);
                let _ = writeln!(out, "\t);");
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "/** Execute a query and return the number of affected rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "Promise<number>");
                let _ = writeln!(out, "\tconst [result] = await pool.execute(");
                let _ = writeln!(out, "\t\t`{}`{},", sql, param_array);
                let _ = writeln!(out, "\t);");
                let _ = writeln!(out, "\treturn (result as {{ affectedRows: number }}).affectedRows;");
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
            "pool: Pool".to_string()
        } else {
            format!("pool: Pool, {}", param_list)
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
            let _ = writeln!(out, "\tpool: Pool,");
            for p in params {
                let _ = writeln!(out, "\t{}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): {ret} {{");
        }

        if params.is_empty() {
            let _ = writeln!(
                out,
                "\tconst [flatRows] = await pool.execute<RowDataPacket[]>(`{sql}`);"
            );
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            let _ = writeln!(
                out,
                "\tconst [flatRows] = await pool.execute<RowDataPacket[]>(`{sql}`, [{}]);",
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
            |name, ty| format!("{} as {ty}", ts_member_access("row", name)),
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
            return Ok(generate_zod_enum(&type_name, &enum_info.values, &self.manifest.naming));
        }
        let mut out = String::new();
        let _ = writeln!(out, "export enum {} {{", type_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "\t{} = \"{}\",", variant, value);
        }
        let _ = write!(out, "}}");
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
                    "javascript-mysql2 does not support row_type = \"zod\": the inferred `export type X = \
                     z.infer<...>` alias is TypeScript-only syntax a plain .js file cannot carry -- use \
                     typescript-mysql2 for Zod row types"
                        .to_string(),
                ));
            }
            if self.outer_join_unions {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-mysql2 does not support outer_join_unions: the discriminated union is a \
                     TypeScript `type X = A & (B | C)` alias, which plain .js cannot carry -- use \
                     typescript-mysql2"
                        .to_string(),
                ));
            }
            if self.field_case == TsFieldCase::Camel {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-mysql2 does not support field_case = \"camelCase\": the field remap needs a \
                     TypeScript `as T` assertion, which plain .js cannot carry -- use typescript-mysql2"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl TypescriptMysql2Backend {
    /// JSDoc-mode counterpart of `generate_query_fn`. Row types have no
    /// `RowDataPacket` base in this mode (see `js_mode`'s doc comment), so
    /// every fetch goes through a bare, ungenericized `pool.execute(...)`.
    fn generate_query_fn_js(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        if self.structs_only {
            return Ok(String::new());
        }

        const POOL_TYPE: &str = "import(\"mysql2/promise\").Pool";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let query_sig_params: Vec<(String, String)> = std::iter::once(("pool".to_string(), POOL_TYPE.to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();

        let param_array = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            format!(", [{}]", args.join(", "))
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
                let execute_expr = format!("await pool.execute(\n\t\t`{}`{},\n\t)", sql, param_array);
                let _ = writeln!(
                    out,
                    "\tconst [rows] = {};",
                    js_type_cast(&format!("[Array<{struct_name}>, unknown]"), &execute_expr)
                );
                let _ = writeln!(out, "\tconst row = rows[0];");
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
                let execute_expr = format!("await pool.execute(\n\t\t`{}`{},\n\t)", sql, param_array);
                let _ = writeln!(
                    out,
                    "\tconst [rows] = {};",
                    js_type_cast(&format!("[Array<{struct_name}>, unknown]"), &execute_expr)
                );
                let _ = writeln!(out, "\treturn rows[0] ?? null;");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many => {
                write_signature(
                    &mut out,
                    &format!("Fetch all {} rows.", struct_name),
                    &query_sig_params,
                    &format!("Promise<{}[]>", struct_name),
                );
                let execute_expr = format!("await pool.execute(\n\t\t`{}`{},\n\t)", sql, param_array);
                let _ = writeln!(
                    out,
                    "\tconst [rows] = {};",
                    js_type_cast(&format!("[Array<{struct_name}>, unknown]"), &execute_expr)
                );
                let _ = writeln!(out, "\treturn rows;");
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec => {
                write_signature(
                    &mut out,
                    "Execute a query returning no rows.",
                    &query_sig_params,
                    "Promise<void>",
                );
                let _ = writeln!(out, "\tawait pool.execute(");
                let _ = writeln!(out, "\t\t`{}`{},", sql, param_array);
                let _ = writeln!(out, "\t);");
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_signature(
                    &mut out,
                    "Execute a query and return the number of affected rows.",
                    &query_sig_params,
                    "Promise<number>",
                );
                // ~keep `pool.execute(...)` without a type argument returns
                // mysql2's `QueryResult` union, which is not guaranteed to
                // be array-like (only some of its members are), so `tsc
                // --strict` rejects the `[result] = ...` array destructure
                // outright (TS2488) -- not just an inexact element type.
                // Casting the whole call to a concrete tuple first (mirroring
                // the TS path's `pool.execute<T[]>` type argument) resolves
                // both that and the field access below in one step.
                let execute_expr = format!("await pool.execute(\n\t\t`{}`{},\n\t)", sql, param_array);
                let _ = writeln!(
                    out,
                    "\tconst [result] = {};",
                    js_type_cast("[{ affectedRows: number }, unknown]", &execute_expr)
                );
                let _ = writeln!(out, "\treturn result.affectedRows;");
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
                        ("pool".to_string(), POOL_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params_type_name)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!("Execute {} for each item in the batch.", analyzed.name),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\tconst conn = await pool.getConnection();");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait conn.beginTransaction();");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let args: Vec<String> = params.iter().map(|p| ts_member_access("item", &p.field_name)).collect();
                    let _ = writeln!(out, "\t\t\tawait conn.execute(");
                    let args_str = args.join(", ");
                    let _ = writeln!(out, "\t\t\t\t`{}`, [{}],", sql, args_str);
                    let _ = writeln!(out, "\t\t\t);");
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait conn.commit();");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait conn.rollback();");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}} finally {{");
                    let _ = writeln!(out, "\t\tconn.release();");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let batch_sig_params = vec![
                        ("pool".to_string(), POOL_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params[0].full_type)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!("Execute {} for each item in the batch.", analyzed.name),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\tconst conn = await pool.getConnection();");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait conn.beginTransaction();");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tawait conn.execute(`{}`, [item]);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait conn.commit();");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait conn.rollback();");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}} finally {{");
                    let _ = writeln!(out, "\t\tconn.release();");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else {
                    let batch_sig_params = vec![
                        ("pool".to_string(), POOL_TYPE.to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!("Execute {} for each item in the batch.", analyzed.name),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\tconst conn = await pool.getConnection();");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait conn.beginTransaction();");
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "\t\t\tawait conn.execute(`{}`);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait conn.commit();");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait conn.rollback();");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}} finally {{");
                    let _ = writeln!(out, "\t\tconn.release();");
                    let _ = writeln!(out, "\t}}");
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

        const POOL_TYPE: &str = "import(\"mysql2/promise\").Pool";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let sig_params: Vec<(String, String)> = std::iter::once(("pool".to_string(), POOL_TYPE.to_string()))
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

        let execute_expr = if params.is_empty() {
            format!("await pool.execute(`{sql}`)")
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            format!("await pool.execute(`{sql}`, [{}])", args.join(", "))
        };
        let _ = writeln!(
            out,
            "\tconst [flatRows] = {};",
            js_type_cast("[Array<Record<string, unknown>>, unknown]", &execute_expr)
        );

        let fold = generate_ts_grouped_fold_body(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
            true,
            |name, _ty| ts_member_access("row", name),
        );
        out.push_str(&fold);
        let _ = write!(out, "}}");
        Ok(out)
    }

    /// JSDoc-mode counterpart of `generate_enum_def`.
    fn generate_enum_def_js(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let variants: Vec<(String, String)> = enum_info
            .values
            .iter()
            .map(|value| {
                (
                    enum_variant_name(value, &self.manifest.naming).to_string(),
                    value.clone(),
                )
            })
            .collect();
        Ok(super::typescript_common::generate_js_enum_def(&type_name, &variants))
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
    use super::TypescriptMysql2Backend;
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
                sql_type: "int".to_string(),
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
                sql_type: "decimal".to_string(),
                nullable: true,
                join_group: Some("o".to_string()),
                nullable_before_join: false,
            },
        ]
    }

    /// Before the fix, `row_type = "zod"` returned before the
    /// `outer_join_unions` branch was ever reached, silently discarding the
    /// discriminated union. The `Packet` type must also switch from
    /// `interface extends` to an intersection `type` alias, since TS
    /// interfaces cannot extend a union.
    #[test]
    fn test_zod_row_type_combined_with_outer_join_unions_emits_a_real_union_schema() {
        let mut backend = TypescriptMysql2Backend::new("mysql").unwrap();
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
        assert!(
            row_struct.contains("export type GetUserOrdersRowPacket = RowDataPacket & GetUserOrdersRow;"),
            "Packet type must be an intersection, not `interface extends`, since TS \
             interfaces cannot extend a union; got:\n{row_struct}"
        );
    }

    #[test]
    fn test_zod_row_type_without_outer_join_unions_is_unchanged() {
        let mut backend = TypescriptMysql2Backend::new("mysql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "row_type".to_string(),
                "zod".to_string(),
            )]))
            .unwrap();

        let row_struct = backend
            .generate_row_struct("GetUserOrders", &discriminated_join_columns())
            .unwrap();

        let mut expected = crate::backends::typescript_common::generate_zod_row_struct(
            "GetUserOrdersRow",
            "GetUserOrders",
            &discriminated_join_columns(),
        );
        expected.push_str("\n\nexport interface GetUserOrdersRowPacket extends RowDataPacket, GetUserOrdersRow {}");
        assert_eq!(row_struct, expected);
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

    fn camel_backend() -> TypescriptMysql2Backend {
        let mut backend = TypescriptMysql2Backend::new("mysql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .unwrap();
        backend
    }

    /// This must fail before the fix: `RowDataPacket`'s `[column: string]:
    /// any` index signature is inherited by anything that `extends` it, so
    /// even a strict camelCase-only interface would still silently accept
    /// `row.user_id` -- masking exactly the bug this backend is supposed to
    /// prevent. Under Camel the declared type must not extend RowDataPacket
    /// at all.
    #[test]
    fn test_row_struct_drops_row_data_packet_base_under_camel_case() {
        let backend = camel_backend();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("export interface GetSessionRow {"),
            "must be a plain interface with no base; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains("extends RowDataPacket"),
            "must not extend RowDataPacket under camelCase; got:\n{row_struct}"
        );
    }

    #[test]
    fn test_one_query_fn_remaps_fields_under_camel_case() {
        let backend = camel_backend();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("pool.execute<RowDataPacket[]>("),
            "must fetch through bare RowDataPacket, not the struct type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row.user_id as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    /// This must fail before the fix: mysql2's remap passed `|name, _ty|`
    /// and dropped the type, so every remapped field came back as `any`
    /// through `RowDataPacket`'s `[column: string]: any` index signature.
    /// Every other remap backend casts to the column's declared type, and
    /// without it a wrong type mapping in the manifest is invisible to
    /// `tsc`.
    #[test]
    fn test_remapped_fields_are_cast_to_their_declared_type() {
        let backend = camel_backend();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            !query_fn.contains("userId: row.user_id,"),
            "an uncast read leaves the field `any`; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("id: row.id as number,"),
            "every remapped field must carry its cast; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_many_query_fn_remaps_fields_under_camel_case() {
        let backend = camel_backend();
        let mut query = make_one_query_with_snake_case_column();
        query.command = QueryCommand::Many;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("pool.execute<RowDataPacket[]>("),
            "must fetch through bare RowDataPacket, not the struct type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return rows.map((row) => ({"),
            "must map each row; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row.user_id as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_one_query_fn_keeps_the_typed_generic_under_the_default_snake_case() {
        let backend = TypescriptMysql2Backend::new("mysql").unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("pool.execute<GetSessionRow[]>("),
            "default field_case must keep the original typed generic unchanged; got:\n{query_fn}"
        );
        let row_struct = result.row_struct.as_deref().unwrap();
        assert!(
            row_struct.contains("extends RowDataPacket"),
            "default field_case must keep the RowDataPacket base; got:\n{row_struct}"
        );
    }

    /// Zod mode's bridging `{struct_name}Packet` type exists only to satisfy
    /// mysql2's `execute<T extends RowDataPacket>` constraint for a fetch
    /// that trusts the schema's inferred shape. Under Camel that fetch
    /// always goes through bare `RowDataPacket` instead (see
    /// `generate_row_struct`), so the Packet type has no reason to exist and
    /// must not be generated.
    #[test]
    fn test_zod_row_type_drops_packet_type_under_camel_case() {
        let mut backend = camel_backend();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "row_type".to_string(),
                "zod".to_string(),
            )]))
            .unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            !row_struct.contains("Packet"),
            "camelCase zod mode must not emit a Packet bridging type; got:\n{row_struct}"
        );
    }

    /// MySQL/MariaDB idiomatically backtick-quotes identifiers
    /// (`` `users`.`id` ``). Unescaped, that backtick would terminate the
    /// generated template literal early — this is reachable in real MySQL
    /// SQL, not just adversarial input.
    #[test]
    fn test_query_fn_escapes_user_backtick_in_sql() {
        let backend = TypescriptMysql2Backend::new("mysql").unwrap();
        let query = make_one_query("SELECT `id` FROM `users`");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"SELECT \`id\` FROM \`users\`"),
            "user backticks must be escaped; got:\n{query_fn}"
        );
    }

    /// A literal `${` in the user's SQL must not become a live JS
    /// interpolation.
    #[test]
    fn test_query_fn_escapes_user_dollar_brace_in_sql() {
        let backend = TypescriptMysql2Backend::new("mysql").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE name = 'literal ${evil}'");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"'literal \${evil}'"),
            "user's literal ${{}} must be escaped; got:\n{query_fn}"
        );
    }

    /// A literal backslash in the user's SQL must be doubled.
    #[test]
    fn test_query_fn_escapes_user_backslash_in_sql() {
        let backend = TypescriptMysql2Backend::new("mysql").unwrap();
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
    fn test_grouped_typescript_mysql2_structs() {
        let backend = TypescriptMysql2Backend::new("mysql").unwrap();
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
    fn test_grouped_typescript_mysql2_query_fn() {
        let backend = TypescriptMysql2Backend::new("mysql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("getUsersWithOrders"), "missing fn; got:\n{query_fn}");
        assert!(
            query_fn.contains("Promise<GetUsersWithOrdersRow[]>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("pool.execute<RowDataPacket[]>"),
            "must use pool.execute; got:\n{query_fn}"
        );
        assert!(query_fn.contains("row.id"), "must use dot access; got:\n{query_fn}");
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
    fn test_structs_only_suppresses_query_fn_and_drops_pool_but_keeps_row_data_packet() {
        let mut backend = TypescriptMysql2Backend::new("mysql").unwrap();
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
                .contains("interface GetUserByIdRow extends RowDataPacket"),
            "row struct must still be emitted"
        );

        let header = backend.file_header();
        assert!(
            !header.contains("Pool"),
            "the unused Pool driver import must be dropped; got:\n{header}"
        );
        assert!(
            header.contains("RowDataPacket"),
            "RowDataPacket stays imported since the row type still extends it; got:\n{header}"
        );
    }

    #[test]
    fn test_structs_only_unset_leaves_query_fn_present() {
        let backend = TypescriptMysql2Backend::new("mysql").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = ?");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert!(
            !result.query_fn.as_deref().unwrap().is_empty(),
            "without structs_only the query function must still be generated"
        );
        assert!(backend.file_header().contains("Pool, RowDataPacket"));
    }

    #[test]
    fn test_structs_only_rejects_invalid_value() {
        let mut backend = TypescriptMysql2Backend::new("mysql").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    #[test]
    fn test_structs_only_combined_with_zod_row_type_keeps_zod_import_and_schema() {
        let mut backend = TypescriptMysql2Backend::new("mysql").unwrap();
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
        assert!(!header.contains("Pool"), "got:\n{header}");
    }

    /// This must fail before the fix: the header hardcoded `RowDataPacket`
    /// on the justification that every generated row type extends or
    /// intersects it. Under Camel they no longer do, and `structs_only`
    /// removes the query functions that hold the only other reference — so
    /// the import was emitted with nothing referencing it, which
    /// `noUnusedLocals` rejects.
    #[test]
    fn test_camel_case_with_structs_only_emits_no_driver_import() {
        let mut backend = camel_backend();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "structs_only".to_string(),
                "true".to_string(),
            )]))
            .unwrap();

        assert_eq!(backend.file_header(), "");
    }

    /// The zod import is independent of the driver import, so it must
    /// survive on its own when the driver import drops out.
    #[test]
    fn test_camel_case_with_structs_only_keeps_the_zod_import_alone() {
        let mut backend = camel_backend();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("structs_only".to_string(), "true".to_string()),
                ("row_type".to_string(), "zod".to_string()),
            ]))
            .unwrap();

        assert_eq!(backend.file_header(), "import { z } from \"zod\";\n");
    }

    /// Without `structs_only` the query functions still fetch through
    /// `pool.execute<RowDataPacket[]>`, so the import is genuinely
    /// referenced even though the row types dropped the base.
    #[test]
    fn test_camel_case_keeps_row_data_packet_when_query_fns_are_generated() {
        let backend = camel_backend();
        assert_eq!(
            backend.file_header(),
            "import type { Pool, RowDataPacket } from \"mysql2/promise\";\n"
        );
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
        let backend = TypescriptMysql2Backend::new("mysql").unwrap();
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
        let backend = TypescriptMysql2Backend::new("mysql").unwrap();
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

    // -- javascript-mysql2 (JSDoc emit mode, #81) -----------------------------

    fn js_backend() -> TypescriptMysql2Backend {
        TypescriptMysql2Backend::new_js("mysql").unwrap()
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
                sql_type: "int".to_string(),
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
            q.sql = "SELECT id, bio FROM users WHERE id = ?".to_string();
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

    #[test]
    fn test_js_mode_name_is_javascript_mysql2() {
        assert_eq!(js_backend().name(), "javascript-mysql2");
    }

    #[test]
    fn test_js_mode_file_header_has_no_ts_only_imports() {
        assert_eq!(js_backend().file_header(), "");
    }

    #[test]
    fn test_js_mode_row_struct_emits_nullable_column_as_type_or_null_with_no_row_data_packet_base() {
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
        assert!(
            !row_struct.contains("RowDataPacket"),
            "JSDoc typedefs are structural; there is no RowDataPacket base to carry; got:\n{row_struct}"
        );
    }

    #[test]
    fn test_js_mode_one_query_fn_is_plain_js_with_jsdoc_types() {
        let backend = js_backend();
        let query = query_with_nullable_and_non_nullable_columns(QueryCommand::One);
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("@param {import(\"mysql2/promise\").Pool} pool"),
            "got:\n{query_fn}"
        );
        assert!(query_fn.contains("@param {number} id"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("@returns {Promise<GetUserByIdRow>}"),
            "`:one` must not be nullable; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export async function getUserById(pool, id) {"),
            "got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("pool.execute<"),
            "must not use a TS generic type argument; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains(" as "),
            "must not use a TS `as` assertion; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains(
                "const row = rows[0];\n\tif (row === undefined) {\n\t\tthrow new Error(\"no row found for query: GetUserById\");\n\t}\n\treturn row;"
            ),
            "`:one` must throw on a missing row, not return null; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("?? null"),
            "`:one` must not return null; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_many_query_fn_returns_promise_array() {
        let backend = js_backend();
        let query = query_with_nullable_and_non_nullable_columns(QueryCommand::Many);
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("@returns {Promise<GetUserByIdRow[]>}"),
            "got:\n{query_fn}"
        );
        assert!(query_fn.contains("return rows;"), "got:\n{query_fn}");
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    #[test]
    fn test_js_mode_exec_query_fn_returns_promise_void() {
        let backend = js_backend();
        let mut query = query_with_nullable_and_non_nullable_columns(QueryCommand::Exec);
        query.sql = "DELETE FROM users WHERE id = ?".to_string();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {Promise<void>}"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("export async function getUserById(pool, id) {"),
            "got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_exec_result_uses_jsdoc_cast_not_as() {
        let backend = js_backend();
        let mut query = query_with_nullable_and_non_nullable_columns(QueryCommand::ExecResult);
        query.sql = "DELETE FROM users WHERE id = ?".to_string();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {Promise<number>}"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("/** @type {[{ affectedRows: number }, unknown]} */ (await pool.execute("),
            "the whole execute() call must be cast to a concrete tuple, since mysql2's untyped \
             QueryResult union is not guaranteed array-like and `tsc --strict` rejects destructuring \
             it directly (TS2488); got:\n{query_fn}"
        );
        assert!(query_fn.contains("const [result] ="), "got:\n{query_fn}");
        assert!(query_fn.contains("return result.affectedRows;"), "got:\n{query_fn}");
        assert!(!query_fn.contains(" as "), "got:\n{query_fn}");
    }

    /// Regression (verified against real `tsc --checkJs --strict`): mysql2's
    /// untyped `pool.execute(...)` returns a `QueryResult` union that is not
    /// guaranteed array-like, so `const [flatRows] = await pool.execute(...)`
    /// fails to destructure under `--strict` (TS2488) without this cast.
    #[test]
    fn test_js_mode_grouped_query_fn_casts_flat_rows() {
        let backend = js_backend();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(
                "const [flatRows] = /** @type {[Array<Record<string, unknown>>, unknown]} */ (await pool.execute("
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
                .expect_err(&format!("javascript-mysql2 must reject {key} = {value}"));
            assert!(err.to_string().contains("javascript-mysql2"), "{err}");
        }
    }
}
