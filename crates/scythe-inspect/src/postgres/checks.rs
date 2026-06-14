//! Postgres catalog checks — Phase 0 set.
//!
//! Three checks ship at Phase 0; each is a single query against `pg_catalog`
//! that returns one row per offending object. The query strings are exposed
//! as `pub(crate) const SQL_*: &str` so unit tests can parser-validate them
//! without a live connection.
//!
//! Detection patterns are clean-room reimplementations of the equivalent
//! supabase/splinter lints (see `ATTRIBUTIONS.md`):
//!
//! - `SC-INS01` <- splinter `0001 unindexed_foreign_keys`
//! - `SC-INS02` <- splinter `0006 policy_exists_rls_disabled`
//! - `SC-INS03` <- splinter `0009 duplicate_index`

use scythe_lint::reporters::Finding;
use scythe_lint::types::Severity;
use tokio_postgres::Client;

use crate::error::InspectError;

// ---------------------------------------------------------------------------
// SQL bodies — kept as plain `const` so they're easy to read and so a unit
// test can pass each one through `sqlparser` to catch syntax regressions on
// every PR.
// ---------------------------------------------------------------------------

/// SC-INS01 — foreign-key columns with no covering index.
///
/// For every `pg_constraint` of type `f`, group the FK column set and check
/// that no index whose leading columns match the FK column order exists. A
/// covering index is one whose `indkey` prefix equals the constraint's
/// `conkey` set, in order.
pub(crate) const SQL_MISSING_FK_INDEX: &str = "
SELECT n.nspname            AS schema_name,
       cl.relname           AS table_name,
       c.conname            AS constraint_name,
       array_agg(att.attname ORDER BY u.ord) AS columns
FROM pg_constraint c
JOIN pg_class cl    ON cl.oid = c.conrelid
JOIN pg_namespace n ON n.oid  = cl.relnamespace
JOIN unnest(c.conkey) WITH ORDINALITY AS u(attnum, ord) ON TRUE
JOIN pg_attribute att
       ON att.attrelid = c.conrelid
      AND att.attnum   = u.attnum
WHERE c.contype = 'f'
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indrelid = c.conrelid
          AND (i.indkey::int2[])[0:array_length(c.conkey, 1) - 1] = c.conkey
      )
GROUP BY n.nspname, cl.relname, c.conname
ORDER BY n.nspname, cl.relname, c.conname
";

/// SC-INS02 — tables with `pg_policy` rows but `relrowsecurity = false`.
///
/// `CREATE POLICY` succeeds even when row-level security is disabled, so this
/// is a common silent misconfiguration: the policies exist but never apply.
pub(crate) const SQL_POLICY_RLS_DISABLED: &str = "
SELECT n.nspname           AS schema_name,
       c.relname           AS table_name,
       count(p.polname)::bigint AS policy_count
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_policy p    ON p.polrelid = c.oid
WHERE c.relrowsecurity = false
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
GROUP BY n.nspname, c.relname
ORDER BY n.nspname, c.relname
";

/// SC-INS03 — indexes with identical definitions modulo name.
///
/// Two `CREATE INDEX` statements that differ only in the index name produce
/// the same `indexdef` after normalising the `INDEX <name> ON` prefix;
/// grouping by the normalised form catches accidental duplicates.
pub(crate) const SQL_DUPLICATE_INDEX: &str = "
SELECT schemaname AS schema_name,
       tablename  AS table_name,
       array_agg(indexname ORDER BY indexname) AS duplicate_indexes
FROM pg_indexes
WHERE schemaname NOT IN ('pg_catalog', 'information_schema')
GROUP BY schemaname, tablename,
         regexp_replace(indexdef, ' INDEX [^ ]+ ON ', ' INDEX <name> ON ')
HAVING count(*) > 1
ORDER BY schemaname, tablename
";

// ---------------------------------------------------------------------------
// Static catalog metadata — used by `--list-checks` and by `make_finding`.
// ---------------------------------------------------------------------------

pub(crate) const SC_INS01_DESC: &str = "foreign-key columns without a covering index — every join through the constraint forces a sequential scan";
pub(crate) const SC_INS02_DESC: &str =
    "table has CREATE POLICY definitions but ROW LEVEL SECURITY is disabled — policies never apply";
