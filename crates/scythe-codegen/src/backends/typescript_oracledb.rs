use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, fn_name, row_struct_name, to_camel_case, to_pascal_case};
use scythe_backend::types::resolve_type;
use std::collections::HashMap;
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, GroupedQueryFn, ResolvedColumn, ResolvedParam};
use crate::backends::typescript_common::{
    TsFieldCase, TsRowShape, TsRowType, escape_ts_double_quoted_literal, escape_ts_template_literal,
    generate_grouped_interface_structs, generate_ts_grouped_fold_body, generate_ts_interface_row_struct,
    generate_ts_union_row_struct, parse_bool_option, reject_unknown_options,
};
use crate::singularize;

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/typescript-oracledb.toml");

pub struct TypescriptOracledbBackend {
    manifest: BackendManifest,
    row_type: TsRowType,
    /// Emit outer-join nullability as a discriminated union instead of
    /// independent per-column optionals. Opt-in: the flat shape stays the
    /// default and the cross-target shape.
    outer_join_unions: bool,
    /// When true, only emit type definitions (interfaces, enums,
    /// composites) — no query functions, and no `oracledb` driver import
    /// (which would otherwise be unused).
    structs_only: bool,
}

impl TypescriptOracledbBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "oracle" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("typescript-oracledb only supports Oracle, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self {
            manifest,
            row_type: TsRowType::default(),
            outer_join_unions: false,
            structs_only: false,
        })
    }
}

/// Rewrite $1, $2, ... positional params to :1, :2, ... for Oracle.
impl CodegenBackend for TypescriptOracledbBackend {
    fn name(&self) -> &str {
        "typescript-oracledb"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["oracle"]
    }

    fn apply_options(&mut self, options: &HashMap<String, String>) -> Result<(), ScytheError> {
        reject_unknown_options(
            &["row_type", "outer_join_unions", "structs_only", "field_case"],
            options,
        )?;

        if let Some(rt) = options.get("row_type") {
            self.row_type = TsRowType::from_option(rt)?;
        }
        if let Some(value) = options.get("outer_join_unions") {
            self.outer_join_unions = parse_bool_option("outer_join_unions", value)?;
        }
        if let Some(value) = options.get("structs_only") {
            self.structs_only = parse_bool_option("structs_only", value)?;
        }
        if let Some(value) = options.get("field_case") {
            // Registration only, no `TsFieldCase` field: this backend's
            // `:one`/`:many`/`:grouped` code already reconstructs every row
            // field by field unconditionally (`row["COL"] as ty`, keyed off
            // the driver's own uppercase convention, independent of
            // `field_case`), writing each into `col.field_name` -- so
            // nothing here needs to branch on Snake vs. Camel the way the
            // other backends' blind-cast paths do. What still has to
            // happen is validating and writing this, because it is the
            // only thing that makes the central rename in `resolve.rs`
            // (which reads this string) produce camelCase field names at
            // all -- accepting the option via `reject_unknown_options`
            // above and never writing it here would make it a certified
            // no-op.
            TsFieldCase::from_option(value)?;
            self.manifest.naming.field_case = value.clone();
        }
        Ok(())
    }

