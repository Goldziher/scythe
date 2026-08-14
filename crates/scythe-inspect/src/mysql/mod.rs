//! MySQL/MariaDB support: catalog introspection for schema drift
//! ([`MySqlCatalogSource`], via [`SchemaCatalogDriver`]) and the `SC-INS`
//! live health checks ([`MySqlDriver`], via [`DbDriver`]).
//!
//! Both engines share one wire protocol and one `mysql_async` client, so one
//! implementation serves both — matching how
//! `scythe_core::dialect::SqlDialect::from_str` already normalises `"mysql"`
//! and `"mariadb"` to the same dialect (there is no separate `SqlDialect`
//! variant for MariaDB at all), and how [`crate::registry`]'s checks key on
//! `"mysql"` for either. [`MySqlDriver::engine`] always reports `"mysql"`,
//! the same fixed identifier [`MySqlCatalogSource::engine`] already uses, for
//! the same reason: a caller serving a MariaDB request is expected to pass
//! that connection to this driver under the `"mysql"` key rather than expect
//! a `"mariadb"`-specific one.
//!
//! `information_schema.COLUMNS`, `.TABLES`, `.STATISTICS` and
//! `.TABLE_CONSTRAINTS` — the views both this module's catalog fetch and the
//! `SC-INS-MY*` checks in `mysql/checks.toml` read — carry the same column
//! set on MariaDB for every column this crate reads from them; nothing here
//! needed a MariaDB-specific query.
//!
//! ## Scope
//!
//! MySQL has no separate "schema" concept from "database" —
//! `information_schema.COLUMNS.TABLE_SCHEMA` *is* the database name. The
//! scope this reads is the connection's current database (`SELECT
//! DATABASE()`) unioned with `declared_schemas`, mirroring
//! [`fetch_live_schema`](crate::schema_diff::live::fetch_live_schema)'s
//! PostgreSQL scope: the search path first, DDL-declared qualifiers appended
//! after it. A bare table name declared in two schemas in scope resolves to
//! whichever schema comes first in that order, for the same reason PostgreSQL
//! does — it is what an unqualified reference on this connection would
//! resolve to.
//!
//! ## What this does not do
//!
//! - No enum modeling. MySQL's `ENUM(...)`/`SET(...)` are declared inline on
//!   the column, with no separate named type the way PostgreSQL's `CREATE
//!   TYPE ... AS ENUM` has; see [`types::neutral_type_for_mysql`] for why
//!   both map to the neutral `string` type instead. `SchemaDescription::enums`
//!   is always empty for this driver.
//! - The `SC-INS-MY*` checks cover a deliberately narrow slice of MySQL's
//!   operational surface (no primary key, duplicate indexes, an
//!   `AUTO_INCREMENT` column nearing its type's range, and a table left on
//!   the `MEMORY` engine) — the checks PostgreSQL's canonical set has no
//!   MySQL equivalent for (row-level security, extensions, `SECURITY
//!   DEFINER` search-path pinning) were deliberately left unimplemented
//!   rather than guessed at.

pub mod runner;
pub mod types;

use std::collections::HashMap;

use async_trait::async_trait;
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, Row, Value};
use scythe_lint::reporters::Finding;
use scythe_lint::types::Severity;

use crate::driver::{CheckCatalogEntry, DbDriver};
use crate::error::InspectError;
use crate::registry::CheckRegistry;
use crate::schema_diff::model::{ColumnDescription, SchemaDescription, TableDescription, object_key};
use crate::schema_diff::source::SchemaCatalogDriver;
use crate::spec::CheckSpec;
use crate::suppression::SuppressionEngine;

use types::neutral_type_for_mysql;

/// A [`SchemaCatalogDriver`] backed by an `mysql_async::Conn`.
///
/// Construction is infallible and connectionless, matching every other
/// driver in this crate: call [`connect`](SchemaCatalogDriver::connect)
/// before [`fetch_schema`](SchemaCatalogDriver::fetch_schema).
pub struct MySqlCatalogSource {
    conn: Option<Conn>,
}

impl MySqlCatalogSource {
    /// Build an unconnected driver.
    pub fn new() -> Self {
        Self { conn: None }
    }
}

