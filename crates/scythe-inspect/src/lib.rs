//! Live-database health-check engine for scythe.
//!
//! Connects to a running database via a connection URL, runs a set of
//! catalog/operational checks, and surfaces issues as
//! [`scythe_lint::reporters::Finding`] values that can be emitted through the
//! same reporters used by `scythe audit` (human / SARIF / JSON).
//!
//! ## Engines
//!
//! - **PostgreSQL**: full Phase 0 support via [`PostgresDriver`] backed by
//!   `tokio-postgres`. Three checks ship: SC-INS01 (missing FK index),
//!   SC-INS02 (policy exists with RLS disabled), SC-INS03 (duplicate index).
//! - **MySQL**: stub only ([`MysqlDriver`] returns
//!   [`InspectError::Unsupported`]). The stub exists to keep the
//!   [`DbDriver`] trait shape engine-agnostic; a real driver lands in Phase 3.
//!
//! Other engines (MSSQL, Snowflake, Oracle) are not yet wired.

pub mod driver;
pub mod error;
pub mod mysql;
pub mod postgres;

pub use driver::{CheckCatalogEntry, DbDriver};
pub use error::InspectError;
pub use mysql::MysqlDriver;
pub use postgres::PostgresDriver;
