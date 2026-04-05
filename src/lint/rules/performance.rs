use std::borrow::Cow;

use sqlparser::ast::*;

use crate::lint::rule::LintRule;
use crate::lint::types::*;

// ---------------------------------------------------------------------------
// SC-P01: OrderWithoutLimit
// ---------------------------------------------------------------------------

pub struct OrderWithoutLimit;

impl LintRule for OrderWithoutLimit {
    fn id(&self) -> &'static str {
        "SC-P01"
    }
    fn name(&self) -> &'static str {
        "order-without-limit"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Performance
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "ORDER BY without LIMIT may cause unnecessary sorting of large result sets"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        if let Statement::Query(query) = ctx.stmt
            && query.order_by.is_some()
            && query.limit_clause.is_none()
            && query.fetch.is_none()
        {
            return vec![Violation {
                rule_id: Cow::Borrowed(self.id()),
                message: "ORDER BY without LIMIT — consider adding LIMIT".into(),
                fix: None,
            }];
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// SC-P02: LikeStartsWithWildcard
// ---------------------------------------------------------------------------

pub struct LikeStartsWithWildcard;

impl LintRule for LikeStartsWithWildcard {
    fn id(&self) -> &'static str {
        "SC-P02"
    }
    fn name(&self) -> &'static str {
        "like-starts-with-wildcard"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Performance
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "LIKE pattern starting with % prevents index usage"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let mut violations = Vec::new();
        walk_exprs(ctx.stmt, &mut |expr| match expr {
            Expr::Like { pattern, .. } | Expr::ILike { pattern, .. } => {
                if let Some(s) = extract_string_value(pattern)
                    && s.starts_with('%')
                {
                    violations.push(Violation {
                        rule_id: Cow::Borrowed("SC-P02"),
                        message: format!(
                            "LIKE pattern \"{}\" starts with % — index cannot be used",
                            s
                        ),
                        fix: None,
                    });
                }
            }
            _ => {}
        });
        violations
    }
}

// ---------------------------------------------------------------------------
// SC-P03: NotInSubquery
// ---------------------------------------------------------------------------

pub struct NotInSubquery;

impl LintRule for NotInSubquery {
    fn id(&self) -> &'static str {
        "SC-P03"
    }
    fn name(&self) -> &'static str {
        "not-in-subquery"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Performance
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "NOT IN (SELECT ...) has unexpected NULL behavior; prefer NOT EXISTS"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let mut violations = Vec::new();
        walk_exprs(ctx.stmt, &mut |expr| {
            if let Expr::InSubquery { negated: true, .. } = expr {
                violations.push(Violation {
                    rule_id: Cow::Borrowed("SC-P03"),
                    message: "NOT IN (SELECT ...) — consider NOT EXISTS for NULL safety".into(),
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

fn extract_string_value(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Value(v) => match &v.value {
            Value::SingleQuotedString(s)
            | Value::EscapedStringLiteral(s)
            | Value::DollarQuotedString(sqlparser::ast::DollarQuotedString { value: s, .. }) => {
                Some(s.as_str())
            }
            _ => None,
        },
        _ => None,
    }
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
        Expr::Like {
            expr: inner,
            pattern,
            ..
        }
        | Expr::ILike {
            expr: inner,
            pattern,
            ..
        } => {
            walk_expr(inner, visitor);
            walk_expr(pattern, visitor);
        }
        Expr::InSubquery { expr: inner, .. } => {
            walk_expr(inner, visitor);
        }
        Expr::InList {
            expr: inner, list, ..
        } => {
            walk_expr(inner, visitor);
            for e in list {
                walk_expr(e, visitor);
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
    use crate::analyzer;
    use crate::catalog::Catalog;
    use crate::lint::rule::LintRule;
    use crate::parser::parse_query;

    fn make_catalog() -> Catalog {
        Catalog::from_ddl(&[
            "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, email TEXT);",
        ])
        .unwrap()
    }

    fn make_ctx<'a>(
        query: &'a crate::parser::Query,
        analyzed: &'a crate::analyzer::AnalyzedQuery,
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

    // SC-P01

    #[test]
    fn order_without_limit_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name ListUsers\n-- @returns :many\nSELECT id, name FROM users ORDER BY name;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = OrderWithoutLimit.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn order_with_limit_ok() {
        let cat = make_catalog();
        let q = parse_query("-- @name ListUsers\n-- @returns :many\nSELECT id, name FROM users ORDER BY name LIMIT 10;").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = OrderWithoutLimit.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-P02

    #[test]
    fn like_leading_wildcard_fires() {
        let cat = make_catalog();
        let q = parse_query("-- @name SearchUsers\n-- @returns :many\nSELECT id, name FROM users WHERE name LIKE '%foo';").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = LikeStartsWithWildcard.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn like_trailing_wildcard_ok() {
        let cat = make_catalog();
        let q = parse_query("-- @name SearchUsers\n-- @returns :many\nSELECT id, name FROM users WHERE name LIKE 'foo%';").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = LikeStartsWithWildcard.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-P03

    #[test]
    fn not_in_subquery_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetActive\n-- @returns :many\nSELECT id FROM users WHERE id NOT IN (SELECT id FROM users WHERE name IS NULL);",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NotInSubquery.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn in_subquery_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name GetActive\n-- @returns :many\nSELECT id FROM users WHERE id IN (SELECT id FROM users WHERE name IS NOT NULL);",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NotInSubquery.check_query(&ctx);
        assert!(v.is_empty());
    }
}
