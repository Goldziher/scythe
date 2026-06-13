//! Matcher `"create_index_concurrency"` — SC-MIG03
//! require-concurrent-index-creation.
//!
//! Takes no `matcher_args`. Fires on `CREATE INDEX` statements that omit the
//! `CONCURRENTLY` modifier. Emits a hit with bindings `index` (or `<unnamed>`)
//! and `table`.
//!
//! Migration-safety motivation: non-concurrent index creation takes an
//! `ACCESS EXCLUSIVE` lock on the table for the duration of the build,
//! blocking reads and writes. Always use `CREATE INDEX CONCURRENTLY` in
//! production migrations.

use sqlparser::ast::Statement;

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_create_index_concurrency(
    ctx: &LintContext<'_>,
    _args: &toml::Table,
) -> Vec<MatcherHit> {
    let Statement::CreateIndex(idx) = ctx.stmt else {
        return Vec::new();
    };
    if idx.concurrently {
        return Vec::new();
    }
    let mut hit = MatcherHit::empty();
    let index_name = idx
        .name
        .as_ref()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "<unnamed>".to_string());
    hit.bindings.insert("index".to_string(), index_name);
    hit.bindings
        .insert("table".to_string(), idx.table_name.to_string());
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
    fn fires_on_non_concurrent_create_index() {
        let sql = "CREATE INDEX idx_users_email ON users(email);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_create_index_concurrency(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("index").map(|s| s.as_str()),
            Some("idx_users_email")
        );
        assert_eq!(
            hits[0].bindings.get("table").map(|s| s.as_str()),
            Some("users")
        );
    }

    #[test]
    fn no_match_concurrent_create_index() {
        let sql = "CREATE INDEX CONCURRENTLY idx_users_email ON users(email);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_create_index_concurrency(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_create_index_concurrency(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn unnamed_index_uses_placeholder_binding() {
        // CREATE INDEX without an explicit name is valid in some dialects;
        // we surface a placeholder rather than crashing.
        let sql = "CREATE INDEX ON users(email);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_create_index_concurrency(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("index").map(|s| s.as_str()),
            Some("<unnamed>")
        );
    }
}
