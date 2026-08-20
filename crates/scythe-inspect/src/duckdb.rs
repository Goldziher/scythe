//! Offline DuckDB schema execution and catalog introspection.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use duckdb::{Config, Connection, params};
use scythe_core::catalog::{
    Catalog, CatalogBuilder, CatalogObjectName, ColumnDefinition, EnumDefinition, RelationDefinition,
};
use scythe_core::dialect::SqlDialect;

use crate::error::{InspectError, SchemaExecutionErrorCategory};

const DUCKDB_ENGINE: &str = "duckdb";
const INTERNAL_SCHEMAS: [&str; 2] = ["information_schema", "pg_catalog"];

/// Execute trusted schema files in order inside a hardened in-memory DuckDB database.
///
/// Each call creates an isolated bundled-DuckDB connection. External access,
/// extension autoloading, extension auto-installation, and community extensions
/// are disabled in the database configuration before any schema SQL executes.
/// Schema SQL remains executable input and must come from a trusted source.
pub fn execute_duckdb_schema_files(paths: &[PathBuf]) -> Result<Catalog, InspectError> {
    let connection = open_secured_connection()?;
    for path in paths {
        let sql = std::fs::read_to_string(path).map_err(|_| InspectError::SchemaExecution {
            engine: DUCKDB_ENGINE,
            path: path.clone(),
            operation: "reading schema SQL",
            category: SchemaExecutionErrorCategory::Read,
        })?;
        connection
            .execute_batch(&sql)
            .map_err(|_| InspectError::SchemaExecution {
                engine: DUCKDB_ENGINE,
                path: path.clone(),
                operation: "executing schema DDL",
                category: SchemaExecutionErrorCategory::SqlRejected,
            })?;
    }
    build_catalog(&connection)
}

fn open_secured_connection() -> Result<Connection, InspectError> {
    let config = Config::default()
        .enable_external_access(false)
        .and_then(|config| config.enable_autoload_extension(false))
        .and_then(|config| config.with("allow_community_extensions", "false"))
        .map_err(connect_error)?;
    Connection::open_in_memory_with_flags(config).map_err(connect_error)
}

fn connect_error(source: duckdb::Error) -> InspectError {
    InspectError::Connect {
        engine: DUCKDB_ENGINE,
        source: Box::new(source),
    }
}

fn query_error(operation: &str, source: duckdb::Error) -> InspectError {
    InspectError::Query {
        engine: DUCKDB_ENGINE,
        check_id: format!("duckdb-catalog/{operation}"),
        source: Box::new(source),
    }
}

fn build_catalog(connection: &Connection) -> Result<Catalog, InspectError> {
    let mut builder = CatalogBuilder::new(SqlDialect::PostgreSQL).engine(DUCKDB_ENGINE);
    let enums = fetch_enums(connection)?;
    let enum_types = enum_type_resolutions(&enums)?;
    for definition in fetch_relations(connection, &enum_types)? {
        builder = builder.relation(definition);
    }
    for definition in enums {
        builder = builder.enum_type(EnumDefinition::new(definition.name, definition.values));
    }
    builder.build().map_err(|source| InspectError::CatalogConstruction {
        engine: DUCKDB_ENGINE,
        operation: "validating introspected schema",
        source,
    })
}

fn fetch_relations(
    connection: &Connection,
    enum_types: &HashMap<EnumResolutionKey, String>,
) -> Result<Vec<RelationDefinition>, InspectError> {
    let placeholders = INTERNAL_SCHEMAS.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT table_schema, table_name, table_type
         FROM information_schema.tables
         WHERE table_schema NOT IN ({placeholders})
         ORDER BY table_schema, table_name"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|source| query_error("relations", source))?;
    let rows = statement
        .query_map(params![INTERNAL_SCHEMAS[0], INTERNAL_SCHEMAS[1]], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|source| query_error("relations", source))?;

    let mut definitions = Vec::new();
    for row in rows {
        let (schema, name, table_type) = row.map_err(|source| query_error("relations", source))?;
        let columns = fetch_columns(connection, &schema, &name, enum_types)?;
        let object_name = CatalogObjectName::qualified(schema, name);
        let definition = if table_type == "VIEW" {
            RelationDefinition::view(object_name, columns)
        } else {
            RelationDefinition::table(object_name, columns)
        };
        definitions.push(definition);
    }
    Ok(definitions)
}

fn fetch_columns(
    connection: &Connection,
    schema: &str,
    table: &str,
    enum_types: &HashMap<EnumResolutionKey, String>,
) -> Result<Vec<ColumnDefinition>, InspectError> {
    let primary_keys = fetch_primary_keys(connection, schema, table)?;
    let mut statement = connection
        .prepare(
            "SELECT columns.database_oid, columns.schema_oid, columns.column_name, columns.data_type,
                    columns.is_nullable, columns.column_default
             FROM duckdb_columns() AS columns
             WHERE columns.schema_name = ? AND columns.table_name = ? AND NOT columns.internal
             ORDER BY columns.column_index",
        )
        .map_err(|source| query_error("columns", source))?;
    let rows = statement
        .query_map(params![schema, table], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|source| query_error("columns", source))?;

    let mut definitions = Vec::new();
    for row in rows {
        let (database_oid, schema_oid, name, raw_sql_type, nullable, default) =
            row.map_err(|source| query_error("columns", source))?;
        let primary_key = primary_keys.contains(&name);
        let resolved_sql_type = resolve_enum_sql_type(database_oid, schema_oid, &raw_sql_type, enum_types)
            .unwrap_or_else(|| raw_sql_type.clone());
        let mut definition = ColumnDefinition::new(name, raw_sql_type, nullable).resolved_sql_type(resolved_sql_type);
        if let Some(default) = default {
            definition = definition.default(default);
        }
        if primary_key {
            definition = definition.primary_key();
        }
        definitions.push(definition);
    }
    Ok(definitions)
}

