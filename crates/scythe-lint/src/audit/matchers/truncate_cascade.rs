//! Matcher `"truncate_cascade"` — SC-MIG08 ban-truncate-cascade.
//!
//! Takes no `matcher_args`. Fires on `TRUNCATE … CASCADE`, emitting one
//! hit per truncated table. Bindings: `table`.
//!
//! Migration-safety motivation: `TRUNCATE … CASCADE` silently truncates
//! every table referencing the target via a foreign key. The blast radius
//! is unbounded from the statement alone — a developer reading the
//! migration cannot tell which tables will be cleared.

use sqlparser::ast::{CascadeOption, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_truncate_cascade(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let Statement::Truncate(truncate) = ctx.stmt else {
        return Vec::new();
    };
    if !matches!(truncate.cascade, Some(CascadeOption::Cascade)) {
        return Vec::new();
    }
    truncate
        .table_names
        .iter()
        .map(|target| {
            let mut hit = MatcherHit::empty();
            hit.bindings
                .insert("table".to_string(), target.name.to_string());
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
    fn fires_on_truncate_cascade() {
        let sql = "TRUNCATE TABLE users CASCADE;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_truncate_cascade(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("table").map(|s| s.as_str()),
            Some("users")
        );
    }

    #[test]
    fn no_match_truncate_without_cascade() {
        let sql = "TRUNCATE TABLE users;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_truncate_cascade(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_truncate_restrict() {
        let sql = "TRUNCATE TABLE users RESTRICT;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_truncate_cascade(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_each_table_when_multiple() {
        let sql = "TRUNCATE TABLE users, orders CASCADE;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_truncate_cascade(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_truncate_cascade(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
