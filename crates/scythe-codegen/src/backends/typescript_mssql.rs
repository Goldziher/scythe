use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, fn_name, row_struct_name, to_camel_case, to_pascal_case};
use scythe_backend::types::resolve_type;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};
use crate::backends::typescript_common::{
    TsFieldCase, TsRowShape, TsRowType, escape_ts_template_literal, generate_grouped_interface_structs,
    generate_ts_grouped_fold_body, generate_ts_interface_row_struct, generate_ts_many_row_remap,
    generate_ts_one_row_remap, generate_ts_union_row_struct, generate_zod_grouped_structs, generate_zod_row_struct,
    generate_zod_union_row_struct, parse_bool_option, reject_unknown_options,
};
use crate::singularize;

/// Map neutral type to mssql SQL type constant.
fn neutral_to_sql_type(neutral_type: &str) -> &'static str {
    match neutral_type {
        "int16" => "sql.SmallInt",
        "int32" => "sql.Int",
        "int64" => "sql.BigInt",
        "float32" => "sql.Real",
        "float64" => "sql.Float",
        "numeric" | "decimal" => "sql.VarChar",
        "bool" => "sql.Bit",
        "string" => "sql.NVarChar",
        "text" => "sql.Text",
        "date" => "sql.Date",
        "datetime" => "sql.DateTime",
        "datetime_tz" => "sql.DateTimeOffset",
        "uuid" => "sql.UniqueIdentifier",
        "binary" => "sql.Binary",
        _ => "sql.VarChar",
    }
}

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/typescript-mssql.toml");

pub struct TypescriptMssqlBackend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
    /// When true, only emit type definitions (interfaces/Zod schemas, enums,
    /// composites) — no query functions, and no `mssql` driver import
    /// (which would otherwise be unused).
    structs_only: bool,
    /// Under `Camel`, `:one`/`:opt`/`:many` reconstruct the row field by
    /// field from the driver's raw (snake_case) keys instead of trusting the
    /// `request.query<StructName>` generic -- see
    /// [`generate_ts_one_row_remap`]/[`generate_ts_many_row_remap`]. `Snake`
    /// (the default) keeps that generic, which is sound there.
    field_case: TsFieldCase,
}

impl TypescriptMssqlBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "mssql" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("typescript-mssql only supports MSSQL, got engine '{}'", engine),
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

/// Rewrite $1, $2, ... positional params to @p1, @p2, ... for MSSQL.
impl CodegenBackend for TypescriptMssqlBackend {
    fn name(&self) -> &str {
        "typescript-mssql"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["mssql"]
    }

