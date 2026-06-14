//! `DbDriver` — the engine-agnostic trait every live-DB driver implements.

use async_trait::async_trait;
use scythe_lint::reporters::Finding;
use scythe_lint::types::Severity;

use crate::error::InspectError;

/// One row in the check catalog returned by [`DbDriver::checks`].
///
/// Used by `--list-checks` to print a table without connecting to a database.
/// Fields are owned `String`s so the catalog can be built from the TOML
/// registry at runtime without requiring `'static` string literals.
#[derive(Debug, Clone)]
pub struct CheckCatalogEntry {
    /// Stable rule identifier (e.g. `"SC-INS01"`).
    pub id: String,
    /// Short human-readable name (e.g. `"missing-fk-index"`).
    pub name: String,
    /// Default severity emitted by the check.
    pub severity: Severity,
    /// One-line description suitable for a catalog table row.
    pub description: String,
}

/// Engine-agnostic interface for live-database inspection.
///
/// Construction is infallible — drivers do not connect at `new()`. Call
/// [`connect`](Self::connect) before [`run_all`](Self::run_all). This split
/// lets `--list-checks` print the catalog without touching a database.
#[async_trait]
pub trait DbDriver: Send + Sync {
    /// Stable engine identifier — e.g. `"postgres"`, `"mysql"`. Must be the
    /// same string [`scythe_core::dialect::SqlDialect::from_str`] accepts.
    fn engine(&self) -> &'static str;

    /// Open a connection to the given URL and store the client on `self`.
    ///
    /// Implementations should be idempotent: calling `connect` twice replaces
    /// the held connection. Errors are returned as [`InspectError::Connect`].
    async fn connect(&mut self, url: &str) -> Result<(), InspectError>;

    /// Static catalog of every check this driver implements, in stable order.
    /// Safe to call before `connect`.
    ///
    /// Returns a slice backed by a `Vec` stored on the driver — not `'static`
    /// because the catalog is now built at runtime from the TOML registry.
    fn checks(&self) -> &[CheckCatalogEntry];

    /// Run every check in [`checks`](Self::checks) and return their findings.
    ///
    /// Returns [`InspectError::NotConnected`] if `connect` has not succeeded.
    /// Per-check failures are returned as [`InspectError::Query`] and stop the
    /// run — there's no partial-success mode at Phase 0.
    async fn run_all(&self) -> Result<Vec<Finding>, InspectError>;
}
