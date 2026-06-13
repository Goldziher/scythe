//! Matcher `"session_mutation"` — SC-SEC11 session-mutation.
//!
//! Reads `matcher_args.kinds` (array of strings).  Fires when application SQL
//! contains a statement that mutates the security context. Recognized kinds:
//!
//! - `set_role` — `SET [SESSION|LOCAL] ROLE <name>` (or `SET ROLE NONE`).
//! - `set_session_authorization` — `SET SESSION AUTHORIZATION <name | DEFAULT>`.
//! - `reset_role` — `RESET ROLE` (returns to the session role).
//!
//! Note: sqlparser 0.62 does not parse `RESET SESSION AUTHORIZATION` — it
//! errors at parse time, so that variant cannot be detected here.
//!
//! Emits `MatcherHit` with bindings:
//! - `op`     — `"SET ROLE"`, `"SET SESSION AUTHORIZATION"`, or `"RESET ROLE"`
//! - `target` — the role or user specified (or `"NONE"` for `SET ROLE NONE`
//!   and `"to default"` for `RESET ROLE`, so the rendered sentence reads
//!   naturally)
//!
//! CWE-269 (Improper Privilege Management).

use sqlparser::ast::{Reset, Set, SetSessionAuthorizationParamKind, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_session_mutation(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let kinds = read_string_list(args, "kinds");
    if kinds.is_empty() {
        return Vec::new();
    }

    match ctx.stmt {
        Statement::Set(set) => match_set(set, &kinds),
        Statement::Reset(reset) if kinds.iter().any(|k| k == "reset_role") => {
            match_reset_role(&reset.reset)
        }
        _ => Vec::new(),
    }
}

fn match_set(set: &Set, kinds: &[String]) -> Vec<MatcherHit> {
    match set {
        Set::SetRole { role_name, .. } if kinds.iter().any(|k| k == "set_role") => {
            let target = role_name
                .as_ref()
                .map(|i| i.value.clone())
                .unwrap_or_else(|| "NONE".to_string());
            vec![make_hit("SET ROLE", target)]
        }
        Set::SetSessionAuthorization(param)
            if kinds.iter().any(|k| k == "set_session_authorization") =>
        {
            let target = match &param.kind {
                SetSessionAuthorizationParamKind::Default => "DEFAULT".to_string(),
                SetSessionAuthorizationParamKind::User(name) => name.to_string(),
            };
            vec![make_hit("SET SESSION AUTHORIZATION", target)]
        }
        _ => Vec::new(),
    }
}

fn match_reset_role(reset: &Reset) -> Vec<MatcherHit> {
    let Reset::ConfigurationParameter(name) = reset else {
        return Vec::new();
    };
    let parameter = name
        .0
        .last()
        .and_then(|p| p.as_ident().map(|i| i.value.as_str()))
        .unwrap_or_default();
    if parameter.eq_ignore_ascii_case("role") {
        vec![make_hit("RESET ROLE", "to default".to_string())]
    } else {
        Vec::new()
    }
}

fn make_hit(op: &str, target: String) -> MatcherHit {
    let mut hit = MatcherHit::empty();
    hit.bindings.insert("op".to_string(), op.to_string());
    hit.bindings.insert("target".to_string(), target);
    hit
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

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::analyzer::AnalyzedQuery;
    use scythe_core::catalog::Catalog;
    use scythe_core::dialect::SqlDialect;
    use scythe_core::parser::{Annotations, QueryCommand};
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;

    fn make_args(kinds: &[&str]) -> toml::Table {
        let mut t = toml::Table::new();
        let arr: toml::value::Array = kinds
            .iter()
            .map(|s| toml::Value::String((*s).to_string()))
            .collect();
        t.insert("kinds".to_string(), toml::Value::Array(arr));
        t
    }

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
    fn fires_on_set_role() {
        let sql = "SET ROLE admin";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["set_role", "set_session_authorization"]);
        let hits = match_session_mutation(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("op").map(|s| s.as_str()),
            Some("SET ROLE")
        );
        assert_eq!(
            hits[0].bindings.get("target").map(|s| s.as_str()),
            Some("admin")
        );
    }

    #[test]
    fn fires_on_set_session_authorization() {
        let sql = "SET SESSION AUTHORIZATION alice";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["set_role", "set_session_authorization"]);
        let hits = match_session_mutation(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("op").map(|s| s.as_str()),
            Some("SET SESSION AUTHORIZATION")
        );
        assert_eq!(
            hits[0].bindings.get("target").map(|s| s.as_str()),
            Some("alice")
        );
    }

    #[test]
    fn no_match_plain_set() {
        let sql = "SET search_path = public";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["set_role", "set_session_authorization"]);
        let hits = match_session_mutation(&ctx, &args);
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_reset_role_when_kind_enabled() {
        let sql = "RESET ROLE";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["set_role", "set_session_authorization", "reset_role"]);
        let hits = match_session_mutation(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("op").map(|s| s.as_str()),
            Some("RESET ROLE")
        );
    }

    #[test]
    fn reset_role_silent_when_kind_omitted() {
        let sql = "RESET ROLE";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["set_role", "set_session_authorization"]);
        let hits = match_session_mutation(&ctx, &args);
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_reset_other_parameter() {
        let sql = "RESET search_path";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["set_role", "set_session_authorization", "reset_role"]);
        let hits = match_session_mutation(&ctx, &args);
        assert!(hits.is_empty());
    }
}
