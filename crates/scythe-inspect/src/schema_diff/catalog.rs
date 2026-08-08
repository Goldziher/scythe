//! Reduce scythe's DDL-derived [`Catalog`] to a [`SchemaDescription`].
//!
//! This is the half of the drift comparison that needs no database. It runs
//! over exactly the catalog `scythe generate` compiles against, so a drift
//! finding is a statement about the code scythe would emit, not about some
//! parallel reading of the DDL.

use scythe_core::analyzer::sql_type_to_neutral;
use scythe_core::catalog::Catalog;

use crate::error::InspectError;

use super::model::{ColumnDescription, EnumDescription, SchemaDescription, TableDescription, object_key};

/// Describe the schema scythe parsed from committed DDL.
///
/// Every column gets a neutral type: [`sql_type_to_neutral`] is total, falling
/// back to `string` for a DDL type it does not recognise. The `None`
/// ("cannot compare") case therefore only ever arrives from the live side,
/// where a PostgreSQL type outside scythe's neutral vocabulary really is
/// unmappable.
///
/// `nullability_is_authoritative` is always `true` here. Scythe's catalog
/// stores views in the same table map as ordinary tables and does not record
/// which is which, so the decision to skip nullability on views is taken from
/// the live side's `relkind` instead — see
/// [`TableDescription::nullability_is_authoritative`].
///
/// # Errors
///
/// [`InspectError::AmbiguousSchemaObject`] when two tables or two enum types
/// in the DDL reduce to the same bare comparison key — `tenant_a.orders` and
/// `tenant_b.orders`, say. The live side breaks that tie by search-path
/// position; the DDL has no search path, so picking one would be a guess that
/// silently reports the loser's columns as drift.
pub fn describe_catalog(catalog: &Catalog) -> Result<SchemaDescription, InspectError> {
    let mut description = SchemaDescription::new();

    // `tables_iter`/`enums_iter` walk an `AHashMap` whose seed is randomised
    // per process, so both the surviving entry on a collision and the names in
    // the resulting error message would otherwise vary between runs. Sorting
    // first makes the outcome identical on every run, which is the difference
    // between a reproducible failure and a CI job that is only sometimes red.
    let mut tables: Vec<_> = catalog.tables_iter().collect();
    tables.sort_unstable_by_key(|(name, _)| *name);

    for (name, table) in tables {
        let mut described = TableDescription::new(name.clone());
        for column in &table.columns {
            described = described.with_column(ColumnDescription::new(
                column.name.clone(),
                sql_type_to_neutral(&column.sql_type, catalog).into_owned(),
                column.nullable,
            ));
        }

        let key = object_key(name);
        if let Some(existing) = description.tables.get(&key) {
            return Err(ambiguous("tables", &key, &existing.display_name, name));
        }
        description.tables.insert(key, described);
    }

    let mut enums: Vec<_> = catalog.enums_iter().collect();
    enums.sort_unstable_by_key(|(name, _)| *name);

    for (name, enum_type) in enums {
        let key = object_key(name);
        if let Some(existing) = description.enums.get(&key) {
            return Err(ambiguous("enum types", &key, &existing.display_name, name));
        }
        description
            .enums
            .insert(key, EnumDescription::new(name.clone(), enum_type.values.clone()));
    }

    Ok(description)
}

