//! Matcher `"cartesian_join"` — SC-SEC08 cartesian-join.
//!
//! No `matcher_args`.  Emits empty `MatcherHit` for each cartesian-product
//! shape detected: two messages are possible (comma-FROM and
//! ON-true/CROSS JOIN).  The message field in the TOML carries the
//! comma-FROM variant; the ON-true/CROSS JOIN variant has its own message.
//!
//! The baseline output includes only the comma-FROM message for the smoke
//! fixture, so we emit two distinct MatcherHits for the two shapes — but the
//! TOML message template is shared.  To keep output byte-identical we emit
//! `MatcherHit`s with a `shape` binding that the message template can use,
//! OR we keep both messages hard-coded by emitting pre-rendered strings.
//!
//! The existing rule emits two different message strings for the two cases.
//! The smoke fixture only triggers the comma-FROM shape.  To keep output
//! byte-identical while using a single message template, we need two separate
//! TOML rules — or we include both messages inside one matcher by varying the
//! binding.  The TOML approach uses a `msg` binding for the full text.
//!
//! Decision: emit `MatcherHit::with_binding("msg", <full_message>)` and use
//! `{msg}` as the TOML message template.  This is the cleanest approach that
//! doesn't require two separate TOML rules.

use sqlparser::ast::{Expr, JoinConstraint, JoinOperator, SetExpr, Statement, Value, ValueWithSpan};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

const MSG_COMMA_FROM: &str = "comma-separated FROM with no WHERE — this is a cartesian product";
const MSG_UNCONSTRAINED_JOIN: &str = "unconstrained join (ON true / CROSS JOIN) — produces a cartesian product";

pub fn match_cartesian_join(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let mut hits = Vec::new();
    if let Statement::Query(q) = ctx.stmt {
        walk_set_expr(&q.body, &mut hits);
    }
    hits
}

fn walk_set_expr(set_expr: &SetExpr, hits: &mut Vec<MatcherHit>) {
    match set_expr {
        SetExpr::Select(select) => {
            if select.from.len() > 1 && select.selection.is_none() {
                hits.push(MatcherHit::with_binding("msg", MSG_COMMA_FROM));
            }
            for twj in &select.from {
                for join in &twj.joins {
                    if join_is_unconstrained(&join.join_operator) {
                        hits.push(MatcherHit::with_binding("msg", MSG_UNCONSTRAINED_JOIN));
                    }
                }
            }
        }
        SetExpr::Query(q) => walk_set_expr(&q.body, hits),
        SetExpr::SetOperation { left, right, .. } => {
            walk_set_expr(left, hits);
            walk_set_expr(right, hits);
        }
        _ => {}
    }
}

fn join_is_unconstrained(op: &JoinOperator) -> bool {
    match op {
        JoinOperator::CrossJoin(_) => true,
        JoinOperator::Inner(c)
        | JoinOperator::LeftOuter(c)
        | JoinOperator::RightOuter(c)
        | JoinOperator::FullOuter(c)
        | JoinOperator::Join(c) => match c {
            JoinConstraint::On(expr) => is_literal_true(expr),
            _ => false,
        },
        _ => false,
    }
}

fn is_literal_true(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Value(ValueWithSpan {
            value: Value::Boolean(true),
            ..
        })
    )
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

    fn make_ctx_parts(sql: &str) -> (sqlparser::ast::Statement, AnalyzedQuery, Catalog, Annotations) {
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
    fn fires_on_comma_from_no_where() {
        let sql = "SELECT a.id, b.name FROM users a, orders b";
        let (stmt, analyzed, catalog, annotations) = make_ctx_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_cartesian_join(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("msg").map(|s| s.as_str()), Some(MSG_COMMA_FROM));
    }

    #[test]
    fn no_match_with_where_clause() {
        let sql = "SELECT a.id, b.name FROM users a, orders b WHERE a.id = b.user_id";
        let (stmt, analyzed, catalog, annotations) = make_ctx_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_cartesian_join(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_cross_join() {
        let sql = "SELECT a.id, b.name FROM users a CROSS JOIN orders b";
        let (stmt, analyzed, catalog, annotations) = make_ctx_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_cartesian_join(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("msg").map(|s| s.as_str()),
            Some(MSG_UNCONSTRAINED_JOIN)
        );
    }
}
