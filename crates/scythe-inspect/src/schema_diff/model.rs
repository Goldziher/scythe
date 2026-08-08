//! The neutral schema description both sides of a drift comparison are
//! reduced to.
//!
//! Drift is a comparison between two things that are described very
//! differently: a [`scythe_core::catalog::Catalog`] parsed from committed DDL,
//! and rows read out of `pg_catalog`. Reducing both to one description is what
//! lets [`diff`](super::diff) be pure and synchronous — the interesting logic
//! (which rule fires, and when a comparison must be skipped) then unit-tests
//! against hand-built descriptions with no database anywhere near it.

use std::collections::BTreeMap;

/// One side of a drift comparison: the tables and enum types a schema
/// contains, keyed by their lookup name.
///
/// `BTreeMap` rather than a hash map so findings come out in a stable order.
/// A drift report that reshuffles between runs is unreadable in a CI diff and
/// impossible to snapshot-test.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaDescription {
    /// Tables (and views) keyed by [`object_key`].
    pub tables: BTreeMap<String, TableDescription>,
    /// Enum types keyed by lowercase bare type name.
    pub enums: BTreeMap<String, EnumDescription>,
}

impl SchemaDescription {
    /// An empty description — no tables, no enums.
    pub fn new() -> Self {
        Self::default()
    }
}

/// A table or view, with the columns it exposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDescription {
    /// Name as it should appear in a finding message — schema-qualified when
    /// the source knew the schema (`public.users`), bare otherwise.
    pub display_name: String,
    /// Columns keyed by lowercase column name.
    pub columns: BTreeMap<String, ColumnDescription>,
    /// Whether this side's `nullable` flags mean anything for this relation.
    ///
    /// `false` for views and materialized views: PostgreSQL stores
    /// `attnotnull = false` for every view column regardless of what the
    /// underlying table declares, while scythe's catalog copies the base
    /// column's `NOT NULL` through into the view. Comparing the two would
    /// report a nullability mismatch on every non-null column of every view —
    /// a flood of false positives that would bury the real SC-DRF06 hits.
    pub nullability_is_authoritative: bool,
}

impl TableDescription {
    /// A table whose nullability flags are trustworthy (the ordinary case).
    pub fn new(display_name: impl Into<String>) -> Self {
        Self {
            display_name: display_name.into(),
            columns: BTreeMap::new(),
            nullability_is_authoritative: true,
        }
    }

    /// Add a column, keyed by its lowercased name.
    pub fn with_column(mut self, column: ColumnDescription) -> Self {
        self.columns.insert(column.name.to_lowercase(), column);
        self
    }

    /// Mark this relation's nullability as not comparable (a view).
    pub fn without_authoritative_nullability(mut self) -> Self {
        self.nullability_is_authoritative = false;
        self
    }
}

/// A single column, reduced to the two properties drift compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDescription {
    /// Column name as written in its source, used for messages.
    pub name: String,
    /// scythe's neutral type name, or `None` when this side's type has no
    /// neutral equivalent.
    ///
    /// `None` means "cannot compare", never "mismatch". A PostgreSQL type
    /// scythe has no opinion about — `xml`, `point`, an extension type — is
    /// not evidence that the schema drifted, and reporting it as one would
    /// make the check fire constantly on schemas it does not understand.
    pub neutral_type: Option<String>,
    /// Whether the column accepts NULL.
    pub nullable: bool,
}

impl ColumnDescription {
    /// A column with a known neutral type.
    pub fn new(name: impl Into<String>, neutral_type: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            neutral_type: Some(neutral_type.into()),
            nullable,
        }
    }

    /// A column whose type has no neutral equivalent, so type comparison must
    /// be skipped for it.
    pub fn unmappable(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            neutral_type: None,
            nullable,
        }
    }
}

/// An enum type and the values it admits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDescription {
    /// Name as it should appear in a finding message.
    pub display_name: String,
    /// The enum's values, in declaration order.
    pub values: Vec<String>,
}

impl EnumDescription {
    /// Build an enum description from its display name and values.
    pub fn new(display_name: impl Into<String>, values: Vec<String>) -> Self {
        Self {
            display_name: display_name.into(),
            values,
        }
    }
}

/// The key a table or enum is matched on across the two sides of a comparison.
///
/// Both sides are reduced to the bare, lowercased name. Scythe's catalog
/// stores whatever the DDL wrote — `users` from `CREATE TABLE users`,
/// `public.users` from `CREATE TABLE public.users` — while `pg_catalog` always
/// knows the schema. Matching on the qualified name would report the same
/// table as both missing from the database (SC-DRF01) and missing from the DDL
/// (SC-DRF02) purely because one side spelled the schema out.
pub fn object_key(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.rsplit_once('.') {
        Some((_schema, bare)) => bare.to_string(),
        None => lower,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_key_strips_the_schema_qualifier() {
        assert_eq!(object_key("public.users"), "users");
        assert_eq!(object_key("users"), "users");
    }

    #[test]
    fn object_key_lowercases() {
        assert_eq!(object_key("Public.Users"), "users");
        assert_eq!(object_key("USERS"), "users");
    }

    /// The whole point of stripping the schema: `CREATE TABLE public.users`
    /// and a live `public`.`users` must land on the same key, or the table is
    /// reported as simultaneously missing from both sides.
    #[test]
    fn qualified_and_bare_spellings_share_a_key() {
        assert_eq!(object_key("public.users"), object_key("users"));
    }

    #[test]
    fn with_column_keys_on_the_lowercased_name() {
        let table = TableDescription::new("users").with_column(ColumnDescription::new("Id", "int32", false));
        assert!(table.columns.contains_key("id"));
        assert_eq!(table.columns["id"].name, "Id");
    }

    #[test]
    fn unmappable_column_has_no_neutral_type() {
        assert_eq!(ColumnDescription::unmappable("shape", true).neutral_type, None);
    }
}
