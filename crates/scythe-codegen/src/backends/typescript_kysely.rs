use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_camel_case, to_pascal_case,
};
use scythe_backend::types::resolve_type;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::GroupedQueryFn;
use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};
use crate::backends::typescript_common::{
    TsRowType, generate_grouped_interface_structs, generate_ts_grouped_fold_body, generate_ts_interface_row_struct,
    generate_ts_union_row_struct, generate_zod_enum, generate_zod_grouped_structs, generate_zod_row_struct,
    parse_bool_option,
};
use crate::singularize;

const DEFAULT_MANIFEST_PG: &str = include_str!("../../manifests/typescript-kysely.toml");
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
/// Kysely dialect (PostgreSQL, MySQL, SQLite, MSSQL) and every third-party
/// dialect (wasm-sqlite, node:sqlite, PGlite, ...) without scythe knowing
/// they exist. The `engine` passed to [`TypescriptKyselyBackend::new`] only
/// selects the scalar type manifest (matching what the underlying driver
/// naturally returns), never the SQL placeholder text: scythe's analyzer
/// already normalizes every engine's native placeholder syntax (`$N` for
/// PostgreSQL, bare `?` for MySQL/SQLite, and `@pN` rewritten to `?` for
/// MSSQL — see `convert_mssql_placeholders` in `scythe-core`) down to
/// positional placeholders before any backend sees the SQL, so a single
/// interpolation pass handles all four engines identically.
pub struct TypescriptKyselyBackend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
}

impl TypescriptKyselyBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_PG,
            "mysql" | "mariadb" => DEFAULT_MANIFEST_MYSQL,
            "sqlite" | "sqlite3" => DEFAULT_MANIFEST_SQLITE,
            "mssql" => DEFAULT_MANIFEST_MSSQL,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!(
                        "typescript-kysely only supports PostgreSQL/MySQL/MariaDB/SQLite/MSSQL, got engine '{}'",
                        engine
                    ),
                ));
            }
        };
        let manifest = super::load_or_default_manifest("backends/typescript-kysely/manifest.toml", default_toml)?;
        Ok(Self {
            manifest,
            row_type: TsRowType::default(),
            outer_join_unions: false,
        })
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
fn interpolate_kysely_params(sql: &str, exprs: &[String]) -> String {
    super::rewrite_pg_placeholders(sql, |n| {
        let idx = n.saturating_sub(1) as usize;
        match exprs.get(idx) {
            Some(expr) => format!("${{{}}}", expr),
            None => format!("${{p{n}}}"),
        }
    })
}

