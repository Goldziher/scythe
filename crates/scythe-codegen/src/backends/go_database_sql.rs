use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name, to_pascal_case};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::GeneratedCode;
use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};
use crate::backends::go_common::go_file_header;

pub struct GoDatabaseSqlBackend {
    manifest: BackendManifest,
    engine: String,
}

impl GoDatabaseSqlBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let manifest_toml = match engine {
            "mysql" => include_str!("../../manifests/go-database-sql.mysql.toml"),
            "mariadb" => include_str!("../../manifests/go-database-sql.mariadb.toml"),
            "mssql" => include_str!("../../manifests/go-database-sql.mssql.toml"),
            "sqlite" | "sqlite3" => include_str!("../../manifests/go-database-sql.sqlite.toml"),
            "duckdb" => include_str!("../../manifests/go-database-sql.duckdb.toml"),
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!(
                        "go-database-sql supports MySQL, MSSQL, SQLite, and DuckDB, got engine '{}'",
                        engine
                    ),
                ));
            }
        };
        let manifest = super::parse_manifest(manifest_toml)?;
        Ok(Self {
            manifest,
            engine: engine.to_string(),
        })
    }

    // ~keep Measured against go-duckdb v2.3.3 with `SELECT ?::VARCHAR`: binding a `*T` fails with
    // "could not bind parameter / unsupported data type: unknown type" whether or not the pointer
    // is nil, while an untyped nil and a bare value both bind. The manifest's `nullable = "*{T}"`
    // therefore breaks every nullable parameter on DuckDB, not just the NULL ones, so each
    // pointer-typed argument is dereferenced at the bind site. Board #228.
    fn nullable_bind_helper(&self) -> &'static str {
        if self.engine == "duckdb" {
            GO_DUCKDB_BIND_HELPER
        } else {
            ""
        }
    }

    fn bind_arg(&self, param: &ResolvedParam, expr: String) -> String {
        if self.engine == "duckdb" && param.full_type.starts_with('*') {
            format!("{DUCKDB_BIND_FN}({expr})")
        } else {
            expr
        }
    }
}

const DUCKDB_BIND_FN: &str = "duckdbBindValue";

const GO_DUCKDB_BIND_HELPER: &str = "\n// go-duckdb cannot bind a typed pointer, so a nullable\n// parameter is passed as its value or as an untyped nil.\nfunc duckdbBindValue[T any](v *T) any {\n\tif v == nil {\n\t\treturn nil\n\t}\n\treturn *v\n}";

impl CodegenBackend for GoDatabaseSqlBackend {
    fn name(&self) -> &str {
        "go-database-sql"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["mysql", "mariadb", "mssql", "sqlite", "duckdb"]
    }

    fn file_header(&self) -> String {
        let header = go_file_header(
            "package queries",
            &["context", "database/sql"],
            &[],
            &self.manifest,
            &[],
        );
        format!("{header}{}", self.nullable_bind_helper())
    }

