//! Matcher `"drop_statement"` — SC-MIG01 ban-drop-table and SC-MIG02
//! ban-drop-column.
//!
//! Reads `matcher_args.kinds` (array of strings). Recognized values:
//! - `table`  — fires on `DROP TABLE …`. Emits one hit per dropped table.
//!   Bindings: `table`.
//! - `column` — fires on `ALTER TABLE … DROP COLUMN …`. Emits one hit per
//!   dropped column. Bindings: `table`, `column`.
//!
//! Migration-safety motivation: dropping a table or column is irreversible
//! at the storage layer and breaks any concurrently deployed application
//! version still reading from it.

use sqlparser::ast::{AlterTableOperation, ObjectType, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_drop_statement(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let kinds = read_string_list(args, "kinds");
    if kinds.is_empty() {
        return Vec::new();
    }

    match ctx.stmt {
        Statement::Drop {
            object_type: ObjectType::Table,
            names,
            ..
        } if kinds.iter().any(|k| k == "table") => names
            .iter()
            .map(|n| {
                let mut hit = MatcherHit::empty();
                hit.bindings.insert("table".to_string(), n.to_string());
                hit
            })
            .collect(),

        Statement::AlterTable(alter) if kinds.iter().any(|k| k == "column") => {
            let table = alter.name.to_string();
            alter
                .operations
                .iter()
                .filter_map(|op| match op {
                    AlterTableOperation::DropColumn { column_names, .. } => Some(column_names),
                    _ => None,
                })
                .flat_map(|columns| {
                    columns.iter().map({
                        let table = table.clone();
                        move |c| {
                            let mut hit = MatcherHit::empty();
                            hit.bindings.insert("table".to_string(), table.clone());
                            hit.bindings.insert("column".to_string(), c.value.clone());
                            hit
                        }
                    })
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn read_string_list(args: &toml::Table, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_ascii_lowercase()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::analyzer::AnalyzedQuery;
    use scythe_core::catalog::Catalog;
    use scythe_core::dialect::SqlDialect;
    use scythe_core::parser::{Annotations, QueryCommand};
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;

    fn make_args(kinds: &[&str]) -> toml::Table {
        let mut t = toml::Table::new();
        let arr: toml::value::Array = kinds
            .iter()
            .map(|s| toml::Value::String((*s).to_string()))
            .collect();
        t.insert("kinds".to_string(), toml::Value::Array(arr));
        t
    }

    fn make_parts(
        sql: &str,
    ) -> (
        sqlparser::ast::Statement,
        AnalyzedQuery,
        Catalog,
        Annotations,
    ) {
        let stmt = Parser::parse_sql(&PostgreSqlDialect {}, sql)
            .unwrap()
            .remove(0);
        let analyzed = AnalyzedQuery {
            name: "q".to_string(),
            command: QueryCommand::Many,
            sql: sql.to_string(),
            columns: vec![],
            params: vec![],
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
            optional_params: vec![],
            group_by: None,
            custom: vec![],
        };
        let catalog = Catalog::from_ddl(&[]).unwrap();
        let annotations = Annotations {
            name: "q".to_string(),
            command: QueryCommand::Many,
            param_docs: vec![],
            nullable_overrides: vec![],
            nonnull_overrides: vec![],
            json_mappings: vec![],
            deprecated: None,
            optional_params: vec![],
            group_by: None,
            custom: vec![],
        };
        (stmt, analyzed, catalog, annotations)
    }

    fn make_ctx<'a>(
        sql: &'a str,
        stmt: &'a sqlparser::ast::Statement,
        analyzed: &'a AnalyzedQuery,
        catalog: &'a Catalog,
        annotations: &'a Annotations,
    ) -> LintContext<'a> {
        LintContext {
            sql,
            stmt,
            analyzed,
            catalog,
            annotations,
            dialect: SqlDialect::PostgreSQL,
        }
    }

    #[test]
    fn fires_on_drop_table() {
        let sql = "DROP TABLE users;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_drop_statement(&ctx, &make_args(&["table", "column"]));
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("table").map(|s| s.as_str()),
            Some("users")
        );
    }

    #[test]
    fn fires_on_drop_multiple_tables() {
        let sql = "DROP TABLE users, orders;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_drop_statement(&ctx, &make_args(&["table"]));
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn fires_on_alter_drop_column() {
        let sql = "ALTER TABLE users DROP COLUMN legacy_id;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_drop_statement(&ctx, &make_args(&["table", "column"]));
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("table").map(|s| s.as_str()),
            Some("users")
        );
        assert_eq!(
            hits[0].bindings.get("column").map(|s| s.as_str()),
            Some("legacy_id")
        );
    }

    #[test]
    fn no_match_drop_view() {
        let sql = "DROP VIEW user_summary;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_drop_statement(&ctx, &make_args(&["table", "column"]));
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_alter_add_column() {
        let sql = "ALTER TABLE users ADD COLUMN nickname text;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_drop_statement(&ctx, &make_args(&["table", "column"]));
        assert!(hits.is_empty());
    }

    #[test]
    fn kinds_filter_table_only_skips_column_drop() {
        let sql = "ALTER TABLE users DROP COLUMN legacy_id;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_drop_statement(&ctx, &make_args(&["table"]));
        assert!(hits.is_empty());
    }
}
