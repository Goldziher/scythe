use std::borrow::Cow;

use sqlparser::ast::*;

use crate::rule::LintRule;
use crate::types::*;

// ---------------------------------------------------------------------------
// SC-T01: PreferExplicitJoin
// ---------------------------------------------------------------------------

pub struct PreferExplicitJoin;

impl LintRule for PreferExplicitJoin {
    fn id(&self) -> &'static str {
        "SC-T01"
    }
    fn name(&self) -> &'static str {
        "prefer-explicit-join"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Style
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "Implicit join (FROM a, b WHERE ...) — prefer explicit JOIN syntax"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let mut violations = Vec::new();
        if let Statement::Query(query) = ctx.stmt {
            walk_set_expr_for_implicit_join(&query.body, &mut violations, self.id());
        }
        violations
    }
}

fn walk_set_expr_for_implicit_join(
    set_expr: &SetExpr,
    violations: &mut Vec<Violation>,
    rule_id: &'static str,
) {
    match set_expr {
        SetExpr::Select(select) => {
            // Multiple tables in FROM with no JOINs = implicit join
            if select.from.len() > 1 && select.from.iter().all(|twj| twj.joins.is_empty()) {
                violations.push(Violation {
                    rule_id: Cow::Borrowed(rule_id),
                    message: "implicit join (comma-separated tables) — prefer explicit JOIN".into(),
                    fix: None,
                });
            }
        }
        SetExpr::Query(query) => {
            walk_set_expr_for_implicit_join(&query.body, violations, rule_id);
        }
        SetExpr::SetOperation { left, right, .. } => {
            walk_set_expr_for_implicit_join(left, violations, rule_id);
            walk_set_expr_for_implicit_join(right, violations, rule_id);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// SC-T02: PreferCoalesceOverCase
// ---------------------------------------------------------------------------

pub struct PreferCoalesceOverCase;

impl LintRule for PreferCoalesceOverCase {
    fn id(&self) -> &'static str {
        "SC-T02"
    }
    fn name(&self) -> &'static str {
        "prefer-coalesce-over-case"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Style
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "CASE WHEN x IS NULL THEN y ELSE x END can be COALESCE(x, y)"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let mut violations = Vec::new();
        walk_exprs(ctx.stmt, &mut |expr| {
            if is_coalesce_pattern(expr) {
                violations.push(Violation {
                    rule_id: Cow::Borrowed("SC-T02"),
                    message: "CASE WHEN x IS NULL THEN y ELSE x END — use COALESCE(x, y)".into(),
                    fix: None,
                });
            }
        });
        violations
    }
}

/// Detect: CASE WHEN <x> IS NULL THEN <y> ELSE <x> END
/// where the else_result matches the tested expression.
fn is_coalesce_pattern(expr: &Expr) -> bool {
    if let Expr::Case {
        operand: None,
        conditions,
        else_result: Some(else_expr),
        ..
    } = expr
        && conditions.len() == 1
    {
        let cond = &conditions[0].condition;
        if let Expr::IsNull(tested) = cond {
            // Check that else_result == tested expression
            let tested_str = format!("{}", tested);
            let else_str = format!("{}", else_expr);
            return tested_str == else_str;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// SC-T03: PreferCountStar
// ---------------------------------------------------------------------------

pub struct PreferCountStar;

impl LintRule for PreferCountStar {
    fn id(&self) -> &'static str {
        "SC-T03"
    }
    fn name(&self) -> &'static str {
        "prefer-count-star"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Style
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "COUNT(1) is equivalent to COUNT(*) — prefer COUNT(*) for clarity"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let mut violations = Vec::new();
        walk_exprs(ctx.stmt, &mut |expr| {
            if let Expr::Function(func) = expr {
                let fname = func.name.to_string().to_lowercase();
                if fname == "count"
                    && let FunctionArguments::List(ref arglist) = func.args
                    && arglist.args.len() == 1
                    && let FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Value(v))) =
                        &arglist.args[0]
                    && matches!(&v.value, Value::Number(n, _) if n == "1")
                {
                    violations.push(Violation {
                        rule_id: Cow::Borrowed("SC-T03"),
                        message: "COUNT(1) — prefer COUNT(*)".into(),
                        fix: Some(LintFix {
                            description: "Replace with COUNT(*)".into(),
                            replacement: "COUNT(*)".into(),
                        }),
                    });
                }
            }
        });
        violations
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn walk_exprs(stmt: &Statement, visitor: &mut dyn FnMut(&Expr)) {
    if let Statement::Query(q) = stmt {
        walk_query_exprs(q, visitor)
    }
}

fn walk_query_exprs(query: &Query, visitor: &mut dyn FnMut(&Expr)) {
    walk_set_expr_exprs(&query.body, visitor);
}

fn walk_set_expr_exprs(set_expr: &SetExpr, visitor: &mut dyn FnMut(&Expr)) {
    match set_expr {
        SetExpr::Select(select) => {
            for item in &select.projection {
                match item {
                    SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                        walk_expr(expr, visitor);
                    }
                    _ => {}
                }
            }
            if let Some(ref sel) = select.selection {
                walk_expr(sel, visitor);
            }
            if let Some(ref having) = select.having {
                walk_expr(having, visitor);
            }
        }
        SetExpr::Query(query) => walk_query_exprs(query, visitor),
        SetExpr::SetOperation { left, right, .. } => {
            walk_set_expr_exprs(left, visitor);
            walk_set_expr_exprs(right, visitor);
        }
        _ => {}
    }
}

fn walk_expr(expr: &Expr, visitor: &mut dyn FnMut(&Expr)) {
    visitor(expr);
    match expr {
        Expr::BinaryOp { left, right, .. } => {
            walk_expr(left, visitor);
            walk_expr(right, visitor);
        }
        Expr::UnaryOp { expr: inner, .. } => {
            walk_expr(inner, visitor);
        }
        Expr::Nested(inner) => {
            walk_expr(inner, visitor);
        }
        Expr::Function(func) => {
            if let FunctionArguments::List(ref arglist) = func.args {
                for arg in &arglist.args {
                    match arg {
                        FunctionArg::Unnamed(FunctionArgExpr::Expr(e))
                        | FunctionArg::Named {
                            arg: FunctionArgExpr::Expr(e),
                            ..
                        }
                        | FunctionArg::ExprNamed {
                            arg: FunctionArgExpr::Expr(e),
                            ..
                        } => {
                            walk_expr(e, visitor);
                        }
                        _ => {}
                    }
                }
            }
        }
        Expr::Case {
            conditions,
            else_result,
            operand,
            ..
        } => {
            if let Some(op) = operand {
                walk_expr(op, visitor);
            }
            for cw in conditions {
                walk_expr(&cw.condition, visitor);
                walk_expr(&cw.result, visitor);
            }
            if let Some(er) = else_result {
                walk_expr(er, visitor);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::LintRule;
    use scythe_core::analyzer;
    use scythe_core::catalog::Catalog;
    use scythe_core::parser::parse_query;

    fn make_catalog() -> Catalog {
        Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, email TEXT);",
            "CREATE TABLE posts (id SERIAL PRIMARY KEY, user_id INTEGER NOT NULL, title TEXT NOT NULL);",
        ])
        .unwrap()
    }

    fn make_ctx<'a>(
        query: &'a scythe_core::parser::Query,
        analyzed: &'a scythe_core::analyzer::AnalyzedQuery,
        catalog: &'a Catalog,
    ) -> LintContext<'a> {
        LintContext {
            sql: &query.sql,
            stmt: &query.stmt,
            analyzed,
            catalog,
            annotations: &query.annotations,
        }
    }

    // SC-T01

    #[test]
    fn implicit_join_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name ListJoined\n-- @returns :many\nSELECT u.id, p.title FROM users u, posts p WHERE u.id = p.user_id;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = PreferExplicitJoin.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn explicit_join_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name ListJoined\n-- @returns :many\nSELECT u.id, p.title FROM users u JOIN posts p ON u.id = p.user_id;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = PreferExplicitJoin.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-T02

    #[test]
    fn case_is_null_pattern_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetEmail\n-- @returns :many\nSELECT CASE WHEN email IS NULL THEN 'none' ELSE email END AS email_val FROM users;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = PreferCoalesceOverCase.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn coalesce_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetEmail\n-- @returns :many\nSELECT COALESCE(email, 'none') AS email_val FROM users;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = PreferCoalesceOverCase.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-T03

    #[test]
    fn count_1_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name CountUsers\n-- @returns :one\nSELECT COUNT(1) AS total FROM users;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = PreferCountStar.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn count_star_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name CountUsers\n-- @returns :one\nSELECT COUNT(*) AS total FROM users;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = PreferCountStar.check_query(&ctx);
        assert!(v.is_empty());
    }
}
