//! Matcher `"select_star_over_pii_columns"` — SC-SEC07 select-star-pii.
//!
//! Reads `matcher_args.column_patterns` (array of strings).  Fires when
//! `SELECT *` (wildcard or qualified wildcard) is used against a table whose
//! catalog definition contains a column whose name (lowercased) contains any
//! of the column patterns as a substring.
//!
//! Emits one `MatcherHit` per (table, first matching column) with bindings:
//! - `table`   — table name as written in the SQL
//! - `column`  — the offending column name
//! - `pattern` — the matched pattern substring
//!
//! Tables not found in the catalog are skipped (no hit).
//!
//! CWE-200 (Information Exposure).

use sqlparser::ast::{SelectItem, SetExpr, Statement, TableFactor, TableWithJoins};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_select_star_over_pii_columns(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let column_patterns = read_string_list(args, "column_patterns");
    if column_patterns.is_empty() {
        return Vec::new();
    }

    let Statement::Query(q) = ctx.stmt else {
        return Vec::new();
    };

    let mut hits = Vec::new();
    check_set_expr(&q.body, &column_patterns, ctx, &mut hits);
    hits
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

fn check_set_expr(set_expr: &SetExpr, column_patterns: &[String], ctx: &LintContext<'_>, hits: &mut Vec<MatcherHit>) {
    match set_expr {
        SetExpr::Select(select) => {
            let has_wildcard = select
                .projection
                .iter()
                .any(|item| matches!(item, SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _)));
            if !has_wildcard {
                return;
            }
            for twj in &select.from {
                check_table_with_joins(twj, column_patterns, ctx, hits);
            }
        }
        SetExpr::Query(q) => check_set_expr(&q.body, column_patterns, ctx, hits),
        SetExpr::SetOperation { left, right, .. } => {
            check_set_expr(left, column_patterns, ctx, hits);
            check_set_expr(right, column_patterns, ctx, hits);
        }
        _ => {}
    }
}

fn check_table_with_joins(
    twj: &TableWithJoins,
    column_patterns: &[String],
    ctx: &LintContext<'_>,
    hits: &mut Vec<MatcherHit>,
) {
    check_table_factor(&twj.relation, column_patterns, ctx, hits);
    for join in &twj.joins {
        check_table_factor(&join.relation, column_patterns, ctx, hits);
    }
}

fn check_table_factor(
    factor: &TableFactor,
    column_patterns: &[String],
    ctx: &LintContext<'_>,
    hits: &mut Vec<MatcherHit>,
) {
    if let TableFactor::Table { name, .. } = factor {
        let table_as_written = name.to_string();
        let table_def = ctx.catalog.get_table(&table_as_written).or_else(|| {
            name.0
                .last()
                .and_then(|p| p.as_ident().map(|i| i.value.as_str()))
                .and_then(|bare| ctx.catalog.get_table(bare))
        });

        if let Some(table) = table_def {
            for col in &table.columns {
                let col_lower = col.name.to_ascii_lowercase();
                if let Some(pat) = column_patterns.iter().find(|p| col_lower.contains(p.as_str())) {
                    let mut hit = MatcherHit::empty();
                    hit.bindings.insert("table".to_string(), table_as_written.clone());
                    hit.bindings.insert("column".to_string(), col.name.clone());
                    hit.bindings.insert("pattern".to_string(), pat.clone());
                    hits.push(hit);
                    break;
                }
            }
        }
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

    fn make_args(patterns: &[&str]) -> toml::Table {
        let mut t = toml::Table::new();
        let arr: toml::value::Array = patterns.iter().map(|s| toml::Value::String((*s).to_string())).collect();
        t.insert("column_patterns".to_string(), toml::Value::Array(arr));
        t
    }

    fn make_parts_with_ddl(
        sql: &str,
        ddl: &[&str],
    ) -> (sqlparser::ast::Statement, AnalyzedQuery, Catalog, Annotations) {
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
        let catalog = Catalog::from_ddl(ddl).unwrap();
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
    fn fires_when_star_on_pii_table() {
        let ddl = &["CREATE TABLE users (id SERIAL, email TEXT, password TEXT);"];
        let sql = "SELECT * FROM users";
        let (stmt, analyzed, catalog, annotations) = make_parts_with_ddl(sql, ddl);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["password", "email"]);
        let hits = match_select_star_over_pii_columns(&ctx, &args);
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.bindings.get("table").map(|s| s.as_str()), Some("users"));
    }

    #[test]
    fn no_match_select_star_on_clean_table() {
        let ddl = &["CREATE TABLE products (id SERIAL, name TEXT, price NUMERIC);"];
        let sql = "SELECT * FROM products";
        let (stmt, analyzed, catalog, annotations) = make_parts_with_ddl(sql, ddl);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["password", "email", "ssn"]);
        let hits = match_select_star_over_pii_columns(&ctx, &args);
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_explicit_column_list() {
        let ddl = &["CREATE TABLE users (id SERIAL, email TEXT, password TEXT);"];
        let sql = "SELECT id, email FROM users";
        let (stmt, analyzed, catalog, annotations) = make_parts_with_ddl(sql, ddl);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["password", "email"]);
        let hits = match_select_star_over_pii_columns(&ctx, &args);
        assert!(hits.is_empty());
    }
}
