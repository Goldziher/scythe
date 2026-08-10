//! Live-database health-check engine for scythe.
//!
//! Connects to a running database via a connection URL, runs a set of
//! catalog/operational checks, and surfaces issues as
//! [`scythe_lint::reporters::Finding`] values that can be emitted through the
//! same reporters used by `scythe audit` (human / SARIF / JSON).
//!
//! ## Engines
//!
//! **PostgreSQL only.** [`PostgresDriver`], backed by `tokio-postgres`, is the
//! one implemented driver: the SC-INS checks are `pg_catalog` queries, query
//! verification uses the extended-query protocol's describe response, and
//! schema drift reads `pg_attribute.attnotnull`. None of those has an
//! equivalent implemented here for another engine.
//!
//! Every other engine — MySQL, MariaDB, SQLite, MSSQL, Oracle, Snowflake,
//! Redshift — gets [`UnsupportedDriver`], which refuses every operation with
//! [`InspectError::Unsupported`] naming *that* engine. It deliberately does not
//! pretend to connect and does not return an empty finding set: an inspection
//! that reports nothing because it never ran is indistinguishable from a clean
//! database.

pub mod config;
pub mod driver;
pub mod error;
pub mod neutral;
pub mod postgres;
pub mod registry;
pub mod schema_diff;
pub mod spec;
pub mod suppression;
pub mod unsupported;
pub mod verify;

pub use config::{InspectConfig, SuppressionRule, parse_inspect_section};
pub use driver::{CheckCatalogEntry, DbDriver};
pub use error::InspectError;
pub use neutral::normalize_neutral_type;
pub use postgres::PostgresDriver;
pub use registry::CheckRegistry;
pub use schema_diff::{
    DriftSeverities, SchemaDescription, describe_catalog, diff_schemas, drift_findings, fetch_live_schema,
};
pub use spec::{CheckCategory, CheckSpec, ConfigError, load_checks_from_file, parse_check_file};
pub use suppression::SuppressionEngine;
pub use unsupported::UnsupportedDriver;
pub use verify::verify_queries;