    fn file_header(&self) -> String {
        if self.structs_only {
            return String::new();
        }
        "import oracledb from 'oracledb';\n".to_string()
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
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
        // `sql` here goes into a double-quoted JS string, not a template
        // literal, so it needs double-quote escaping.
        let sql = escape_ts_double_quoted_literal(&super::rewrite_pg_placeholders(
            &super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!(":{n}"),
        ));

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let bind_array = if params.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                params
                    .iter()
                    .map(|p| p.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let has_returning = sql.to_uppercase().contains("RETURNING");

        // Every read path below reconstructs the row field by field rather
        // than blind-casting the driver's row, so each cast has to agree
        // with what `generate_row_struct` declared for that column -- and
        // `outer_join_unions` changes that for a join discriminant. See
        // [`TsRowShape::cast_type`].
        let row_shape = TsRowShape::from_outer_join_unions(self.outer_join_unions);

        let mut out = String::new();

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(
                    out,
                    "export async function {}(conn: oracledb.Connection{}{}): Promise<{} | null> {{",
                    func_name, sep, param_list, struct_name
                );

                if has_returning {
                    let out_bind_entries: Vec<String> = columns
                        .iter()
                        .map(|col| {
                            let nt = col.neutral_type.as_str();
                            let oratype = match nt {
                                "int32" | "int64" | "float32" | "float64" | "decimal" => "oracledb.NUMBER",
                                "date" | "datetime" | "datetime_tz" | "time" | "time_tz" => "oracledb.DATE",
                                _ => "oracledb.STRING",
                            };
                            format!("{{ dir: oracledb.BIND_OUT, type: {} }}", oratype)
                        })
                        .collect();
                    let into_clause = columns
                        .iter()
                        .enumerate()
                        .map(|(i, _)| format!(":{}", params.len() + i + 1))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let full_sql = format!("{} INTO {}", sql, into_clause);
                    let all_binds = if params.is_empty() {
                        format!("[{}]", out_bind_entries.join(", "))
                    } else {
                        let input_names: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
                        format!("[{}, {}]", input_names.join(", "), out_bind_entries.join(", "))
                    };
                    let _ = writeln!(
                        out,
                        "\tconst result = await conn.execute(\"{}\", {});",
                        full_sql, all_binds
                    );
                    let _ = writeln!(out, "\tif (!result.outBinds) {{");
                    let _ = writeln!(out, "\t\treturn null;");
                    let _ = writeln!(out, "\t}}");
                    let _ = writeln!(out, "\tconst outBinds = result.outBinds as unknown[][];");
                    let _ = writeln!(out, "\treturn {{");
                    for (i, col) in columns.iter().enumerate() {
                        let _ = writeln!(
                            out,
                            "\t\t{}: (outBinds[{}] ?? [])[0] as {},",
                            col.field_name,
                            i,
                            row_shape.cast_type(col)
                        );
                    }
                    let _ = writeln!(out, "\t}};");
                    let _ = write!(out, "}}");
                } else {
                    let _ = writeln!(
                        out,
                        "\tconst result = await conn.execute(\"{}\", {}, {{ outFormat: oracledb.OUT_FORMAT_OBJECT }});",
                        sql, bind_array
                    );
                    let _ = writeln!(out, "\tif (!result.rows || result.rows.length === 0) {{");
                    let _ = writeln!(out, "\t\treturn null;");
                    let _ = writeln!(out, "\t}}");
                    let _ = writeln!(out, "\tconst row = result.rows[0] as Record<string, unknown>;");
                    let _ = writeln!(out, "\treturn {{");
                    for col in columns {
                        let _ = writeln!(
                            out,
                            "\t\t{}: row[\"{}\"] as {},",
                            col.field_name,
                            col.name.to_uppercase(),
                            row_shape.cast_type(col)
                        );
                    }
                    let _ = writeln!(out, "\t}};");
                    let _ = write!(out, "}}");
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "export async function {}(conn: oracledb.Connection{}{}): Promise<{}[]> {{",
                    func_name, sep, param_list, struct_name
                );
                let _ = writeln!(
                    out,
                    "\tconst result = await conn.execute(\"{}\", {}, {{ outFormat: oracledb.OUT_FORMAT_OBJECT }});",
                    sql, bind_array
                );
                let _ = writeln!(out, "\tif (!result.rows) {{");
                let _ = writeln!(out, "\t\treturn [];");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn result.rows.map((rawRow) => {{");
                let _ = writeln!(out, "\t\tconst row = rawRow as Record<string, unknown>;");
                let _ = writeln!(out, "\t\treturn {{");
                for col in columns {
                    let _ = writeln!(
                        out,
                        "\t\t\t{}: row[\"{}\"] as {},",
                        col.field_name,
                        col.name.to_uppercase(),
                        row_shape.cast_type(col)
                    );
                }
                let _ = writeln!(out, "\t\t}};");
                let _ = writeln!(out, "\t}});");
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "export async function {}(conn: oracledb.Connection{}{}): Promise<void> {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "\tawait conn.execute(\"{}\", {});", sql, bind_array);
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "export async function {}(conn: oracledb.Connection{}{}): Promise<number> {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "\tconst result = await conn.execute(\"{}\", {});", sql, bind_array);
                let _ = writeln!(out, "\treturn result.rowsAffected ?? 0;");
                let _ = write!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                let items_type = if params.len() > 1 {
                    let tuple_types: Vec<String> = params.iter().map(|p| p.full_type.clone()).collect();
                    format!("[{}]", tuple_types.join(", "))
                } else if params.len() == 1 {
                    params[0].full_type.clone()
                } else {
                    "number".to_string()
                };
                let _ = writeln!(
                    out,
                    "export async function {}(conn: oracledb.Connection, items: {}[]): Promise<void> {{",
                    batch_fn_name, items_type
                );
                if params.is_empty() {
                    let _ = writeln!(out, "\tfor (let i = 0; i < items.length; i++) {{");
                    let _ = writeln!(out, "\t\tawait conn.execute(\"{}\");", sql);
                    let _ = writeln!(out, "\t}}");
                } else {
                    let _ = writeln!(
                        out,
                        "\tawait conn.executeMany(\"{}\", items.map(item => Array.isArray(item) ? item : [item]));",
                        sql
                    );
                }
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
        let sql_clean =
            super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        // Unlike `generate_query_fn` above, this splices `sql` into a
        // backtick template literal, so it needs
        // `escape_ts_template_literal` rather than double-quote escaping.
        let sql = escape_ts_template_literal(&super::rewrite_pg_placeholders(&sql_clean, |n| format!(":{n}")));

        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let inline_params = if params.is_empty() {
            "conn: oracledb.Connection".to_string()
        } else {
            format!("conn: oracledb.Connection, {}", param_list)
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
            let _ = writeln!(out, "\tconn: oracledb.Connection,");
            for p in params {
                let _ = writeln!(out, "\t{}: {},", p.field_name, p.full_type);
            }
            let _ = writeln!(out, "): {ret} {{");
        }

        if params.is_empty() {
            let _ = writeln!(out, "\tconst result = await conn.execute(");
            let _ = writeln!(out, "\t\t`{sql}`,");
            let _ = writeln!(out, "\t\t[],");
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            let _ = writeln!(out, "\tconst result = await conn.execute(");
            let _ = writeln!(out, "\t\t`{sql}`,");
            let _ = writeln!(out, "\t\t[{}],", args.join(", "));
        }
        let _ = writeln!(out, "\t\t{{ outFormat: oracledb.OUT_FORMAT_OBJECT }},");
        let _ = writeln!(out, "\t);");
        let _ = writeln!(
            out,
            "\tconst flatRows = (result.rows ?? []) as unknown as Record<string, unknown>[];"
        );

        let fold = generate_ts_grouped_fold_body(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
            key_column,
            false,
            |name, ty| format!("row['{}'] as {ty}", name.to_uppercase()),
        );
        out.push_str(&fold);
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let values: Vec<String> = enum_info.values.iter().map(|v| format!("\"{}\"", v)).collect();
        let _ = writeln!(out, "export type {} = {};", type_name, values.join(" | "));
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
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
}

#[cfg(test)]
mod tests {
    use super::TypescriptOracledbBackend;
    use crate::backend_trait::CodegenBackend;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

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

    /// oracledb splices SQL into a double-quoted JS string, not a template
    /// literal, so a literal `"` in the user's SQL must be escaped or it
    /// terminates the string early. Backtick and `${` are inert in a
    /// double-quoted string and must NOT be escaped.
    #[test]
    fn test_query_fn_escapes_user_double_quote_in_sql() {
        let backend = TypescriptOracledbBackend::new("oracle").unwrap();
        let query = make_one_query(r#"SELECT id FROM users WHERE name = "oops""#);
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r#"WHERE name = \"oops\""#),
            "user double quotes must be escaped; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_query_fn_escapes_user_backslash_in_sql() {
        let backend = TypescriptOracledbBackend::new("oracle").unwrap();
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

    /// The grouped query fn splices SQL into a backtick template literal
    /// (unlike the plain query fn above, which uses a double-quoted
    /// string), so it needs backtick escaping instead of quote escaping.
    #[test]
    fn test_grouped_query_fn_escapes_user_backtick_in_sql() {
        let backend = TypescriptOracledbBackend::new("oracle").unwrap();
        let mut query = make_grouped_query();
        query.sql = "SELECT u.id, u.name, o.id AS order_id, o.total FROM users u JOIN orders o ON o.user_id = u.id \
                     WHERE u.name = `oops`"
            .to_string();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r"WHERE u.name = \`oops\`"),
            "user backtick must be escaped; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_grouped_typescript_oracledb_structs() {
        let backend = TypescriptOracledbBackend::new("oracle").unwrap();
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
    fn test_grouped_typescript_oracledb_query_fn() {
        let backend = TypescriptOracledbBackend::new("oracle").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(query_fn.contains("getUsersWithOrders"), "missing fn; got:\n{query_fn}");
        assert!(
            query_fn.contains("Promise<GetUsersWithOrdersRow[]>"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("conn.execute"),
            "must use conn.execute; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("OUT_FORMAT_OBJECT"),
            "must set outFormat; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("row['ID']"),
            "must use UPPERCASE key for Oracle; got:\n{query_fn}"
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
        let mut backend = TypescriptOracledbBackend::new("oracle").unwrap();
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
            !header.contains("oracledb"),
            "the unused oracledb driver import must be dropped; got:\n{header}"
        );
    }

    #[test]
    fn test_structs_only_unset_leaves_query_fn_present() {
        let backend = TypescriptOracledbBackend::new("oracle").unwrap();
        let query = make_one_query("SELECT id FROM users WHERE id = $1");
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        assert!(
            !result.query_fn.as_deref().unwrap().is_empty(),
            "without structs_only the query function must still be generated"
        );
        assert!(backend.file_header().contains("import oracledb from 'oracledb';"));
    }

    #[test]
    fn test_structs_only_rejects_invalid_value() {
        let mut backend = TypescriptOracledbBackend::new("oracle").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "structs_only".to_string(),
            "maybe".to_string(),
        )]));
        assert!(result.is_err(), "expected 'maybe' to be rejected");
    }

    /// Unlike the other eight TypeScript backends, typescript-oracledb does
    /// not implement Zod row types at all — `generate_row_struct` never
    /// branches on `row_type` (a pre-existing gap, not something this
    /// `structs_only` change introduces or fixes). Setting `row_type =
    /// "zod"` is accepted by `apply_options` (it's a generic parse) but has
    /// no effect: the row struct is always a plain `interface`. This test
    /// documents that combined behavior rather than asserting a `z.object`
    /// schema that this backend cannot produce.
    #[test]
    fn test_structs_only_combined_with_zod_row_type_option_has_no_zod_effect_on_this_backend() {
        let mut backend = TypescriptOracledbBackend::new("oracle").unwrap();
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
            result
                .row_struct
                .as_deref()
                .unwrap()
                .contains("interface GetUserByIdRow"),
            "oracledb has no Zod row type support; the plain interface is still emitted"
        );

        let header = backend.file_header();
        assert!(
            !header.contains("oracledb"),
            "the unused oracledb driver import must still be dropped; got:\n{header}"
        );
    }

    fn make_one_query_with_nullable_column() -> AnalyzedQuery {
        AnalyzedQuery {
            name: "GetUserById".to_string(),
            command: QueryCommand::One,
            sql: "SELECT id, nickname FROM users WHERE id = $1".to_string(),
            columns: vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "nickname".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
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

    /// This must fail before the fix: casting through `col.lang_type`
    /// (`string`, the base type with no nullable wrapper) instead of
    /// `col.full_type` (`string | null`) asserts a nullable column as
    /// non-optional -- laundering a real `null` past `tsc` into a type that
    /// claims it can never happen.
    #[test]
    fn test_one_query_fn_casts_nullable_column_through_full_type() {
        let backend = TypescriptOracledbBackend::new("oracle").unwrap();
        let query = make_one_query_with_nullable_column();
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r#"nickname: row["NICKNAME"] as string | null,"#),
            "nullable column must cast through full_type, not lang_type; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_many_query_fn_casts_nullable_column_through_full_type() {
        let backend = TypescriptOracledbBackend::new("oracle").unwrap();
        let mut query = make_one_query_with_nullable_column();
        query.command = QueryCommand::Many;
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r#"nickname: row["NICKNAME"] as string | null,"#),
            "nullable column must cast through full_type, not lang_type; got:\n{query_fn}"
        );
    }

    /// This must fail before the fix: `field_case` was accepted by
    /// `reject_unknown_options` but never written to
    /// `self.manifest.naming.field_case`, so a real manifest setting
    /// `field_case = "camelCase"` had no effect -- the declared row struct
    /// stayed snake_case and the per-column reconstruction (which already
    /// writes into `col.field_name` unconditionally) never saw a renamed
    /// field either.
    #[test]
    fn test_field_case_option_renames_declared_and_reconstructed_fields() {
        let mut backend = TypescriptOracledbBackend::new("oracle").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "field_case".to_string(),
                "camelCase".to_string(),
            )]))
            .unwrap();
        let query = AnalyzedQuery {
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
        };
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            row_struct.contains("userId: number;"),
            "field_case must rename the declared struct field; got:\n{row_struct}"
        );
        assert!(
            query_fn.contains(r#"userId: row["USER_ID"] as number,"#),
            "field_case must rename the reconstructed field too, still reading Oracle's own \
             uppercase raw key; got:\n{query_fn}"
        );
    }