impl Default for MySqlCatalogSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SchemaCatalogDriver for MySqlCatalogSource {
    fn engine(&self) -> &'static str {
        "mysql"
    }

    async fn connect(&mut self, url: &str) -> Result<(), InspectError> {
        let opts = Opts::from_url(url).map_err(|e| InspectError::Connect {
            engine: "mysql",
            source: Box::new(mysql_async::Error::from(e)),
        })?;
        let conn = Conn::new(opts).await.map_err(|e| InspectError::Connect {
            engine: "mysql",
            source: Box::new(e),
        })?;
        self.conn = Some(conn);
        Ok(())
    }

    async fn fetch_schema(&mut self, declared_schemas: &[String]) -> Result<SchemaDescription, InspectError> {
        let conn = self
            .conn
            .as_mut()
            .ok_or(InspectError::NotConnected { engine: "mysql" })?;

        let current = current_database(conn).await?;
        let mut scope: Vec<String> = current.into_iter().collect();
        for schema in declared_schemas {
            if !scope.iter().any(|existing| existing == schema) {
                scope.push(schema.clone());
            }
        }

        if scope.is_empty() {
            // No default database on this connection, and the committed DDL
            // qualified nothing either: there is no schema this fetch could
            // read, and returning an empty `SchemaDescription` for that would
            // report a clean bill of health for a database this never looked
            // at — the same failure `EmptySchemaScope` exists to name for
            // PostgreSQL.
            return Err(InspectError::EmptySchemaScope {
                search_path: Vec::new(),
                declared: declared_schemas.to_vec(),
            });
        }

        let mut description = SchemaDescription::new();
        fetch_tables(conn, &scope, &mut description).await?;
        Ok(description)
    }
}

fn query_error(step: &str, error: mysql_async::Error) -> InspectError {
    InspectError::Query {
        engine: "mysql",
        check_id: format!("mysql-catalog/{step}"),
        source: Box::new(error),
    }
}

/// Quote a value for interpolation into a `TABLE_SCHEMA IN (...)` list.
///
/// Schema names here originate from `SELECT DATABASE()` (the server's own
/// answer) and from the committed DDL's own schema qualifiers, not from
/// unvalidated external input — but doubling an embedded `'` is the
/// ANSI-standard SQL string-literal escape and costs nothing to apply
/// unconditionally, so a database or DDL-declared name that happens to
/// contain a quote still round-trips instead of breaking the query.
fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// The database this connection defaults to, or `None` when it has none
/// selected (`SELECT DATABASE()` reports `NULL` in that case).
async fn current_database(conn: &mut Conn) -> Result<Option<String>, InspectError> {
    let rows: Vec<Row> = conn
        .query("SELECT DATABASE()")
        .await
        .map_err(|e| query_error("current-database", e))?;

    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };

    match row.as_ref(0) {
        Some(Value::Bytes(bytes)) => Ok(Some(String::from_utf8_lossy(bytes).into_owned())),
        _ => Ok(None),
    }
}

/// Read a text column out of a fixed-column-order result row, by position.
///
/// Positional rather than by name: this module only ever runs the one query
/// it built itself, so the column order is known at the call site, and a
/// position lookup needs no fallible name resolution against the driver's
/// column metadata.
fn text_column(row: &Row, position: usize, step: &str) -> Result<String, InspectError> {
    match row.as_ref(position) {
        Some(Value::Bytes(bytes)) => Ok(String::from_utf8_lossy(bytes).into_owned()),
        _ => Err(InspectError::Query {
            engine: "mysql",
            check_id: format!("mysql-catalog/{step}"),
            source: Box::<dyn std::error::Error + Send + Sync>::from(format!(
                "expected a text value in column position {position}, got something else or NULL"
            )),
        }),
    }
}

