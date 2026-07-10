//! Matcher `"add_constraint_without_using_index"` — drives SC-MIG14
//! (disallowed-unique-constraint) and SC-MIG15
//! (adding-primary-key-without-using-index).
//!
//! `matcher_args.kinds` selects which constraint kinds to flag — accepted
//! values: `"unique"`, `"primary_key"`. The matcher walks
//! `ALTER TABLE … ADD CONSTRAINT …` and fires when a plain `UNIQUE` or
//! `PRIMARY KEY` constraint is added without the `USING INDEX` clause.
//!
//! Migration-safety motivation: `ALTER TABLE … ADD CONSTRAINT … UNIQUE (…)` /
//! `… PRIMARY KEY (…)` builds the backing index inline under an `ACCESS
//! EXCLUSIVE` lock, blocking reads and writes for the duration. The safe
//! pattern is `CREATE [UNIQUE] INDEX CONCURRENTLY …;` followed by
//! `ALTER TABLE … ADD CONSTRAINT … {UNIQUE|PRIMARY KEY} USING INDEX …;`,
//! which only takes the lock long enough to attach the pre-built index.
//!
//! Bindings: `table`, `constraint_kind` (`"unique"` or `"primary key"`), and
//! optional `constraint_name` when the constraint is explicitly named.

use sqlparser::ast::{AlterTableOperation, Statement, TableConstraint};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

fn read_kinds(args: &toml::Table) -> Vec<String> {
    args.get("kinds")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub fn match_add_constraint_without_using_index(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let Statement::AlterTable(alter) = ctx.stmt else {
        return Vec::new();
    };
    let kinds = read_kinds(args);
    let want_unique = kinds.iter().any(|k| k == "unique");
    let want_pk = kinds.iter().any(|k| k == "primary_key");
    if !want_unique && !want_pk {
        return Vec::new();
    }

    let table = alter.name.to_string();
    alter
        .operations
        .iter()
        .filter_map(|op| match op {
            AlterTableOperation::AddConstraint { constraint, .. } => match constraint {
                TableConstraint::Unique(u) if want_unique => Some(("unique", u.name.as_ref().map(|i| i.value.clone()))),
                TableConstraint::PrimaryKey(p) if want_pk => {
                    Some(("primary key", p.name.as_ref().map(|i| i.value.clone())))
                }
                _ => None,
            },
            _ => None,
        })
        .map(|(kind, name)| {
            let mut hit = MatcherHit::empty();
            hit.bindings.insert("table".to_string(), table.clone());
            hit.bindings.insert("constraint_kind".to_string(), kind.to_string());
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

    fn args_kinds(kinds: &[&str]) -> toml::Table {
        let arr: toml::Value = kinds
            .iter()
            .map(|k| toml::Value::String((*k).to_string()))
            .collect::<Vec<_>>()
            .into();
        let mut t = toml::Table::new();
        t.insert("kinds".to_string(), arr);
        t
    }

    #[test]
    fn fires_on_unique_without_using_index() {
        let sql = "ALTER TABLE users ADD CONSTRAINT users_email_uniq UNIQUE (email);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_constraint_without_using_index(&ctx, &args_kinds(&["unique"]));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("table").map(|s| s.as_str()), Some("users"));
        assert_eq!(
            hits[0].bindings.get("constraint_kind").map(|s| s.as_str()),
            Some("unique")
        );
        assert_eq!(
            hits[0].bindings.get("constraint_name").map(|s| s.as_str()),
            Some("users_email_uniq")
        );
    }

    #[test]
    fn no_match_unique_using_index() {
        let sql = "ALTER TABLE users ADD CONSTRAINT users_email_uniq UNIQUE USING INDEX users_email_idx;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_constraint_without_using_index(&ctx, &args_kinds(&["unique"]));
        assert!(hits.is_empty());
    }

    #[test]
    fn fires_on_primary_key_without_using_index() {
        let sql = "ALTER TABLE accounts ADD CONSTRAINT accounts_pk PRIMARY KEY (id);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_constraint_without_using_index(&ctx, &args_kinds(&["primary_key"]));
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("constraint_kind").map(|s| s.as_str()),
            Some("primary key")
        );
    }

    #[test]
    fn no_match_primary_key_using_index() {
        let sql = "ALTER TABLE accounts ADD CONSTRAINT accounts_pk PRIMARY KEY USING INDEX accounts_pk_idx;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_constraint_without_using_index(&ctx, &args_kinds(&["primary_key"]));
        assert!(hits.is_empty());
    }

    #[test]
    fn kinds_filter_isolates_rules() {
        let sql = "ALTER TABLE accounts ADD CONSTRAINT accounts_pk PRIMARY KEY (id);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_constraint_without_using_index(&ctx, &args_kinds(&["unique"]));
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_check_constraint() {
        let sql = "ALTER TABLE accounts ADD CONSTRAINT balance_nn CHECK (balance >= 0);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_constraint_without_using_index(&ctx, &args_kinds(&["unique", "primary_key"]));
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_unrelated_statement() {
        let sql = "SELECT 1;";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_constraint_without_using_index(&ctx, &args_kinds(&["unique"]));
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_when_kinds_empty() {
        let sql = "ALTER TABLE users ADD CONSTRAINT users_email_uniq UNIQUE (email);";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let hits = match_add_constraint_without_using_index(&ctx, &toml::Table::new());
        assert!(hits.is_empty());
    }
}
