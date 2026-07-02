//! Error types for the live-DB inspection pipeline.

use thiserror::Error;

/// Errors that arise while connecting to a database, running catalog queries,
/// or interpreting a connection URL.
#[derive(Debug, Error)]
pub enum InspectError {
    /// Connection setup failed (TLS handshake, auth, network, etc.).
    #[error("connection to {engine} failed: {source}")]
    Connect {
        /// Engine that was being connected to (e.g. `"postgres"`).
        engine: &'static str,
        /// Underlying driver error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A catalog query failed at execution time.
    #[error("{engine} catalog query {check_id} failed: {source}")]
    Query {
        /// Engine that ran the query.
        engine: &'static str,
        /// Identifier of the check whose SQL failed.
        check_id: String,
        /// Underlying driver error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The requested engine has no implementation yet (e.g. MySQL at Phase 0).
    #[error("engine {0:?} is not yet supported by scythe-inspect")]
    Unsupported(&'static str),

    /// No connection URL could be resolved from CLI, env, or config.
    #[error("no database URL provided — pass a positional URL, set DATABASE_URL, or set SCYTHE_DATABASE_URL")]
    UrlMissing,

    /// A driver method was called before [`DbDriver::connect`] succeeded.
    #[error("driver {engine} is not connected — call connect() before run_all()")]
    NotConnected {
        /// Engine whose method was called.
        engine: &'static str,
    },

    /// A message template `{var}` placeholder had no matching column in the
    /// SQL row.  The canonical-time binding validation in
    /// [`crate::spec::validate_message_bindings`] should make this unreachable
    /// for built-in checks; this variant exists as a defence-in-depth guard
    /// for user-defined checks.
    #[error("check {check_id}: message placeholder '{{{binding}}}' not found in query result")]
    MessageBindingMissing {
        /// ID of the check whose message template is broken.
        check_id: String,
        /// The `{var}` name that was absent from the row.
        binding: String,
    },
}
