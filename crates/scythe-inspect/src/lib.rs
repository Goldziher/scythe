//! Live-database health-check engine for scythe.
//!
//! Connects to a running database via a connection URL, runs a set of
//! catalog/operational checks, and surfaces issues as
//! [`scythe_lint::reporters::Finding`] values that can be emitted through the
//! same reporters used by `scythe audit` (human / SARIF / JSON).
//!
//! ## Engines
//!
//! **The `SC-INS` health checks are implemented for PostgreSQL and
//! MySQL/MariaDB.** [`PostgresDriver`] (backed by `tokio-postgres`) and
//! [`mysql::MySqlDriver`] (backed by `mysql_async`) are the two drivers
//! implementing them, each from its own TOML-driven check registry
//! (`postgres/checks.toml`, `mysql/checks.toml`) merged into one
//! [`CheckRegistry`] by [`CheckRegistry::canonical`]. The two check sets are
//! not symmetric: PostgreSQL's row-level-security, extension and
//! `SECURITY DEFINER` search-path checks have no MySQL equivalent and are not
//! approximated there, and query verification
//! ([`verify_queries`]) uses PostgreSQL's extended-query protocol describe
//! response, which MySQL has no equivalent driver call for.
//!
//! Every engine with no [`DbDriver`] implementation — SQLite, MSSQL, Oracle,
//! Snowflake, Redshift — gets [`UnsupportedDriver`], which refuses every
//! operation with [`InspectError::Unsupported`] naming *that* engine. It
//! deliberately does not pretend to connect and does not return an empty
//! finding set: an inspection that reports nothing because it never ran is
//! indistinguishable from a clean database.
//!
//! **Catalog introspection — tables, columns, neutral types, nullability,
//! primary keys — is a separate, narrower concern from the health checks
//! above, and it is not PostgreSQL-only.** [`schema_diff::SchemaCatalogDriver`]
//! is the engine-agnostic trait for reading a live database into a
//! [`SchemaDescription`]; [`sqlite::SqliteCatalogSource`] and
//! [`mysql::MySqlCatalogSource`] implement it alongside PostgreSQL's existing
//! [`fetch_live_schema`]. See [`schema_diff`]'s module docs for why the two
//! concerns are split and what implementing one gets an engine that
//! implementing the other does not.

pub mod config;
pub mod driver;
pub mod error;
pub mod mysql;
pub mod neutral;
pub mod postgres;
pub mod registry;
pub mod schema_diff;
pub mod spec;
pub mod sqlite;
pub mod suppression;
pub mod unsupported;
pub mod verify;

pub use config::{InspectConfig, SuppressionRule, parse_inspect_section};
pub use driver::{CheckCatalogEntry, DbDriver};
pub use error::InspectError;
pub use mysql::{MySqlCatalogSource, MySqlDriver};
pub use neutral::normalize_neutral_type;
pub use postgres::PostgresDriver;
pub use registry::CheckRegistry;
pub use schema_diff::{
    DriftSeverities, SchemaCatalogDriver, SchemaDescription, describe_catalog, diff_schemas, drift_findings,
    fetch_live_schema,
};
pub use spec::{CheckCategory, CheckSpec, ConfigError, load_checks_from_file, parse_check_file};
pub use sqlite::SqliteCatalogSource;
pub use suppression::SuppressionEngine;
pub use unsupported::UnsupportedDriver;
pub use verify::verify_queries;
