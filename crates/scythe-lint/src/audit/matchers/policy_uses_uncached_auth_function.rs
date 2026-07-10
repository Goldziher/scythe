//! Matcher `"policy_uses_uncached_auth_function"` — SC-RLS03.
//!
//! Takes no `matcher_args`. Fires on `CREATE POLICY` statements whose USING or
//! WITH CHECK expression calls one of the per-request auth helpers
//! (`auth.uid()`, `auth.jwt()`, `auth.role()`, `auth.email()`, or
//! `current_setting(…)`) directly, without wrapping it in a scalar subquery
//! `(select …)`. Postgres re-evaluates the function for every candidate row,
//! costing per-row JIT and planner overhead; wrapping the call in
//! `(select …)` lets the optimiser cache the result as an InitPlan and
//! evaluate it once per query.
//!
//! Detection inspired by supabase/splinter lint `0003_auth_rls_initplan`
//! (see `ATTRIBUTIONS.md`). Splinter substring-matches the policy expression
//! text; scythe walks the typed `Expr` AST, stopping at any `Expr::Subquery`
//! boundary (the wrapping is the safe form).

use sqlparser::ast::{Expr, FunctionArg, FunctionArgExpr, FunctionArguments, ObjectName, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

const PROBLEMATIC_AUTH_FUNCTIONS: &[(&str, &str)] =
    &[("auth", "uid"), ("auth", "jwt"), ("auth", "role"), ("auth", "email")];
const PROBLEMATIC_SINGLE_FUNCTIONS: &[&str] = &["current_setting"];

fn matches_problematic_call(name: &ObjectName) -> bool {
    let parts: Vec<String> = name
        .0
        .iter()
        .filter_map(|seg| seg.as_ident().map(|i| i.value.to_ascii_lowercase()))
        .collect();
    match parts.as_slice() {
        [single] => PROBLEMATIC_SINGLE_FUNCTIONS.contains(&single.as_str()),
        [schema, func] => PROBLEMATIC_AUTH_FUNCTIONS.iter().any(|(s, f)| s == schema && f == func),
        _ => false,
    }
}

fn walk_for_bare_auth_call(expr: &Expr) -> bool {
    match expr {
        Expr::Function(f) => {
            if matches_problematic_call(&f.name) {
                return true;
            }
            if let FunctionArguments::List(list) = &f.args {
                for arg in &list.args {
                    let inner = match arg {
                        FunctionArg::Named { arg, .. }
                        | FunctionArg::ExprNamed { arg, .. }
                        | FunctionArg::Unnamed(arg) => arg,
                    };
                    if let FunctionArgExpr::Expr(e) = inner
                        && walk_for_bare_auth_call(e)
                    {
                        return true;
                    }
                }
            }
            false
        }
        Expr::Nested(inner) => walk_for_bare_auth_call(inner),
        Expr::BinaryOp { left, right, .. } => walk_for_bare_auth_call(left) || walk_for_bare_auth_call(right),
        Expr::UnaryOp { expr, .. } => walk_for_bare_auth_call(expr),
        Expr::Cast { expr, .. } => walk_for_bare_auth_call(expr),
        Expr::IsNull(e)
        | Expr::IsNotNull(e)
        | Expr::IsTrue(e)
        | Expr::IsNotTrue(e)
        | Expr::IsFalse(e)
        | Expr::IsNotFalse(e)
        | Expr::IsUnknown(e)
        | Expr::IsNotUnknown(e) => walk_for_bare_auth_call(e),
        Expr::Between { expr, low, high, .. } => {
            walk_for_bare_auth_call(expr) || walk_for_bare_auth_call(low) || walk_for_bare_auth_call(high)
        }
        Expr::InList { expr, list, .. } => walk_for_bare_auth_call(expr) || list.iter().any(walk_for_bare_auth_call),
        Expr::Subquery(_) => false,
        _ => false,
    }
}

pub fn match_policy_uses_uncached_auth_function(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let Statement::CreatePolicy(policy) = ctx.stmt else {
        return Vec::new();
    };
    let using_hit = policy.using.as_ref().is_some_and(walk_for_bare_auth_call);
    let with_check_hit = policy.with_check.as_ref().is_some_and(walk_for_bare_auth_call);
    if !using_hit && !with_check_hit {
        return Vec::new();
    }
    let mut hit = MatcherHit::empty();
    hit.bindings.insert("policy".to_string(), policy.name.to_string());
    hit.bindings.insert("table".to_string(), policy.table_name.to_string());
    let clause = if using_hit { "USING" } else { "WITH CHECK" };
    hit.bindings.insert("clause".to_string(), clause.to_string());
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
    fn fires_on_bare_auth_uid() {
        let sql = "CREATE POLICY tenant ON tenants USING (tenant_id = auth.uid());";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_uses_uncached_auth_function(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("policy").map(|s| s.as_str()), Some("tenant"));
    }

    #[test]
    fn no_match_wrapped_auth_uid() {
        let sql = "CREATE POLICY tenant ON tenants USING (tenant_id = (select auth.uid()));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_uses_uncached_auth_function(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_current_setting() {
        let sql = "CREATE POLICY tenant ON tenants USING (tenant_id = current_setting('app.tenant'));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_uses_uncached_auth_function(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn fires_on_with_check_bare_auth() {
        let sql = "CREATE POLICY tenant ON tenants FOR INSERT WITH CHECK (tenant_id = auth.uid());";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_uses_uncached_auth_function(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("clause").map(|s| s.as_str()), Some("WITH CHECK"));
    }

    #[test]
    fn no_match_no_auth_call() {
        let sql = "CREATE POLICY tenant ON tenants USING (tenant_id = 42);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_uses_uncached_auth_function(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_uses_uncached_auth_function(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
