//! The live-driver contract: the three operations [`crate::runner`] needs
//! from a database, and nothing else.
//!
//! This module pulls in no driver dependency itself, which is what lets
//! `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`
//! compile the trait without linking a database client. The
//! implementations live in [`crate::executors`], each behind its own
//! feature (`pg`, `mysql`, `mariadb`, `sqlite`, `mssql`, `oracle`) plus the
//! `live-tests` gate for actually dialing a database. Selecting an engine
//! whose feature is off is a hard error in [`crate::runner`], never a
//! silent skip.

use std::future::Future;

use ahash::AHashMap;

use crate::fixture::Engine;

/// One row of a live query result: every column the driver reported for
/// this row, each explicitly marked null or non-null.
///
/// Deliberately *not* "the names of the columns that were null" -- encoding
/// nullness by omission makes a column the driver reports under a different
/// name (e.g. Oracle upper-casing an unquoted identifier so `total` comes
/// back as `TOTAL`) indistinguishable from "observed non-null", which would
/// make A2 soundness fail-open on exactly the engine most likely to need it.
/// There is deliberately no `Default` impl: a default-constructed row would
/// have to pick some answer for "is column X null", and every answer is
/// wrong for a column the caller hasn't populated yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedRow {
    columns: AHashMap<String, bool>,
}

impl ObservedRow {
    /// Build a row from the exact columns the driver reported, each mapped
    /// to whether it was observed NULL. Callers (executor implementations)
    /// must report every column the query returns -- a column silently
    /// missing from `columns` is indistinguishable from a name/case
    /// mismatch, which [`ObservedRow::is_null`] refuses to paper over.
    pub fn new(columns: impl IntoIterator<Item = (String, bool)>) -> Self {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    /// Whether `column` was observed NULL in this row.
    ///
    /// Errors -- rather than silently reporting `false` -- when `column`
    /// was never reported by the driver at all. A2 soundness must never be
    /// fail-open on a column-name mismatch: an engine that returns a
    /// differently-cased or differently-named column must surface a hard
    /// error here, not be silently treated as "never null".
    pub fn is_null(&self, column: &str) -> Result<bool, MissingColumn> {
        self.columns.get(column).copied().ok_or_else(|| MissingColumn {
            column: column.to_string(),
        })
    }

    /// The full set of column names this row reported, for callers that
    /// need to validate a fixture's expected columns against what the
    /// driver actually returned before calling [`ObservedRow::is_null`].
    pub fn columns(&self) -> impl Iterator<Item = &str> {
        self.columns.keys().map(String::as_str)
    }
}

/// `column` was expected in an [`ObservedRow`] but the driver never reported
/// it -- a name or case mismatch between the query's declared columns and
/// what the engine actually returned.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("column {column:?} was not present in the observed row -- check for a name/case mismatch with the driver")]
pub struct MissingColumn {
    pub column: String,
}

/// A live database driver capable of seeding a schema, seeding one run's
/// data, and running a fixture's query to observe per-row nullness.
///
/// Implementors own their own connection lifecycle; this trait only
/// describes the three operations the conformance runner needs from one.
///
/// `Send`, not `Send + Sync`: the runner drives one executor at a time
/// through `&mut self`, never shares one across threads concurrently, and
/// `rusqlite::Connection` -- correctly, given SQLite's threading model --
/// is `Send` but not `Sync`. Requiring `Sync` here would make it impossible
/// to ever implement this trait for SQLite.
pub trait Executor: Send {
    /// The engine this executor drives, used to resolve schema files and
    /// per-engine seed/expectation overrides.
    const ENGINE: Engine;

    /// The executor's own error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Apply `schema_sql` (the fixture's resolved schema profile for this
    /// engine) to a fresh database or schema.
    fn seed_schema(&mut self, schema_sql: &str) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Execute one run's seed statements, in the given order.
    fn seed_run(&mut self, statements: &[String]) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Execute `query_sql` and report, per row in result order, every
    /// column's observed nullness. Row order must match the query's
    /// `ORDER BY` exactly -- callers match declared rows ordinally.
    /// Implementors must report every column the query returns, by its
    /// exact driver-reported name -- see [`ObservedRow`].
    fn query_nullness(&mut self, query_sql: &str)
    -> impl Future<Output = Result<Vec<ObservedRow>, Self::Error>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_null_reports_true_for_a_null_column() {
        let row = ObservedRow::new([("total".to_string(), true), ("id".to_string(), false)]);
        assert_eq!(row.is_null("total"), Ok(true));
    }

    #[test]
    fn is_null_reports_false_for_a_non_null_column() {
        let row = ObservedRow::new([("total".to_string(), true), ("id".to_string(), false)]);
        assert_eq!(row.is_null("id"), Ok(false));
    }

    #[test]
    fn is_null_errors_on_a_column_the_driver_never_reported() {
        // The Oracle-uppercasing scenario: the fixture expects "total" but
        // the driver reported "TOTAL". This must be a hard error, not a
        // silent "false" (== "observed non-null").
        let row = ObservedRow::new([("TOTAL".to_string(), true)]);
        assert_eq!(
            row.is_null("total"),
            Err(MissingColumn {
                column: "total".to_string()
            })
        );
    }

    #[test]
    fn columns_lists_every_reported_column() {
        let row = ObservedRow::new([("total".to_string(), true), ("id".to_string(), false)]);
        let mut columns: Vec<&str> = row.columns().collect();
        columns.sort_unstable();
        assert_eq!(columns, vec!["id", "total"]);
    }

    #[test]
    fn an_empty_row_reports_missing_for_every_column() {
        // With no `Default` impl, the only way to get an "empty" row is to
        // build one explicitly -- and even then, every lookup on it must
        // still be a hard error, not a silent "non-null".
        let row = ObservedRow::new([]);
        assert!(row.is_null("id").is_err());
    }
}
