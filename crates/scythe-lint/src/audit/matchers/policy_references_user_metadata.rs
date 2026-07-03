//! Matcher `"policy_references_user_metadata"` — SC-RLS01.
//!
//! Takes no `matcher_args`. Fires on `CREATE POLICY` statements whose USING or
//! WITH CHECK expression references `user_metadata` — Supabase's
//! end-user-editable JWT claim bag. Trusting `user_metadata` for
//! authorization lets any signed-in user rewrite their own permissions; the
//! safe path uses `app_metadata` (set only by server-side trusted code).
//!
//! Detection inspired by supabase/splinter lint `0015_rls_references_user_metadata`
//! (see `ATTRIBUTIONS.md`). Splinter substring-matches the rendered policy
//! expression text; scythe renders the typed `Expr` AST to its display form
//! and substring-matches against `user_metadata`. This catches both
//! `auth.jwt() -> 'user_metadata'` and
//! `current_setting('request.jwt.claims') -> 'user_metadata'` shapes.

use sqlparser::ast::{Expr, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

const USER_METADATA: &str = "user_metadata";

fn expr_mentions_user_metadata(expr: &Expr) -> bool {
    expr.to_string().to_ascii_lowercase().contains(USER_METADATA)
}

pub fn match_policy_references_user_metadata(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let Statement::CreatePolicy(policy) = ctx.stmt else {
        return Vec::new();
    };
    let mentions = policy.using.as_ref().is_some_and(expr_mentions_user_metadata)
        || policy.with_check.as_ref().is_some_and(expr_mentions_user_metadata);
    if !mentions {
        return Vec::new();
    }
    let mut hit = MatcherHit::empty();
    hit.bindings.insert("policy".to_string(), policy.name.to_string());
    hit.bindings.insert("table".to_string(), policy.table_name.to_string());
    vec![hit]
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
    fn fires_on_jwt_user_metadata_reference() {
        let sql = "CREATE POLICY trust_jwt ON tenants USING (auth.jwt() -> 'user_metadata' ->> 'tenant_id' = tenant_id::text);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_references_user_metadata(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("policy").map(|s| s.as_str()), Some("trust_jwt"));
        assert_eq!(hits[0].bindings.get("table").map(|s| s.as_str()), Some("tenants"));
    }

    #[test]
    fn no_match_app_metadata_reference() {
        let sql = "CREATE POLICY trust_jwt ON tenants USING (auth.jwt() -> 'app_metadata' ->> 'tenant_id' = tenant_id::text);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_references_user_metadata(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_with_check_reference() {
        let sql = "CREATE POLICY trust_jwt ON tenants FOR INSERT WITH CHECK (auth.jwt() -> 'user_metadata' ->> 'tenant_id' = tenant_id::text);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_references_user_metadata(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_references_user_metadata(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
