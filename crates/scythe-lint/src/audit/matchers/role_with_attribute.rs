//! Matcher `"role_with_attribute"` — SC-SEC04 superuser-role.
//!
//! Reads `matcher_args.attributes` (array of strings, case-insensitive).
//! Recognized values: `superuser`, `createdb`, `createrole`, `replication`,
//! `bypassrls`. Fires once per matched attribute against:
//! - `CREATE ROLE … <attribute>` (or `<attribute>` set as a direct field)
//! - `ALTER ROLE … WITH <attribute>`
//!
//! Emits a `MatcherHit` per matched attribute with bindings `role` (first name
//! in `names`) and `attribute` (the matched attribute name, lowercased).
//!
//! For backwards compatibility, a scalar `attribute = "..."` is also accepted
//! and treated as a single-element list.
//!
//! CWE-269 (Improper Privilege Management).

use sqlparser::ast::{AlterRoleOperation, RoleOption, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

const RECOGNIZED_ATTRIBUTES: &[&str] = &["superuser", "createdb", "createrole", "replication", "bypassrls"];

pub fn match_role_with_attribute(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let wanted = read_wanted_attributes(args);
    if wanted.is_empty() {
        return Vec::new();
    }

    match ctx.stmt {
        Statement::CreateRole(cr) => {
            let role = cr
                .names
                .first()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "<unknown>".to_string());
            wanted
                .iter()
                .filter(|attr| attribute_set_on_create_role(attr, cr))
                .map(|attr| make_hit(role.clone(), attr))
                .collect()
        }
        Statement::AlterRole {
            name,
            operation: AlterRoleOperation::WithOptions { options },
        } => {
            let role = name.to_string();
            wanted
                .iter()
                .filter(|attr| attribute_set_in_options(attr, options))
                .map(|attr| make_hit(role.clone(), attr))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn read_wanted_attributes(args: &toml::Table) -> Vec<String> {
    let normalize = |s: &str| {
        let lower = s.to_ascii_lowercase();
        if RECOGNIZED_ATTRIBUTES.contains(&lower.as_str()) {
            Some(lower)
        } else {
            None
        }
    };
    if let Some(arr) = args.get("attributes").and_then(|v| v.as_array()) {
        return arr.iter().filter_map(|v| v.as_str().and_then(normalize)).collect();
    }
    if let Some(s) = args.get("attribute").and_then(|v| v.as_str()) {
        return normalize(s).into_iter().collect();
    }
    Vec::new()
}

fn attribute_set_on_create_role(attribute: &str, cr: &sqlparser::ast::CreateRole) -> bool {
    match attribute {
        "superuser" => cr.superuser == Some(true),
        "createdb" => cr.create_db == Some(true),
        "createrole" => cr.create_role == Some(true),
        "replication" => cr.replication == Some(true),
        "bypassrls" => cr.bypassrls == Some(true),
        _ => false,
    }
}

fn attribute_set_in_options(attribute: &str, options: &[RoleOption]) -> bool {
    options.iter().any(|opt| {
        matches!(
            (attribute, opt),
            ("superuser", RoleOption::SuperUser(true))
                | ("createdb", RoleOption::CreateDB(true))
                | ("createrole", RoleOption::CreateRole(true))
                | ("replication", RoleOption::Replication(true))
                | ("bypassrls", RoleOption::BypassRLS(true))
        )
    })
}

fn make_hit(role: String, attribute: &str) -> MatcherHit {
    let mut hit = MatcherHit::empty();
    hit.bindings.insert("role".to_string(), role);
    hit.bindings.insert("attribute".to_string(), attribute.to_string());
    hit
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

    fn make_args(attribute: &str) -> toml::Table {
        let mut t = toml::Table::new();
        t.insert("attribute".to_string(), toml::Value::String(attribute.to_string()));
        t
    }

    fn make_args_list(attributes: &[&str]) -> toml::Table {
        let mut t = toml::Table::new();
        let arr: toml::value::Array = attributes
            .iter()
            .map(|s| toml::Value::String((*s).to_string()))
            .collect();
        t.insert("attributes".to_string(), toml::Value::Array(arr));
        t
    }

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
    fn fires_on_create_role_superuser() {
        let sql = "CREATE ROLE dba SUPERUSER";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_role_with_attribute(&ctx, &make_args("superuser"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("role").map(|s| s.as_str()), Some("dba"));
        assert_eq!(hits[0].bindings.get("attribute").map(|s| s.as_str()), Some("superuser"));
    }

    #[test]
    fn fires_on_alter_role_superuser() {
        let sql = "ALTER ROLE dba WITH SUPERUSER";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_role_with_attribute(&ctx, &make_args("superuser"));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("role").map(|s| s.as_str()), Some("dba"));
    }

    #[test]
    fn no_match_nosuperuser() {
        let sql = "CREATE ROLE readonly NOSUPERUSER LOGIN";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_role_with_attribute(&ctx, &make_args("superuser"));
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_once_per_matched_attribute_in_create_role() {
        let sql = "CREATE ROLE chaos SUPERUSER CREATEROLE BYPASSRLS";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args_list(&["superuser", "createdb", "createrole", "replication", "bypassrls"]);
        let hits = match_role_with_attribute(&ctx, &args);
        let mut attrs: Vec<&str> = hits
            .iter()
            .map(|h| h.bindings.get("attribute").unwrap().as_str())
            .collect();
        attrs.sort();
        assert_eq!(attrs, vec!["bypassrls", "createrole", "superuser"]);
    }

    #[test]
    fn fires_on_alter_role_for_listed_attribute() {
        let sql = "ALTER ROLE chaos WITH CREATEROLE";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args_list(&["superuser", "createrole"]);
        let hits = match_role_with_attribute(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("attribute").map(|s| s.as_str()),
            Some("createrole")
        );
    }

    #[test]
    fn unknown_attribute_is_silently_dropped() {
        let sql = "CREATE ROLE x SUPERUSER";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args_list(&["telekinesis", "superuser"]);
        let hits = match_role_with_attribute(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("attribute").map(|s| s.as_str()), Some("superuser"));
    }
}