    fn file_header_for_results(&self, generated: &[GeneratedCode]) -> String {
        let header = go_file_header(
            "package queries",
            &["context", "database/sql"],
            &[],
            &self.manifest,
            generated,
        );
        format!("{header}{}", self.nullable_bind_helper())
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        _query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();
        let _ = writeln!(out, "type {} struct {{", struct_name);
        for col in columns {
            let field = to_pascal_case(&col.field_name);
            let json_tag = &col.field_name;
            let _ = writeln!(out, "\t{} {} `json:\"{}\"`", field, col.full_type, json_tag);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut sql =
            super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        if self.engine == "mssql" {
            sql = super::rewrite_pg_placeholders(&sql, |n| format!("@p{n}"));
        }
        let sql = crate::sql_literal::escape_go_interpreted_string(&sql);

        let param_list = params
            .iter()
            .map(|p| {
                let field = to_pascal_case(&p.field_name);
                format!("{} {}", field, p.full_type)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let args = params
            .iter()
            .map(|p| self.bind_arg(p, to_pascal_case(&p.field_name).into_owned()))
            .collect::<Vec<_>>();

        let mut out = String::new();

        match &analyzed.command {
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB{}{}) error {{",
                    func_name, sep, param_list
                );
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                let _ = writeln!(out, "\t_, err := db.ExecContext(ctx, \"{}\"{})", sql, args_str);
                let _ = writeln!(out, "\treturn err");
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB{}{}) (int64, error) {{",
                    func_name, sep, param_list
                );
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                let _ = writeln!(out, "\tresult, err := db.ExecContext(ctx, \"{}\"{})", sql, args_str);
                let _ = writeln!(out, "\tif err != nil {{");
                let _ = writeln!(out, "\t\treturn 0, err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn result.RowsAffected()");
                let _ = write!(out, "}}");
            }
            QueryCommand::One => {
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB{}{}) ({}, error) {{",
                    func_name, sep, param_list, struct_name
                );
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                let _ = writeln!(out, "\trow := db.QueryRowContext(ctx, \"{}\"{})", sql, args_str);
                let _ = writeln!(out, "\tvar r {}", struct_name);
                let scan_fields: Vec<String> = columns
                    .iter()
                    .map(|c| format!("&r.{}", to_pascal_case(&c.field_name)))
                    .collect();
                // ~keep sql.ErrNoRows propagates through err unmodified -- QueryRow's
                // own Scan already gives :one the "error if absent" contract for free.
                let _ = writeln!(out, "\terr := row.Scan({})", scan_fields.join(", "));
                let _ = writeln!(out, "\treturn r, err");
                let _ = write!(out, "}}");
            }
            QueryCommand::Opt => {
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB{}{}) (*{}, error) {{",
                    func_name, sep, param_list, struct_name
                );
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                let _ = writeln!(out, "\trow := db.QueryRowContext(ctx, \"{}\"{})", sql, args_str);
                let _ = writeln!(out, "\tvar r {}", struct_name);
                let scan_fields: Vec<String> = columns
                    .iter()
                    .map(|c| format!("&r.{}", to_pascal_case(&c.field_name)))
                    .collect();
                let _ = writeln!(out, "\tif err := row.Scan({}); err != nil {{", scan_fields.join(", "));
                let _ = writeln!(out, "\t\tif err == sql.ErrNoRows {{");
                let _ = writeln!(out, "\t\t\treturn nil, nil");
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(out, "\t\treturn nil, err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn &r, nil");
                let _ = write!(out, "}}");
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}Batch", func_name);
                if params.len() > 1 {
                    let params_struct_name = format!("{}BatchParams", func_name);
                    let _ = writeln!(out, "type {} struct {{", params_struct_name);
                    for p in params {
                        let field = to_pascal_case(&p.field_name);
                        let _ = writeln!(out, "\t{} {}", field, p.full_type);
                    }
                    let _ = writeln!(out, "}}");
                    let _ = writeln!(out);
                    let _ = writeln!(
                        out,
                        "func {}(ctx context.Context, db *sql.DB, items []{}) error {{",
                        batch_fn_name, params_struct_name
                    );
                } else if params.len() == 1 {
                    let _ = writeln!(
                        out,
                        "func {}(ctx context.Context, db *sql.DB, items []{}) error {{",
                        batch_fn_name, params[0].full_type
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "func {}(ctx context.Context, db *sql.DB, count int) error {{",
                        batch_fn_name
                    );
                }
                let _ = writeln!(out, "\ttx, err := db.BeginTx(ctx, nil)");
                let _ = writeln!(out, "\tif err != nil {{");
                let _ = writeln!(out, "\t\treturn err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tdefer tx.Rollback()");
                if params.is_empty() {
                    let _ = writeln!(out, "\tfor i := 0; i < count; i++ {{");
                    let _ = writeln!(out, "\t\t_, err := tx.ExecContext(ctx, \"{}\")", sql);
                } else {
                    let _ = writeln!(out, "\tfor _, item := range items {{");
                    if params.len() > 1 {
                        let item_args: Vec<String> = params
                            .iter()
                            .map(|p| self.bind_arg(p, format!("item.{}", to_pascal_case(&p.field_name))))
                            .collect();
                        let _ = writeln!(
                            out,
                            "\t\t_, err := tx.ExecContext(ctx, \"{}\", {})",
                            sql,
                            item_args.join(", ")
                        );
                    } else {
                        let item_arg = params
                            .first()
                            .map_or_else(|| "item".to_string(), |p| self.bind_arg(p, "item".to_string()));
                        let _ = writeln!(out, "\t\t_, err := tx.ExecContext(ctx, \"{}\", {})", sql, item_arg);
                    }
                }
                let _ = writeln!(out, "\t\tif err != nil {{");
                let _ = writeln!(out, "\t\t\treturn err");
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn tx.Commit()");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *sql.DB{}{}) ([]{}, error) {{",
                    func_name, sep, param_list, struct_name
                );
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                let _ = writeln!(out, "\trows, err := db.QueryContext(ctx, \"{}\"{})", sql, args_str);
                let _ = writeln!(out, "\tif err != nil {{");
                let _ = writeln!(out, "\t\treturn nil, err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tdefer rows.Close()");
                let _ = writeln!(out, "\tvar result []{}", struct_name);
                let _ = writeln!(out, "\tfor rows.Next() {{");
                let _ = writeln!(out, "\t\tvar r {}", struct_name);
                let scan_fields: Vec<String> = columns
                    .iter()
                    .map(|c| format!("&r.{}", to_pascal_case(&c.field_name)))
                    .collect();
                let _ = writeln!(
                    out,
                    "\t\tif err := rows.Scan({}); err != nil {{",
                    scan_fields.join(", ")
                );
                let _ = writeln!(out, "\t\t\treturn nil, err");
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(out, "\t\tresult = append(result, r)");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn result, rows.Err()");
                let _ = write!(out, "}}");
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped is handled by generate_grouped_query_fn, not generate_query_fn")
            }
        }

        Ok(out)
    }