    #[test]
    fn test_field_case_option_rejects_invalid_value() {
        let mut backend = TypescriptOracledbBackend::new("oracle").unwrap();
        let result = backend.apply_options(&std::collections::HashMap::from([(
            "field_case".to_string(),
            "PascalCase".to_string(),
        )]));
        assert!(result.is_err(), "expected 'PascalCase' to be rejected");
    }

    fn make_one_query_with_outer_join() -> AnalyzedQuery {
        AnalyzedQuery {
            name: "GetUserOrder".to_string(),
            command: QueryCommand::One,
            sql: "SELECT u.id, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id".to_string(),
            columns: vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                // NOT NULL in the schema, so it discriminates the join.
                AnalyzedColumn {
                    name: "total".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    join_group: Some("o".to_string()),
                    nullable_before_join: false,
                    ..Default::default()
                },
                // Independently nullable, so it says nothing about the join.
                AnalyzedColumn {
                    name: "notes".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    join_group: Some("o".to_string()),
                    nullable_before_join: true,
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

    /// This must fail before the fix: this backend reconstructs every row
    /// unconditionally, and its casts moved from `lang_type` to `full_type`,
    /// so with `outer_join_unions = true` it cast a join discriminant to
    /// `string | null` while the union's matched variant declares it
    /// `string` and the unmatched one declares it `null` -- assignable to
    /// neither (TS2322). No `field_case` involved: this combination alone
    /// stopped compiling.
    #[test]
    fn test_outer_join_unions_casts_discriminant_to_the_matched_variant_type() {
        let mut backend = TypescriptOracledbBackend::new("oracle").unwrap();
        backend
            .apply_options(&std::collections::HashMap::from([(
                "outer_join_unions".to_string(),
                "true".to_string(),
            )]))
            .unwrap();
        let result = crate::generate_with_backend(&make_one_query_with_outer_join(), &backend).unwrap();
        let row_struct = result.row_struct.as_deref().unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            row_struct.contains("total: string; notes: string | null"),
            "the matched variant declares the discriminant non-null; got:\n{row_struct}"
        );
        assert!(
            query_fn.contains(r#"total: row["TOTAL"] as string,"#),
            "the discriminant must be cast to the matched variant's type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains(r#"notes: row["NOTES"] as string | null,"#),
            "a column that was already nullable keeps its full_type cast; got:\n{query_fn}"
        );
    }

    /// Without `outer_join_unions` the row is a plain interface declaring
    /// `string | null`, so the cast must stay `full_type` -- the narrowing
    /// above is specific to the union shape.
    #[test]
    fn test_flat_rows_keep_the_full_type_cast_for_join_columns() {
        let backend = TypescriptOracledbBackend::new("oracle").unwrap();
        let result = crate::generate_with_backend(&make_one_query_with_outer_join(), &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains(r#"total: row["TOTAL"] as string | null,"#),
            "flat rows declare the column nullable, so the cast stays nullable; got:\n{query_fn}"
        );
    }
}
