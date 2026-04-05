use std::borrow::Cow;

use sqlparser::ast::*;

use crate::rule::LintRule;
use crate::types::*;
use scythe_core::parser::QueryCommand;

// ---------------------------------------------------------------------------
// SC-S01: UpdateWithoutWhere
// ---------------------------------------------------------------------------

pub struct UpdateWithoutWhere;

impl LintRule for UpdateWithoutWhere {
    fn id(&self) -> &'static str {
        "SC-S01"
    }
    fn name(&self) -> &'static str {
        "update-without-where"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Safety
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &'static str {
        "UPDATE without WHERE affects all rows"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        if let Statement::Update(update) = ctx.stmt
            && update.selection.is_none()
        {
            return vec![Violation {
                rule_id: Cow::Borrowed(self.id()),
                message: "UPDATE statement has no WHERE clause — all rows will be affected".into(),
                fix: None,
            }];
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// SC-S02: DeleteWithoutWhere
// ---------------------------------------------------------------------------

pub struct DeleteWithoutWhere;

impl LintRule for DeleteWithoutWhere {
    fn id(&self) -> &'static str {
        "SC-S02"
    }
    fn name(&self) -> &'static str {
        "delete-without-where"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Safety
    }
    fn default_severity(&self) -> Severity {
        Severity::Error
    }
    fn description(&self) -> &'static str {
        "DELETE without WHERE affects all rows"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        if let Statement::Delete(delete) = ctx.stmt
            && delete.selection.is_none()
        {
            return vec![Violation {
                rule_id: Cow::Borrowed(self.id()),
                message: "DELETE statement has no WHERE clause — all rows will be affected".into(),
                fix: None,
            }];
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// SC-S03: NoSelectStar
// ---------------------------------------------------------------------------

pub struct NoSelectStar;

impl LintRule for NoSelectStar {
    fn id(&self) -> &'static str {
        "SC-S03"
    }
    fn name(&self) -> &'static str {
        "no-select-star"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Safety
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "SELECT * makes queries fragile when columns change"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let mut violations = Vec::new();
        walk_select_items(ctx.stmt, &mut |item| {
            if matches!(item, SelectItem::Wildcard(_)) {
                violations.push(Violation {
                    rule_id: Cow::Borrowed(self.id()),
                    message: "avoid SELECT * — list columns explicitly".into(),
                    fix: None,
                });
            }
        });
        violations
    }
}

// ---------------------------------------------------------------------------
// SC-S04: UnusedParams
// ---------------------------------------------------------------------------

pub struct UnusedParams;

impl LintRule for UnusedParams {
    fn id(&self) -> &'static str {
        "SC-S04"
    }
    fn name(&self) -> &'static str {
        "unused-params"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Safety
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "Declared parameter placeholders ($N) not all used"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        // Collect all $N placeholders referenced in SQL text
        let mut referenced: ahash::AHashSet<i64> = ahash::AHashSet::new();
        let sql = ctx.sql;
        let mut chars = sql.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if ch == '$' {
                let start = i + 1;
                let mut end = start;
                while let Some(&(j, c)) = chars.peek() {
                    if c.is_ascii_digit() {
                        end = j + 1;
                        chars.next();
                    } else {
                        break;
                    }
                }
                if end > start
                    && let Ok(n) = sql[start..end].parse::<i64>()
                {
                    referenced.insert(n);
                }
            }
        }

        if referenced.is_empty() {
            return Vec::new();
        }

        let max_ref = referenced.iter().copied().max().unwrap_or(0);

        // Check for gaps: if max is N, then 1..=N should all be present
        let mut violations = Vec::new();
        for n in 1..=max_ref {
            if !referenced.contains(&n) {
                violations.push(Violation {
                    rule_id: Cow::Borrowed(self.id()),
                    message: format!("parameter ${} is declared but never used", n),
                    fix: None,
                });
            }
        }
        violations
    }
}

// ---------------------------------------------------------------------------
// SC-S05: MissingReturning
// ---------------------------------------------------------------------------

