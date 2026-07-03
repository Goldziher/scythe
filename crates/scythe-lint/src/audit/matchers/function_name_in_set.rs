//! Matcher `"function_name_in_set"` — SC-SEC01 dangerous-function.
//!
//! Reads `matcher_args.functions` (array of strings) and walks the statement
//! AST for `Expr::Function` nodes.  For each function call whose last name
//! segment matches (case-insensitive) one of the configured names, emits a
//! `MatcherHit` with binding `func -> <matched name>`.
//!
//! Ported 1:1 from `rules/security/dangerous_function.rs`.

use std::ops::ControlFlow;

use sqlparser::ast::{Expr, ObjectName, Statement, Visit, Visitor};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_function_name_in_set(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let functions = read_function_list(args);
    if functions.is_empty() {
        return Vec::new();
    }

    let mut collector = Collector {
        functions: &functions,
        hits: Vec::new(),
    };
    let _ = ctx.stmt.visit(&mut collector);

    collector
        .hits
        .into_iter()
        .map(|name| MatcherHit::with_binding("func", name))
        .collect()
}

/// Read `matcher_args.functions` as a list of lowercase strings.
fn read_function_list(args: &toml::Table) -> Vec<String> {
    args.get("functions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_ascii_lowercase()))
                .collect()
        })
        .unwrap_or_default()
}

fn last_name_segment(name: &ObjectName) -> Option<String> {
    name.0.last().and_then(|p| p.as_ident().map(|i| i.value.clone()))
}

struct Collector<'a> {
    functions: &'a [String],
    hits: Vec<String>,
}

impl Visitor for Collector<'_> {
    type Break = ();

    fn pre_visit_statement(&mut self, _stmt: &Statement) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_relation(&mut self, _relation: &ObjectName) -> ControlFlow<Self::Break> {
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        if let Expr::Function(func) = expr
            && let Some(last) = last_name_segment(&func.name)
            && self.functions.iter().any(|d| d.eq_ignore_ascii_case(&last))
        {
            self.hits.push(last);
        }
        ControlFlow::Continue(())
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

    fn make_args(fns: &[&str]) -> toml::Table {
        let mut t = toml::Table::new();
        let arr: toml::value::Array = fns.iter().map(|s| toml::Value::String((*s).to_string())).collect();
        t.insert("functions".to_string(), toml::Value::Array(arr));
        t
    }

    fn make_ctx(
        sql: &str,
    ) -> (
        sqlparser::ast::Statement,
        scythe_core::analyzer::AnalyzedQuery,
        scythe_core::catalog::Catalog,
        scythe_core::parser::Annotations,
    ) {
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

    #[test]
    fn fires_on_dangerous_function() {
        let sql = "SELECT pg_read_file('/etc/passwd')";
        let (stmt, analyzed, catalog, annotations) = make_ctx(sql);
        let ctx = LintContext {
            sql,
            stmt: &stmt,
            analyzed: &analyzed,
            catalog: &catalog,
            annotations: &annotations,
            dialect: SqlDialect::PostgreSQL,
        };
        let args = make_args(&["pg_read_file"]);
        let hits = match_function_name_in_set(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("func").map(|s| s.as_str()), Some("pg_read_file"));
    }

    #[test]
    fn case_insensitive_match() {
        let sql = "SELECT PG_READ_FILE('/etc/passwd')";
        let (stmt, analyzed, catalog, annotations) = make_ctx(sql);
        let ctx = LintContext {
            sql,
            stmt: &stmt,
            analyzed: &analyzed,
            catalog: &catalog,
            annotations: &annotations,
            dialect: SqlDialect::PostgreSQL,
        };
        let args = make_args(&["pg_read_file"]);
        let hits = match_function_name_in_set(&ctx, &args);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn no_match_when_function_not_in_set() {
        let sql = "SELECT now()";
        let (stmt, analyzed, catalog, annotations) = make_ctx(sql);
        let ctx = LintContext {
            sql,
            stmt: &stmt,
            analyzed: &analyzed,
            catalog: &catalog,
            annotations: &annotations,
            dialect: SqlDialect::PostgreSQL,
        };
        let args = make_args(&["pg_read_file"]);
        let hits = match_function_name_in_set(&ctx, &args);
        assert!(hits.is_empty());
    }
}
