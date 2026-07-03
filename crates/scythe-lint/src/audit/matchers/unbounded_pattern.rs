//! Matcher `"unbounded_pattern"` — SC-SEC09 unbounded-like.
//!
//! No `matcher_args`.  Walks the statement AST for `Expr::Like` /
//! `Expr::ILike` nodes whose pattern starts AND ends with `%`.
//! Emits a `MatcherHit` with binding `pattern -> "%admin%"`.
//!
//! Ported 1:1 from `rules/security/unbounded_like.rs`.

use std::ops::ControlFlow;

use sqlparser::ast::{Expr, ObjectName, Statement, Value, ValueWithSpan, Visit, Visitor};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_unbounded_pattern(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let mut collector = Collector { hits: Vec::new() };
    let _ = ctx.stmt.visit(&mut collector);

    collector
        .hits
        .into_iter()
        .map(|pat| MatcherHit::with_binding("pattern", pat))
        .collect()
}

struct Collector {
    hits: Vec<String>,
}

impl Visitor for Collector {
    type Break = ();

    fn pre_visit_statement(&mut self, _stmt: &Statement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_relation(&mut self, _relation: &ObjectName) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        match expr {
            Expr::Like { pattern, .. } | Expr::ILike { pattern, .. } => {
                if let Some(s) = extract_string_literal(pattern)
                    && s.starts_with('%')
                    && s.ends_with('%')
                    && s.len() >= 2
                {
                    self.hits.push(s);
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

fn extract_string_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Value(ValueWithSpan {
            value: Value::SingleQuotedString(s),
            ..
        })
        | Expr::Value(ValueWithSpan {
            value: Value::DoubleQuotedString(s),
            ..
        })
        | Expr::Value(ValueWithSpan {
            value: Value::EscapedStringLiteral(s),
            ..
        })
        | Expr::Value(ValueWithSpan {
            value: Value::NationalStringLiteral(s),
            ..
        }) => Some(s.clone()),
        _ => None,
    }
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
    fn fires_on_both_wildcard_like() {
        let sql = "SELECT * FROM users WHERE name LIKE '%admin%'";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_unbounded_pattern(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("pattern").map(|s| s.as_str()), Some("%admin%"));
    }

    #[test]
    fn no_match_on_prefix_only_like() {
        let sql = "SELECT * FROM users WHERE name LIKE 'admin%'";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_unbounded_pattern(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_ilike() {
        let sql = "SELECT * FROM users WHERE name ILIKE '%ADMIN%'";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_unbounded_pattern(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("pattern").map(|s| s.as_str()), Some("%ADMIN%"));
    }
}
