//! Matcher `"grantee_includes"` — SC-SEC03 grant-to-public.
//!
//! Reads `matcher_args.grantee` (string).  For `grantee = "public"`, fires
//! when any grantee in a `GRANT` statement has `GranteesType::Public`.
//! Emits an empty `MatcherHit`.
//!
//! Ported from `rules/security/grants.rs` (GrantToPublic).

use sqlparser::ast::{GranteesType, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_grantee_includes(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let grantee = args.get("grantee").and_then(|v| v.as_str()).unwrap_or_default();

    match grantee {
        "public" => match_grant_to_public(ctx),
        _ => Vec::new(),
    }
}

fn match_grant_to_public(ctx: &LintContext<'_>) -> Vec<MatcherHit> {
    if let Statement::Grant(g) = ctx.stmt
        && g.grantees
            .iter()
            .any(|gr| matches!(gr.grantee_type, GranteesType::Public))
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

    fn make_args(grantee: &str) -> toml::Table {
        let mut t = toml::Table::new();
        t.insert("grantee".to_string(), toml::Value::String(grantee.to_string()));
        t
    }

    fn make_ctx(sql: &str) -> (sqlparser::ast::Statement, AnalyzedQuery, Catalog, Annotations) {
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
    fn fires_on_grant_to_public() {
        let sql = "GRANT SELECT ON TABLE users TO PUBLIC";
        let (stmt, analyzed, catalog, annotations) = make_ctx(sql);
        let ctx = ctx_from(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_grantee_includes(&ctx, &make_args("public"));
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn no_match_for_specific_role() {
        let sql = "GRANT SELECT ON TABLE users TO app_user";
        let (stmt, analyzed, catalog, annotations) = make_ctx(sql);
        let ctx = ctx_from(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_grantee_includes(&ctx, &make_args("public"));
        assert!(hits.is_empty());
    }
}
