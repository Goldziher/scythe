//! Matcher `"role_password_literal"` — SC-SEC05 literal-password.
//!
//! No `matcher_args`.  Fires when:
//! - `CREATE ROLE … PASSWORD '<literal>'`
//! - `ALTER ROLE … PASSWORD '<literal>'`
//!
//! `PASSWORD NULL` (`NullPassword`) is intentionally excluded — that is a
//! deliberate disable of password authentication, not a hard-coded credential.
//!
//! Emits a `MatcherHit` with binding `role` = the role name.
//!
//! CWE-798 (Use of Hard-coded Credentials).

use sqlparser::ast::{
    AlterRoleOperation, Expr, Password, RoleOption, Statement, Value, ValueWithSpan,
};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_role_password_literal(ctx: &LintContext<'_>, _args: &toml::Table) -> Vec<MatcherHit> {
    match ctx.stmt {
        Statement::CreateRole(cr) => {
            if let Some(Password::Password(expr)) = &cr.password
                && is_string_literal(expr)
            {
                let role = cr
                    .names
                    .first()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string());
                return vec![MatcherHit::with_binding("role", role)];
            }
        }
        Statement::AlterRole {
            name,
            operation: AlterRoleOperation::WithOptions { options },
        } => {
            for opt in options {
                if let RoleOption::Password(Password::Password(expr)) = opt
                    && is_string_literal(expr)
                {
                    let role = name.to_string();
                    return vec![MatcherHit::with_binding("role", role)];
                }
            }
        }
        _ => {}
    }
    Vec::new()
}

fn is_string_literal(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Value(ValueWithSpan {
            value: Value::SingleQuotedString(_) | Value::DoubleQuotedString(_),
            ..
        })
    )
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
    fn fires_on_create_role_with_literal_password() {
        let sql = "CREATE ROLE appuser PASSWORD 'hunter2'";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_role_password_literal(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("role").map(|s| s.as_str()),
            Some("appuser")
        );
    }

    #[test]
    fn no_match_password_null() {
        let sql = "CREATE ROLE appuser PASSWORD NULL";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_role_password_literal(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_alter_role_with_literal_password() {
        let sql = "ALTER ROLE appuser WITH PASSWORD 'secret123'";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_role_password_literal(&ctx, &toml::Table::new());
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("role").map(|s| s.as_str()),
            Some("appuser")
        );
    }
}
