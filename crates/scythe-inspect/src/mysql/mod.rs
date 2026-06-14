//! MySQL driver — **stub only** at Phase 0.
//!
//! The stub exists so that the [`DbDriver`] trait surface stays
//! engine-agnostic. Every method either errors with
//! [`InspectError::Unsupported`] or returns an empty catalog. A real
//! implementation backed by `sqlx-mysql` lands in Phase 3.

use async_trait::async_trait;
use scythe_lint::reporters::Finding;

use crate::driver::{CheckCatalogEntry, DbDriver};
use crate::error::InspectError;

/// MySQL driver placeholder. Construct with [`MysqlDriver::new`]; every
/// operation returns [`InspectError::Unsupported`].
#[derive(Debug, Default)]
pub struct MysqlDriver;

impl MysqlDriver {
    /// Construct a new MySQL driver. Connection happens later via
    /// [`DbDriver::connect`] — which currently returns
    /// [`InspectError::Unsupported`].
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DbDriver for MysqlDriver {
    fn engine(&self) -> &'static str {
        "mysql"
    }

    async fn connect(&mut self, _url: &str) -> Result<(), InspectError> {
        Err(InspectError::Unsupported("mysql"))
    }

    fn checks(&self) -> &'static [CheckCatalogEntry] {
        &[]
    }

    async fn run_all(&self) -> Result<Vec<Finding>, InspectError> {
        Err(InspectError::Unsupported("mysql"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_returns_unsupported() {
        let mut d = MysqlDriver::new();
        let err = d.connect("mysql://localhost/x").await.unwrap_err();
        assert!(matches!(err, InspectError::Unsupported("mysql")));
    }

    #[test]
    fn engine_name_is_mysql() {
        assert_eq!(MysqlDriver::new().engine(), "mysql");
    }

    #[test]
    fn checks_catalog_is_empty() {
        assert!(MysqlDriver::new().checks().is_empty());
    }
}
