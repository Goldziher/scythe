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
    generate_ts_grouped_fold_body, generate_ts_interface_row_struct, generate_ts_many_row_remap,
    generate_ts_one_row_remap, generate_ts_union_row_struct, generate_zod_grouped_structs, generate_zod_row_struct,
    generate_zod_union_row_struct, parse_bool_option, ts_index_access, ts_member_access, ts_property_key,
    ts_row_not_found_throw,
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
    fn file_header_with_value_type(&self, needs_value_type: bool, needs_blob_type: bool) -> String {
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
        })
    }
}

impl CodegenBackend for TypescriptDuckdbBackend {
    fn name(&self) -> &str {
        "typescript-duckdb"
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::TypescriptDuckdbBackend;
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
}
