//! Matcher `"security_definer_no_search_path"` — SC-SEC10.
//!
//! No `matcher_args`.  Fires when a `CREATE FUNCTION` statement has
//! `SECURITY DEFINER` without a `SET search_path` option.
//! Emits a `MatcherHit` with binding `function_name -> <name>`.
//!
//! Ported 1:1 from `rules/security/security_definer.rs`.

use sqlparser::ast::{FunctionSecurity, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_security_definer_no_search_path(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    if let Statement::CreateFunction(cf) = ctx.stmt
        && matches!(cf.security, Some(FunctionSecurity::Definer))
    {
        let has_search_path = cf.set_params.iter().any(|p| {
            p.name
                .0
                .last()
                .and_then(|seg| seg.as_ident().map(|i| i.value.to_ascii_lowercase()))
                .as_deref()
                == Some("search_path")
        });
        if !has_search_path {
            let name = cf.name.to_string();
            return vec![MatcherHit::with_binding("function_name", name)];
        }
    }
    Vec::new()
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
    fn fires_when_no_search_path() {
        let sql = "CREATE FUNCTION danger() RETURNS void LANGUAGE plpgsql SECURITY DEFINER AS $$ BEGIN END $$";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_security_definer_no_search_path(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("function_name").map(|s| s.as_str()),
            Some("danger")
        );
    }

    #[test]
    fn no_match_without_security_definer() {
        let sql = "CREATE FUNCTION safe_fn() RETURNS void LANGUAGE plpgsql AS $$ BEGIN END $$";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_security_definer_no_search_path(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