fn ambiguous(kind: &'static str, key: &str, first: &str, second: &str) -> InspectError {
    InspectError::AmbiguousSchemaObject {
        kind,
        key: key.to_string(),
        first: first.to_string(),
        second: second.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::dialect::SqlDialect;

    fn catalog_from(ddl: &str) -> Catalog {
        Catalog::from_ddl_with_dialect(&[ddl], &SqlDialect::PostgreSQL).expect("catalog from ddl")
    }

    #[test]
    fn describes_columns_with_neutral_types_and_nullability() {
        let catalog = catalog_from(
            "CREATE TABLE users (
                 id    integer PRIMARY KEY,
                 email text NOT NULL,
                 bio   text
             );",
        );
        let description = describe_catalog(&catalog).expect("describe catalog");

        let users = &description.tables["users"];
        assert_eq!(users.columns["id"].neutral_type.as_deref(), Some("int32"));
        assert!(!users.columns["email"].nullable);
        assert!(users.columns["bio"].nullable);
    }

    /// A schema-qualified `CREATE TABLE` must land on the same key a live
    /// `public`.`users` does, or the table is reported as missing from both
    /// sides at once.
    #[test]
    fn schema_qualified_tables_key_on_the_bare_name() {
        let description =
            describe_catalog(&catalog_from("CREATE TABLE public.users (id integer);")).expect("describe catalog");
        assert!(description.tables.contains_key("users"));
        assert_eq!(
            description.tables["users"].display_name, "public.users",
            "the message must still show the qualified name the DDL wrote"
        );
    }

    #[test]
    fn describes_enum_values_in_declaration_order() {
        let catalog = catalog_from("CREATE TYPE status AS ENUM ('active', 'banned');");
        let description = describe_catalog(&catalog).expect("describe catalog");
        assert_eq!(description.enums["status"].values, vec!["active", "banned"]);
    }

    /// A catalog built from no DDL at all describes an empty schema rather
    /// than panicking, so `scythe check` on a config with an empty schema glob
    /// reports every live table as SC-DRF02 instead of failing.
    #[test]
    fn an_empty_catalog_describes_an_empty_schema() {
        let description = describe_catalog(&catalog_from("")).expect("describe catalog");
        assert_eq!(description, SchemaDescription::new());
    }

    /// The DDL side never yields `None`: `sql_type_to_neutral` is total.
    #[test]
    fn every_ddl_column_gets_a_neutral_type() {
        let catalog = catalog_from("CREATE TABLE t (a integer, b text, c timestamptz, d numeric(10,2));");
        let description = describe_catalog(&catalog).expect("describe catalog");
        for column in description.tables["t"].columns.values() {
            assert!(
                column.neutral_type.is_some(),
                "column {} has no neutral type",
                column.name
            );
        }
    }

    /// Views land in the catalog's table map alongside real tables, which is
    /// why the drift check must not restrict the live query to `relkind='r'`:
    /// doing so would report every view as a missing table.
    #[test]
    fn views_are_described_as_tables() {
        let catalog = catalog_from(
            "CREATE TABLE users (id integer, active boolean NOT NULL);
             CREATE VIEW active_users AS SELECT id, active FROM users;",
        );
        let description = describe_catalog(&catalog).expect("describe catalog");
        assert!(description.tables.contains_key("active_users"));
    }

    /// Two same-named tables in different schemas collapse onto one bare
    /// comparison key. Silently keeping whichever the hash map happened to
    /// yield would report the loser's every column as drift, and would report
    /// a *different* table's columns on the next run, because the catalog's
    /// `AHashMap` seed is randomised per process.
    #[test]
    fn same_named_tables_in_two_schemas_are_reported_as_ambiguous() {
        let catalog = catalog_from(
            "CREATE TABLE tenant_a.orders (id integer);
             CREATE TABLE tenant_b.orders (id integer);",
        );

        let error = describe_catalog(&catalog).expect_err("a collision must not be resolved by guessing");

        let InspectError::AmbiguousSchemaObject {
            kind,
            key,
            first,
            second,
        } = error
        else {
            panic!("expected AmbiguousSchemaObject, got {error:?}");
        };
        assert_eq!(kind, "tables");
        assert_eq!(key, "orders");
        assert_eq!(first, "tenant_a.orders");
        assert_eq!(second, "tenant_b.orders");
    }

    /// The same collision among enum types, which SC-DRF07 compares on the
    /// same bare key.
    #[test]
    fn same_named_enums_in_two_schemas_are_reported_as_ambiguous() {
        let catalog = catalog_from(
            "CREATE TYPE tenant_a.status AS ENUM ('active');
             CREATE TYPE tenant_b.status AS ENUM ('banned');",
        );

        let error = describe_catalog(&catalog).expect_err("a collision must not be resolved by guessing");
        assert!(
            matches!(&error, InspectError::AmbiguousSchemaObject { kind, key, .. }
                if *kind == "enum types" && key == "status"),
            "got {error:?}"
        );
    }

    /// The error must name the same two objects in the same order on every
    /// run, or a collision surfaces as an intermittently-worded CI failure.
    /// Sorting before the walk is what guarantees it; the catalog's own
    /// iteration order does not.
    #[test]
    fn the_ambiguity_error_is_identical_across_repeated_runs() {
        let ddl = "CREATE TABLE tenant_b.orders (id integer);
                   CREATE TABLE tenant_a.orders (id integer);
                   CREATE TABLE tenant_c.orders (id integer);";

        let rendered: Vec<String> = (0..8)
            .map(|_| describe_catalog(&catalog_from(ddl)).expect_err("collision").to_string())
            .collect();

        assert!(
            rendered.windows(2).all(|pair| pair[0] == pair[1]),
            "the message varied between runs: {rendered:?}"
        );
        assert!(rendered[0].contains("tenant_a.orders"), "{}", rendered[0]);
        assert!(rendered[0].contains("tenant_b.orders"), "{}", rendered[0]);
    }
}
