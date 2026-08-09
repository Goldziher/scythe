//! Matcher `"weak_hash_over_sensitive_column"` — SC-SEC06 weak-hash-in-auth.
//!
//! Reads `matcher_args.functions` (array of strings) and
//! `matcher_args.column_patterns` (array of strings).
//!
//! Fires when a call to a weak hash function (e.g. `md5`, `sha1`) has an
//! argument that is an identifier whose last segment (case-insensitive) contains
//! any of the column patterns as a substring.
//!
//! Emits `MatcherHit` with bindings `func` = function name, `column` = full
//! identifier rendered as written.
//!
//! CWE-327 (Use of Broken or Risky Cryptographic Algorithm), CWE-916.

use std::ops::ControlFlow;

use sqlparser::ast::{Expr, FunctionArg, FunctionArgExpr, FunctionArguments, ObjectName, Statement, Visit, Visitor};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_weak_hash_over_sensitive_column(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let functions = read_string_list(args, "functions");
    let column_patterns = read_string_list(args, "column_patterns");

    if functions.is_empty() || column_patterns.is_empty() {
        return Vec::new();
    }

    let mut collector = Collector {
        functions: &functions,
        column_patterns: &column_patterns,
        hits: Vec::new(),
    };
    let _ = ctx.stmt.visit(&mut collector);
    collector.hits
}

fn read_string_list(args: &toml::Table, key: &str) -> Vec<String> {
    args.get(key)
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
    column_patterns: &'a [String],
    hits: Vec<MatcherHit>,
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
            && self.functions.iter().any(|f| f.eq_ignore_ascii_case(&last))
        {
            let func_name = last.to_ascii_lowercase();
            if let FunctionArguments::List(arg_list) = &func.args {
                for arg in &arg_list.args {
                    let inner_expr = match arg {
                        FunctionArg::Named {
                            arg: FunctionArgExpr::Expr(e),
                            ..
                        } => Some(e),
                        FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => Some(e),
                        _ => None,
                    };
                    if let Some(e) = inner_expr
                        && let Some(col) = extract_sensitive_column(e, self.column_patterns)
                    {
                        let mut hit = MatcherHit::empty();
                        hit.bindings.insert("func".to_string(), func_name.clone());
                        hit.bindings.insert("column".to_string(), col);
                        self.hits.push(hit);
                    }
                }
            }
        }
        ControlFlow::Continue(())
    }
}

