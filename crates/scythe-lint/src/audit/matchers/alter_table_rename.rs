//! Matcher `"alter_table_rename_column"` — SC-MIG04 renaming-column.
//!
//! Takes no `matcher_args`. Walks `ALTER TABLE … RENAME COLUMN <old> TO <new>`
//! and fires once per rename. Bindings: `table`, `old_column`, `new_column`.
//!
//! Migration-safety motivation: a column rename breaks any application
//! version still reading from the old name. Stage it as add-new → backfill
//! → drop-old instead.

use sqlparser::ast::{AlterTableOperation, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_alter_table_rename_column(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let Statement::AlterTable(alter) = ctx.stmt else {
        return Vec::new();
    };
    let table = alter.name.to_string();
    alter
        .operations
        .iter()
        .filter_map(|op| match op {
            AlterTableOperation::RenameColumn {
                old_column_name,
                new_column_name,
            } => {
                let mut hit = MatcherHit::empty();
                hit.bindings.insert("table".to_string(), table.clone());
                hit.bindings
                    .insert("old_column".to_string(), old_column_name.value.clone());
                hit.bindings
                    .insert("new_column".to_string(), new_column_name.value.clone());
                Some(hit)
            }
            _ => None,
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
    fn fires_on_rename_column() {
        let sql = "ALTER TABLE users RENAME COLUMN nick TO username;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_table_rename_column(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("table").map(|s| s.as_str()), Some("users"));
        assert_eq!(hits[0].bindings.get("old_column").map(|s| s.as_str()), Some("nick"));
        assert_eq!(hits[0].bindings.get("new_column").map(|s| s.as_str()), Some("username"));
    }

    #[test]
    fn no_match_add_column() {
        let sql = "ALTER TABLE users ADD COLUMN nickname text;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_table_rename_column(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_table_rename_column(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
