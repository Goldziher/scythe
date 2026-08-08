use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_camel_case, to_pascal_case,
};
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
    generate_ts_one_row_remap, generate_ts_union_row_struct, generate_zod_enum, generate_zod_grouped_structs,
    generate_zod_row_struct, generate_zod_union_row_struct, js_fn_signature_line, parse_bool_option,
};
use crate::singularize;

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/typescript-pg.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/typescript-pg.redshift.toml");

pub struct TypescriptPgBackend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
    /// When true, only emit type definitions (interfaces/Zod schemas, enums,
    /// composites) — no query functions, and no `pg` driver import (which
    /// would otherwise be unused).
    structs_only: bool,
    /// Under `Camel`, `:one`/`:opt`/`:many` reconstruct the row field by
    /// field from the driver's raw (snake_case) keys instead of trusting the
    /// `client.query<StructName>` generic -- see
    /// [`generate_ts_one_row_remap`]/[`generate_ts_many_row_remap`]. `Snake`
    /// (the default) keeps that generic, which is sound there.
    field_case: TsFieldCase,
    /// Emit plain, JSDoc-annotated `.js` instead of `.ts` -- the
    /// `javascript-pg` registry name (#81). Selected once at construction by
    /// [`Self::new_js`]; not a backend option, since it changes the emitted
    /// file's language, not a stylistic knob within one language. `.js`
    /// output has no `interface`/`type`/`enum`/`as` syntax to carry
    /// `outer_join_unions`, Zod, or the `camelCase` remap cast, so those
    /// three options are rejected outright in this mode (see
    /// `apply_options`) rather than silently downgraded.
    js_mode: bool,
}

impl TypescriptPgBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_TOML,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!(
                        "typescript-pg only supports PostgreSQL/Redshift, got engine '{}'",
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
            js_mode: false,
        })
    }

    /// As [`Self::new`], but selecting the `javascript-pg` JSDoc emit mode.
    pub fn new_js(engine: &str) -> Result<Self, ScytheError> {
        let mut backend = Self::new(engine)?;
        backend.js_mode = true;
        Ok(backend)
    }
}

impl CodegenBackend for TypescriptPgBackend {
    fn name(&self) -> &str {
        if self.js_mode { "javascript-pg" } else { "typescript-pg" }
    }