    fn file_header(&self) -> String {
        if self.structs_only {
            if self.row_type == TsRowType::Zod {
                return "import { z } from \"zod\";\n".to_string();
            }
            return String::new();
        }
        let mut header = "import sql from \"mssql\";\n".to_string();
        if self.row_type == TsRowType::Zod {
            header.push_str("import { z } from \"zod\";\n");
        }
        header
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
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
        if self.structs_only {
            return Ok(String::new());
        }

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let sql = escape_ts_template_literal(&super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!("@p{n}"),
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
            std::iter::once(("pool".to_string(), "sql.ConnectionPool".to_string()))
                .chain(params.iter().map(|p| (p.field_name.clone(), p.full_type.clone())))
                .collect();

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(out, "/** Fetch a single {} or null. */", struct_name);
                let ret = format!("{} | null", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                let _ = writeln!(out, "\tconst request = pool.request();");
                for (i, p) in params.iter().enumerate() {
                    let sql_type = neutral_to_sql_type(&p.neutral_type);
                    let _ = writeln!(out, "\trequest.input(\"p{}\", {}, {});", i + 1, sql_type, p.field_name);
                }
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\tconst result = await request.query<{}>(`{}`);", struct_name, sql);
                        let _ = writeln!(out, "\treturn result.recordset[0] ?? null;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(
                            out,
                            "\tconst result = await request.query<Record<string, unknown>>(`{}`);",
                            sql
                        );
                        let _ = writeln!(out, "\tconst row = result.recordset[0];");
                        out.push_str(&generate_ts_one_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            |name, ty| format!("row['{name}'] as {ty}"),
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
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("pool".to_string(), "sql.ConnectionPool".to_string()),
                        ("items".to_string(), format!("{}[]", params_type_name)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tconst transaction = pool.transaction();");
                    let _ = writeln!(out, "\tawait transaction.begin();");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tconst request = transaction.request();");
                    for (i, p) in params.iter().enumerate() {
                        let sql_type = neutral_to_sql_type(&p.neutral_type);
                        let _ = writeln!(
                            out,
                            "\t\t\trequest.input(\"p{}\", {}, item.{});",
                            i + 1,
                            sql_type,
                            p.field_name
                        );
                    }
                    let _ = writeln!(out, "\t\t\tawait request.query(`{}`);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait transaction.commit();");
                    let _ = writeln!(out, "\t}} catch (e) {{");
                    let _ = writeln!(out, "\t\tawait transaction.rollback();");
                    let _ = writeln!(out, "\t\tthrow e;");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("pool".to_string(), "sql.ConnectionPool".to_string()),
                        ("items".to_string(), format!("{}[]", params[0].full_type)),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tconst transaction = pool.transaction();");
                    let _ = writeln!(out, "\tawait transaction.begin();");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tconst request = transaction.request();");
                    let sql_type = neutral_to_sql_type(&params[0].neutral_type);
                    let _ = writeln!(out, "\t\t\trequest.input(\"p1\", {}, item);", sql_type);
                    let _ = writeln!(out, "\t\t\tawait request.query(`{}`);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait transaction.commit();");
                    let _ = writeln!(out, "\t}} catch (e) {{");
                    let _ = writeln!(out, "\t\tawait transaction.rollback();");
                    let _ = writeln!(out, "\t\tthrow e;");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(out, "/** Execute {} for each item in the batch. */", analyzed.name);
                    let batch_sig_params = vec![
                        ("pool".to_string(), "sql.ConnectionPool".to_string()),
                        ("count".to_string(), "number".to_string()),
                    ];
                    write_fn_sig(&mut out, &batch_fn_name, &batch_sig_params, "void");
                    let _ = writeln!(out, "\tconst transaction = pool.transaction();");
                    let _ = writeln!(out, "\tawait transaction.begin();");
                    let _ = writeln!(out, "\ttry {{");
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "\t\t\tconst request = transaction.request();");
                    let _ = writeln!(out, "\t\t\tawait request.query(`{}`);", sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t\tawait transaction.commit();");
                    let _ = writeln!(out, "\t}} catch (e) {{");
                    let _ = writeln!(out, "\t\tawait transaction.rollback();");
                    let _ = writeln!(out, "\t\tthrow e;");
                    let _ = writeln!(out, "\t}}");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "/** Fetch all {} rows. */", struct_name);
                let ret = format!("{}[]", struct_name);
                write_fn_sig(&mut out, &func_name, &query_sig_params, &ret);
                let _ = writeln!(out, "\tconst request = pool.request();");
                for (i, p) in params.iter().enumerate() {
                    let sql_type = neutral_to_sql_type(&p.neutral_type);
                    let _ = writeln!(out, "\trequest.input(\"p{}\", {}, {});", i + 1, sql_type, p.field_name);
                }
                match self.field_case {
                    TsFieldCase::Snake => {
                        let _ = writeln!(out, "\tconst result = await request.query<{}>(`{}`);", struct_name, sql);
                        let _ = writeln!(out, "\treturn result.recordset;");
                    }
                    TsFieldCase::Camel => {
                        let _ = writeln!(
                            out,
                            "\tconst result = await request.query<Record<string, unknown>>(`{}`);",
                            sql
                        );
                        let _ = writeln!(out, "\tconst rows = result.recordset;");
                        out.push_str(&generate_ts_many_row_remap(
                            columns,
                            TsRowShape::from_outer_join_unions(self.outer_join_unions),
                            |name, ty| format!("row['{name}'] as {ty}"),
                        ));
                    }
                }
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "/** Execute a query returning no rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "void");
                let _ = writeln!(out, "\tconst request = pool.request();");
                for (i, p) in params.iter().enumerate() {
                    let sql_type = neutral_to_sql_type(&p.neutral_type);
                    let _ = writeln!(out, "\trequest.input(\"p{}\", {}, {});", i + 1, sql_type, p.field_name);
                }
                let _ = writeln!(out, "\tawait request.query(`{}`);", sql);
                let _ = write!(out, "}}");
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped command is routed through generate_grouped_query_fn, not generate_query_fn")
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "/** Execute a query and return the number of affected rows. */");
                write_fn_sig(&mut out, &func_name, &query_sig_params, "number");
                let _ = writeln!(out, "\tconst request = pool.request();");
                for (i, p) in params.iter().enumerate() {
                    let sql_type = neutral_to_sql_type(&p.neutral_type);
                    let _ = writeln!(out, "\trequest.input(\"p{}\", {}, {});", i + 1, sql_type, p.field_name);
                }
                let _ = writeln!(out, "\tconst result = await request.query(`{}`);", sql);
                let _ = writeln!(out, "\treturn result.rowsAffected[0] ?? 0;");
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
        let sql_clean = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let sql = escape_ts_template_literal(&super::rewrite_pg_placeholders(&sql_clean, |n| format!("@p{n}")));

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let inline_params = if params.is_empty() {
            "pool: sql.ConnectionPool".to_string()
        } else {
            format!("pool: sql.ConnectionPool, {}", param_list)
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
            let _ = writeln!(out, "\tpool: sql.ConnectionPool,");
            for p in params {
                let _ = writeln!(out, "\t{}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): {ret} {{");
        }

        let _ = writeln!(out, "\tconst request = pool.request();");
        for (i, (_ap, rp)) in analyzed.params.iter().zip(params.iter()).enumerate() {
            let sql_type = neutral_to_sql_type(&rp.neutral_type);
            let _ = writeln!(out, "\trequest.input('p{}', {}, {});", i + 1, sql_type, rp.field_name);
        }
        let _ = writeln!(out, "\tconst {{ recordset: flatRows }} = await request.query(`{sql}`);");

        let fold = generate_ts_grouped_fold_body(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
            false,
            |name, ty| format!("row['{name}'] as {ty}"),
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
            ));
        }
        let mut out = String::new();
        let variants: Vec<String> = enum_info.values.iter().map(|v| format!("\"{}\"", v)).collect();
        let _ = write!(out, "export type {} = {};", type_name, variants.join(" | "));
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
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
            let _ = writeln!(out, "\t{}: {};", to_camel_case(&field.name), ts_type);
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
    use super::TypescriptMssqlBackend;
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
        let mut backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
        let mut backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
        AnalyzedQuery {
            name: "GetUserById".to_string(),
            command: QueryCommand::One,
            sql: sql.to_string(),
            columns: vec![AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            }],
            params: vec![],
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
            optional_params: vec![],
            group_by: None,
            custom: vec![],
        }
    }

