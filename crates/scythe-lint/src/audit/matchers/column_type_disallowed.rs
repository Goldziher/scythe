//! Matcher `"column_type_disallowed"` — SC-MIG10 prefer-bigint-over-int,
//! SC-MIG11 prefer-text-over-varchar, SC-MIG12 prefer-timestamptz,
//! SC-MIG13 prefer-identity-over-serial.
//!
//! Reads `matcher_args.disallowed` (array of lowercase type strings) and
//! `matcher_args.suggested` (string). Fires when a column's type, after
//! lowercasing, either exactly matches a disallowed entry or starts with a
//! disallowed entry followed by `(` (for parameterised types such as
//! `varchar(n)`).
//!
//! Walks columns in:
//! - `Statement::CreateTable` — iterates `columns`.
//! - `Statement::AlterTable` — iterates `AddColumn` operations.
//!
//! Emits one hit per offending column with bindings `table`, `column`,
//! `actual_type`, and `suggested_type`.
//!
//! Migration-safety motivation: certain column type choices impose hidden
//! risks — 32-bit integer overflow, write-blocking length changes, naive
//! timestamp timezone shifts, and non-standard sequence shorthand. Each
//! SC-MIG10..13 rule passes a `disallowed` list targeting one risk class.

use sqlparser::ast::{AlterTableOperation, Statement};

use crate::audit::registry::MatcherHit;
use crate::types::LintContext;

pub fn match_column_type_disallowed(ctx: &LintContext<'_>, args: &toml::Table) -> Vec<MatcherHit> {
    let disallowed = read_string_list(args, "disallowed");
    if disallowed.is_empty() {
        return Vec::new();
    }
    let suggested = args
        .get("suggested")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    match ctx.stmt {
        Statement::CreateTable(ct) => {
            let table = ct.name.to_string();
            ct.columns
                .iter()
                .filter_map(|col| {
                    let ty = col.data_type.to_string().to_ascii_lowercase();
                    if is_disallowed(&ty, &disallowed) {
                        let mut hit = MatcherHit::empty();
                        hit.bindings.insert("table".to_string(), table.clone());
                        hit.bindings
                            .insert("column".to_string(), col.name.value.clone());
                        hit.bindings.insert("actual_type".to_string(), ty);
                        hit.bindings
                            .insert("suggested_type".to_string(), suggested.clone());
                        Some(hit)
                    } else {
                        None
                    }
                })
                .collect()
        }

        Statement::AlterTable(alter) => {
            let table = alter.name.to_string();
            alter
                .operations
                .iter()
                .filter_map(|op| match op {
                    AlterTableOperation::AddColumn { column_def, .. } => {
                        let ty = column_def.data_type.to_string().to_ascii_lowercase();
                        if is_disallowed(&ty, &disallowed) {
                            let mut hit = MatcherHit::empty();
                            hit.bindings.insert("table".to_string(), table.clone());
                            hit.bindings
                                .insert("column".to_string(), column_def.name.value.clone());
                            hit.bindings.insert("actual_type".to_string(), ty);
                            hit.bindings
                                .insert("suggested_type".to_string(), suggested.clone());
                            Some(hit)
                        } else {
                            None
                        }
                    }
                    _ => None,
                })
                .collect()
        }

        _ => Vec::new(),
    }
}