impl CodegenBackend for TypescriptKyselyBackend {
    fn name(&self) -> &str {
        "typescript-kysely"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "mysql", "mariadb", "sqlite", "mssql"]
    }

    fn file_header(&self) -> String {
        let mut header =
            "/** Auto-generated by scythe. Do not edit. */\n\nimport { type Kysely, sql } from \"kysely\";\n"
                .to_string();
        if self.row_type == TsRowType::Zod {
            header.push_str("import { z } from \"zod\";\n");
        }
        header
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        if self.row_type == TsRowType::Zod {
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
        _columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");

        let cleaned = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let exprs: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
        let sql_text = interpolate_kysely_params(&cleaned, &exprs);

        let inline_params = if params.is_empty() {
            "db: Kysely<DB>".to_string()
        } else {
            format!("db: Kysely<DB>, {}", param_list)
        };

        // `<DB = any>` keeps the handle generic instead of hardcoding
        // `Kysely<any>`: callers with a schema-typed `Kysely<MyDB>` get it
        // threaded straight through (so the rest of their query-builder
        // calls on `db` stay checked), while callers with no schema still
        // get `any` for free via the default -- without ever spelling `any`
        // at the generated call site.
        let write_fn_sig = |out: &mut String, name: &str, params_inline: &str, ret: &str| {
            let oneliner = format!(
                "export async function {}<DB = any>({}): {} {{",
                name, params_inline, ret
            );
            if oneliner.len() <= 80 {
                let _ = writeln!(out, "{}", oneliner);
            } else {
                let mut parts = vec!["\tdb: Kysely<DB>".to_string()];
                for p in params {
                    parts.push(format!("\t{}: {}", p.field_name, p.full_type));
                }
                let _ = writeln!(out, "export async function {}<DB = any>(", name);
                for part in &parts {
                    let _ = writeln!(out, "{},", part);
                }
                let _ = writeln!(out, "): {} {{", ret);
            }
        };

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(out, "/** Fetch a single {} or null. */", struct_name);
                let ret = format!("Promise<{} | null>", struct_name);
                write_fn_sig(&mut out, &func_name, &inline_params, &ret);
                let _ = writeln!(
                    out,
                    "\tconst result = await sql<{}>`{}`.execute(db);",
                    struct_name, sql_text
                );
                let _ = writeln!(out, "\treturn result.rows[0] ?? null;");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "/** Fetch all {} rows. */", struct_name);
                let ret = format!("Promise<{}[]>", struct_name);
                write_fn_sig(&mut out, &func_name, &inline_params, &ret);
                let _ = writeln!(
                    out,
                    "\tconst result = await sql<{}>`{}`.execute(db);",
                    struct_name, sql_text
                );
                let _ = writeln!(out, "\treturn result.rows;");
                let _ = write!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                if params.len() > 1 {
                    let params_type_name = format!("{}BatchParams", struct_name);
                    let item_exprs: Vec<String> = params.iter().map(|p| format!("item.{}", p.field_name)).collect();
                    let batch_sql = interpolate_kysely_params(&cleaned, &item_exprs);

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
                    let batch_params = format!("db: Kysely<DB>, items: {}[]", params_type_name);
                    write_fn_sig(&mut out, &batch_fn_name, &batch_params, "Promise<void>");
                    let _ = writeln!(out, "\tawait db.transaction().execute(async (trx) => {{");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tawait sql`{}`.execute(trx);", batch_sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t}});");
                    let _ = write!(out, "}}");
                } else if params.len() == 1 {
                    let batch_sql = interpolate_kysely_params(&cleaned, &["item".to_string()]);

                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    let batch_params = format!("db: Kysely<DB>, items: {}[]", params[0].full_type);
                    write_fn_sig(&mut out, &batch_fn_name, &batch_params, "Promise<void>");
                    let _ = writeln!(out, "\tawait db.transaction().execute(async (trx) => {{");
                    let _ = writeln!(out, "\t\tfor (const item of items) {{");
                    let _ = writeln!(out, "\t\t\tawait sql`{}`.execute(trx);", batch_sql);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t}});");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "/** Execute {} for each item in the batch within a transaction. */",
                        analyzed.name
                    );
                    write_fn_sig(
                        &mut out,
                        &batch_fn_name,
                        "db: Kysely<DB>, count: number",
                        "Promise<void>",
                    );
                    let _ = writeln!(out, "\tawait db.transaction().execute(async (trx) => {{");
                    let _ = writeln!(out, "\t\tfor (let i = 0; i < count; i++) {{");
                    let _ = writeln!(out, "\t\t\tawait sql`{}`.execute(trx);", sql_text);
                    let _ = writeln!(out, "\t\t}}");
                    let _ = writeln!(out, "\t}});");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "/** Execute a query returning no rows. */");
                write_fn_sig(&mut out, &func_name, &inline_params, "Promise<void>");
                let _ = writeln!(out, "\tawait sql`{}`.execute(db);", sql_text);
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "/** Execute a query and return the number of affected rows. */");
                write_fn_sig(&mut out, &func_name, &inline_params, "Promise<number>");
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
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let cleaned = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let exprs: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
        let sql_text = interpolate_kysely_params(&cleaned, &exprs);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let inline_params = if params.is_empty() {
            "db: Kysely<DB>".to_string()
        } else {
            format!("db: Kysely<DB>, {}", param_list)
        };
        let ret = format!("Promise<{parent_struct_name}[]>");

        let oneliner = format!("export async function {func_name}<DB = any>({inline_params}): {ret} {{");
        if oneliner.len() <= 80 {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "{oneliner}");
        } else {
            let _ = writeln!(out, "/** Fetch grouped {} rows. */", analyzed.name);
            let _ = writeln!(out, "export async function {func_name}<DB = any>(");
            let _ = writeln!(out, "\tdb: Kysely<DB>,");
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
        let fold = generate_ts_grouped_fold_body(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
            |name, ty| format!("row['{}'] as {}", name, ty),
        );
        out.push_str(&fold);
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
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
        if let Some(value) = options.get("row_type") {
            self.row_type = TsRowType::from_option(value)?;
        }
        if let Some(value) = options.get("outer_join_unions") {
            self.outer_join_unions = parse_bool_option("outer_join_unions", value)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::TypescriptKyselyBackend;
    use crate::backend_trait::CodegenBackend;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, GroupByConfig};
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
        AnalyzedQuery {
            name: "GetUsersWithOrders".to_string(),
            command: QueryCommand::Grouped,
            sql: "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\nFROM users u\nJOIN orders o ON o.user_id = u.id".to_string(),
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

    fn make_one_query(sql: &str, params: Vec<AnalyzedParam>) -> AnalyzedQuery {
        AnalyzedQuery {
            name: "GetUserById".to_string(),
            command: QueryCommand::One,
            sql: sql.to_string(),
            columns: vec![
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
            ],
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

    #[test]
    fn test_engine_selects_manifest_and_rejects_unsupported() {
        assert!(TypescriptKyselyBackend::new("postgresql").is_ok());
        assert!(TypescriptKyselyBackend::new("mysql").is_ok());
        assert!(TypescriptKyselyBackend::new("mariadb").is_ok());
        assert!(TypescriptKyselyBackend::new("sqlite").is_ok());
        assert!(TypescriptKyselyBackend::new("mssql").is_ok());
        assert!(TypescriptKyselyBackend::new("oracle").is_err());
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
            }],
        );
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("db: Kysely<DB>") && query_fn.contains("<DB = any>"),
            "must take a generic Kysely handle defaulting to any, never hardcode Kysely<any>; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("sql<GetUserByIdRow>`SELECT id, name FROM users WHERE id = ${id}`.execute(db)"),
            "must interpolate params through the sql tag, not emit dialect placeholder text; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("$1"),
            "must not leak the postgres placeholder into the sql tag; got:\n{query_fn}"
        );
        assert!(query_fn.contains("result.rows[0] ?? null"), "got:\n{query_fn}");
    }

    /// Bare `?` covers MySQL and SQLite queries natively, and MSSQL too: the
    /// core parser rewrites `@pN` down to bare `?` before any backend sees
    /// the SQL (see `convert_mssql_placeholders` in scythe-core), so the
    /// same interpolation pass must handle all three.
    #[test]
    fn test_query_fn_interpolates_bare_placeholders_for_mysql_sqlite_and_mssql() {
        for engine in ["mysql", "sqlite", "mssql"] {
            let backend = TypescriptKyselyBackend::new(engine).unwrap();
            let query = make_one_query(
                "SELECT id, name FROM users WHERE id = ?",
                vec![AnalyzedParam {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    position: 1,
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
            }],
        );
        query.command = QueryCommand::Batch;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("db.transaction().execute(async (trx) => {"),
            "got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("sql`INSERT INTO users (name) VALUES (${item}"),
            "got:\n{query_fn}"
        );
        assert!(query_fn.contains(".execute(trx)"), "got:\n{query_fn}");
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

    #[test]
    fn test_outer_join_unions_option_applies_to_kysely_row_struct() {
        let mut backend = TypescriptKyselyBackend::new("postgresql").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "outer_join_unions".to_string(),
                "true".to_string(),
            )]))
            .unwrap();
        assert!(backend.outer_join_unions);
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
}
