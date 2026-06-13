//! Matcher `"constraint_missing_not_valid"` — SC-MIG05
//! constraint-missing-not-valid.
//!
//! Takes no `matcher_args`. Walks `ALTER TABLE … ADD CONSTRAINT …` and fires
//! when a `FOREIGN KEY` or `CHECK` constraint is added without `NOT VALID`.
//! Emits one hit per offending constraint with bindings `table` and
//! `constraint_kind` (`"foreign key"` or `"check"`), plus optional
//! `constraint_name` when the constraint is explicitly named.
//!
//! Migration-safety motivation: adding a `FOREIGN KEY` or `CHECK` constraint
//! without `NOT VALID` validates every existing row under an `ACCESS
//! EXCLUSIVE` lock, blocking writes for the duration of the scan. The safe
//! pattern is `ADD CONSTRAINT … NOT VALID;` followed by a non-blocking
//! `VALIDATE CONSTRAINT` in a second migration.

use sqlparser::ast::{AlterTableOperation, Statement, TableConstraint};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_constraint_missing_not_valid(
    ctx: &LintContext<'_>,
    _args: &toml::Table,
) -> Vec<MatcherHit> {
    let Statement::AlterTable(alter) = ctx.stmt else {
        return Vec::new();
    };
    let table = alter.name.to_string();
    alter
        .operations
        .iter()
        .filter_map(|op| match op {
            AlterTableOperation::AddConstraint {
                constraint,
                not_valid: false,
            } => match constraint {
                TableConstraint::ForeignKey(fk) => {
                    Some(("foreign key", fk.name.as_ref().map(|i| i.value.clone())))
                }
                TableConstraint::Check(check) => {
                    Some(("check", check.name.as_ref().map(|i| i.value.clone())))
                }
                _ => None,
            },
            _ => None,
        })
        .map(|(kind, name)| {
            let mut hit = MatcherHit::empty();
            hit.bindings.insert("table".to_string(), table.clone());
            hit.bindings
                .insert("constraint_kind".to_string(), kind.to_string());
            if let Some(n) = name {
                hit.bindings.insert("constraint_name".to_string(), n);
            }
            hit
        })
        .collect()
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

    fn make_parts(
        sql: &str,
    ) -> (
        sqlparser::ast::Statement,
        AnalyzedQuery,
        Catalog,
        Annotations,
    ) {
        let stmt = Parser::parse_sql(&PostgreSqlDialect {}, sql)
            .unwrap()
            .remove(0);
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
    fn fires_on_fk_without_not_valid() {
        let sql = "ALTER TABLE orders ADD CONSTRAINT orders_user_fk FOREIGN KEY (user_id) REFERENCES users(id);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_constraint_missing_not_valid(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("table").map(|s| s.as_str()),
            Some("orders")
        );
        assert_eq!(
            hits[0].bindings.get("constraint_kind").map(|s| s.as_str()),
            Some("foreign key")
        );
        assert_eq!(
            hits[0].bindings.get("constraint_name").map(|s| s.as_str()),
            Some("orders_user_fk")
        );
    }

    #[test]
    fn no_match_fk_with_not_valid() {
        let sql = "ALTER TABLE orders ADD CONSTRAINT orders_user_fk FOREIGN KEY (user_id) REFERENCES users(id) NOT VALID;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_constraint_missing_not_valid(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_check_without_not_valid() {
        let sql = "ALTER TABLE accounts ADD CONSTRAINT balance_non_negative CHECK (balance >= 0);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_constraint_missing_not_valid(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("constraint_kind").map(|s| s.as_str()),
            Some("check")
        );
    }

    #[test]
    fn no_match_add_primary_key() {
        let sql = "ALTER TABLE users ADD PRIMARY KEY (id);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_constraint_missing_not_valid(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_constraint_missing_not_valid(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
