//! Live-database health-check engine for scythe.
//!
//! Connects to a running database via a connection URL, runs a set of
//! catalog/operational checks, and surfaces issues as
//! [`scythe_lint::reporters::Finding`] values that can be emitted through the
//! same reporters used by `scythe audit` (human / SARIF / JSON).
//!
//! ## Engines
//!
//! - **PostgreSQL**: full support via [`PostgresDriver`] backed by
//!   `tokio-postgres`. Checks are defined in TOML and driven by
//!   [`CheckRegistry::canonical`].
//! - **MySQL**: stub only ([`MysqlDriver`] returns
//!   [`InspectError::Unsupported`]). The stub exists to keep the
//!   [`DbDriver`] trait shape engine-agnostic; a real driver lands in Phase 3.
//!
//! Other engines (MSSQL, Snowflake, Oracle) are not yet wired.

pub mod config;
pub mod driver;
pub mod error;
pub mod mysql;
pub mod postgres;
pub mod registry;
pub mod spec;
pub mod suppression;

pub use config::{InspectConfig, SuppressionRule, parse_inspect_section};
pub use driver::{CheckCatalogEntry, DbDriver};
pub use error::InspectError;
pub use mysql::MysqlDriver;
pub use postgres::PostgresDriver;
pub use registry::CheckRegistry;
pub use spec::{CheckCategory, CheckSpec, ConfigError, load_checks_from_file, parse_check_file};
pub use suppression::SuppressionEngine;
