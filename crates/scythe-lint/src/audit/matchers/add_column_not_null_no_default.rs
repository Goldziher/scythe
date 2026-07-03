//! Matcher `"add_column_not_null_no_default"` — SC-MIG18
//! adding-not-nullable-field.
//!
//! Takes no `matcher_args`. Walks `ALTER TABLE … ADD COLUMN …` and fires when
//! the new column is declared `NOT NULL` without a `DEFAULT`. Emits one hit per
//! offending column with bindings `table` and `column`.
//!
//! Migration-safety motivation: on PostgreSQL < 11, adding a `NOT NULL` column
//! without a default fails on any non-empty table because every existing row
//! is rewritten with `NULL` and then violates the constraint. PG 11+
//! short-circuits a constant `DEFAULT` into the catalog without a rewrite,
//! making the safe pattern `ADD COLUMN … NOT NULL DEFAULT <expr>;`. Adding a
//! `NOT NULL` column without a default also breaks any deployed application
//! version that inserts into the table without supplying the new column.

use sqlparser::ast::{AlterTableOperation, ColumnOption, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_add_column_not_null_no_default(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let Statement::AlterTable(alter) = ctx.stmt else {
        return Vec::new();
    };
    let table = alter.name.to_string();
    alter
        .operations
        .iter()
        .filter_map(|op| match op {
            AlterTableOperation::AddColumn { column_def, .. } => {
                let has_not_null = column_def
                    .options
                    .iter()
                    .any(|o| matches!(o.option, ColumnOption::NotNull));
                let has_default = column_def
                    .options
                    .iter()
                    .any(|o| matches!(o.option, ColumnOption::Default(_)));
                if has_not_null && !has_default {
                    Some(column_def.name.value.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .map(|column| {
            let mut hit = MatcherHit::empty();
            hit.bindings.insert("table".to_string(), table.clone());
            hit.bindings.insert("column".to_string(), column);
            hit
        })
        .collect()
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

    fn make_parts(sql: &str) -> (sqlparser::ast::Statement, AnalyzedQuery, Catalog, Annotations) {
        let stmt = Parser::parse_sql(&PostgreSqlDialect {}, sql).unwrap().remove(0);
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
            positional_param_docs: vec![],
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
    fn fires_on_not_null_without_default() {
        let sql = "ALTER TABLE users ADD COLUMN email text NOT NULL;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_column_not_null_no_default(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("table").map(|s| s.as_str()), Some("users"));
        assert_eq!(hits[0].bindings.get("column").map(|s| s.as_str()), Some("email"));
    }

    #[test]
    fn no_match_not_null_with_default() {
        let sql = "ALTER TABLE users ADD COLUMN email text NOT NULL DEFAULT '';";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_column_not_null_no_default(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_nullable_column() {
        let sql = "ALTER TABLE users ADD COLUMN email text;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_column_not_null_no_default(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_nullable_with_default() {
        let sql = "ALTER TABLE users ADD COLUMN email text DEFAULT '';";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_column_not_null_no_default(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_once_per_column() {
        let sql = "ALTER TABLE users ADD COLUMN a text NOT NULL, ADD COLUMN b text NOT NULL DEFAULT '';";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_column_not_null_no_default(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("column").map(|s| s.as_str()), Some("a"));
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_column_not_null_no_default(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
