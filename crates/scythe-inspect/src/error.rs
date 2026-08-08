//! Error types for the live-DB inspection pipeline.

use thiserror::Error;

/// Render an error together with its full `source()` chain.
///
/// `tokio_postgres::Error` displays as a bare `"db error"` — the SQLSTATE and
/// the server's message live further down the chain, so anything user-facing
/// must walk it or the report says nothing actionable.
pub fn error_chain(error: &dyn std::error::Error) -> String {
    let mut rendered = error.to_string();
    let mut cause = error.source();
    while let Some(current) = cause {
        rendered.push_str(": ");
        rendered.push_str(&current.to_string());
        cause = current.source();
    }
    rendered
}

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

    /// Two objects in the committed DDL reduce to the same schema-drift
    /// comparison key.
    ///
    /// Drift matches on the bare, unqualified name because scythe's catalog
    /// stores whatever the DDL wrote (`users` or `public.users`) while
    /// `pg_catalog` always knows the schema.  When the DDL declares the same
    /// bare name in two schemas, that key no longer identifies one object and
    /// there is no search path on this side to arbitrate — unlike the live
    /// side, which resolves the same collision by search-path position.
    ///
    /// Reported rather than resolved by picking one: whichever were picked
    /// would be a guess, and the guess would silently produce phantom
    /// missing-table and missing-column findings for the object that lost.
    #[error(
        "schema drift cannot tell {kind} `{first}` and `{second}` apart — both reduce to `{key}`, \
         and the DDL gives no search path to say which one the database's `{key}` is"
    )]
    AmbiguousSchemaObject {
        /// What collided: `"tables"` or `"enum types"`.
        kind: &'static str,
        /// The shared bare key both objects reduced to.
        key: String,
        /// The lexicographically first colliding name, as the DDL wrote it.
        first: String,
        /// The lexicographically second colliding name, as the DDL wrote it.
        second: String,
    },
}
