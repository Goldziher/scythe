//! Matcher `"alter_column_drop_not_null"` — SC-MIG17 ban-drop-not-null.
//!
//! Takes no `matcher_args`. Walks `ALTER TABLE … ALTER COLUMN … DROP NOT NULL`
//! and fires once per offending operation. Emits a hit with bindings `table`
//! and `column`.
//!
//! Migration-safety motivation: relaxing a `NOT NULL` contract breaks any
//! deployed application version that still treats the column as non-null —
//! including ORM mappings that expect a non-`Option` type. The safe pattern is
//! to deploy code that tolerates `NULL` first, then drop the constraint in a
//! later release.

use sqlparser::ast::{AlterColumnOperation, AlterTableOperation, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_alter_column_drop_not_null(
    ctx: &LintContext<'_>,
    _args: &toml::Table,
) -> Vec<MatcherHit> {
    let Statement::AlterTable(alter) = ctx.stmt else {
        return Vec::new();
    };
    let table = alter.name.to_string();
    alter
        .operations
        .iter()
        .filter_map(|op| match op {
            AlterTableOperation::AlterColumn {
                column_name,
                op: AlterColumnOperation::DropNotNull,
            } => Some(column_name.value.clone()),
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
    fn fires_on_drop_not_null() {
        let sql = "ALTER TABLE users ALTER COLUMN email DROP NOT NULL;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_drop_not_null(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("table").map(|s| s.as_str()),
            Some("users")
        );
        assert_eq!(
            hits[0].bindings.get("column").map(|s| s.as_str()),
            Some("email")
        );
    }

    #[test]
    fn no_match_set_not_null() {
        let sql = "ALTER TABLE users ALTER COLUMN email SET NOT NULL;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_drop_not_null(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_drop_default() {
        let sql = "ALTER TABLE users ALTER COLUMN email DROP DEFAULT;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_drop_not_null(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_drop_not_null(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
