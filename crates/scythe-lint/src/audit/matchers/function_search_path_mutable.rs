//! Matcher `"function_search_path_mutable"` — SC-SEC12
//! function-search-path-mutable.
//!
//! Takes no `matcher_args`. Fires on `CREATE FUNCTION` statements that omit a
//! `SET search_path = …` option **and** are NOT `SECURITY DEFINER`. The
//! SECURITY DEFINER case is owned by SC-SEC10 (escalating it to `error`
//! because a hijacked search path under DEFINER privileges is a direct
//! privilege-escalation primitive); SC-SEC12 covers the remaining functions
//! at `warn` severity for general hygiene. Emits a hit with binding
//! `function_name`.
//!
//! Detection inspired by supabase/splinter lint `0011_function_search_path_mutable`;
//! see `ATTRIBUTIONS.md`. Splinter inspects `pg_proc.proconfig` at runtime;
//! scythe inspects the typed `CreateFunction.set_params` list at lint time.

use sqlparser::ast::{FunctionSecurity, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_function_search_path_mutable(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    let Statement::CreateFunction(cf) = ctx.stmt else {
        return Vec::new();
    };
    if matches!(cf.security, Some(FunctionSecurity::Definer)) {
        return Vec::new();
    }
    let has_search_path = cf.set_params.iter().any(|p| {
        p.name
            .0
            .last()
            .and_then(|seg| seg.as_ident().map(|i| i.value.to_ascii_lowercase()))
            .as_deref()
            == Some("search_path")
    });
    if has_search_path {
        return Vec::new();
    }
    vec![MatcherHit::with_binding("function_name", cf.name.to_string())]
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
    fn fires_on_invoker_function_without_search_path() {
        let sql = "CREATE FUNCTION public.add_one(i int) RETURNS int LANGUAGE sql AS $$ SELECT i + 1 $$;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_function_search_path_mutable(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("function_name").map(|s| s.as_str()),
            Some("public.add_one")
        );
    }

    #[test]
    fn no_match_invoker_function_with_search_path() {
        let sql = "CREATE FUNCTION public.add_one(i int) RETURNS int LANGUAGE sql SET search_path = pg_catalog, public AS $$ SELECT i + 1 $$;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_function_search_path_mutable(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_security_definer_function_without_search_path() {
        // SECURITY DEFINER is owned by SC-SEC10 — SC-SEC12 must NOT fire here
        // to avoid double-counting findings on the same statement.
        let sql = "CREATE FUNCTION admin_op() RETURNS void LANGUAGE sql SECURITY DEFINER AS $$ SELECT 1 $$;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_function_search_path_mutable(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_security_definer_with_search_path() {
        let sql = "CREATE FUNCTION admin_op() RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path = '' AS $$ SELECT 1 $$;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_function_search_path_mutable(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_function_search_path_mutable(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
