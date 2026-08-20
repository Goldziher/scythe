//! Error types for the live-DB inspection pipeline.

use thiserror::Error;

/// Safe, stable categories for schema-file failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum SchemaExecutionErrorCategory {
    /// The schema file could not be read.
    Read,
    /// The embedded database rejected the schema SQL.
    SqlRejected,
}

impl SchemaExecutionErrorCategory {
    /// Stable code suitable for conversion at language boundaries.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Read => "SCHEMA_READ_ERROR",
            Self::SqlRejected => "SCHEMA_SQL_REJECTED",
        }
    }
}

impl std::fmt::Display for SchemaExecutionErrorCategory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read => formatter.write_str("schema file could not be read"),
            Self::SqlRejected => formatter.write_str("database rejected schema SQL"),
        }
    }
}

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
    /// Reading or executing one configured schema file failed.
    #[error(
        "{engine} schema file `{path}` failed while {operation} [{code}]: {category}",
        code = category.code(),
        path = path.display()
    )]
    SchemaExecution {
        /// Engine executing the schema.
        engine: &'static str,
        /// File whose read or execution failed.
        path: std::path::PathBuf,
        /// Operation in progress when the failure occurred.
        operation: &'static str,
        /// Sanitized category. The engine error is deliberately not retained because it may echo SQL.
        category: SchemaExecutionErrorCategory,
    },

    /// DuckDB metadata cannot identify a column's enum when multiple same-schema types share labels.
    #[error(
        "duckdb catalog construction cannot resolve enum identity [AMBIGUOUS_ENUM_IDENTITY] in schema `{schema}`: types {types:?} share the same labels"
    )]
    AmbiguousEnumIdentity {
        /// Schema containing the colliding enum types.
        schema: String,
        /// Qualified enum type names, sorted for deterministic diagnostics.
        types: Vec<String>,
    },

    /// Engine introspection produced definitions rejected by the catalog boundary.
    #[error("{engine} catalog construction failed while {operation}: {source}")]
    CatalogConstruction {
        /// Engine whose inspected definitions were rejected.
        engine: &'static str,
        /// Introspection operation in progress.
        operation: &'static str,
        /// Catalog validation error.
        #[source]
        source: scythe_core::errors::ScytheError,
    },

    /// Connection setup failed (TLS handshake, auth, network, etc.).
    ///
    /// Rendered through [`error_chain`] rather than `{source}`: a
    /// `tokio_postgres::Error` displays as a bare `"db error"`, which makes a
    /// wrong password, a missing database and a missing role indistinguishable.
    /// The server's `FATAL: ...` text lives one level further down the chain.
    #[error("connection to {engine} failed: {}", error_chain(&**source))]
    Connect {
        /// Engine that was being connected to (e.g. `"postgres"`).
        engine: &'static str,
        /// Underlying driver error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A catalog query failed at execution time.
    ///
    /// Rendered through [`error_chain`] for the same reason as
    /// [`InspectError::Connect`]: the SQLSTATE and the server's message are
    /// not in the top-level `Display`.
    #[error("{engine} catalog query {check_id} failed: {}", error_chain(&**source))]
    Query {
        /// Engine that ran the query.
        engine: &'static str,
        /// Identifier of the check whose SQL failed.
        check_id: String,
        /// Underlying driver error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The requested engine has no `scythe inspect` implementation.
    ///
    /// Carries the engine the *user* asked for, not the engine of whichever
    /// driver happened to be constructed. Naming the wrong engine here reads
    /// as a scythe bug rather than an unsupported-engine notice, and sends the
    /// reader looking for a defect that is not there.
    #[error(
        "engine `{engine}` is not supported by `scythe inspect` — live inspection, \
         schema drift and the SC-INS checks are implemented for PostgreSQL \
         (`postgres`, `postgresql`) and MySQL/MariaDB (`mysql`, `mariadb`) only"
    )]
    Unsupported {
        /// The engine name as the user spelled it (URL scheme or `--dialect`).
        engine: String,
    },

    /// A message placeholder resolved to a column whose driver-reported type
    /// this runner cannot render as text.
    ///
    /// Reported rather than rendered as an empty string: a blank in the middle
    /// of a finding message is indistinguishable from a genuinely empty value,
    /// and a check that reports `ratio= ts=` has told the reader nothing while
    /// looking like it worked.
    #[error(
        "check {check_id}: message placeholder '{{{binding}}}' is bound to column `{binding}` of \
         {engine} type `{type_name}`, which cannot be rendered as text — cast it to text in the \
         check's SQL and re-alias it as `{binding}`"
    )]
    UnrenderableBinding {
        /// ID of the check whose message could not be rendered.
        check_id: String,
        /// The `{var}` name, which is also the projected column name.
        binding: String,
        /// Engine that reported the type (e.g. `"postgres"`, `"mysql"`).
        engine: &'static str,
        /// The driver-reported type name for that column.
        type_name: String,
    },

    /// [`drift_findings`](crate::drift_findings) was called with nothing to
    /// compare.
    ///
    /// An empty candidate set is not a clean schema — it is a check that never
    /// ran. Returning "no findings" for it would report success for work that
    /// was never done, which is exactly what a drift gate exists to prevent.
    #[error(
        "schema drift has nothing to compare: no schema description was supplied — \
         a `[[sql]]` block whose schema glob matched no file cannot be checked for drift"
    )]
    NoSchemasToCompare,

    /// Every schema the drift check would read is missing from the database.
    ///
    /// `current_schemas(false)` skips search-path entries that do not exist, so
    /// a `search_path` naming only absent schemas resolves to nothing. Reading
    /// nothing and comparing it against nothing is a silent all-clear on a
    /// database the check never actually looked at.
    #[error(
        "schema drift found no schema to read: neither the connection's search_path ({search_path:?}) \
         nor the schemas the committed DDL qualifies ({declared:?}) name a schema that exists in \
         this database"
    )]
    EmptySchemaScope {
        /// The search path as `current_schemas(false)` reported it.
        search_path: Vec<String>,
        /// Schema qualifiers taken from the DDL's own object names.
        declared: Vec<String>,
    },

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

