//! Matcher `"alter_table_rename_table"` — SC-MIG07 renaming-table.
//!
//! Takes no `matcher_args`. Walks `ALTER TABLE <old> RENAME TO <new>` (and
//! the `RENAME AS <new>` form supported by some dialects) and fires once
//! per rename. Bindings: `old_table`, `new_table`.
//!
//! Migration-safety motivation: renaming a table breaks every application
//! version still selecting from the old name. Stage it as create-new →
//! dual-write/backfill → switch reads → drop-old across releases.

use sqlparser::ast::{AlterTableOperation, RenameTableNameKind, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_alter_table_rename_table(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let Statement::AlterTable(alter) = ctx.stmt else {
        return Vec::new();
    };
    let old_table = alter.name.to_string();
    alter
        .operations
        .iter()
        .filter_map(|op| match op {
            AlterTableOperation::RenameTable { table_name } => {
                let new_table = match table_name {
                    RenameTableNameKind::To(name) | RenameTableNameKind::As(name) => name.to_string(),
                };
                let mut hit = MatcherHit::empty();
                hit.bindings.insert("old_table".to_string(), old_table.clone());
                hit.bindings.insert("new_table".to_string(), new_table);
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
    fn fires_on_rename_to() {
        let sql = "ALTER TABLE users RENAME TO accounts;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_table_rename_table(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("old_table").map(|s| s.as_str()), Some("users"));
        assert_eq!(hits[0].bindings.get("new_table").map(|s| s.as_str()), Some("accounts"));
    }

    #[test]
    fn no_match_rename_column() {
        let sql = "ALTER TABLE users RENAME COLUMN nick TO username;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_table_rename_table(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_table_rename_table(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
