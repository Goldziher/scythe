//! Matcher `"create_domain_with_constraint"` — SC-MIG16
//! ban-create-domain-with-constraint.
//!
//! Takes no `matcher_args`. Matches `CREATE DOMAIN … CHECK (…)` (any
//! constraint on the domain). Emits one hit per offending statement.
//!
//! Migration-safety motivation: adding a `CHECK` constraint to a domain
//! causes Postgres to validate every existing row in every table that uses
//! the domain, holding `ACCESS EXCLUSIVE` on each one for the duration of
//! the scan. The safe pattern is to add the constraint on the column itself
//! with `NOT VALID` and `VALIDATE CONSTRAINT` as separate steps, where the
//! lock can be released between validation work.
//!
//! Bindings: `domain` (qualified name of the domain).

use sqlparser::ast::Statement;

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_create_domain_with_constraint(
    ctx: &LintContext<'_>,
    _args: &toml::Table,
) -> Vec<MatcherHit> {
    let Statement::CreateDomain(domain) = ctx.stmt else {
        return Vec::new();
    };
    if domain.constraints.is_empty() {
        return Vec::new();
    }
    let mut hit = MatcherHit::empty();
    hit.bindings
        .insert("domain".to_string(), domain.name.to_string());
    vec![hit]
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
    fn fires_on_create_domain_with_check() {
        let sql = "CREATE DOMAIN positive_int AS integer CHECK (VALUE > 0);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_create_domain_with_constraint(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("domain").map(|s| s.as_str()),
            Some("positive_int")
        );
    }

    #[test]
    fn no_match_create_domain_without_check() {
        let sql = "CREATE DOMAIN positive_int AS integer;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_create_domain_with_constraint(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_create_table_with_check() {
        let sql = "CREATE TABLE foo (id integer CHECK (id > 0));";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_create_domain_with_constraint(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_create_domain_with_constraint(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