    fn generate_grouped_structs(
        &self,
        parent_struct_name: &str,
        child_struct_name: &str,
        parent_columns: &[crate::backend_trait::ResolvedColumn],
        child_columns: &[crate::backend_trait::ResolvedColumn],
        _key_column: &str,
    ) -> Result<String, ScytheError> {
        let mut out = String::new();

        let _ = writeln!(out, "type {} struct {{", child_struct_name);
        for col in child_columns {
            let field = to_pascal_case(&col.field_name);
            let _ = writeln!(out, "\t{} {} `json:\"{}\"`", field, col.full_type, col.field_name);
        }
        let _ = writeln!(out, "}}");
        let _ = writeln!(out);

        let _ = writeln!(out, "type {} struct {{", parent_struct_name);
        for col in parent_columns {
            let field = to_pascal_case(&col.field_name);
            let _ = writeln!(out, "\t{} {} `json:\"{}\"`", field, col.full_type, col.field_name);
        }
        let _ = writeln!(out, "\tChildren []{} `json:\"children\"`", child_struct_name);
        let _ = write!(out, "}}");

        Ok(out)
    }

    fn generate_grouped_query_fn(
        &self,
        request: &crate::backend_trait::GroupedQueryFn<'_>,
    ) -> Result<String, ScytheError> {
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let all_columns = request.all_columns;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut sql =
            super::clean_sql_oneline_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        if self.engine == "mssql" {
            sql = super::rewrite_pg_placeholders(&sql, |n| format!("@p{n}"));
        }
        let sql = crate::sql_literal::escape_go_interpreted_string(&sql);

        let param_list = params
            .iter()
            .map(|p| format!("{} {}", to_pascal_case(&p.field_name), p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let args_str = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params
                .iter()
                .map(|p| self.bind_arg(p, to_pascal_case(&p.field_name).into_owned()))
                .collect();
            format!(", {}", args.join(", "))
        };

        let key_go_type = parent_columns
            .iter()
            .find(|c| c.name == key_column || c.field_name == key_column)
            .map(|c| c.full_type.as_str())
            .unwrap_or("int64");

        let mut out = String::new();

        let _ = writeln!(
            out,
            "func {}(ctx context.Context, db *sql.DB{}{}) ([]{}, error) {{",
            func_name, sep, param_list, parent_struct_name
        );
        let _ = writeln!(out, "\trows, err := db.QueryContext(ctx, \"{}\"{})", sql, args_str);
        let _ = writeln!(out, "\tif err != nil {{");
        let _ = writeln!(out, "\t\treturn nil, err");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tdefer rows.Close()");
        let _ = writeln!(out, "\tvar result []{}", parent_struct_name);
        let _ = writeln!(out, "\tindex := make(map[{}]*{})", key_go_type, parent_struct_name);
        let _ = writeln!(out, "\tfor rows.Next() {{");

        for col in all_columns {
            let _ = writeln!(out, "\t\tvar {} {}", col.field_name, col.full_type);
        }

        let scan_refs: Vec<String> = all_columns.iter().map(|c| format!("&{}", c.field_name)).collect();
        let _ = writeln!(out, "\t\tif err := rows.Scan({}); err != nil {{", scan_refs.join(", "));
        let _ = writeln!(out, "\t\t\treturn nil, err");
        let _ = writeln!(out, "\t\t}}");

        let _ = writeln!(out, "\t\tchild := {} {{", child_struct_name);
        for col in child_columns {
            let _ = writeln!(out, "\t\t\t{}: {},", to_pascal_case(&col.field_name), col.field_name);
        }
        let _ = writeln!(out, "\t\t}}");

        let _ = writeln!(out, "\t\tif parent, ok := index[{}]; ok {{", key_column);
        let _ = writeln!(out, "\t\t\tparent.Children = append(parent.Children, child)");
        let _ = writeln!(out, "\t\t}} else {{");
        let _ = writeln!(out, "\t\t\tresult = append(result, {} {{", parent_struct_name);
        for col in parent_columns {
            let _ = writeln!(out, "\t\t\t\t{}: {},", to_pascal_case(&col.field_name), col.field_name);
        }
        let _ = writeln!(out, "\t\t\t\tChildren: []{}{{child}},", child_struct_name);
        let _ = writeln!(out, "\t\t\t}})");
        let _ = writeln!(out, "\t\t\tindex[{}] = &result[len(result)-1]", key_column);
        let _ = writeln!(out, "\t\t}}");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\treturn result, rows.Err()");
        let _ = write!(out, "}}");

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "type {} string", type_name);
        let _ = writeln!(out);
        let _ = writeln!(out, "const (");
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "\t{}{} {} = \"{}\"", type_name, variant, type_name, value);
        }
        let _ = write!(out, ")");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "type {} struct {{", name);
        if !composite.fields.is_empty() {
            for field in &composite.fields {
                let field_name = to_pascal_case(&field.name);
                let go_type = resolve_type(&field.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .unwrap_or_else(|_| "any".to_string());
                let json_tag = &field.name;
                let _ = writeln!(out, "\t{} {} `json:\"{}\"`", field_name, go_type, json_tag);
            }
        }
        let _ = write!(out, "}}");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    use crate::backends::get_backend;
    use crate::generate_with_backend;

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
            aq.sql = "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\n\
                  SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\n\
                  FROM users u\n\
                  JOIN orders o ON o.user_id = u.id"
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
    fn test_grouped_go_database_sql_structs() {
        let backend = get_backend("go-database-sql", "mysql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("type GetUsersWithOrdersChildRow struct"),
            "missing child struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("OrderId"),
            "child struct missing OrderId field; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("type GetUsersWithOrdersRow struct"),
            "missing parent struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("Children []GetUsersWithOrdersChildRow"),
            "parent struct missing Children field; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("type GetUsersWithOrdersRow struct").unwrap();
        assert!(
            child_pos < parent_pos,
            "child struct must be defined before parent struct"
        );

        assert!(result.model_struct.is_none(), "grouped must not produce a model_struct");
    }

    #[test]
    fn test_grouped_go_database_sql_query_fn() {
        let backend = get_backend("go-database-sql", "mysql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &*backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("func GetUsersWithOrders("),
            "missing function; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("[]GetUsersWithOrdersRow, error)"),
            "wrong return type; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("db.QueryContext(ctx,"),
            "must use db.QueryContext; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("index := make(map["),
            "must declare index map; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("parent.Children = append(parent.Children, child)"),
            "must fold child into existing parent; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Children: []GetUsersWithOrdersChildRow{child}"),
            "must initialize Children slice; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("rows.Err()"),
            "must return rows.Err(); got:\n{query_fn}"
        );
    }

    fn make_nullable_param_query() -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = "ListUsersByNickname".to_string();
            aq.command = QueryCommand::Many;
            aq.sql =
                "-- @name ListUsersByNickname\n-- @returns :many\nSELECT id FROM users WHERE nickname = $1 AND id > $2"
                    .to_string();
            aq.columns = vec![AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            }];
            aq.params = vec![
                AnalyzedParam {
                    name: "nickname".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    position: 1,
                    ..Default::default()
                },
                AnalyzedParam {
                    name: "min_id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    position: 2,
                    ..Default::default()
                },
            ];
        })
    }

    /// Board #228. Measured against go-duckdb v2.3.3: binding a `*T` fails with
    /// "could not bind parameter / unsupported data type: unknown type" whether or not the
    /// pointer is nil, so every nullable parameter -- not only the NULL ones -- must be
    /// dereferenced before it reaches the driver.
    #[test]
    fn should_deref_nullable_params_at_the_bind_site_on_duckdb() {
        let backend = get_backend("go-database-sql", "duckdb").unwrap();
        let code = generate_with_backend(&make_nullable_param_query(), &*backend).unwrap();
        let query_fn = code.query_fn.clone().expect("a query fn must be emitted");

        assert!(
            query_fn.contains("Nickname *string"),
            "the public signature must keep the pointer; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("duckdbBindValue(Nickname)"),
            "nullable param must be dereferenced at the bind site; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("duckdbBindValue(MinId)"),
            "a non-nullable param binds directly and must not be wrapped; got:\n{query_fn}"
        );

        let header = backend.file_header_for_results(&[code]);
        assert!(
            header.contains("func duckdbBindValue[T any](v *T) any {"),
            "the helper the bind sites call must be emitted; got:\n{header}"
        );
    }

    /// The other database/sql engines bind a `*T` natively, so wrapping there would be a
    /// gratuitous divergence -- and would emit a helper no call site references, which
    /// `go build` rejects only for imports, not for functions, so nothing else would catch it.
    #[test]
    fn should_not_deref_nullable_params_on_engines_that_bind_pointers() {
        for engine in ["mysql", "mariadb", "sqlite", "mssql"] {
            let backend = get_backend("go-database-sql", engine).unwrap();
            let code = generate_with_backend(&make_nullable_param_query(), &*backend).unwrap();
            let query_fn = code.query_fn.clone().expect("a query fn must be emitted");

            assert!(
                !query_fn.contains("duckdbBindValue"),
                "{engine} must bind the pointer directly; got:\n{query_fn}"
            );
            let header = backend.file_header_for_results(&[code]);
            assert!(
                !header.contains("duckdbBindValue"),
                "{engine} must not emit the DuckDB helper; got:\n{header}"
            );
        }
    }

    fn make_simple_query(name: &str, sql: &str, columns: Vec<AnalyzedColumn>) -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = name.to_string();
            aq.command = QueryCommand::Many;
            aq.sql = sql.to_string();
            aq.columns = columns;
            aq.params = vec![];
        })
    }

    /// Regression test for #100: a schema with only int/string columns must
    /// not emit a `"time"` import, or `go build` fails with "imported and
    /// not used".
    #[test]
    fn test_go_database_sql_file_header_omits_time_for_int_and_string_only_schema() {
        let backend = get_backend("go-database-sql", "mysql").unwrap();
        let query = make_simple_query(
            "ListWidgets",
            "-- @name ListWidgets\n-- @returns :many\nSELECT id, name FROM widgets",
            vec![
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
        );
        let code = generate_with_backend(&query, &*backend).unwrap();
        let header = backend.file_header_for_results(&[code]);

        assert!(
            !header.contains("\"time\""),
            "int/string-only schema must not import \"time\"; got:\n{header}"
        );
        // The always-required imports must still be present, or the file
        // fails to compile for a different reason.
        assert!(header.contains("\"context\""), "got:\n{header}");
        assert!(header.contains("\"database/sql\""), "got:\n{header}");
    }

    /// Positive counterpart: a schema with a temporal column must still
    /// emit the `"time"` import.
    #[test]
    fn test_go_database_sql_file_header_includes_time_when_columns_need_it() {
        let backend = get_backend("go-database-sql", "mysql").unwrap();
        let query = make_simple_query(
            "ListOrders",
            "-- @name ListOrders\n-- @returns :many\nSELECT id, created_at FROM orders",
            vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "created_at".to_string(),
                    neutral_type: "datetime".to_string(),
                    nullable: false,
                    ..Default::default()
                },
            ],
        );
        let code = generate_with_backend(&query, &*backend).unwrap();
        let header = backend.file_header_for_results(&[code]);

        assert!(header.contains("\"time\""), "got:\n{header}");
    }
}