    fn make_one_query_with_snake_case_column() -> AnalyzedQuery {
        AnalyzedQuery {
            name: "GetSession".to_string(),
            command: QueryCommand::One,
            sql: "SELECT id, user_id FROM sessions WHERE id = $1".to_string(),
            columns: vec![
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
            ],
            params: vec![],
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
            optional_params: vec![],
            group_by: None,
            custom: vec![],
        }
    }

    /// This must fail before the fix: trusting `request.query<StructName>`'s
    /// generic is unsound once `field_case = "camelCase"` renames the
    /// declared fields -- mssql still returns snake_case keys.
    #[test]
    fn test_one_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
            query_fn.contains("request.query<Record<string, unknown>>("),
            "must not trust the StructName generic; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("userId: row['user_id'] as number,"),
            "must remap the declared camelCase field from the driver's raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_many_query_fn_remaps_fields_under_camel_case() {
        let mut backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
            query_fn.contains("request.query<Record<string, unknown>>("),
            "must not trust the StructName generic; got:\n{query_fn}"
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
    fn test_one_query_fn_keeps_the_typed_generic_under_the_default_snake_case() {
        let backend = TypescriptMssqlBackend::new("mssql").unwrap();
        let query = make_one_query_with_snake_case_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("request.query<GetSessionRow>("),
            "default field_case must keep the original typed generic unchanged; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("Record<string, unknown>"),
            "must not switch to the remap path under the default; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_query_fn_escapes_user_backtick_in_sql() {
        let backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
        let backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
        let backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
        AnalyzedQuery {
            name: "GetUsersWithOrders".to_string(),
            command: QueryCommand::Grouped,
            sql: "SELECT u.id, u.name, o.id AS order_id, o.total FROM users u JOIN orders o ON o.user_id = u.id"
                .to_string(),
            columns: all_cols,
            params: vec![],
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
            optional_params: vec![],
            group_by: Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: parent_cols,
                child_columns: child_cols,
            }),
            custom: vec![],
        }
    }

    #[test]
    fn test_grouped_typescript_mssql_structs() {
        let backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
    fn test_grouped_typescript_mssql_query_fn() {
        let backend = TypescriptMssqlBackend::new("mssql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("getUsersWithOrders"), "missing fn; got:\n{query_fn}");
        assert!(
            query_fn.contains("Promise<GetUsersWithOrdersRow[]>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("pool.request()"),
            "must use pool.request; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("recordset: flatRows"),
            "must destructure recordset; got:\n{query_fn}"
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
    fn test_structs_only_suppresses_query_fn_and_driver_import() {
        let mut backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
            !header.contains("mssql"),
            "the unused mssql driver import must be dropped; got:\n{header}"
        );
    }

    #[test]
    fn test_structs_only_unset_leaves_query_fn_present() {
        let backend = TypescriptMssqlBackend::new("mssql").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = $1");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert!(
            !result.query_fn.as_deref().unwrap().is_empty(),
            "without structs_only the query function must still be generated"
        );
        assert!(backend.file_header().contains("import sql from \"mssql\";"));
    }

    #[test]
    fn test_structs_only_rejects_invalid_value() {
        let mut backend = TypescriptMssqlBackend::new("mssql").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    #[test]
    fn test_structs_only_combined_with_zod_row_type_keeps_zod_import_and_schema() {
        let mut backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
        assert!(!header.contains("mssql"), "got:\n{header}");
    }

    fn make_batch_query(name: &str, sql: &str, params: Vec<AnalyzedParam>) -> AnalyzedQuery {
        AnalyzedQuery {
            name: name.to_string(),
            command: QueryCommand::Batch,
            sql: sql.to_string(),
            columns: vec![],
            params,
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
            optional_params: vec![],
            group_by: None,
            custom: vec![],
        }
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
        let backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
    /// Note: mssql's `json` scalar maps to plain `string` (no embedded
    /// comma), so this mainly exercises the general wrapped-signature path
    /// with the same shape as the other backends; the comma-preservation
    /// property itself is exercised where `json` maps to
    /// `Record<string, unknown>` (pg, mysql2, postgres, duckdb, snowflake,
    /// node-sqlite, wasm-sqlite, better-sqlite3).
    #[test]
    fn test_batch_signature_wrap_preserves_json_param_type_intact() {
        let backend = TypescriptMssqlBackend::new("mssql").unwrap();
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
            query_fn.contains("\titems: string[],"),
            "the json param's type must survive intact on the items line; got:\n{query_fn}"
        );
    }
}