async fn fetch_tables(
    conn: &mut Conn,
    scope: &[String],
    description: &mut SchemaDescription,
) -> Result<(), InspectError> {
    let schema_list = scope.iter().map(|s| quote_literal(s)).collect::<Vec<_>>().join(", ");

    // Column order fixed here and read positionally in `text_column` below —
    // 0 schema, 1 table, 2 table_type, 3 column, 4 data_type, 5 column_type,
    // 6 is_nullable, 7 column_key.
    let sql = format!(
        "SELECT c.TABLE_SCHEMA, c.TABLE_NAME, t.TABLE_TYPE, c.COLUMN_NAME, \
                c.DATA_TYPE, c.COLUMN_TYPE, c.IS_NULLABLE, c.COLUMN_KEY \
         FROM information_schema.COLUMNS c \
         JOIN information_schema.TABLES t \
           ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME \
         WHERE c.TABLE_SCHEMA IN ({schema_list}) \
         ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION"
    );

    let rows: Vec<Row> = conn
        .query(sql.as_str())
        .await
        .map_err(|e| query_error("information_schema.columns", e))?;

    // A table name can appear in more than one schema in scope; the first
    // scope-ordered occurrence wins, exactly like
    // `schema_diff::live::fetch_tables` resolves the same collision for
    // PostgreSQL — see this module's doc comment for why.
    let mut winning_rank: HashMap<String, usize> = HashMap::new();

    for row in &rows {
        let schema_name = text_column(row, 0, "table_schema")?;
        let table_name = text_column(row, 1, "table_name")?;
        let table_type = text_column(row, 2, "table_type")?;
        let column_name = text_column(row, 3, "column_name")?;
        let data_type = text_column(row, 4, "data_type")?;
        let column_type = text_column(row, 5, "column_type")?;
        let is_nullable = text_column(row, 6, "is_nullable")?;
        let column_key = text_column(row, 7, "column_key")?;

        let Some(rank) = scope.iter().position(|s| *s == schema_name) else {
            // Cannot happen given the `WHERE c.TABLE_SCHEMA IN (...)` clause
            // above, which is built from `scope` itself — skip rather than
            // panic if the server ever disagrees.
            continue;
        };

        let key = object_key(&table_name);
        match winning_rank.get(&key) {
            Some(&winner) if winner != rank => continue,
            Some(_) => {}
            None => {
                winning_rank.insert(key.clone(), rank);
                let mut table = TableDescription::new(format!("{schema_name}.{table_name}"));
                if table_type != "BASE TABLE" {
                    table = table.without_authoritative_nullability();
                }
                description.tables.insert(key.clone(), table);
            }
        }

        let neutral_type = neutral_type_for_mysql(&data_type, &column_type);
        let nullable = is_nullable.eq_ignore_ascii_case("YES");
        let mut column = match neutral_type {
            Some(neutral) => ColumnDescription::new(column_name, neutral, nullable),
            None => ColumnDescription::unmappable(column_name, nullable),
        };
        if column_key == "PRI" {
            column = column.as_primary_key();
        }

        if let Some(table) = description.tables.get_mut(&key) {
            table.columns.insert(column.name.to_lowercase(), column);
        }
    }

    Ok(())
}

/// Build the [`Finding`] reported when a check's SQL cannot be executed.
///
/// Mirrors `postgres::check_failure_finding`: a check may fail for reasons
/// unrelated to the schema under inspection (a permission the connection
/// lacks, a bug in the check's own SQL), and degrading to a warning keeps the
/// remaining checks useful instead of losing the whole run to one bad query.
fn check_failure_finding(spec: &CheckSpec, error: &InspectError) -> Finding {
    let detail = crate::error::error_chain(error);

    Finding {
        file: String::new(),
        query_name: None,
        rule_id: spec.id.clone(),
        rule_name: Some(format!("{}-check-failed", spec.name)),
        rule_description: Some(spec.description.clone()),
        severity: Severity::Warn,
        message: format!("check `{}` could not be executed: {detail}", spec.id),
        line: None,
        column: None,
        cwe: Vec::new(),
        source: Some("inspect".to_string()),
    }
}

/// MySQL/MariaDB driver for the `SC-INS-MY*` live health checks. Holds a
/// `mysql_async::Conn` after [`connect`](DbDriver::connect) succeeds; methods
/// that need it return [`InspectError::NotConnected`] otherwise.
///
/// [`engine`](DbDriver::engine) always reports `"mysql"` — see this module's
/// doc comment for why a MariaDB connection is served under that identifier
/// rather than a `"mariadb"`-specific one.
///
/// The check registry is built once at construction time from the embedded
/// canonical TOML; it is stored on the driver so `checks()` can return a
/// borrowed slice backed by the registry, matching [`crate::postgres::PostgresDriver`].
pub struct MySqlDriver {
    conn: Option<Conn>,
    /// Canonical check registry, built at `new()`.
    registry: CheckRegistry,
    /// Catalog entries derived from `registry` at construction, stored so
    /// `checks()` can return a `&[CheckCatalogEntry]` without lifetime
    /// gymnastics.
    catalog: Vec<CheckCatalogEntry>,
    /// Suppression engine built from `[[inspect.suppression]]` rules.
    ///
    /// `None` means no suppression rules are configured.
    suppression: Option<SuppressionEngine>,
}

impl MySqlDriver {
    /// Construct an unconnected driver and load the canonical check registry.
    /// Call [`DbDriver::connect`] before [`DbDriver::run_all`].
    pub fn new() -> Self {
        let registry = CheckRegistry::canonical();
        let catalog = registry
            .for_engine("mysql")
            .map(|spec| CheckCatalogEntry {
                id: spec.id.clone(),
                name: spec.name.clone(),
                severity: spec.severity,
                description: spec.description.clone(),
            })
            .collect();
        Self {
            conn: None,
            registry,
            catalog,
            suppression: None,
        }
    }