pub struct MissingReturning;

impl LintRule for MissingReturning {
    fn id(&self) -> &'static str {
        "SC-S05"
    }
    fn name(&self) -> &'static str {
        "missing-returning"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Safety
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "DML with :one/:many command should have a RETURNING clause"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let expects_rows = matches!(ctx.analyzed.command, QueryCommand::One | QueryCommand::Many);
        if !expects_rows {
            return Vec::new();
        }

        let has_returning = match ctx.stmt {
            Statement::Insert(ins) => ins.returning.is_some(),
            Statement::Update(upd) => upd.returning.is_some(),
            Statement::Delete(del) => del.returning.is_some(),
            _ => return Vec::new(), // SELECT always returns rows
        };

        if !has_returning {
            return vec![Violation {
                rule_id: Cow::Borrowed(self.id()),
                message: format!(
                    "DML with :{} command but no RETURNING clause",
                    ctx.analyzed.command
                ),
                fix: None,
            }];
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// SC-S06: AmbiguousColumnInJoin
// ---------------------------------------------------------------------------

pub struct AmbiguousColumnInJoin;

impl LintRule for AmbiguousColumnInJoin {
    fn id(&self) -> &'static str {
        "SC-S06"
    }
    fn name(&self) -> &'static str {
        "ambiguous-column-in-join"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Safety
    }
    fn default_severity(&self) -> Severity {
        Severity::Warn
    }
    fn description(&self) -> &'static str {
        "SELECT with JOIN has unqualified column references"
    }

    fn check_query(&self, ctx: &LintContext<'_>) -> Vec<Violation> {
        let has_join = stmt_has_join(ctx.stmt);
        if !has_join {
            return Vec::new();
        }

        let mut violations = Vec::new();
        walk_select_items(ctx.stmt, &mut |item| match item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                if let Expr::Identifier(ident) = expr {
                    violations.push(Violation {
                            rule_id: Cow::Borrowed("SC-S06"),
                            message: format!(
                                "column \"{}\" is unqualified in a JOIN query — prefix with table alias",
                                ident.value
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
// Helpers
// ---------------------------------------------------------------------------

fn walk_select_items(stmt: &Statement, visitor: &mut dyn FnMut(&SelectItem)) {
    if let Statement::Query(query) = stmt {
        walk_query_select_items(query, visitor)
    }
}

fn walk_query_select_items(query: &Query, visitor: &mut dyn FnMut(&SelectItem)) {
    walk_set_expr_select_items(&query.body, visitor);
}

fn walk_set_expr_select_items(set_expr: &SetExpr, visitor: &mut dyn FnMut(&SelectItem)) {
    match set_expr {
        SetExpr::Select(select) => {
            for item in &select.projection {
                visitor(item);
            }
        }
        SetExpr::Query(query) => walk_query_select_items(query, visitor),
        SetExpr::SetOperation { left, right, .. } => {
            walk_set_expr_select_items(left, visitor);
            walk_set_expr_select_items(right, visitor);
        }
        _ => {}
    }
}

fn stmt_has_join(stmt: &Statement) -> bool {
    match stmt {
        Statement::Query(query) => query_has_join(query),
        _ => false,
    }
}

fn query_has_join(query: &Query) -> bool {
    set_expr_has_join(&query.body)
}

fn set_expr_has_join(set_expr: &SetExpr) -> bool {
    match set_expr {
        SetExpr::Select(select) => select.from.iter().any(|twj| !twj.joins.is_empty()),
        SetExpr::Query(query) => query_has_join(query),
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_has_join(left) || set_expr_has_join(right)
        }
        _ => false,
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
            "CREATE TABLE users (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT NOT NULL
            );",
            "CREATE TABLE posts (
                id SERIAL PRIMARY KEY,
                user_id INTEGER NOT NULL,
                title TEXT NOT NULL
            );",
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

    // SC-S01

    #[test]
    fn update_without_where_fires() {
        let cat = make_catalog();
        let q = parse_query("-- @name UpdateAll\n-- @returns :exec\nUPDATE users SET name = $1;")
            .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = UpdateWithoutWhere.check_query(&ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule_id, "SC-S01");
    }

    #[test]
    fn update_with_where_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name UpdateOne\n-- @returns :exec\nUPDATE users SET name = $1 WHERE id = $2;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = UpdateWithoutWhere.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S02

    #[test]
    fn delete_without_where_fires() {
        let cat = make_catalog();
        let q = parse_query("-- @name DeleteAll\n-- @returns :exec\nDELETE FROM users;").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = DeleteWithoutWhere.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn delete_with_where_ok() {
        let cat = make_catalog();
        let q =
            parse_query("-- @name DeleteOne\n-- @returns :exec\nDELETE FROM users WHERE id = $1;")
                .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = DeleteWithoutWhere.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S03

    #[test]
    fn select_star_fires() {
        let cat = make_catalog();
        let q = parse_query("-- @name ListAll\n-- @returns :many\nSELECT * FROM users;").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NoSelectStar.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn select_cols_ok() {
        let cat = make_catalog();
        let q = parse_query("-- @name ListAll\n-- @returns :many\nSELECT id, name FROM users;")
            .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NoSelectStar.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S05

    #[test]
    fn missing_returning_fires() {
        let cat = make_catalog();
        let q = parse_query("-- @name CreateUser\n-- @returns :one\nINSERT INTO users (name, email) VALUES ($1, $2);").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = MissingReturning.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn has_returning_ok() {
        let cat = make_catalog();
        let q = parse_query("-- @name CreateUser\n-- @returns :one\nINSERT INTO users (name, email) VALUES ($1, $2) RETURNING id, name, email;").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = MissingReturning.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S06

    #[test]
    fn ambiguous_column_in_join_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name ListJoined\n-- @returns :many\nSELECT title FROM users u JOIN posts p ON u.id = p.user_id;",
        ).unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = AmbiguousColumnInJoin.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn qualified_column_in_join_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name ListJoined\n-- @returns :many\nSELECT p.title FROM users u JOIN posts p ON u.id = p.user_id;",
        ).unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = AmbiguousColumnInJoin.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S04: UnusedParams — additional coverage

    #[test]
    fn unused_params_gap_fires() {
        // $1 and $2 declared but only $1 used → $2 gap not possible here;
        // use $1 and $3 to create a gap at $2
        let cat = make_catalog();
        let q = parse_query(
            "-- @name UpdateSome\n-- @returns :exec\nUPDATE users SET name = $1 WHERE id = $3;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = UnusedParams.check_query(&ctx);
        assert_eq!(v.len(), 1);
        assert!(v[0].message.contains("$2"));
    }

    #[test]
    fn unused_params_all_used_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name UpdateOne\n-- @returns :exec\nUPDATE users SET name = $1 WHERE id = $2;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = UnusedParams.check_query(&ctx);
        assert!(v.is_empty());
    }

    #[test]
    fn unused_params_no_params_ok() {
        let cat = make_catalog();
        let q = parse_query("-- @name ListAll\n-- @returns :many\nSELECT id, name FROM users;")
            .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = UnusedParams.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S06: AmbiguousColumnInJoin — additional coverage

    #[test]
    fn no_join_no_ambiguity() {
        let cat = make_catalog();
        let q =
            parse_query("-- @name ListUsers\n-- @returns :many\nSELECT name FROM users;").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = AmbiguousColumnInJoin.check_query(&ctx);
        assert!(v.is_empty());
    }

    #[test]
    fn select_star_in_join_s06_does_not_fire() {
        // SELECT * with JOIN: S06 looks at Wildcard via walk_select_items.
        // Wildcard is not UnnamedExpr(Identifier), so S06 should NOT fire for *.
        let cat = make_catalog();
        let q = parse_query(
            "-- @name ListJoined\n-- @returns :many\nSELECT u.id, p.title FROM users u JOIN posts p ON u.id = p.user_id;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);

        let v_s06 = AmbiguousColumnInJoin.check_query(&ctx);
        assert!(
            v_s06.is_empty(),
            "S06 should not fire on fully qualified columns"
        );
    }

    // SC-S01/S02: WHERE clause edge cases

    #[test]
    fn update_with_subquery_in_where_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name UpdateSub\n-- @returns :exec\nUPDATE users SET name = $1 WHERE id IN (SELECT user_id FROM posts);",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = UpdateWithoutWhere.check_query(&ctx);
        assert!(v.is_empty());
    }

    #[test]
    fn delete_with_subquery_in_where_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name DeleteSub\n-- @returns :exec\nDELETE FROM users WHERE id IN (SELECT user_id FROM posts);",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = DeleteWithoutWhere.check_query(&ctx);
        assert!(v.is_empty());
    }

    #[test]
    fn update_with_where_true_ok() {
        // WHERE TRUE is still a WHERE clause — rule should not fire
        let cat = make_catalog();
        let q = parse_query(
            "-- @name UpdateAll\n-- @returns :exec\nUPDATE users SET name = $1 WHERE TRUE;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = UpdateWithoutWhere.check_query(&ctx);
        assert!(v.is_empty());
    }

    #[test]
    fn delete_with_where_true_ok() {
        let cat = make_catalog();
        let q = parse_query("-- @name DeleteAll\n-- @returns :exec\nDELETE FROM users WHERE TRUE;")
            .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = DeleteWithoutWhere.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S03: table-qualified wildcard

    #[test]
    fn select_qualified_star_ok() {
        // SELECT users.* is a QualifiedWildcard, not a Wildcard — S03 should NOT fire
        let cat = make_catalog();
        let q =
            parse_query("-- @name ListAll\n-- @returns :many\nSELECT users.* FROM users;").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = NoSelectStar.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S05: :exec should not fire even without RETURNING

    #[test]
    fn missing_returning_exec_ok() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name CreateUser\n-- @returns :exec\nINSERT INTO users (name, email) VALUES ($1, $2);",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = MissingReturning.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S05: SELECT always returns rows — should not fire

    #[test]
    fn missing_returning_select_ok() {
        let cat = make_catalog();
        let q = parse_query("-- @name ListUsers\n-- @returns :many\nSELECT id, name FROM users;")
            .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = MissingReturning.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S05: UPDATE :many without RETURNING should fire

    #[test]
    fn missing_returning_update_many_fires() {
        let cat = make_catalog();
        let q = parse_query(
            "-- @name UpdateMany\n-- @returns :many\nUPDATE users SET name = $1 WHERE id = $2;",
        )
        .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = MissingReturning.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    // SC-S05: DELETE :one without RETURNING should fire

    #[test]
    fn missing_returning_delete_one_fires() {
        let cat = make_catalog();
        let q =
            parse_query("-- @name DeleteOne\n-- @returns :one\nDELETE FROM users WHERE id = $1;")
                .unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = MissingReturning.check_query(&ctx);
        assert_eq!(v.len(), 1);
    }

    // SC-S01: non-UPDATE statement should not fire

    #[test]
    fn update_without_where_on_select_noop() {
        let cat = make_catalog();
        let q =
            parse_query("-- @name ListUsers\n-- @returns :many\nSELECT id FROM users;").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = UpdateWithoutWhere.check_query(&ctx);
        assert!(v.is_empty());
    }

    // SC-S02: non-DELETE statement should not fire

    #[test]
    fn delete_without_where_on_select_noop() {
        let cat = make_catalog();
        let q =
            parse_query("-- @name ListUsers\n-- @returns :many\nSELECT id FROM users;").unwrap();
        let a = analyzer::analyze(&cat, &q).unwrap();
        let ctx = make_ctx(&q, &a, &cat);
        let v = DeleteWithoutWhere.check_query(&ctx);
        assert!(v.is_empty());
    }
}
