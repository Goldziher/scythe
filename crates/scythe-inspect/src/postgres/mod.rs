//! PostgreSQL driver — connects via `tokio-postgres` and runs checks from the
//! TOML-driven registry.

use std::collections::HashMap;

use async_trait::async_trait;
use scythe_lint::reporters::Finding;
use scythe_lint::types::Severity;
use tokio_postgres::{Client, NoTls};

use crate::driver::{CheckCatalogEntry, DbDriver};
use crate::error::InspectError;
use crate::registry::CheckRegistry;
use crate::spec::CheckSpec;
use crate::suppression::SuppressionEngine;

pub mod runner;

/// Build the [`Finding`] reported when a check's SQL cannot be executed.
///
/// A check may fail for reasons that have nothing to do with the schema under
/// inspection: a catalog view missing on an unusual build, a role without
/// permission to read it, or a bug in the check's own SQL.  Reporting the
/// failure as a warning keeps the remaining checks useful instead of losing the
/// whole run to one bad query.
///
/// The finding keeps the failing check's `rule_id` so existing suppression
/// rules continue to address it by ID.
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

/// The check ID for SC-INS10 (rls-disabled-in-public). When `api_schemas` is
/// configured, the post-run filter restricts SC-INS10 findings to schemas in
/// that list.  When `api_schemas` is empty the filter defaults to `["public"]`.
///
/// SC-INS10's SQL reports tables without RLS across ALL user schemas so the
/// filter (not the SQL) determines scope.  This keeps the SQL simple while
/// making the scope configurable without SQL parameterisation.
const SC_INS10_ID: &str = "SC-INS10";

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
    /// Suppression engine built from `[[inspect.suppression]]` rules.
    ///
    /// `None` means no suppression rules are configured.
    suppression: Option<SuppressionEngine>,
    /// Schemas to apply for SC-INS10 (rls-disabled-in-public).
    ///
    /// SC-INS10 findings whose `schema_name` binding is NOT in this list are
    /// dropped.  Defaults to `["public"]` when the list is empty.
    api_schemas: Vec<String>,
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
            suppression: None,
            api_schemas: Vec::new(),
        }
    }

    /// Build a driver with a pre-configured registry (e.g. after applying
    /// severity overrides and user checks from `[inspect]` config).
    ///
    /// The catalog is derived from the provided registry.
    pub fn with_registry(registry: CheckRegistry) -> Self {
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
            suppression: None,
            api_schemas: Vec::new(),
        }
    }

    /// Set the suppression engine.  Call before `connect()` / `run_all()`.
    ///
    /// The engine is bound to this driver's registry here, which is what tells
    /// it *which* result column `[[inspect.suppression]] object = "…"` and
    /// `schema = "…"` compare against. Without that binding it falls back to
    /// searching the row, and searching the row is what made suppression depend
    /// on hash-map iteration order.
    pub fn set_suppression(&mut self, mut engine: SuppressionEngine) {
        engine.bind_to_registry(&self.registry);
        self.suppression = Some(engine);
    }

    /// Set the api_schemas list for SC-INS10 filtering.  An empty list
    /// falls back to `["public"]`.
    pub fn set_api_schemas(&mut self, schemas: Vec<String>) {
        self.api_schemas = schemas;
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
    fn engine(&self) -> &str {
        "postgres"
    }

    async fn connect(&mut self, url: &str) -> Result<(), InspectError> {
        let (client, connection) = tokio_postgres::connect(url, NoTls)
            .await
            .map_err(|e| InspectError::Connect {
                engine: "postgres",
                source: Box::new(e),
            })?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("scythe-inspect: postgres connection task error: {e}");
            }
        });

        let version_row =
            client
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

        let effective_api_schemas: Vec<&str> = if self.api_schemas.is_empty() {
            vec!["public"]
        } else {
            self.api_schemas.iter().map(|s| s.as_str()).collect()
        };

        let mut all_pairs = Vec::new();

        for spec in self.registry.for_engine("postgres") {
            if let Some(min_major) = spec.min_pg_version
                && pg_version < min_major.saturating_mul(10_000)
            {
                continue;
            }

            let pairs = match runner::run_check_with_bindings(client, spec).await {
                Ok(pairs) => pairs,
                Err(error) => {
                    all_pairs.push((check_failure_finding(spec, &error), HashMap::new()));
                    continue;
                }
            };

            for (finding, bindings) in pairs {
                // The schema column comes from the check's own declaration, not
                // from a substring search over a randomly-ordered map — the
                // same defect that made suppression non-deterministic.
                if finding.rule_id == SC_INS10_ID
                    && let Some(column) = spec.schema_binding.as_deref()
                    && let Some(schema) = bindings.get(column).map(String::as_str)
                    && !effective_api_schemas.contains(&schema)
                {
                    continue;
                }

                all_pairs.push((finding, bindings));
            }
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
    fn engine_name_is_postgres() {
        assert_eq!(PostgresDriver::new().engine(), "postgres");
    }

    #[test]
    fn catalog_lists_canonical_checks() {
        use crate::spec::CANONICAL_CHECK_IDS;
        let d = PostgresDriver::new();
        let catalog = d.checks();
        assert_eq!(catalog.len(), CANONICAL_CHECK_IDS.len());
        assert_eq!(catalog[0].id, "SC-INS01");
        assert_eq!(catalog[1].id, "SC-INS02");
        assert_eq!(catalog[2].id, "SC-INS03");
    }

    /// Regression guard for the SC-INS13 outage: PostgreSQL only defines the
    /// two-argument `round(v, s)` for `numeric`, so `round(float8, int)` fails
    /// with SQLSTATE 42883 on every server version.  Because `run_all` used to
    /// abort on the first failing check, this single expression took the entire
    /// `scythe inspect` command down.
    #[test]
    fn no_check_calls_two_arg_round_on_a_float() {
        let registry = CheckRegistry::canonical();
        for spec in registry.for_engine("postgres") {
            assert!(
                !spec.sql.contains("round((last_value::float8"),
                "check {} calls round() on a float8 — PostgreSQL has no round(double precision, integer)",
                spec.id
            );
        }
    }

    /// A check whose SQL the server rejects must degrade to a single warning
    /// rather than aborting the run, so the remaining checks still report.
    #[test]
    fn failed_check_becomes_a_warning_finding() {
        let registry = CheckRegistry::canonical();
        let spec = registry.for_engine("postgres").next().expect("at least one check");

        let error = InspectError::MessageBindingMissing {
            check_id: spec.id.clone(),
            binding: "schema_name".to_string(),
        };
        let finding = check_failure_finding(spec, &error);

        assert_eq!(finding.rule_id, spec.id);
        assert_eq!(finding.severity, Severity::Warn);
        assert_eq!(finding.source.as_deref(), Some("inspect"));
        assert!(
            finding.message.contains("could not be executed"),
            "message should explain the check did not run: {}",
            finding.message
        );
    }

    #[tokio::test]
    async fn run_all_without_connect_errors() {
        let d = PostgresDriver::new();
        let err = d.run_all().await.unwrap_err();
        assert!(matches!(err, InspectError::NotConnected { engine: "postgres" }));
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

    #[test]
    fn with_registry_builds_catalog_correctly() {
        use crate::spec::CANONICAL_CHECK_IDS;
        let reg = CheckRegistry::canonical();
        let d = PostgresDriver::with_registry(reg);
        assert_eq!(d.checks().len(), CANONICAL_CHECK_IDS.len());
    }
}