/// Find the first sensitive column identifier reachable from `expr`.
///
/// Recurses through the shapes real salted/wrapped hash calls take —
/// `md5(password || salt)` (`BinaryOp`), `md5((password))` (`Nested`),
/// `md5(lower(password))` (nested `Function`), and `md5(password::text)`
/// (`Cast`) — rather than only matching a bare identifier one level down.
fn extract_sensitive_column(expr: &Expr, patterns: &[String]) -> Option<String> {
    match expr {
        Expr::Identifier(ident) => {
            let lower = ident.value.to_ascii_lowercase();
            if patterns.iter().any(|p| lower.contains(p.as_str())) {
                Some(ident.value.clone())
            } else {
                None
            }
        }
        Expr::CompoundIdentifier(parts) => {
            if let Some(last) = parts.last() {
                let lower = last.value.to_ascii_lowercase();
                if patterns.iter().any(|p| lower.contains(p.as_str())) {
                    let rendered = parts.iter().map(|i| i.value.as_str()).collect::<Vec<_>>().join(".");
                    return Some(rendered);
                }
            }
            None
        }
        Expr::Nested(inner) => extract_sensitive_column(inner, patterns),
        Expr::Cast { expr: inner, .. } => extract_sensitive_column(inner, patterns),
        Expr::BinaryOp { left, right, .. } => {
            extract_sensitive_column(left, patterns).or_else(|| extract_sensitive_column(right, patterns))
        }
        Expr::Function(func) => {
            if let FunctionArguments::List(arg_list) = &func.args {
                for arg in &arg_list.args {
                    let inner = match arg {
                        FunctionArg::Named {
                            arg: FunctionArgExpr::Expr(e),
                            ..
                        } => Some(e),
                        FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => Some(e),
                        _ => None,
                    };
                    if let Some(found) = inner.and_then(|e| extract_sensitive_column(e, patterns)) {
                        return Some(found);
                    }
                }
            }
            None
        }
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

    fn make_args(functions: &[&str], column_patterns: &[&str]) -> toml::Table {
        let mut t = toml::Table::new();
        let fn_arr: toml::value::Array = functions
            .iter()
            .map(|s| toml::Value::String((*s).to_string()))
            .collect();
        t.insert("functions".to_string(), toml::Value::Array(fn_arr));
        let pat_arr: toml::value::Array = column_patterns
            .iter()
            .map(|s| toml::Value::String((*s).to_string()))
            .collect();
        t.insert("column_patterns".to_string(), toml::Value::Array(pat_arr));
        t
    }

    fn make_parts(sql: &str) -> (sqlparser::ast::Statement, AnalyzedQuery, Catalog, Annotations) {
        let stmt = Parser::parse_sql(&PostgreSqlDialect {}, sql).unwrap().remove(0);
        let analyzed = AnalyzedQuery::build(|aq| {
            aq.name = "q".to_string();
            aq.command = QueryCommand::Many;
            aq.sql = sql.to_string();
            aq.columns = vec![];
            aq.params = vec![];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = None;
            aq.custom = vec![];
        });
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
    fn fires_on_md5_over_password() {
        let sql = "SELECT md5(password) FROM users";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["md5", "sha1"], &["password", "passwd", "secret"]);
        let hits = match_weak_hash_over_sensitive_column(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("func").map(|s| s.as_str()), Some("md5"));
        assert_eq!(hits[0].bindings.get("column").map(|s| s.as_str()), Some("password"));
    }

    #[test]
    fn no_match_md5_over_non_sensitive_column() {
        let sql = "SELECT md5(username) FROM users";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["md5", "sha1"], &["password", "passwd", "secret"]);
        let hits = match_weak_hash_over_sensitive_column(&ctx, &args);
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_compound_identifier_users_dot_password() {
        let sql = "SELECT id FROM users WHERE sha1(users.password) = $1";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["md5", "sha1"], &["password"]);
        let hits = match_weak_hash_over_sensitive_column(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("func").map(|s| s.as_str()), Some("sha1"));
        assert_eq!(
            hits[0].bindings.get("column").map(|s| s.as_str()),
            Some("users.password")
        );
    }

    #[test]
    fn fires_on_update_set_with_md5_over_password() {
        let sql = "UPDATE users SET password = md5(password) WHERE id = $1";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["md5"], &["password"]);
        let hits = match_weak_hash_over_sensitive_column(&ctx, &args);
        assert_eq!(hits.len(), 1);
    }

    /// Regression for #138: `md5(password || salt)` is the shape real
    /// salted-MD5 auth SQL takes. The naive `md5(password)` case (the
    /// control above) already fired; the salted form must too.
    #[test]
    fn fires_on_md5_over_salted_password() {
        let sql = "SELECT md5(password || salt) FROM users";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["md5", "sha1"], &["password", "passwd", "secret"]);
        let hits = match_weak_hash_over_sensitive_column(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("func").map(|s| s.as_str()), Some("md5"));
        assert_eq!(hits[0].bindings.get("column").map(|s| s.as_str()), Some("password"));
    }

    /// Regression for #138: `md5(lower(password))` wraps the sensitive
    /// column in another function call before hashing.
    #[test]
    fn fires_on_md5_over_wrapped_password() {
        let sql = "SELECT md5(lower(password)) FROM users";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["md5", "sha1"], &["password", "passwd", "secret"]);
        let hits = match_weak_hash_over_sensitive_column(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("column").map(|s| s.as_str()), Some("password"));
    }

    /// A wrapped/salted call over a *non*-sensitive column must still not
    /// fire — the recursive walk must not become a blanket "any argument
    /// anywhere" match.
    #[test]
    fn no_match_md5_over_wrapped_non_sensitive_column() {
        let sql = "SELECT md5(lower(username) || salt) FROM users";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["md5", "sha1"], &["password", "passwd", "secret"]);
        let hits = match_weak_hash_over_sensitive_column(&ctx, &args);
        assert!(hits.is_empty());
    }
}