/// Return `true` if `ty` exactly matches any disallowed entry, or starts with
/// a disallowed entry immediately followed by `(`.
///
/// Exact-match and prefix-before-`(` semantics are used so that `bigint` does
/// not fire when `int` is disallowed (a substring check would false-positive).
fn is_disallowed(ty: &str, disallowed: &[String]) -> bool {
    disallowed
        .iter()
        .any(|d| ty == d.as_str() || ty.starts_with(&format!("{d}(")))
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

    fn make_args(disallowed: &[&str], suggested: &str) -> toml::Table {
        let mut t = toml::Table::new();
        let arr: toml::value::Array = disallowed
            .iter()
            .map(|s| toml::Value::String((*s).to_string()))
            .collect();
        t.insert("disallowed".to_string(), toml::Value::Array(arr));
        t.insert(
            "suggested".to_string(),
            toml::Value::String(suggested.to_string()),
        );
        t
    }

    #[test]
    fn fires_on_create_table_int() {
        let sql = "CREATE TABLE t (id int)";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["int", "integer", "int4", "smallint", "int2"], "bigint");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bindings.get("table").map(|s| s.as_str()), Some("t"));
        assert_eq!(
            hits[0].bindings.get("column").map(|s| s.as_str()),
            Some("id")
        );
        assert_eq!(
            hits[0].bindings.get("actual_type").map(|s| s.as_str()),
            Some("int")
        );
        assert_eq!(
            hits[0].bindings.get("suggested_type").map(|s| s.as_str()),
            Some("bigint")
        );
    }

    #[test]
    fn fires_on_alter_table_add_column_integer() {
        let sql = "ALTER TABLE users ADD COLUMN legacy_id integer";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["int", "integer", "int4", "smallint", "int2"], "bigint");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("table").map(|s| s.as_str()),
            Some("users")
        );
        assert_eq!(
            hits[0].bindings.get("column").map(|s| s.as_str()),
            Some("legacy_id")
        );
    }

    #[test]
    fn no_false_positive_on_bigint() {
        // `int` is disallowed but `bigint` must NOT fire — prefix-match `int`
        // would falsely match `bigint` if we used substring checks.
        let sql = "CREATE TABLE t (id bigint)";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["int", "integer", "int4", "smallint", "int2"], "bigint");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert!(
            hits.is_empty(),
            "bigint must not fire when only int/integer/int4/smallint/int2 are disallowed"
        );
    }

    #[test]
    fn fires_on_varchar_parameterised() {
        let sql = "CREATE TABLE t (name varchar(255))";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["varchar", "character varying", "char"], "text");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("actual_type").map(|s| s.as_str()),
            Some("varchar(255)")
        );
    }

    #[test]
    fn fires_on_character_varying_parameterised() {
        let sql = "CREATE TABLE t (name character varying(100))";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["varchar", "character varying", "char"], "text");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("actual_type").map(|s| s.as_str()),
            Some("character varying(100)")
        );
    }

    #[test]
    fn fires_on_timestamp_without_time_zone() {
        let sql = "CREATE TABLE t (ts timestamp without time zone)";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["timestamp", "timestamp without time zone"], "timestamptz");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn fires_on_bare_timestamp() {
        let sql = "CREATE TABLE t (ts timestamp)";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["timestamp", "timestamp without time zone"], "timestamptz");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("actual_type").map(|s| s.as_str()),
            Some("timestamp")
        );
    }

    #[test]
    fn no_match_on_timestamptz() {
        let sql = "CREATE TABLE t (ts timestamptz)";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["timestamp", "timestamp without time zone"], "timestamptz");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert!(hits.is_empty(), "timestamptz must not fire");
    }

    #[test]
    fn fires_on_serial() {
        let sql = "CREATE TABLE t (id serial)";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["serial", "bigserial", "smallserial"], "identity");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("actual_type").map(|s| s.as_str()),
            Some("serial")
        );
    }

    #[test]
    fn fires_on_bigserial() {
        let sql = "CREATE TABLE t (id bigserial)";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["serial", "bigserial", "smallserial"], "identity");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].bindings.get("actual_type").map(|s| s.as_str()),
            Some("bigserial")
        );
    }

    #[test]
    fn no_match_on_rename_column() {
        let sql = "ALTER TABLE users RENAME COLUMN old_name TO new_name";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["int", "integer"], "bigint");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert!(hits.is_empty());
    }

    #[test]
    fn no_match_on_select() {
        let sql = "SELECT 1";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["int", "integer"], "bigint");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert!(hits.is_empty());
    }

    #[test]
    fn emitted_bindings_for_representative_case() {
        let sql = "CREATE TABLE orders (qty int)";
        let (stmt, analyzed, catalog, annotations) = make_parts(sql);
        let ctx = make_ctx(sql, &stmt, &analyzed, &catalog, &annotations);
        let args = make_args(&["int", "integer", "int4", "smallint", "int2"], "bigint");
        let hits = match_column_type_disallowed(&ctx, &args);
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert_eq!(h.bindings.get("table").map(|s| s.as_str()), Some("orders"));
        assert_eq!(h.bindings.get("column").map(|s| s.as_str()), Some("qty"));
        assert_eq!(
            h.bindings.get("actual_type").map(|s| s.as_str()),
            Some("int")
        );
        assert_eq!(
            h.bindings.get("suggested_type").map(|s| s.as_str()),
            Some("bigint")
        );
    }
}
