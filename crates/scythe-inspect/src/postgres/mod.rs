//! PostgreSQL driver — connects via `tokio-postgres` and runs Phase 0 checks.

use async_trait::async_trait;
use scythe_lint::reporters::Finding;
use scythe_lint::types::Severity;
use tokio_postgres::{Client, NoTls};

use crate::driver::{CheckCatalogEntry, DbDriver};
use crate::error::InspectError;

pub mod checks;

/// Static catalog for `--list-checks` and `checks()` — order is stable.
const CHECK_CATALOG: &[CheckCatalogEntry] = &[
    CheckCatalogEntry {
        id: "SC-INS01",
        name: "missing-fk-index",
        severity: Severity::Warn,
        description: checks::SC_INS01_DESC,
    },
    CheckCatalogEntry {
        id: "SC-INS02",
        name: "policy-exists-rls-disabled",
        severity: Severity::Error,
        description: checks::SC_INS02_DESC,
    },
    CheckCatalogEntry {
        id: "SC-INS03",
        name: "duplicate-index",
        severity: Severity::Warn,
        description: checks::SC_INS03_DESC,
    },
];

/// PostgreSQL driver. Holds a `tokio_postgres::Client` after `connect()`
/// succeeds; methods that need the client return
/// [`InspectError::NotConnected`] otherwise.
pub struct PostgresDriver {
    client: Option<Client>,
}

impl PostgresDriver {
    /// Construct an unconnected driver. Call [`DbDriver::connect`] before
    /// [`DbDriver::run_all`].
    pub fn new() -> Self {
        Self { client: None }
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

        self.client = Some(client);
        Ok(())
    }

    fn checks(&self) -> &'static [CheckCatalogEntry] {
        CHECK_CATALOG
    }

    async fn run_all(&self) -> Result<Vec<Finding>, InspectError> {
        let client = self
            .client
            .as_ref()
            .ok_or(InspectError::NotConnected { engine: "postgres" })?;

        // Run checks sequentially — three pg_catalog queries finish in
        // milliseconds; concurrency would be premature.
        let mut findings = Vec::new();
        findings.extend(checks::check_missing_fk_index(client).await?);
        findings.extend(checks::check_policy_rls_disabled(client).await?);
        findings.extend(checks::check_duplicate_index(client).await?);
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
    fn catalog_lists_three_checks() {
        let d = PostgresDriver::new();
        let catalog = d.checks();
        assert_eq!(catalog.len(), 3);
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
}
