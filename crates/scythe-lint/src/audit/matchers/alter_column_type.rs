//! Matcher `"alter_column_type"` — SC-MIG09 ban-alter-column-type.
//!
//! Takes no `matcher_args`. Walks `ALTER TABLE … ALTER COLUMN … TYPE …`
//! (also `SET DATA TYPE`) and fires once per altered column. Bindings:
//! `table`, `column`, `target_type`.
//!
//! Migration-safety motivation: changing a column's type rewrites the
//! whole table on Postgres ≤ 12 and still takes `ACCESS EXCLUSIVE` on
//! ≥ 13 in most cases — blocking reads and writes for the duration.
//! Prefer add-new-column → backfill → swap → drop-old.

use sqlparser::ast::{AlterColumnOperation, AlterTableOperation, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_alter_column_type(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let Statement::AlterTable(alter) = ctx.stmt else {
        return Vec::new();
    };
    let table = alter.name.to_string();
    alter
        .operations
        .iter()
        .filter_map(|op| match op {
            AlterTableOperation::AlterColumn {
                column_name,
                op: AlterColumnOperation::SetDataType { data_type, .. },
            } => {
                let mut hit = MatcherHit::empty();
                hit.bindings.insert("table".to_string(), table.clone());
                hit.bindings.insert("column".to_string(), column_name.value.clone());
                hit.bindings.insert("target_type".to_string(), data_type.to_string());
                Some(hit)
            }
            _ => None,
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
    fn fires_on_alter_column_type() {
        let sql = "ALTER TABLE users ALTER COLUMN id TYPE bigint;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_type(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("table").map(|s| s.as_str()), Some("users"));
        assert_eq!(hits[0].bindings.get("column").map(|s| s.as_str()), Some("id"));
        assert_eq!(
            hits[0].bindings.get("target_type").map(|s| s.to_ascii_lowercase()),
            Some("bigint".to_string())
        );
    }

    #[test]
    fn fires_on_set_data_type_form() {
        let sql = "ALTER TABLE users ALTER COLUMN name SET DATA TYPE varchar(255);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_type(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("column").map(|s| s.as_str()), Some("name"));
    }

    #[test]
    fn no_match_set_not_null() {
        let sql = "ALTER TABLE users ALTER COLUMN email SET NOT NULL;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_type(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_set_default() {
        let sql = "ALTER TABLE users ALTER COLUMN active SET DEFAULT true;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_type(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_rename_column() {
        let sql = "ALTER TABLE users RENAME COLUMN nick TO username;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_type(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_alter_column_type(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