    /// Build a driver with a pre-configured registry (e.g. after applying
    /// severity overrides and user checks from `[inspect]` config).
    ///
    /// The catalog is derived from the provided registry.
    pub fn with_registry(registry: CheckRegistry) -> Self {
        let catalog = registry
            .for_engine("mysql")
            .map(|spec| CheckCatalogEntry {
                id: spec.id.clone(),
                name: spec.name.clone(),
                severity: spec.severity,
                description: spec.description.clone(),
            })
            .collect();
        Self {
            conn: None,
            registry,
            catalog,
            suppression: None,
        }
    }

    /// Set the suppression engine. Call before `connect()` / `run_all()`.
    pub fn set_suppression(&mut self, mut engine: SuppressionEngine) {
        engine.bind_to_registry(&self.registry);
        self.suppression = Some(engine);
    }
}

impl Default for MySqlDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DbDriver for MySqlDriver {
    fn engine(&self) -> &str {
        "mysql"
    }

    async fn connect(&mut self, url: &str) -> Result<(), InspectError> {
        let opts = Opts::from_url(url).map_err(|e| InspectError::Connect {
            engine: "mysql",
            source: Box::new(mysql_async::Error::from(e)),
        })?;
        let conn = Conn::new(opts).await.map_err(|e| InspectError::Connect {
            engine: "mysql",
            source: Box::new(e),
        })?;
        self.conn = Some(conn);
        Ok(())
    }

    fn checks(&self) -> &[CheckCatalogEntry] {
        &self.catalog
    }

    async fn run_all(&mut self) -> Result<Vec<Finding>, InspectError> {
        let conn = self
            .conn
            .as_mut()
            .ok_or(InspectError::NotConnected { engine: "mysql" })?;

        let mut all_pairs = Vec::new();

        for spec in self.registry.for_engine("mysql") {
            let pairs = match runner::run_check_with_bindings(conn, spec).await {
                Ok(pairs) => pairs,
                Err(error) => {
                    all_pairs.push((check_failure_finding(spec, &error), HashMap::new()));
                    continue;
                }
            };
            all_pairs.extend(pairs);
        }

        let findings = if let Some(sup) = &self.suppression {
            sup.filter(all_pairs)
        } else {
            all_pairs.into_iter().map(|(f, _)| f).collect()
        };

        Ok(findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_name_is_mysql() {
        assert_eq!(MySqlCatalogSource::new().engine(), "mysql");
    }

    #[tokio::test]
    async fn fetch_schema_without_connect_errors() {
        let mut source = MySqlCatalogSource::new();
        let error = source.fetch_schema(&[]).await.unwrap_err();
        assert!(matches!(error, InspectError::NotConnected { engine: "mysql" }));
    }

    #[test]
    fn quote_literal_doubles_an_embedded_quote() {
        assert_eq!(quote_literal("app"), "'app'");
        assert_eq!(quote_literal("weird'db"), "'weird''db'");
    }

    #[test]
    fn driver_engine_name_is_mysql() {
        assert_eq!(MySqlDriver::new().engine(), "mysql");
    }

    #[test]
    fn driver_catalog_lists_the_four_canonical_mysql_checks() {
        let d = MySqlDriver::new();
        let catalog = d.checks();
        assert_eq!(catalog.len(), 4);
        assert_eq!(catalog[0].id, "SC-INS-MY01");
        assert_eq!(catalog[1].id, "SC-INS-MY02");
        assert_eq!(catalog[2].id, "SC-INS-MY03");
        assert_eq!(catalog[3].id, "SC-INS-MY04");
    }

    /// The canonical registry carries both engines; `checks()` must only
    /// surface the MySQL slice, not PostgreSQL's SC-INS01..13 alongside it.
    #[test]
    fn driver_catalog_excludes_postgres_checks() {
        let d = MySqlDriver::new();
        assert!(d.checks().iter().all(|c| c.id.starts_with("SC-INS-MY")));
    }

    #[tokio::test]
    async fn run_all_without_connect_errors() {
        let mut d = MySqlDriver::new();
        let err = d.run_all().await.unwrap_err();
        assert!(matches!(err, InspectError::NotConnected { engine: "mysql" }));
    }

    #[test]
    fn with_registry_builds_catalog_correctly() {
        let reg = CheckRegistry::canonical();
        let d = MySqlDriver::with_registry(reg);
        assert_eq!(d.checks().len(), 4);
    }
}
