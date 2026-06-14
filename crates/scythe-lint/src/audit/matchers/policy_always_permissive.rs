//! Matcher `"policy_always_permissive"` — SC-RLS02.
//!
//! Takes no `matcher_args`. Fires on `CREATE POLICY` statements whose USING or
//! WITH CHECK expression is a tautology (`true`, `1=1`, parenthesised variants,
//! or `NULL`), making the policy unconditionally permissive. Restricted to
//! permissive policies on write-side commands — SELECT policies with
//! `USING (true)` are a common (and intentional) "everyone can read" pattern,
//! so they are excluded.
//!
//! Detection inspired by supabase/splinter lint `0024_rls_policy_always_true`
//! (see `ATTRIBUTIONS.md`). Splinter normalises the rendered policy
//! expression and compares against the literal tautology set; scythe inspects
//! the typed `Expr` AST so it works regardless of whitespace and casing.

use sqlparser::ast::{CreatePolicyCommand, CreatePolicyType, Expr, Statement, Value};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

fn unwrap_nested(expr: &Expr) -> &Expr {
    match expr {
        Expr::Nested(inner) => unwrap_nested(inner),
        other => other,
    }
}

fn is_tautology(expr: &Expr) -> bool {
    match unwrap_nested(expr) {
        Expr::Value(v) => matches!(v.value, Value::Boolean(true) | Value::Null),
        Expr::BinaryOp {
            left,
            op: sqlparser::ast::BinaryOperator::Eq,
            right,
        } => match (unwrap_nested(left), unwrap_nested(right)) {
            (Expr::Value(l), Expr::Value(r)) => match (&l.value, &r.value) {
                (Value::Number(a, _), Value::Number(b, _)) => a == b,
                (Value::Boolean(a), Value::Boolean(b)) => a == b,
                (Value::SingleQuotedString(a), Value::SingleQuotedString(b)) => a == b,
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

fn is_write_side_command(cmd: &Option<CreatePolicyCommand>) -> bool {
    match cmd {
        // `None` defaults to ALL in Postgres — applies to writes too.
        None => true,
        Some(CreatePolicyCommand::Select) => false,
        Some(_) => true,
    }
}

pub fn match_policy_always_permissive(
    ctx: &LintContext<'_>,
    _args: &toml::Table,
) -> Vec<MatcherHit> {
    let Statement::CreatePolicy(policy) = ctx.stmt else {
        return Vec::new();
    };
    // `None` `policy_type` defaults to PERMISSIVE in Postgres.
    if matches!(policy.policy_type, Some(CreatePolicyType::Restrictive)) {
        return Vec::new();
    }
    if !is_write_side_command(&policy.command) {
        return Vec::new();
    }
    let using_tautology = policy.using.as_ref().is_some_and(is_tautology);
    let with_check_tautology = policy.with_check.as_ref().is_some_and(is_tautology);
    if !using_tautology && !with_check_tautology {
        return Vec::new();
    }
    let mut hit = MatcherHit::empty();
    hit.bindings
        .insert("policy".to_string(), policy.name.to_string());
    hit.bindings
        .insert("table".to_string(), policy.table_name.to_string());
    let clause = if using_tautology {
        "USING"
    } else {
        "WITH CHECK"
    };
    hit.bindings
        .insert("clause".to_string(), clause.to_string());
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
    fn fires_on_using_true_no_command() {
        let sql = "CREATE POLICY allow_all ON tenants USING (true);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_always_permissive(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("policy").map(|s| s.as_str()),
            Some("allow_all")
        );
    }

    #[test]
    fn fires_on_using_1_eq_1_for_update() {
        let sql = "CREATE POLICY allow_all ON tenants FOR UPDATE USING (1 = 1);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_always_permissive(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn fires_on_with_check_true_for_insert() {
        let sql = "CREATE POLICY allow_all ON tenants FOR INSERT WITH CHECK (true);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_always_permissive(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("clause").map(|s| s.as_str()),
            Some("WITH CHECK")
        );
    }

    #[test]
    fn no_match_select_only_policy_with_using_true() {
        // SELECT + USING (true) is the everyone-can-read pattern; intentional.
        let sql = "CREATE POLICY readable ON tenants FOR SELECT USING (true);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_always_permissive(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_restrictive_policy() {
        // Restrictive policies AND with permissive ones; an always-true
        // restrictive policy contributes nothing but is not a security hole.
        let sql = "CREATE POLICY always_pass ON tenants AS RESTRICTIVE FOR UPDATE USING (true);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_always_permissive(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_real_predicate() {
        let sql = "CREATE POLICY tenant_isolation ON tenants FOR UPDATE USING (tenant_id = (select auth.uid()));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_always_permissive(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_nested_true() {
        let sql = "CREATE POLICY allow_all ON tenants USING (((true)));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_always_permissive(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_policy_always_permissive(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