pub(crate) const SC_INS03_DESC: &str = "two or more indexes on the same table have identical definitions modulo name — wasted writes and storage";

fn make_finding(id: &str, name: &str, severity: Severity, message: String) -> Finding {
    Finding {
        file: String::new(),
        query_name: None,
        rule_id: id.to_string(),
        rule_name: Some(name.to_string()),
        rule_description: Some(
            match id {
                "SC-INS01" => SC_INS01_DESC,
                "SC-INS02" => SC_INS02_DESC,
                "SC-INS03" => SC_INS03_DESC,
                _ => "",
            }
            .to_string(),
        ),
        severity,
        message,
        line: None,
        column: None,
        cwe: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Check functions
// ---------------------------------------------------------------------------

/// SC-INS01 — emit one finding per FK constraint without a covering index.
pub async fn check_missing_fk_index(client: &Client) -> Result<Vec<Finding>, InspectError> {
    let rows = client
        .query(SQL_MISSING_FK_INDEX, &[])
        .await
        .map_err(|e| InspectError::Query {
            engine: "postgres",
            check_id: "SC-INS01",
            source: Box::new(e),
        })?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let schema: &str = row.get("schema_name");
            let table: &str = row.get("table_name");
            let constraint: &str = row.get("constraint_name");
            let columns: Vec<String> = row.get("columns");
            let message = format!(
                "foreign-key `{schema}.{table}.{constraint}` on columns ({}) has no covering index — add `CREATE INDEX CONCURRENTLY` on the same column set",
                columns.join(", ")
            );
            make_finding("SC-INS01", "missing-fk-index", Severity::Warn, message)
        })
        .collect())
}

/// SC-INS02 — emit one finding per table with policies and RLS off.
pub async fn check_policy_rls_disabled(client: &Client) -> Result<Vec<Finding>, InspectError> {
    let rows = client
        .query(SQL_POLICY_RLS_DISABLED, &[])
        .await
        .map_err(|e| InspectError::Query {
            engine: "postgres",
            check_id: "SC-INS02",
            source: Box::new(e),
        })?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let schema: &str = row.get("schema_name");
            let table: &str = row.get("table_name");
            let count: i64 = row.get("policy_count");
            let message = format!(
                "`{schema}.{table}` has {count} CREATE POLICY definition(s) but ROW LEVEL SECURITY is disabled — run `ALTER TABLE {schema}.{table} ENABLE ROW LEVEL SECURITY` or drop the policies"
            );
            make_finding("SC-INS02", "policy-exists-rls-disabled", Severity::Error, message)
        })
        .collect())
}

/// SC-INS03 — emit one finding per table with duplicate index definitions.
pub async fn check_duplicate_index(client: &Client) -> Result<Vec<Finding>, InspectError> {
    let rows = client
        .query(SQL_DUPLICATE_INDEX, &[])
        .await
        .map_err(|e| InspectError::Query {
            engine: "postgres",
            check_id: "SC-INS03",
            source: Box::new(e),
        })?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let schema: &str = row.get("schema_name");
            let table: &str = row.get("table_name");
            let indexes: Vec<String> = row.get("duplicate_indexes");
            let message = format!(
                "`{schema}.{table}` has duplicate indexes with identical definitions: {} — drop all but one",
                indexes.join(", ")
            );
            make_finding("SC-INS03", "duplicate-index", Severity::Warn, message)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;

    use super::*;

    /// All three check queries must parse under the Postgres dialect — this
    /// catches typos that would otherwise only surface against a live DB.
    #[test]
    fn sql_missing_fk_index_parses() {
        Parser::parse_sql(&PostgreSqlDialect {}, SQL_MISSING_FK_INDEX)
            .expect("SC-INS01 query parses");
    }

    #[test]
    fn sql_policy_rls_disabled_parses() {
        Parser::parse_sql(&PostgreSqlDialect {}, SQL_POLICY_RLS_DISABLED)
            .expect("SC-INS02 query parses");
    }

    #[test]
    fn sql_duplicate_index_parses() {
        Parser::parse_sql(&PostgreSqlDialect {}, SQL_DUPLICATE_INDEX)
            .expect("SC-INS03 query parses");
    }
}
