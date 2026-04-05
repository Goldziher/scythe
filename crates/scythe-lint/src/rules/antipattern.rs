use std::borrow::Cow;

use sqlparser::ast::*;

use crate::rule::LintRule;
use crate::types::*;

// ---------------------------------------------------------------------------
// SC-A01: NotEqualNull
// ---------------------------------------------------------------------------

pub struct NotEqualNull;

impl LintRule for NotEqualNull {
    fn id(&self) -> &'static str {
        "SC-A01"
    }
    fn name(&self) -> &'static str {
        "not-equal-null"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Antipattern
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &'static str {
        "Comparing with NULL using = or != always yields NULL; use IS NULL / IS NOT NULL"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let mut violations = Vec::new();
        walk_exprs(ctx.stmt, &mut |expr| {
            if let Expr::BinaryOp { left, op, right } = expr {
                let is_comparison = matches!(op, BinaryOperator::Eq | BinaryOperator::NotEq);
                if is_comparison && (is_null_literal(left) || is_null_literal(right)) {
                    let op_str = match op {
                        BinaryOperator::Eq => "=",
                        BinaryOperator::NotEq => "!=",
                        _ => "?",
                    };
                    violations.push(Violation {
                        rule_id: Cow::Borrowed("SC-A01"),
                        message: format!(
                            "comparison `{} NULL` always yields NULL — use IS NULL or IS NOT NULL",
                            op_str
                        ),
                        fix: None,
                    });
                }
            }
        });
        violations
    }
}

// ---------------------------------------------------------------------------
// SC-A02: ImplicitTypeCoercion
// ---------------------------------------------------------------------------

/// Complementary to the analyzer's type checking. Currently a no-op placeholder
/// since the analyzer already flags type mismatches.
pub struct ImplicitTypeCoercion;

impl LintRule for ImplicitTypeCoercion {
    fn id(&self) -> &'static str {
        "SC-A02"
    }
    fn name(&self) -> &'static str {
        "implicit-type-coercion"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Antipattern
    }
    fn default_severity(&self) -> Severity {
        Severity::Off
    }
    fn description(&self) -> &'static str {
        "Implicit type coercion may cause unexpected behavior"
    }
}

// ---------------------------------------------------------------------------
// SC-A03: OrInJoinCondition
// ---------------------------------------------------------------------------

pub struct OrInJoinCondition;

impl LintRule for OrInJoinCondition {
    fn id(&self) -> &'static str {
        "SC-A03"
    }
    fn name(&self) -> &'static str {
        "or-in-join-condition"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Antipattern
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "OR in JOIN ON condition usually prevents index usage"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let mut violations = Vec::new();
        walk_join_conditions(ctx.stmt, &mut |expr| {
            if has_top_level_or(expr) {
                violations.push(Violation {
                    rule_id: Cow::Borrowed("SC-A03"),
                    message: "OR in JOIN ON condition — consider restructuring".into(),
                    fix: None,
                });
            }
        });
        violations
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_null_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::Value(v) if v.value == Value::Null)
}

fn has_top_level_or(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::BinaryOp {
            op: BinaryOperator::Or,
            ..
        }
    )
}

fn walk_exprs(stmt: &Statement, visitor: &mut dyn FnMut(&Expr)) {
    match stmt {
        Statement::Query(q) => walk_query_exprs(q, visitor),
        Statement::Update(u) => {
            if let Some(ref sel) = u.selection {
                walk_expr(sel, visitor);
            }
        }
        Statement::Delete(d) => {
            if let Some(ref sel) = d.selection {
                walk_expr(sel, visitor);
            }
        }
        _ => {}
    }
}

fn walk_query_exprs(query: &Query, visitor: &mut dyn FnMut(&Expr)) {
    walk_set_expr_exprs(&query.body, visitor);
}

