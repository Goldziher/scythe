//! Matcher `"grant_kind"` — SC-SEC02 grant-all.
//!
//! Reads `matcher_args.kind` (string).  For `kind = "all"`, fires when the
//! statement is a `GRANT ALL` (`Privileges::All`).  Emits an empty `MatcherHit`.
//!
//! Ported from `rules/security/grants.rs` (GrantAll).

use sqlparser::ast::{Privileges, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_grant_kind(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    match kind {
        "all" => match_grant_all(ctx),
        _ => Vec::new(),
    }
}

fn match_grant_all(ctx: &LintContext<'_>) -> Vec<MatcherHit> {
    if let Statement::Grant(g) = ctx.stmt
        && matches!(g.privileges, Privileges::All { .. })
    {
        return vec![MatcherHit::empty()];
    }
    Vec::new()
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

    fn make_args(kind: &str) -> toml::Table {
        let mut t = toml::Table::new();
        t.insert("kind".to_string(), toml::Value::String(kind.to_string()));
        t
    }

    fn make_ctx(
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

    fn ctx_from<'a>(
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
    fn fires_on_grant_all() {
        let sql = "GRANT ALL ON TABLE users TO app_user";
        let (stmt, analyzed, catalog, annotations) = make_ctx(sql);
        let ctx = ctx_from(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_grant_kind(&ctx, &make_args("all"));
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn no_match_for_specific_grant() {
        let sql = "GRANT SELECT ON TABLE users TO app_user";
        let (stmt, analyzed, catalog, annotations) = make_ctx(sql);
        let ctx = ctx_from(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_grant_kind(&ctx, &make_args("all"));
        assert!(hits.is_empty());
    }
}