    /// The manifest is shared with `typescript-pg` and says `ts`; JSDoc output
    /// is plain JavaScript and must land in a `.js` file to be runnable.
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
        &["postgresql", "redshift"]
    }

    fn file_header(&self) -> String {
        // JSDoc types are self-contained (`{import("pg").PoolClient}`
        // written directly in the `@param` tag -- see `generate_query_fn`),
        // so `.js` output never needs a driver import at all, typed or
        // otherwise; Zod is one of the options rejected in this mode (see
        // `apply_options`), so there is no zod import to add either. That
        // leaves nothing for this header to carry: the "do not edit" notice
        // lives in the scythe:provenance line every backend now emits.
        if self.js_mode {
            return String::new();
        }
        if self.structs_only {
            if self.row_type == TsRowType::Zod {
                return "import { z } from \"zod\";\n".to_string();
            }
            return String::new();
        }
        let mut header = "import type { PoolClient } from \"pg\";\n".to_string();
        if self.row_type == TsRowType::Zod {
            header.push_str("import { z } from \"zod\";\n");
        }
        header
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        if self.js_mode {
            return Ok(generate_js_typedef_row_struct(&struct_name, query_name, columns));
        }
        if self.row_type == TsRowType::Zod {
            if self.outer_join_unions {
                return Ok(generate_zod_union_row_struct(&struct_name, query_name, columns));
            }
            return Ok(generate_zod_row_struct(&struct_name, query_name, columns));
        }
        if self.outer_join_unions {
            return Ok(generate_ts_union_row_struct(&struct_name, query_name, columns, None));
        }
        Ok(generate_ts_interface_row_struct(&struct_name, query_name, columns))
    }

    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let singular = singularize(table_name);
        let name = to_pascal_case(&singular);
        self.generate_row_struct(&name, columns)
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

        let query_sig_params: Vec<(String, String)> = std::iter::once(("client".to_string(), "PoolClient".to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();

        let write_typed_query =
            |out: &mut String, prefix: &str, type_name: &str, sql: &str, params: &[ResolvedParam]| {
                let _ = writeln!(out, "{}client.query<{}>(", prefix, type_name);
                let _ = writeln!(out, "\t\t`{}`,", sql);
                if !params.is_empty() {
                    let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
                    let _ = writeln!(out, "\t\t[{}],", args.join(", "));
                }
                let _ = writeln!(out, "\t);");
            };

        let write_untyped_query = |out: &mut String, prefix: &str, sql: &str, params: &[ResolvedParam]| {
            let param_str = if params.is_empty() {
                String::new()
            } else {
                let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
                format!(", [{}]", args.join(", "))
            };
            let oneliner = format!("{}client.query(`{}`{});", prefix, sql, param_str);
            let estimated_len = oneliner.replace('\t', "    ").len();
            if estimated_len <= 80 {
                let _ = writeln!(out, "{}", oneliner);
            } else {
                let _ = writeln!(out, "{}client.query(", prefix);
                let _ = writeln!(out, "\t\t`{}`,", sql);
                if !params.is_empty() {
                    let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
                    let _ = writeln!(out, "\t\t[{}],", args.join(", "));
                }
                let _ = writeln!(out, "\t);");
            }
        };

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

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(out, "/** Fetch a single {} or null. */", struct_name);
                let ret = format!("Promise<{} | null>", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                match self.field_case {
                    TsFieldCase::Snake => {
                        write_typed_query(&mut out, "\tconst { rows } = await ", struct_name, &sql, params);
                        let _ = writeln!(out, "\treturn rows[0] ?? null;");
                    }
                    TsFieldCase::Camel => {
                        write_typed_query(
                            &mut out,
                            "\tconst { rows } = await ",
                            "Record<string, unknown>",
                            &sql,
                            params,
                        );
                        let _ = writeln!(out, "\tconst row = rows[0];");
                        out.push_str(&generate_ts_one_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            |name, ty| format!("row.{name} as {ty}"),
                        ));
                    }
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "/** Fetch all {} rows. */", struct_name);
                let ret = format!("Promise<{}[]>", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                match self.field_case {
                    TsFieldCase::Snake => {
                        write_typed_query(&mut out, "\tconst { rows } = await ", struct_name, &sql, params);
                        let _ = writeln!(out, "\treturn rows;");
                    }
                    TsFieldCase::Camel => {
                        write_typed_query(
                            &mut out,
                            "\tconst { rows } = await ",
                            "Record<string, unknown>",
                            &sql,
                            params,
                        );
                        out.push_str(&generate_ts_many_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            |name, ty| format!("row.{name} as {ty}"),
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
                        let _ = writeln!(out, "\t{}: {};", p.field_name, p.full_type);
                    }
                    let _ = writeln!(out, "}}");
                    let _ = writeln!(out);
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("client".to_string(), "PoolClient".to_string()),
                        ("items".to_string(), format!("{}[]", params_type_name)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "Promise<void>");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"BEGIN\");");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tawait client.query(");
                    let _ = writeln!(out, "\t\t\t\t`{}`,", sql);
                    let args: Vec<String> = params.iter().map(|p| format!("item.{}", p.field_name)).collect();
                    let _ = writeln!(out, "\t\t\t\t[{}],", args.join(", "));
                    let _ = writeln!(out, "\t\t\t);");
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait client.query(\"COMMIT\");");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"ROLLBACK\");");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("client".to_string(), "PoolClient".to_string()),
                        ("items".to_string(), format!("{}[]", params[0].full_type)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "Promise<void>");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"BEGIN\");");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tawait client.query(`{}`, [item]);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait client.query(\"COMMIT\");");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"ROLLBACK\");");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_sig_params = vec![
                        ("client".to_string(), "PoolClient".to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "Promise<void>");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"BEGIN\");");
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "\t\t\tawait client.query(`{}`);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait client.query(\"COMMIT\");");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"ROLLBACK\");");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "/** Execute a query returning no rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "Promise<void>");
                write_untyped_query(&mut out, "\tawait ", &sql, params);
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "/** Execute a query and return the number of affected rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "Promise<number>");
                write_untyped_query(&mut out, "\tconst result = await ", &sql, params);
                let _ = writeln!(out, "\treturn result.rowCount ?? 0;");
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
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let inline_params = if params.is_empty() {
            "client: PoolClient".to_string()
        } else {
            format!("client: PoolClient, {}", param_list)
        };
        let ret = format!("Promise<{parent_struct_name}[]>");

        let oneliner = format!("export async function {func_name}({inline_params}): {ret} {{");
        if oneliner.len() <= 80 {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "{oneliner}");
        } else {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "export async function {func_name}(");
            let _ = writeln!(out, "\tclient: PoolClient,");
            for p in params {
                let _ = writeln!(out, "\t{}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): {ret} {{");
        }

        let _ = writeln!(out, "\tconst {{ rows: flatRows }} = await client.query(");
        let _ = writeln!(out, "\t\t`{sql}`,");
        if !params.is_empty() {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            let _ = writeln!(out, "\t\t[{}],", args.join(", "));
        }
        let _ = writeln!(out, "\t);");

        let fold = generate_ts_grouped_fold_body(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
            false,
            |name, ty| format!("row.{name} as {ty}"),
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
            return Ok(generate_zod_enum(&type_name, &enum_info.values));
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
        if self.js_mode {
            return self.generate_composite_def_js(composite);
        }
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = writeln!(out, "/** Composite type {}. */", composite.sql_name);
        let _ = writeln!(out, "export interface {} {{", name);
        if composite.fields.is_empty() {
        } else {
            for field in &composite.fields {
                let ts_type = resolve_type(&field.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .map_err(|e| {
                        ScytheError::new(ErrorCode::InternalError, format!("composite field type error: {}", e))
                    })?;
                let _ = writeln!(out, "\t{}: {};", to_camel_case(&field.name), ts_type);
            }
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

        // Plain `.js` has no `type`/`interface` alias syntax and no `as T`
        // assertion, so none of these three TypeScript-shape options has
        // anywhere to go in JSDoc mode. Rejecting outright (rather than
        // silently ignoring the option, which `reject_unknown_options`
        // exists specifically to avoid doing) keeps a `javascript-pg`
        // manifest that requests one of these an explicit, caught error
        // instead of quietly emitting `typescript-pg`'s default shape.
        if self.js_mode {
            if self.row_type == TsRowType::Zod {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-pg does not support row_type = \"zod\": the inferred `export type X = \
                     z.infer<...>` alias is TypeScript-only syntax a plain .js file cannot carry -- use \
                     typescript-pg for Zod row types"
                        .to_string(),
                ));
            }
            if self.outer_join_unions {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-pg does not support outer_join_unions: the discriminated union is a \
                     TypeScript `type X = A & (B | C)` alias, which plain .js cannot carry -- use \
                     typescript-pg"
                        .to_string(),
                ));
            }
            if self.field_case == TsFieldCase::Camel {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    "javascript-pg does not support field_case = \"camelCase\": the field remap needs a \
                     TypeScript `as T` assertion, which plain .js cannot carry -- use typescript-pg"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl TypescriptPgBackend {
    /// JSDoc-mode counterpart of `generate_query_fn` (see `CodegenBackend::generate_query_fn`).
    ///
    /// `field_case = "camelCase"` is rejected in `apply_options` for
    /// `js_mode`, so unlike the TypeScript path this never needs the
    /// snake-to-camel remap and its columns argument -- every command reads
    /// the driver's row straight through, exactly like the TypeScript path's
    /// `TsFieldCase::Snake` arm.
    fn generate_query_fn_js(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        if self.structs_only {
            return Ok(String::new());
        }

        const CLIENT_TYPE: &str = "import(\"pg\").PoolClient";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let query_sig_params: Vec<(String, String)> = std::iter::once(("client".to_string(), CLIENT_TYPE.to_string()))
            .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
            .collect();

        let write_query = |out: &mut String, prefix: &str, sql: &str, params: &[ResolvedParam]| {
            let param_str = if params.is_empty() {
                String::new()
            } else {
                let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
                format!(", [{}]", args.join(", "))
            };
            let oneliner = format!("{}client.query(`{}`{});", prefix, sql, param_str);
            let estimated_len = oneliner.replace('\t', "    ").len();
            if estimated_len <= 80 {
                let _ = writeln!(out, "{}", oneliner);
            } else {
                let _ = writeln!(out, "{}client.query(", prefix);
                let _ = writeln!(out, "\t\t`{}`,", sql);
                if !params.is_empty() {
                    let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
                    let _ = writeln!(out, "\t\t[{}],", args.join(", "));
                }
                let _ = writeln!(out, "\t);");
            }
        };

        let write_signature = |out: &mut String, description: &str, sig_params: &[(String, String)], ret: &str| {
            out.push_str(&generate_jsdoc_fn_header(description, sig_params, ret));
            let _ = writeln!(out);
            let _ = writeln!(out, "{}", js_fn_signature_line(true, &func_name, sig_params));
        };

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                write_signature(
                    &mut out,
                    &format!("Fetch a single {} or null.", struct_name),
                    &query_sig_params,
                    &format!("Promise<{} | null>", struct_name),
                );
                write_query(&mut out, "\tconst { rows } = await ", &sql, params);
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
                write_query(&mut out, "\tconst { rows } = await ", &sql, params);
                let _ = writeln!(out, "\treturn rows;");
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
                        ("client".to_string(), CLIENT_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params_type_name)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!(
                            "Execute {} for each item in the batch within a transaction.",
                            analyzed.name
                        ),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"BEGIN\");");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tawait client.query(");
                    let _ = writeln!(out, "\t\t\t\t`{}`,", sql);
                    let args: Vec<String> = params.iter().map(|p| format!("item.{}", p.field_name)).collect();
                    let _ = writeln!(out, "\t\t\t\t[{}],", args.join(", "));
                    let _ = writeln!(out, "\t\t\t);");
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait client.query(\"COMMIT\");");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"ROLLBACK\");");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let batch_sig_params = vec![
                        ("client".to_string(), CLIENT_TYPE.to_string()),
                        ("items".to_string(), format!("Array<{}>", params[0].full_type)),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!(
                            "Execute {} for each item in the batch within a transaction.",
                            analyzed.name
                        ),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"BEGIN\");");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tawait client.query(`{}`, [item]);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait client.query(\"COMMIT\");");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"ROLLBACK\");");
                    let _ = writeln!(out, "\t\tthrow error;");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else {
                    let batch_sig_params = vec![
                        ("client".to_string(), CLIENT_TYPE.to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    out.push_str(&generate_jsdoc_fn_header(
                        &format!(
                            "Execute {} for each item in the batch within a transaction.",
                            analyzed.name
                        ),
                        &batch_sig_params,
                        "Promise<void>",
                    ));
                    let _ = writeln!(out);
                    let _ = writeln!(out, "{}", js_fn_signature_line(true, &batch_fn_name, &batch_sig_params));
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"BEGIN\");");
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "\t\t\tawait client.query(`{}`);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait client.query(\"COMMIT\");");
                    let _ = writeln!(out, "\t}} catch (error) {{");
                    let _ = writeln!(out, "\t\tawait client.query(\"ROLLBACK\");");
                    let _ = writeln!(out, "\t\tthrow error;");
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
                write_query(&mut out, "\tawait ", &sql, params);
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                write_signature(
                    &mut out,
                    "Execute a query and return the number of affected rows.",
                    &query_sig_params,
                    "Promise<number>",
                );
                write_query(&mut out, "\tconst result = await ", &sql, params);
                let _ = writeln!(out, "\treturn result.rowCount ?? 0;");
                let _ = write!(out, "}}");
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

        const CLIENT_TYPE: &str = "import(\"pg\").PoolClient";

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = escape_ts_template_literal(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let sig_params: Vec<(String, String)> = std::iter::once(("client".to_string(), CLIENT_TYPE.to_string()))
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

        let _ = writeln!(out, "\tconst {{ rows: flatRows }} = await client.query(");
        let _ = writeln!(out, "\t\t`{sql}`,");
        if !params.is_empty() {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            let _ = writeln!(out, "\t\t[{}],", args.join(", "));
        }
        let _ = writeln!(out, "\t);");

        let fold = generate_ts_grouped_fold_body(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
            true,
            |name, _ty| format!("row.{name}"),
        );
        out.push_str(&fold);
        let _ = write!(out, "}}");
        Ok(out)
    }

    /// JSDoc-mode counterpart of `generate_enum_def`.
    ///
    /// `as const` is TypeScript-only syntax, so the literal-narrowing it
    /// gives the TS path comes from the JSDoc `@type {const}` utility type
    /// (TS 5.5+) instead -- the JSDoc spelling of the same "freeze these
    /// property types to their literals" request.
    fn generate_enum_def_js(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let values_name = format!("{type_name}Values");
        let mut out = String::new();
        let _ = writeln!(out, "/** @type {{const}} */");
        let _ = writeln!(out, "export const {} = {{", values_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "\t{}: \"{}\",", variant, value);
        }
        let _ = writeln!(out, "}};");
        let _ = writeln!(out);
        let _ = write!(
            out,
            "/** @typedef {{typeof {}[keyof typeof {}]}} {} */",
            values_name, values_name, type_name
        );
        Ok(out)
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
    use super::TypescriptPgBackend;
    use crate::backend_trait::CodegenBackend;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

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

    /// This must fail before the fix: trusting `client.query<StructName>`'s
    /// generic is unsound once `field_case = "camelCase"` renames the
    /// declared fields -- node-postgres still returns snake_case keys.
    #[test]
    fn test_one_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptPgBackend::new("postgresql").unwrap();
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
            query_fn.contains("client.query<Record<string, unknown>>("),
            "must not trust the StructName generic; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row.user_id as number,"),
            "must remap the declared camelCase field from the driver's raw key, cast to its \
             declared type -- row is typed Record<string, unknown>, so row.user_id is `unknown` \
             and an uncast assignment to a number field fails tsc; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_many_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptPgBackend::new("postgresql").unwrap();
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
            query_fn.contains("client.query<Record<string, unknown>>("),
            "must not trust the StructName generic; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return rows.map((row) => ({"),
            "must map each row; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row.user_id as number,"),
            "must remap the declared camelCase field from the driver's raw key, cast to its \
             declared type -- row is typed Record<string, unknown>, so row.user_id is `unknown` \
             and an uncast assignment to a number field fails tsc; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_one_query_fn_keeps_the_typed_generic_under_the_default_snake_case() {
        let backend = TypescriptPgBackend::new("postgresql").unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("client.query<GetSessionRow>("),
            "default field_case must keep the original typed generic unchanged; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("Record<string, unknown>"),
            "must not switch to the remap path under the default; got:\n{query_fn}"
        );
    }

    /// node-postgres passes the SQL text through as a plain template
    /// literal (parameters are bound separately via the array argument), but
    /// a literal backtick in the user's SQL would still terminate that
    /// literal early and corrupt the generated file.
    #[test]
    fn test_query_fn_escapes_user_backtick_in_sql() {
        let backend = TypescriptPgBackend::new("postgresql").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE name = `oops`");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"WHERE name = \`oops\`"),
            "user backtick must be escaped; got:\n{query_fn}"
        );
    }

    /// A literal `${` in the user's SQL must not become a live JS
    /// interpolation of an undeclared identifier.
    #[test]
    fn test_query_fn_escapes_user_dollar_brace_in_sql() {
        let backend = TypescriptPgBackend::new("postgresql").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE name = 'literal ${evil}'");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"'literal \${evil}'"),
            "user's literal ${{}} must be escaped; got:\n{query_fn}"
        );
    }

    /// A literal backslash in the user's SQL must be doubled so it stays a
    /// single literal backslash in the generated JS template literal.
    #[test]
    fn test_query_fn_escapes_user_backslash_in_sql() {
        let backend = TypescriptPgBackend::new("postgresql").unwrap();
        let query = make_one_query(r"SELECT id FROM users WHERE name = E'a\\b'");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"E'a\\\\b'"),
            "user backslash must be doubled; got:\n{query_fn}"
        );
    }

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

    /// Before the fix, `row_type = "zod"` returned before the
    /// `outer_join_unions` branch was ever reached, silently discarding the
    /// discriminated union.
    #[test]
    fn test_zod_row_type_combined_with_outer_join_unions_emits_a_real_union_schema() {
        let mut backend = TypescriptPgBackend::new("postgresql").unwrap();
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

    /// Zod without `outer_join_unions` must be byte-identical to the flat
    /// schema — no regression from adding union support.
    #[test]
    fn test_zod_row_type_without_outer_join_unions_is_unchanged() {
        let mut backend = TypescriptPgBackend::new("postgresql").unwrap();
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

    fn make_one_query_with_outer_join() -> AnalyzedQuery {
        AnalyzedQuery::build(|q| {
            q.name = "GetUserOrder".to_string();
            q.command = QueryCommand::One;
            q.sql = "SELECT u.id, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id".to_string();
            q.columns = vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                // NOT NULL in the schema, so it discriminates the join.
                AnalyzedColumn {
                    name: "order_total".to_string(),
                    neutral_type: "decimal".to_string(),
                    nullable: true,
                    join_group: Some("o".to_string()),
                    nullable_before_join: false,
                    ..Default::default()
                },
                // Independently nullable, so it says nothing about the join.
                AnalyzedColumn {
                    name: "order_notes".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    join_group: Some("o".to_string()),
                    nullable_before_join: true,
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

    /// This must fail before the fix: the remap cast every field to
    /// `col.full_type`, but the union's matched variant declares a join
    /// discriminant as `lang_type` and the unmatched one declares it `null`.
    /// `string | null` is assignable to neither, so `field_case =
    /// "camelCase"` and `outer_join_unions = true` — both accepted by the
    /// same `apply_options` allowlist, with no mutual exclusion — produced a
    /// row object that does not type-check (TS2322).
    #[test]
    fn test_camel_case_combined_with_outer_join_unions_type_checks() {
        let mut backend = TypescriptPgBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([
                ("field_case".to_string(), "camelCase".to_string()),
                ("outer_join_unions".to_string(), "true".to_string()),
            ]))
            .unwrap();

        let result = crate::generate_with_backend(&make_one_query_with_outer_join(), &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        // The matched variant of the union, spelled with the renamed fields.
        assert!(
            row_struct.contains("orderTotal: string; orderNotes: string | null"),
            "the matched variant declares the discriminant non-null; got:\n{row_struct}"
        );
        assert!(
            query_fn.contains("orderTotal: row.order_total as string,"),
            "the remap must cast the discriminant to the matched variant's type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("orderNotes: row.order_notes as string | null,"),
            "a column that was already nullable keeps its full_type cast; got:\n{query_fn}"
        );
    }

    /// The narrowing above is specific to the union shape: with
    /// `outer_join_unions` off the row is a flat interface declaring
    /// `string | null`, so the remap must keep casting to `full_type`.
    #[test]
    fn test_camel_case_without_outer_join_unions_keeps_the_full_type_cast() {
        let mut backend = TypescriptPgBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .unwrap();

        let result = crate::generate_with_backend(&make_one_query_with_outer_join(), &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("orderTotal: row.order_total as string | null,"),
            "flat rows declare the column nullable, so the cast stays nullable; got:\n{query_fn}"
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
    fn test_grouped_typescript_pg_structs() {
        let backend = TypescriptPgBackend::new("postgresql").unwrap();
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
            row_struct.contains("id: number"),
            "parent missing id; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("children: GetUsersWithOrdersChildRow[]"),
            "missing children; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("interface GetUsersWithOrdersRow").unwrap();
        assert!(child_pos < parent_pos, "child must precede parent");
        assert!(result.model_struct.is_none(), "grouped must not produce model_struct");
    }

    #[test]
    fn test_grouped_typescript_pg_query_fn() {
        let backend = TypescriptPgBackend::new("postgresql").unwrap();
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
            query_fn.contains("client.query("),
            "must use client.query; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("new Map<unknown, GetUsersWithOrdersRow>()"),
            "must use Map; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("parent.children.push"),
            "must fold children; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("return result"),
            "must return result; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_structs_only_suppresses_query_fn_and_driver_import() {
        let mut backend = TypescriptPgBackend::new("postgresql").unwrap();
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
            !header.contains("pg"),
            "the unused pg driver import must be dropped; got:\n{header}"
        );
    }

    #[test]
    fn test_structs_only_unset_leaves_query_fn_present() {
        let backend = TypescriptPgBackend::new("postgresql").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = $1");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert!(
            !result.query_fn.as_deref().unwrap().is_empty(),
            "without structs_only the query function must still be generated"
        );
        assert!(
            backend
                .file_header()
                .contains("import type { PoolClient } from \"pg\";")
        );
    }

    #[test]
    fn test_structs_only_rejects_invalid_value() {
        let mut backend = TypescriptPgBackend::new("postgresql").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    #[test]
    fn test_structs_only_combined_with_zod_row_type_keeps_zod_import_and_schema() {
        let mut backend = TypescriptPgBackend::new("postgresql").unwrap();
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
        assert!(!header.contains("pg"), "got:\n{header}");
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
        let backend = TypescriptPgBackend::new("postgresql").unwrap();
        let query = make_batch_query(
            "CreateUserAccountRecord",
            "INSERT INTO user_account_record (name, email) VALUES ($1, $2)",
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
        let backend = TypescriptPgBackend::new("postgresql").unwrap();
        let query = make_batch_query(
            "CreateUserAccountRecordPayload",
            "INSERT INTO user_account_record_payload (payload) VALUES ($1)",
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

    // -- javascript-pg (JSDoc emit mode, #81) --------------------------------

    fn js_backend() -> TypescriptPgBackend {
        TypescriptPgBackend::new_js("postgresql").unwrap()
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
    fn test_js_mode_name_is_javascript_pg() {
        assert_eq!(js_backend().name(), "javascript-pg");
    }

    #[test]
    fn test_js_mode_file_header_has_no_ts_only_imports() {
        let header = js_backend().file_header();
        assert_eq!(header, "");
    }

    /// The critical correctness rule for #81: a nullable column is a
    /// property that is always present and may hold `null` -- `{T | null}`
    /// -- never JSDoc's bracket-optional (`[name]`) or `?`-suffix syntax,
    /// which mean the property may be *absent*.
    fn resolved_columns_with_a_nullable_and_a_non_nullable_field() -> Vec<crate::backend_trait::ResolvedColumn> {
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
        assert!(
            row_struct.contains(" * @property {number} id"),
            "non-nullable column keeps its bare type; got:\n{row_struct}"
        );
        assert!(
            !row_struct.contains("[bio]"),
            "must never use bracket-optional syntax: {row_struct}"
        );
        assert!(
            !row_struct.contains("bio?"),
            "must never use `?`-suffix optional syntax: {row_struct}"
        );
    }

    #[test]
    fn test_js_mode_one_query_fn_is_plain_js_with_jsdoc_types() {
        let backend = js_backend();
        let query = query_with_nullable_and_non_nullable_columns(QueryCommand::One);
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("@param {import(\"pg\").PoolClient} client"),
            "got:\n{query_fn}"
        );
        assert!(query_fn.contains("@param {number} id"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("@returns {Promise<GetUserByIdRow | null>}"),
            "got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("export async function getUserById(client, id) {"),
            "signature line must have no type annotations; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("client.query<"),
            "must not use a TS generic type argument; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains(" as "),
            "must not use a TS `as` assertion; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains(
                "const { rows } = await client.query(\n\t\t`SELECT id, bio FROM users WHERE id = $1`,\n\t\t[id],\n\t);"
            ),
            "got:\n{query_fn}"
        );
        assert!(query_fn.contains("return rows[0] ?? null;"), "got:\n{query_fn}");
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
        assert!(
            query_fn.contains("export async function getUserById(client, id) {"),
            "got:\n{query_fn}"
        );
        assert!(query_fn.contains("return rows;"), "got:\n{query_fn}");
        assert!(
            !query_fn.contains(" as "),
            "must not use a TS `as` assertion; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_exec_query_fn_returns_promise_void() {
        let backend = js_backend();
        let mut query = query_with_nullable_and_non_nullable_columns(QueryCommand::Exec);
        query.sql = "UPDATE users SET bio = $1 WHERE id = $2".to_string();
        query.params = vec![
            AnalyzedParam {
                name: "bio".to_string(),
                neutral_type: "string".to_string(),
                nullable: true,
                position: 1,
            },
            AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 2,
            },
        ];
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {Promise<void>}"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("export async function getUserById(client, bio, id) {"),
            "got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("return "),
            "an :exec function returns nothing; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_exec_result_query_fn_returns_row_count() {
        let backend = js_backend();
        let mut query = query_with_nullable_and_non_nullable_columns(QueryCommand::ExecResult);
        query.sql = "DELETE FROM users WHERE id = $1".to_string();
        query.params = vec![AnalyzedParam {
            name: "id".to_string(),
            neutral_type: "int32".to_string(),
            nullable: false,
            position: 1,
        }];
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("@returns {Promise<number>}"), "got:\n{query_fn}");
        assert!(query_fn.contains("return result.rowCount ?? 0;"), "got:\n{query_fn}");
    }

    #[test]
    fn test_js_mode_grouped_typedef_and_query_fn_have_no_ts_generics() {
        let backend = js_backend();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            row_struct.contains("@typedef {object} GetUsersWithOrdersChildRow"),
            "got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("@typedef {object} GetUsersWithOrdersRow"),
            "got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("@property {GetUsersWithOrdersChildRow[]} children"),
            "got:\n{row_struct}"
        );
        assert!(
            query_fn.contains("const result = [];"),
            "must not carry a `: Type[]` annotation; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("const index = new Map();"),
            "must not carry a `<unknown, Type>` generic; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains(" as "),
            "must not use a TS `as` assertion; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_js_mode_rejects_zod_row_type() {
        let mut backend = js_backend();
        let err = backend
            .apply_options(&std::collections::HashMap::from([(
                "row_type".to_string(),
                "zod".to_string(),
            )]))
            .expect_err("javascript-pg must reject row_type = zod");
        assert!(err.to_string().contains("javascript-pg"), "{err}");
    }

    #[test]
    fn test_js_mode_rejects_outer_join_unions() {
        let mut backend = js_backend();
        let err = backend
            .apply_options(&std::collections::HashMap::from([(
                "outer_join_unions".to_string(),
                "true".to_string(),
            )]))
            .expect_err("javascript-pg must reject outer_join_unions");
        assert!(err.to_string().contains("javascript-pg"), "{err}");
    }

    #[test]
    fn test_js_mode_rejects_camel_case_field_case() {
        let mut backend = js_backend();
        let err = backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .expect_err("javascript-pg must reject field_case = camelCase");
        assert!(err.to_string().contains("javascript-pg"), "{err}");
    }

    #[test]
    fn test_js_mode_batch_fn_uses_jsdoc_typedef_not_interface() {
        let backend = js_backend();
        let query = make_batch_query(
            "CreateUserAccountRecord",
            "INSERT INTO user_account_record (name, email) VALUES ($1, $2)",
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
            query_fn.contains("@typedef {object} CreateUserAccountRecordRowBatchParams"),
            "got:\n{query_fn}"
        );
        assert!(!query_fn.contains("export interface"), "got:\n{query_fn}");
        assert!(
            query_fn.contains("export async function createUserAccountRecordBatch(client, items) {"),
            "got:\n{query_fn}"
        );
    }
}