fn fetch_primary_keys(connection: &Connection, schema: &str, table: &str) -> Result<HashSet<String>, InspectError> {
    let mut statement = connection
        .prepare(
            "SELECT key_column_usage.column_name
             FROM information_schema.table_constraints
             JOIN information_schema.key_column_usage USING (
                 constraint_catalog, constraint_schema, constraint_name,
                 table_catalog, table_schema, table_name
             )
             WHERE constraint_type = 'PRIMARY KEY'
               AND table_schema = ? AND table_name = ?",
        )
        .map_err(|source| query_error("primary-keys", source))?;
    let rows = statement
        .query_map(params![schema, table], |row| row.get::<_, String>(0))
        .map_err(|source| query_error("primary-keys", source))?;
    let mut primary_keys = HashSet::new();
    for row in rows {
        primary_keys.insert(row.map_err(|source| query_error("primary-keys", source))?);
    }
    Ok(primary_keys)
}

#[derive(Debug)]
struct InspectedEnum {
    database_oid: i64,
    schema_oid: i64,
    type_oid: i64,
    name: CatalogObjectName,
    values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnumIdentity {
    database_oid: i64,
    schema_oid: i64,
    type_oid: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EnumResolutionKey {
    database_oid: i64,
    schema_oid: i64,
    signature: String,
}

fn fetch_enums(connection: &Connection) -> Result<Vec<InspectedEnum>, InspectError> {
    let mut statement = connection
        .prepare(
            "SELECT database_oid, schema_oid, type_oid, schema_name, type_name, enum_value
             FROM duckdb_types(), UNNEST(labels) WITH ORDINALITY AS enum_values(enum_value, ordinal)
             WHERE logical_type = 'ENUM' AND NOT internal
             ORDER BY schema_name, type_name, ordinal",
        )
        .map_err(|source| query_error("enums", source))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|source| query_error("enums", source))?;

    let mut definitions = Vec::new();
    let mut current_identity: Option<EnumIdentity> = None;
    let mut current_name: Option<CatalogObjectName> = None;
    let mut current_values = Vec::new();
    for row in rows {
        let (database_oid, schema_oid, type_oid, schema, name, value) =
            row.map_err(|source| query_error("enums", source))?;
        let identity = EnumIdentity {
            database_oid,
            schema_oid,
            type_oid,
        };
        let object_name = CatalogObjectName::qualified(schema, name);
        if current_identity.as_ref().is_some_and(|current| current != &identity) {
            if let (Some(previous_identity), Some(previous_name)) =
                (current_identity.replace(identity), current_name.replace(object_name))
            {
                definitions.push(InspectedEnum {
                    database_oid: previous_identity.database_oid,
                    schema_oid: previous_identity.schema_oid,
                    type_oid: previous_identity.type_oid,
                    name: previous_name,
                    values: std::mem::take(&mut current_values),
                });
            }
        } else if current_identity.is_none() {
            current_identity = Some(identity);
            current_name = Some(object_name);
        }
        current_values.push(value);
    }
    if let (Some(identity), Some(name)) = (current_identity, current_name) {
        definitions.push(InspectedEnum {
            database_oid: identity.database_oid,
            schema_oid: identity.schema_oid,
            type_oid: identity.type_oid,
            name,
            values: current_values,
        });
    }
    Ok(definitions)
}

fn enum_type_resolutions(enums: &[InspectedEnum]) -> Result<HashMap<EnumResolutionKey, String>, InspectError> {
    let mut resolutions = HashMap::new();
    for definition in enums {
        let key = EnumResolutionKey {
            database_oid: definition.database_oid,
            schema_oid: definition.schema_oid,
            signature: enum_signature(&definition.values),
        };
        let qualified_name = match definition.name.schema() {
            Some(schema) => format!("{schema}.{}", definition.name.name()),
            None => definition.name.name().to_string(),
        };
        if let Some((previous_type_oid, previous_name)) =
            resolutions.insert(key, (definition.type_oid, qualified_name.clone()))
            && previous_type_oid != definition.type_oid
        {
            let mut types = vec![previous_name, qualified_name];
            types.sort();
            types.dedup();
            return Err(InspectError::AmbiguousEnumIdentity {
                schema: definition.name.schema().unwrap_or("main").to_string(),
                types,
            });
        }
    }
    Ok(resolutions
        .into_iter()
        .map(|(key, (_, qualified_name))| (key, qualified_name))
        .collect())
}

fn resolve_enum_sql_type(
    database_oid: i64,
    schema_oid: i64,
    raw_sql_type: &str,
    enum_types: &HashMap<EnumResolutionKey, String>,
) -> Option<String> {
    let (signature, suffix) = raw_sql_type
        .strip_suffix("[]")
        .map_or((raw_sql_type, ""), |signature| (signature, "[]"));
    let key = EnumResolutionKey {
        database_oid,
        schema_oid,
        signature: signature.to_string(),
    };
    enum_types
        .get(&key)
        .map(|qualified_name| format!("{qualified_name}{suffix}"))
}

fn enum_signature(values: &[String]) -> String {
    let labels = values
        .iter()
        .map(|value| format!("'{}'", value.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("ENUM({labels})")
}
