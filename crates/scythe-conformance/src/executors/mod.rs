//! Per-engine live drivers, each behind its own Cargo feature so
//! `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`
//! never link a database driver by default. See [`crate::runner`] for how
//! these are dispatched -- [`crate::executor::Executor`] is not
//! dyn-compatible, so callers select a concrete type per engine rather than
//! holding these behind a trait object.

#[cfg(feature = "pg")]
pub mod postgres;

#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(any(feature = "mysql", feature = "mariadb"))]
pub mod mysql;

#[cfg(feature = "mssql")]
pub mod mssql;

#[cfg(feature = "oracle")]
pub mod oracle;

/// A namespace name (a Postgres schema, a MySQL/SQL Server database, an
/// Oracle user) unique to this call, for an executor to isolate itself into.
///
/// It must be unique per *connection*, not per process. `run_one_leg`
/// connects once per (fixture, engine), and the test binary runs its
/// `#[tokio::test]`s in parallel by default -- so a fixed name meant two
/// concurrent tests issued `DROP ... IF EXISTS` / `CREATE ...` against the
/// same namespace and dropped it out from under each other mid-run. That
/// surfaced as an opaque "preparing an isolated schema: db error", and
/// would have failed the PostgreSQL, MySQL and MariaDB CI jobs on their
/// first run. SQLite was immune only because each in-memory connection is
/// already its own universe.
///
/// The process id keeps concurrent `cargo test` invocations (and leftovers
/// from an earlier crashed run) from colliding; the counter separates
/// connections within one process. Digits only, so the result is always a
/// safe SQL identifier.
///
/// Namespaces are not dropped at the end of a run: there is no async
/// `Drop`, and the containers CI uses are discarded wholesale. A developer
/// pointing this at a long-lived database will accumulate
/// `scythe_conformance_*` namespaces and can drop them by prefix.
#[cfg(any(
    feature = "pg",
    feature = "mysql",
    feature = "mariadb",
    feature = "mssql",
    feature = "oracle"
))]
pub(crate) fn unique_namespace() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("scythe_conformance_{}_{}", std::process::id(), n)
}

/// Splits `sql` on top-level `;` terminators.
///
/// This is a naive split -- it has no notion of string literals or
/// comments -- which is safe only because this crate's schema files are
/// plain DDL with no embedded semicolons. It is not a general SQL statement
/// splitter and must not be reused for arbitrary SQL.
///
/// Shared by the three drivers whose clients refuse a multi-statement
/// batch: `mysql_async` (MySQL/MariaDB) does not enable multi-statement
/// queries by default, T-SQL rejects `CREATE`-family statements that are
/// not first in their batch, and OCI accepts exactly one statement per
/// prepare. PostgreSQL (`batch_execute`) and SQLite (`execute_batch`) send
/// the file verbatim and never call this.
#[cfg(any(feature = "mysql", feature = "mariadb", feature = "mssql", feature = "oracle"))]
pub(crate) fn split_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(str::to_string)
        .collect()
}
