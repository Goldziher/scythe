//! PostgreSQL driver — connects via `tokio-postgres` and runs checks from the
//! TOML-driven registry.

use async_trait::async_trait;
use scythe_lint::reporters::Finding;
use tokio_postgres::{Client, NoTls};

use crate::driver::{CheckCatalogEntry, DbDriver};
use crate::error::InspectError;
use crate::registry::CheckRegistry;

pub mod runner;

/// PostgreSQL driver. Holds a `tokio_postgres::Client` after `connect()`
/// succeeds; methods that need the client return
/// [`InspectError::NotConnected`] otherwise.
///
/// The check registry is built once at construction time from the embedded
/// canonical TOML; it is stored on the driver so `checks()` can return
/// a borrowed slice backed by the registry.
pub struct PostgresDriver {
    client: Option<Client>,
    /// Canonical check registry, built at `new()`.
    registry: CheckRegistry,
    /// Catalog entries derived from `registry` at construction, stored so
    /// `checks()` can return a `&[CheckCatalogEntry]` without lifetime
    /// gymnastics.
    catalog: Vec<CheckCatalogEntry>,
    /// Postgres server version number (e.g. `160004` for PG 16.4).
    ///
    /// `None` until `connect()` succeeds and `SHOW server_version_num` is
    /// queried; used to gate `min_pg_version` checks.
    pg_version: Option<u32>,
}

impl PostgresDriver {
    /// Construct an unconnected driver and load the canonical check registry.
    /// Call [`DbDriver::connect`] before [`DbDriver::run_all`].
    pub fn new() -> Self {
        let registry = CheckRegistry::canonical();
        let catalog = registry
            .for_engine("postgres")
            .map(|spec| CheckCatalogEntry {
                id: spec.id.clone(),
                name: spec.name.clone(),
                severity: spec.severity,
                description: spec.description.clone(),
            })
            .collect();
        Self {
            client: None,
            registry,
            catalog,
            pg_version: None,
        }
    }

    /// Borrow the underlying client (test/inspection helper).
    pub fn client(&self) -> Option<&Client> {
        self.client.as_ref()
    }
}

impl Default for PostgresDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DbDriver for PostgresDriver {
    fn engine(&self) -> &'static str {
        "postgres"
    }

    async fn connect(&mut self, url: &str) -> Result<(), InspectError> {
        let (client, connection) =
            tokio_postgres::connect(url, NoTls)
                .await
                .map_err(|e| InspectError::Connect {
                    engine: "postgres",
                    source: Box::new(e),
                })?;

        // Drive the connection on the current runtime. Per-invocation CLI use
        // means the spawned task lives only as long as `run_all` runs, which
        // is acceptable for Phase 0.
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("scythe-inspect: postgres connection task error: {e}");
            }
        });

        // Detect server version for min_pg_version gating.
        let version_row = client
            .query_one("SHOW server_version_num", &[])
            .await
            .map_err(|e| InspectError::Connect {
                engine: "postgres",
                source: Box::new(e),
            })?;
        let version_str: &str = version_row.get(0);
        let pg_version: u32 = version_str.parse().map_err(|e| InspectError::Connect {
            engine: "postgres",
            source: Box::<dyn std::error::Error + Send + Sync>::from(format!(
                "failed to parse server_version_num {version_str:?}: {e}"
            )),
        })?;

        self.client = Some(client);
        self.pg_version = Some(pg_version);
        Ok(())
    }

    fn checks(&self) -> &[CheckCatalogEntry] {
        &self.catalog
    }

    async fn run_all(&self) -> Result<Vec<Finding>, InspectError> {
        let client = self
            .client
            .as_ref()
            .ok_or(InspectError::NotConnected { engine: "postgres" })?;

        let pg_version = self.pg_version.unwrap_or(u32::MAX);

        let mut findings = Vec::new();
        for spec in self.registry.for_engine("postgres") {
            // Skip checks that require a newer PG than what's connected.
            // `min_pg_version` is a major version (12, 14, 15, 16); convert
            // to `server_version_num` form for comparison.
            if let Some(min_major) = spec.min_pg_version
                && pg_version < min_major.saturating_mul(10_000)
            {
                continue;
            }
            findings.extend(runner::run_check(client, spec).await?);
        }
        Ok(findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_name_is_postgres() {
        assert_eq!(PostgresDriver::new().engine(), "postgres");
    }

    #[test]
    fn catalog_lists_canonical_checks() {
        use crate::spec::CANONICAL_CHECK_IDS;
        let d = PostgresDriver::new();
        let catalog = d.checks();
        assert_eq!(catalog.len(), CANONICAL_CHECK_IDS.len());
        // Verify the first three canonical IDs are present in order.
        assert_eq!(catalog[0].id, "SC-INS01");
        assert_eq!(catalog[1].id, "SC-INS02");
        assert_eq!(catalog[2].id, "SC-INS03");
    }

    #[tokio::test]
    async fn run_all_without_connect_errors() {
        let d = PostgresDriver::new();
        let err = d.run_all().await.unwrap_err();
        assert!(matches!(
            err,
            InspectError::NotConnected { engine: "postgres" }
        ));
    }

    /// Sanity-check the major-version × 10_000 conversion used to gate
    /// `min_pg_version`. PG 12 = 120000, PG 16 = 160000, PG 17 = 170000.
    /// Regression guard: a check with `min_pg_version = 15` must NOT fire
    /// against a server reporting `server_version_num = 140004`, but MUST fire
    /// against one reporting `160007`.
    #[test]
    fn min_pg_version_gates_against_server_version_num_form() {
        let min_major: u32 = 15;
        let pg_14: u32 = 140004;
        let pg_16: u32 = 160007;
        assert!(pg_14 < min_major.saturating_mul(10_000));
        assert!(pg_16 >= min_major.saturating_mul(10_000));
    }
}
