//! Matcher `"check_constraint_always_true"` — SC-CHK01.
//!
//! Takes no `matcher_args`. Fires when a `CHECK` constraint expression is a
//! tautology (`true`, `1 = 1`, `NULL`, or a parenthesised variant) — the
//! constraint enforces nothing and almost always signals a copy-paste mistake
//! or an unfinished migration. Covers three syntactic positions:
//!
//! - `CREATE TABLE (col TYPE CHECK (…))` — column-level CHECK via
//!   `ColumnOption::Check(CheckConstraint)`.
//! - `CREATE TABLE (col TYPE, CHECK (…))` — table-level CHECK via
//!   `TableConstraint::Check(CheckConstraint)`.
//! - `ALTER TABLE … ADD CONSTRAINT … CHECK (…)` — same `TableConstraint::Check`
//!   shape inside `AlterTableOperation::AddConstraint`.
//!
//! Emits one hit per offending CHECK with bindings `table` and optional
//! `constraint_name` (when the constraint was explicitly named).

use sqlparser::ast::{AlterTableOperation, ColumnOption, Expr, Statement, TableConstraint, Value};

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

fn make_hit(table: &str, name: Option<String>) -> MatcherHit {
    let mut hit = MatcherHit::empty();
    hit.bindings.insert("table".to_string(), table.to_string());
    if let Some(n) = name {
        hit.bindings.insert("constraint_name".to_string(), n);
    }
    hit
}

pub fn match_check_constraint_always_true(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    match ctx.stmt {
        Statement::CreateTable(ct) => {
            let table = ct.name.to_string();
            let mut hits = Vec::new();
            for col in &ct.columns {
                for opt in &col.options {
                    if let ColumnOption::Check(check) = &opt.option
                        && is_tautology(&check.expr)
                    {
                        hits.push(make_hit(&table, check.name.as_ref().map(|i| i.value.clone())));
                    }
                }
            }
            for c in &ct.constraints {
                if let TableConstraint::Check(check) = c
                    && is_tautology(&check.expr)
                {
                    hits.push(make_hit(&table, check.name.as_ref().map(|i| i.value.clone())));
                }
            }
            hits
        }
        Statement::AlterTable(alter) => {
            let table = alter.name.to_string();
            alter
                .operations
                .iter()
                .filter_map(|op| match op {
                    AlterTableOperation::AddConstraint {
                        constraint: TableConstraint::Check(check),
                        ..
                    } if is_tautology(&check.expr) => {
                        Some(make_hit(&table, check.name.as_ref().map(|i| i.value.clone())))
                    }
                    _ => None,
                })
                .collect()
        }
        _ => Vec::new(),
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
    fn fires_on_column_check_true() {
        let sql = "CREATE TABLE x (a int CHECK (true));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_check_constraint_always_true(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("table").map(|s| s.as_str()), Some("x"));
    }

    #[test]
    fn fires_on_table_check_1_eq_1() {
        let sql = "CREATE TABLE x (a int, CONSTRAINT noop CHECK (1 = 1));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_check_constraint_always_true(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("constraint_name").map(|s| s.as_str()),
            Some("noop")
        );
    }

    #[test]
    fn fires_on_alter_table_add_constraint_check_true() {
        let sql = "ALTER TABLE x ADD CONSTRAINT noop CHECK (true);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_check_constraint_always_true(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn no_match_real_check() {
        let sql = "CREATE TABLE x (a int CHECK (a > 0));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_check_constraint_always_true(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_eq() {
        // 1 = 2 is also a tautological constant, but it's always FALSE — that
        // CHECK would forbid every row, so SC-CHK01 stays narrow and does
        // NOT fire here. (A later rule could flag always-false CHECKs.)
        let sql = "CREATE TABLE x (a int, CHECK (1 = 2));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_check_constraint_always_true(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_nested_true() {
        let sql = "CREATE TABLE x (a int CHECK (((true))));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_check_constraint_always_true(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_check_constraint_always_true(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