fn walk_set_expr_exprs(set_expr: &SetExpr, visitor: &mut dyn FnMut(&Expr)) {
    match set_expr {
        SetExpr::Select(select) => {
            if let Some(ref sel) = select.selection {
                walk_expr(sel, visitor);
            }
            if let Some(ref having) = select.having {
                walk_expr(having, visitor);
            }
            // Walk join conditions
            for twj in &select.from {
                for join in &twj.joins {
                    if let Some(expr) = join_constraint_expr(&join.join_operator) {
                        walk_expr(expr, visitor);
                    }
                }
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
        Expr::IsNull(inner) | Expr::IsNotNull(inner) => {
            walk_expr(inner, visitor);
        }
        _ => {}
    }
}

fn walk_join_conditions(stmt: &Statement, visitor: &mut dyn FnMut(&Expr)) {
    if let Statement::Query(query) = stmt {
        walk_set_expr_join_conditions(&query.body, visitor);
    }
}

fn walk_set_expr_join_conditions(set_expr: &SetExpr, visitor: &mut dyn FnMut(&Expr)) {
    match set_expr {
        SetExpr::Select(select) => {
            for twj in &select.from {
                for join in &twj.joins {
                    if let Some(expr) = join_constraint_expr(&join.join_operator) {
                        visitor(expr);
                    }
                }
            }
        }
        SetExpr::Query(query) => {
            walk_set_expr_join_conditions(&query.body, visitor);
        }
        SetExpr::SetOperation { left, right, .. } => {
            walk_set_expr_join_conditions(left, visitor);
            walk_set_expr_join_conditions(right, visitor);
        }
        _ => {}
    }
}

fn join_constraint_expr(op: &JoinOperator) -> Option<&Expr> {
    let constraint = match op {
        JoinOperator::Join(c)
        | JoinOperator::Inner(c)
        | JoinOperator::Left(c)
        | JoinOperator::LeftOuter(c)
        | JoinOperator::Right(c)
        | JoinOperator::RightOuter(c)
        | JoinOperator::FullOuter(c) => c,
        _ => return None,
    };
    match constraint {
        JoinConstraint::On(expr) => Some(expr),
        _ => None,
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
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL);",
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

    // SC-A01

    #[test]
    fn equal_null_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetUser\n-- @returns :many\nSELECT id FROM users WHERE name = NULL;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NotEqualNull.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn is_null_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetUser\n-- @returns :many\nSELECT id FROM users WHERE name IS NULL;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NotEqualNull.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-A03

    #[test]
    fn or_in_join_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name ListJoined\n-- @returns :many\nSELECT u.id, p.title FROM users u JOIN posts p ON u.id = p.user_id OR u.name = p.title;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = OrInJoinCondition.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn and_in_join_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name ListJoined\n-- @returns :many\nSELECT u.id, p.title FROM users u JOIN posts p ON u.id = p.user_id;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = OrInJoinCondition.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-A01 additional tests

    #[test]
    fn not_equal_null_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetUser\n-- @returns :many\nSELECT id FROM users WHERE name != NULL;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NotEqualNull.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn not_equal_null_angle_brackets_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetUser\n-- @returns :many\nSELECT id FROM users WHERE name <> NULL;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NotEqualNull.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn null_equals_col_reversed_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetUser\n-- @returns :many\nSELECT id FROM users WHERE NULL = name;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NotEqualNull.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn is_not_null_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetUser\n-- @returns :many\nSELECT id FROM users WHERE name IS NOT NULL;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NotEqualNull.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-A02

    #[test]
    fn implicit_type_coercion_returns_empty() {
        let cat = make_catalog();
        let q =
            parse_query("-- @name GetUser\n-- @returns :many\nSELECT id FROM users WHERE id = 1;")
                .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = ImplicitTypeCoercion.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-A03 additional tests

    #[test]
    fn or_with_multiple_conditions_in_join_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name ListJoined\n-- @returns :many\nSELECT u.id, p.title FROM users u JOIN posts p ON u.id = p.user_id OR u.name = p.title OR u.id > 0;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = OrInJoinCondition.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn no_join_clean() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetUser\n-- @returns :many\nSELECT id, name FROM users WHERE id = 1;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = OrInJoinCondition.check_query(&ctx);
        assert!(v.is_empty());
    }

    #[test]
    fn or_in_subquery_not_outer_join() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetUser\n-- @returns :many\nSELECT id FROM users WHERE id IN (SELECT user_id FROM posts WHERE user_id = 1 OR title = 'test');",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = OrInJoinCondition.check_query(&ctx);
        assert!(v.is_empty());
    }
}