#[cfg(test)]
mod tests {
    use super::*;

    /// A driver error whose own `Display` says nothing, with the useful text
    /// one level down — the exact shape `tokio_postgres::Error` has.
    #[derive(Debug)]
    struct OpaqueDriverError {
        cause: ServerMessage,
    }

    #[derive(Debug)]
    struct ServerMessage(&'static str);

    impl std::fmt::Display for OpaqueDriverError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("db error")
        }
    }

    impl std::fmt::Display for ServerMessage {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for OpaqueDriverError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.cause)
        }
    }

    impl std::error::Error for ServerMessage {}

    fn opaque(message: &'static str) -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(OpaqueDriverError {
            cause: ServerMessage(message),
        })
    }

    /// The whole point of the variant: a connection failure must say *why*.
    /// Before this, a wrong password, a missing database and a missing role all
    /// rendered as the same four characters.
    #[test]
    fn should_render_the_server_message_when_a_connection_fails() {
        let error = InspectError::Connect {
            engine: "postgres",
            source: opaque("FATAL: database \"does_not_exist\" does not exist"),
        };
        assert_eq!(
            error.to_string(),
            "connection to postgres failed: db error: FATAL: database \"does_not_exist\" does not exist"
        );
    }

    #[test]
    fn should_render_the_server_message_when_a_catalog_query_fails() {
        let error = InspectError::Query {
            engine: "postgres",
            check_id: "SC-INS01".to_string(),
            source: opaque("ERROR: permission denied for table pg_constraint"),
        };
        assert_eq!(
            error.to_string(),
            "postgres catalog query SC-INS01 failed: db error: \
             ERROR: permission denied for table pg_constraint"
        );
    }

    /// The unsupported-engine message must name the engine the user asked for.
    /// Naming a different one sends the reader hunting for a scythe bug.
    #[test]
    fn should_name_the_requested_engine_when_it_is_unsupported() {
        let error = InspectError::Unsupported {
            engine: "sqlite".to_string(),
        };
        let rendered = error.to_string();
        assert!(rendered.starts_with("engine `sqlite` is not supported"), "{rendered}");
        // ~keep A bare `!contains("mysql")` used to stand in for this, and
        // stopped meaning anything once MySQL became a supported engine the
        // message legitimately lists. What must never happen is naming some
        // other engine as the *subject* of the failure -- the original defect,
        // where a SQLite user was told `engine "mysql" is not supported`.
        assert!(!rendered.contains("engine `mysql`"), "{rendered}");
        assert!(!rendered.contains("engine `postgres`"), "{rendered}");
    }

    #[test]
    fn should_name_the_column_and_type_when_a_binding_cannot_be_rendered() {
        let error = InspectError::UnrenderableBinding {
            check_id: "USER-INS-001".to_string(),
            binding: "ratio".to_string(),
            engine: "postgres",
            type_name: "numeric".to_string(),
        };
        assert_eq!(
            error.to_string(),
            "check USER-INS-001: message placeholder '{ratio}' is bound to column `ratio` of \
             postgres type `numeric`, which cannot be rendered as text — cast it to text in the \
             check's SQL and re-alias it as `ratio`"
        );
    }

    /// The variant is shared across drivers — a MySQL check's error must name
    /// `mysql`, not carry over PostgreSQL's label.
    #[test]
    fn should_name_the_engine_that_reported_an_unrenderable_type() {
        let error = InspectError::UnrenderableBinding {
            check_id: "SC-INS-MY03".to_string(),
            binding: "max_value".to_string(),
            engine: "mysql",
            type_name: "decimal".to_string(),
        };
        assert!(error.to_string().contains("mysql type `decimal`"), "{error}");
    }
}
