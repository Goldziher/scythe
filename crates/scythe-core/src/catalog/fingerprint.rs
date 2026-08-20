//! Deterministic schema fingerprinting, used for provenance headers (#68)
//! and drift detection (#79).
//!
//! [`Catalog::fingerprint`] reduces a parsed schema to a short, stable tag
//! that changes if and only if the schema *shape* changes. It deliberately
//! does **not** hash the DDL text: two schema files that differ only in
//! whitespace, comments, or top-level statement order must fingerprint
//! identically, while a real change (a column becoming nullable, a column
//! being reordered, a table being added) must not.
//!
//! This module is a child of [`super`] specifically so it can read
//! [`Catalog`]'s private fields directly — no new public accessors were
//! added to `Catalog` for this.

use ahash::AHashMap;

use sha2::{Digest, Sha256};

use crate::dialect::SqlDialect;

use super::{Catalog, CatalogObjectName, GeneratedColumnKind, RelationKind};

/// Version tag for the fingerprint algorithm itself.
///
/// The trigger for bumping this is **not** "the code in this module
/// changed" -- it is "the emitted `sch2:...` value moved for a schema that
/// did not change". Those are different things. `sch2` adds conditional
/// records for inspected metadata introduced with [`CatalogBuilder`]. The
/// records are absent for parser-equivalent defaults, but an inspected view,
/// generated column, raw type alias, or preserved object identity now moves
/// the hash. That canonical-form expansion requires a new tag so an `sch1`
/// artifact is never mistaken for one produced by the metadata-aware
/// algorithm.
///
/// Bump this when a canonical-form change moves the emitted value for a
/// schema that is otherwise unchanged (a new line kind whose absence was
/// previously indistinguishable from "no such construct", a changed
/// separator, a changed truncation length, etc.). Do not bump it merely
/// because this file's code changed.
const FINGERPRINT_ALGORITHM_TAG: &str = "sch2";

/// Number of leading hash bytes kept (rendered as `2 * TRUNCATED_BYTES` hex
/// characters).
const TRUNCATED_BYTES: usize = 8;

impl Catalog {
    /// Compute a deterministic fingerprint of this catalog's schema shape.
    ///
    /// The result is a short tag of the form `sch2:<16 hex chars>`. Two
    /// catalogs produce the same fingerprint if and only if they have the
    /// same tables (name, columns, column order, column type, column
    /// nullability, primary-key flags), the same enum types (name, ordered
    /// values), the same composite types (name, ordered fields), the same
    /// domain types (name, base type, `NOT NULL`-ness), and the same
    /// [`SqlDialect`].
    ///
    /// # Stability guarantees
    ///
    /// - **Reformat-invariant**: DDL text is never hashed directly, so
    ///   whitespace, comments, and top-level statement reordering that do
    ///   not change the schema produce the same fingerprint.
    /// - **Process-invariant**: map keys are sorted explicitly before
    ///   hashing. `Catalog`'s maps are [`AHashMap`], whose iteration order
    ///   is seeded once per process — relying on it would make the
    ///   fingerprint of an unchanged schema differ from one run to the
    ///   next.
    ///
    /// # What participates
    ///
    /// - Table names, and each column's name, resolved SQL type,
    ///   nullability, and primary-key flag, **in declared order** (column
    ///   order is positional and semantic, so it is never sorted).
    /// - Enum type names and their values, in declared order.
    /// - Composite type names and their fields (name + type), in declared
    ///   order.
    /// - Domain type names, their resolved base type, and their `NOT NULL`
    ///   flag. `domains` also feeds column-type resolution (see
    ///   `type_normalizer::normalize_data_type`),
    ///   but a domain used only as a query cast target, or one declared in a
    ///   different file than the table that uses it in a multi-file schema,
    ///   never surfaces through any table's resolved column type -- so it
    ///   needs its own line to be covered at all.
    /// - The dialect this catalog was parsed with, as the 6-variant
    ///   [`SqlDialect`] rather than the 9-way engine alias — so `mysql` and
    ///   `mariadb`, which both resolve to [`SqlDialect::MySQL`], never
    ///   register as drift against each other.
    /// - Inspected metadata that changes how a catalog is consumed: views,
    ///   generated-column persistence, materially distinct database-reported
    ///   types, and preserved qualified or case-sensitive object names.
    ///
    /// Table, enum, composite, and domain names all have a single leading
    /// `schema.` qualifier stripped before hashing, on every dialect --
    /// mirroring [`Catalog::get_table`]'s dialect-blind, qualifier-agnostic
    /// resolution (see [`canonical_entries`]). So `myschema.users` and
    /// `users`, or MSSQL's `dbo.users` and `users`, fingerprint identically
    /// when they would also resolve to the same lookup. Inspected catalogs
    /// additionally preserve the database-reported object identity; a
    /// qualified or case-sensitive spelling is therefore significant when
    /// that metadata is present.
    ///
    /// # What is excluded, deliberately
    ///
    /// - `Column.default`: it is `expr.to_string()` of the `sqlparser` AST,
    ///   which churns across `sqlparser` version bumps and has no downstream
    ///   consumer.
    /// - scythe's own version: it is not part of `Catalog` at all. If it
    ///   participated in this hash, every scythe release would make every
    ///   previously generated artifact report as drifted. The version still
    ///   appears in generated file headers, just as an independent field
    ///   alongside (not folded into) this fingerprint.
    pub fn fingerprint(&self) -> String {
        let canonical = self.canonical_form();
        let digest = Sha256::digest(canonical.as_bytes());
        let hex: String = digest[..TRUNCATED_BYTES].iter().map(|b| format!("{b:02x}")).collect();
        format!("{FINGERPRINT_ALGORITHM_TAG}:{hex}")
    }

    /// Render this catalog into a line-oriented, tab-separated canonical
    /// form suitable for hashing. Never render via `{:?}` (`Debug`) — that
    /// output is not a stable contract and can change on any dependency or
    /// compiler bump.
    ///
    /// Every user-controlled string (table/enum/composite/domain names,
    /// column names, SQL types, enum values, composite field names) is
    /// passed through [`escape_component`] before it is written into a
    /// line. Without that, a value containing this format's own delimiters
    /// (`\t`, `\n`, `|`, `:`) could either collide with an unrelated value
    /// (two different enums hashing the same) or forge extra fields or
    /// lines that were never in the schema. See [`escape_component`] for the
    /// scheme and why it was chosen over alternatives that would move
    /// already-shipped fingerprints.
    fn canonical_form(&self) -> String {
        let mut lines: Vec<String> = Vec::new();

        lines.push(format!("dialect\t{}", dialect_tag(self.dialect)));

        for (key, table) in canonical_entries(&self.tables) {
            let key = escape_component(&key);
            lines.push(format!("table\t{key}\t{}", table.columns.len()));
            for (idx, column) in table.columns.iter().enumerate() {
                lines.push(format!(
                    "column\t{key}\t{idx}\t{}\t{}\t{}\t{}",
                    escape_component(&column.name),
                    escape_component(&column.sql_type),
                    column.nullable,
                    column.primary_key
                ));
            }
        }

        for (key, enum_type) in canonical_entries(&self.enums) {
            let values = enum_type
                .values
                .iter()
                .map(|value| escape_component(value))
                .collect::<Vec<_>>()
                .join("|");
            lines.push(format!("enum\t{}\t{values}", escape_component(&key)));
        }

        for (key, composite) in canonical_entries(&self.composites) {
            let fields = composite
                .fields
                .iter()
                .map(|field| {
                    format!(
                        "{}:{}",
                        escape_component(&field.name),
                        escape_component(&field.sql_type)
                    )
                })
                .collect::<Vec<_>>()
                .join("|");
            lines.push(format!("composite\t{}\t{fields}", escape_component(&key)));
        }

        for (key, domain) in canonical_entries(&self.domains) {
            lines.push(format!(
                "domain\t{}\t{}\t{}",
                escape_component(&key),
                escape_component(&domain.base_type),
                domain.not_null
            ));
        }

        self.append_inspection_metadata(&mut lines);

        lines.join("\n")
    }

    fn append_inspection_metadata(&self, lines: &mut Vec<String>) {
        append_relation_metadata(self, lines);
        append_preserved_names(lines, "relation", &self.relation_names);
        append_preserved_names(lines, "enum", &self.enum_names);
        append_preserved_names(lines, "composite", &self.composite_names);
        append_preserved_names(lines, "domain", &self.domain_names);
        append_raw_domain_types(self, lines);
    }
}

fn append_relation_metadata(catalog: &Catalog, lines: &mut Vec<String>) {
    for (key, kind) in sorted_entries(&catalog.relation_kinds) {
        if *kind == RelationKind::View {
            lines.push(format!("relation-kind\t{}\tview", escape_component(key)));
        }
    }

    for (relation_key, raw_types) in sorted_entries(&catalog.raw_column_types) {
        for (column_key, raw_type) in sorted_entries(raw_types) {
            let normalized_type = catalog
                .tables
                .get(relation_key)
                .and_then(|table| {
                    table
                        .columns
                        .iter()
                        .find(|column| column.name.trim().to_lowercase() == *column_key)
                })
                .map(|column| column.sql_type.as_str());
            if normalized_type.is_none_or(|resolved| !equivalent_sql_type(raw_type, resolved)) {
                lines.push(format!(
                    "raw-column-type\t{}\t{}\t{}",
                    escape_component(relation_key),
                    escape_component(column_key),
                    escape_component(raw_type)
                ));
            }
        }
    }

    for (relation_key, generated_kinds) in sorted_entries(&catalog.generated_column_kinds) {
        for (column_key, kind) in sorted_entries(generated_kinds) {
            lines.push(format!(
                "generated-column\t{}\t{}\t{}",
                escape_component(relation_key),
                escape_component(column_key),
                generated_column_kind_tag(*kind)
            ));
        }
    }
}

fn append_preserved_names(lines: &mut Vec<String>, object_kind: &str, names: &AHashMap<String, CatalogObjectName>) {
    for (key, name) in sorted_entries(names) {
        if name.schema().is_none() && name.name() == key {
            continue;
        }
        lines.push(format!(
            "object-name\t{}\t{}\t{}\t{}",
            object_kind,
            escape_component(key),
            escape_component(name.schema().unwrap_or("")),
            escape_component(name.name())
        ));
    }
}

fn append_raw_domain_types(catalog: &Catalog, lines: &mut Vec<String>) {
    for (key, raw_type) in sorted_entries(&catalog.raw_domain_types) {
        let normalized_type = catalog.domains.get(key).map(|domain| domain.base_type.as_str());
        if normalized_type.is_none_or(|resolved| !equivalent_sql_type(raw_type, resolved)) {
            lines.push(format!(
                "raw-domain-type\t{}\t{}",
                escape_component(key),
                escape_component(raw_type)
            ));
        }
    }
}

fn sorted_entries<T>(map: &AHashMap<String, T>) -> Vec<(&str, &T)> {
    let mut entries: Vec<(&str, &T)> = map.iter().map(|(key, value)| (key.as_str(), value)).collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
}

fn equivalent_sql_type(raw: &str, resolved: &str) -> bool {
    raw.trim().to_lowercase() == resolved
}

fn generated_column_kind_tag(kind: GeneratedColumnKind) -> &'static str {
    match kind {
        GeneratedColumnKind::Virtual => "virtual",
        GeneratedColumnKind::Stored => "stored",
    }
}

/// Stable string tag for a dialect. Deliberately distinct from
/// [`SqlDialect::from_str`]'s engine-alias parsing (`mariadb`, `crdb`,
/// `duckdb`, `redshift`, ... all collapse to one of these 6 variants before
/// this function ever sees them).
fn dialect_tag(dialect: SqlDialect) -> &'static str {
    match dialect {
        SqlDialect::PostgreSQL => "postgresql",
        SqlDialect::MySQL => "mysql",
        SqlDialect::SQLite => "sqlite",
        SqlDialect::MsSql => "mssql",
        SqlDialect::Oracle => "oracle",
        SqlDialect::Snowflake => "snowflake",
    }
}

/// Return `(key, value)` pairs from `map`, sorted by key, with a single
/// leading schema qualifier stripped — mirroring [`Catalog::get_table`]
/// (`catalog/mod.rs`'s `get_table`), which already treats `myschema.users`
/// and `users` as the same table.
///
/// Two things distinguish this from a naive `public.`-only strip:
///
/// - **Every dialect, not just PostgreSQL.** `get_table` never checks
///   `self.dialect` at all — it splits on `.` and looks up the remainder
///   unconditionally. MSSQL's `dbo.` prefix, or any other schema name on any
///   dialect, resolves exactly the same way `public.` does on PostgreSQL. A
///   fingerprint gated on `dialect == SqlDialect::PostgreSQL` would treat
///   `dbo.users` and `users` as different tables even though `get_table`
///   resolves them to the same one — a guaranteed false-positive drift
///   report the moment someone adds or drops that qualifier.
/// - **Any qualifier text, not just the literal `public`.** `get_table`
///   splits on the first `.` and looks up whatever comes after it,
///   regardless of what the prefix says. So `myschema.users` and `users`
///   are the same lookup too. Stripping only `public.` would leave every
///   other schema name unmirrored.
///
/// If stripping would make two distinct raw keys collide (e.g. both
/// `myschema.users` and a literal bare `users` exist in the same catalog),
/// stripping is abandoned for the whole map and raw keys are used instead.
/// Merging colliding entries would silently drop one of them from the
/// fingerprint; falling back to raw keys keeps both distinguishable. This
/// fallback triggers more often now that any prefix is stripped rather than
/// only `public.` — that is the correct, intended consequence of matching
/// `get_table`'s broader normalization, not a regression.
///
/// A stripped or colliding-fallback key can still collide with another raw
/// key at the hashing layer if it contains one of `canonical_form`'s
/// reserved delimiter bytes; [`escape_component`] is applied to every key
/// this function returns before it reaches a line, independently of this
/// normalization.
fn canonical_entries<T>(map: &AHashMap<String, T>) -> Vec<(String, &T)> {
    let stripped: Vec<(String, &T)> = map
        .iter()
        .map(|(key, value)| (strip_leading_qualifier(key), value))
        .collect();

    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::with_capacity(stripped.len());
    let collides = stripped.iter().any(|(key, _)| !seen.insert(key.as_str()));

    let mut entries: Vec<(String, &T)> = if collides {
        map.iter().map(|(key, value)| (key.clone(), value)).collect()
    } else {
        stripped
    };

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

/// Strip a single leading `schema.` qualifier, if present, by splitting on
/// the first `.` and keeping the remainder — exactly what
/// [`Catalog::get_table`]'s `split_once('.')` branch does when resolving a
/// qualified name. A key with no `.` at all is returned unchanged.
fn strip_leading_qualifier(key: &str) -> String {
    key.split_once('.')
        .map_or_else(|| key.to_string(), |(_, rest)| rest.to_string())
}

/// Escape-only-when-needed encoding for a single user-controlled string
/// before it is written into a [`canonical_form`] line.
///
/// `canonical_form` uses five reserved bytes as structure: `\t` separates
/// fields within a line, `\n` separates lines, and `|` / `:` separate
/// sub-components within a field (enum values, composite `name:type`
/// pairs). Any of the five appearing verbatim inside a *value* — a table,
/// column, enum, composite, or domain name; an enum value; a resolved SQL
/// type — would either forge structure that was never in the schema (a
/// `\t` or `\n` inside a name splitting or adding a line) or make two
/// different schemas collide on one hash (an enum with the single value
/// `"a|b"` versus an enum with two values `"a"` and `"b"` previously both
/// rendered as `a|b`). This function makes the encoding injective by
/// escaping exactly those five bytes with a leading backslash, and copying
/// every other byte through unchanged:
///
/// - `\` becomes `\\`
/// - `|` becomes `\|`
/// - `:` becomes `\:`
/// - `\t` becomes `\t` (the two-character sequence backslash-t, not a
///   literal tab)
/// - `\n` becomes `\n` (backslash-n, not a literal newline)
///
/// This is a single left-to-right pass over `value`'s characters — each
/// input character is matched and emitted exactly once — so there is no
/// "escape the backslash first" ordering concern at all. That ordering only
/// matters for the sequential "find `|`, replace with `\|`; then find `\`,
/// replace with `\\`" style of escaping, where the second pass would
/// re-escape backslashes the first pass just introduced. A single pass over
/// the original characters cannot re-match its own output, so it is
/// injective by construction: decoding (reversing the map) recovers `value`
/// exactly, which is what makes two differently-shaped inputs guaranteed to
/// render as different escaped output.
///
/// # Why not length-prefixing or percent-encoding
///
/// Both are cleaner in isolation, and both are wrong here: they rewrite
/// every value, including the overwhelming majority that contain none of
/// the five reserved bytes. That moves the emitted hash for schemas that
/// did not change. `Catalog::fingerprint`'s `sch2:` tag is compared as an
/// opaque string by `verify_provenance`, with no migration path — so a
/// scheme that isn't the identity function on delimiter-free input would
/// hand every existing user a false `scythe check` drift failure. Escape-
/// only-when-needed is the identity function on exactly that input, which
/// is why [`FINGERPRINT_ALGORITHM_TAG`] does not need to move for this fix.
fn escape_component(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '|' => escaped.push_str("\\|"),
            ':' => escaped.push_str("\\:"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use crate::catalog::{
        CatalogBuilder, CatalogObjectName, ColumnDefinition, CompositeDefinition, DomainDefinition, EnumDefinition,
        GeneratedColumnKind, RelationDefinition,
    };
    use crate::dialect::SqlDialect;

    use super::Catalog;

    #[test]
    fn test_reformatted_ddl_produces_same_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL);"])
        .unwrap();

        let b = Catalog::from_ddl(
            &["-- orders first this time, extra whitespace and comments throughout\n\
             CREATE   TABLE   orders (\n  id INTEGER PRIMARY KEY, -- pk\n  user_id INTEGER NOT NULL\n);\n\n\
             /* users table */\n\
             CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);"],
        )
        .unwrap();

        assert_eq!(
            a.fingerprint(),
            b.fingerprint(),
            "whitespace, comments, and top-level statement order must not affect the fingerprint"
        );
    }

    #[test]
    fn test_nullability_change_produces_different_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER, name TEXT);"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER, name TEXT NOT NULL);"]).unwrap();

        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn test_column_reorder_produces_different_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE t (a INTEGER, b TEXT);"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TABLE t (b TEXT, a INTEGER);"]).unwrap();

        assert_ne!(
            a.fingerprint(),
            b.fingerprint(),
            "column order is positional and must be part of the hash"
        );
    }

    #[test]
    fn test_column_default_change_produces_same_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER, age INTEGER DEFAULT 0);"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER, age INTEGER DEFAULT 1);"]).unwrap();

        assert_eq!(
            a.fingerprint(),
            b.fingerprint(),
            "Column.default must be excluded from the fingerprint"
        );
    }

    #[test]
    fn test_inspected_column_default_change_produces_same_hash() {
        let build = |default: &str| {
            CatalogBuilder::new(SqlDialect::SQLite)
                .relation(RelationDefinition::table(
                    CatalogObjectName::new("items"),
                    vec![ColumnDefinition::new("quantity", "INTEGER", false).default(default)],
                ))
                .build()
                .expect("valid inspected catalog")
        };

        assert_eq!(
            build("0").fingerprint(),
            build("1").fingerprint(),
            "inspected Column.default must remain excluded from the fingerprint"
        );
    }

    #[test]
    fn test_inspected_table_and_view_produce_different_hashes() {
        let table = CatalogBuilder::new(SqlDialect::SQLite)
            .relation(RelationDefinition::table(
                CatalogObjectName::new("items"),
                vec![ColumnDefinition::new("id", "INTEGER", false)],
            ))
            .build()
            .expect("valid inspected table");
        let view = CatalogBuilder::new(SqlDialect::SQLite)
            .relation(RelationDefinition::view(
                CatalogObjectName::new("items"),
                vec![ColumnDefinition::new("id", "INTEGER", false)],
            ))
            .build()
            .expect("valid inspected view");

        assert_ne!(table.fingerprint(), view.fingerprint());
    }

    #[test]
    fn test_inspected_virtual_and_stored_columns_produce_different_hashes() {
        let build = |kind| {
            CatalogBuilder::new(SqlDialect::SQLite)
                .relation(RelationDefinition::table(
                    CatalogObjectName::new("items"),
                    vec![ColumnDefinition::new("computed", "INTEGER", false).generated(kind)],
                ))
                .build()
                .expect("valid inspected catalog")
        };

        assert_ne!(
            build(GeneratedColumnKind::Virtual).fingerprint(),
            build(GeneratedColumnKind::Stored).fingerprint()
        );
    }

    #[test]
    fn test_materially_distinct_raw_column_type_changes_hash() {
        let build = |raw_type: &str| {
            CatalogBuilder::new(SqlDialect::PostgreSQL)
                .relation(RelationDefinition::table(
                    CatalogObjectName::new("items"),
                    vec![ColumnDefinition::new("id", raw_type, false).resolved_sql_type("integer")],
                ))
                .build()
                .expect("valid inspected catalog")
        };

        assert_ne!(build("int4").fingerprint(), build("serial4").fingerprint());
    }

    #[test]
    fn test_materially_distinct_raw_domain_type_changes_hash() {
        let build = || {
            CatalogBuilder::new(SqlDialect::PostgreSQL)
                .domain(DomainDefinition::new(
                    CatalogObjectName::new("identifier"),
                    "bigint",
                    false,
                ))
                .build()
                .expect("valid inspected catalog")
        };
        let mut int8 = build();
        let mut serial8 = build();
        int8.raw_domain_types
            .insert("identifier".to_string(), "int8".to_string());
        serial8
            .raw_domain_types
            .insert("identifier".to_string(), "serial8".to_string());

        assert_ne!(int8.fingerprint(), serial8.fingerprint());
    }

    #[test]
    fn test_preserved_qualified_and_caseful_names_change_hash() {
        let bare = CatalogBuilder::new(SqlDialect::PostgreSQL)
            .relation(RelationDefinition::table(
                CatalogObjectName::new("items"),
                vec![ColumnDefinition::new("id", "integer", false)],
            ))
            .enum_type(EnumDefinition::new(
                CatalogObjectName::new("mood"),
                vec!["happy".to_string()],
            ))
            .composite(CompositeDefinition::new(CatalogObjectName::new("address"), vec![]))
            .domain(DomainDefinition::new(
                CatalogObjectName::new("identifier"),
                "bigint",
                false,
            ))
            .build()
            .expect("valid normalized catalog");
        let preserved = CatalogBuilder::new(SqlDialect::PostgreSQL)
            .relation(RelationDefinition::table(
                CatalogObjectName::qualified("Public", "Items"),
                vec![ColumnDefinition::new("id", "integer", false)],
            ))
            .enum_type(EnumDefinition::new(
                CatalogObjectName::new("Mood"),
                vec!["happy".to_string()],
            ))
            .composite(CompositeDefinition::new(CatalogObjectName::new("Address"), vec![]))
            .domain(DomainDefinition::new(
                CatalogObjectName::new("Identifier"),
                "bigint",
                false,
            ))
            .build()
            .expect("valid preserved-name catalog");

        assert_ne!(bare.fingerprint(), preserved.fingerprint());
    }

    #[test]
    fn test_default_inspection_metadata_remains_parser_compatible() {
        let parsed =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE items (id INTEGER NOT NULL);"], &SqlDialect::SQLite)
                .expect("valid DDL catalog");
        let inspected = CatalogBuilder::new(SqlDialect::SQLite)
            .relation(RelationDefinition::table(
                CatalogObjectName::new("items"),
                vec![ColumnDefinition::new("id", "INTEGER", false)],
            ))
            .build()
            .expect("valid inspected catalog");

        assert_eq!(parsed.fingerprint(), inspected.fingerprint());
    }

    #[test]
    fn test_table_added_produces_different_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER);"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER); CREATE TABLE u (id INTEGER);"]).unwrap();

        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn test_enum_and_composite_participate() {
        let a = Catalog::from_ddl(&["CREATE TYPE mood AS ENUM ('sad', 'happy');"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TYPE mood AS ENUM ('sad', 'happy', 'ok');"]).unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint());

        let c = Catalog::from_ddl(&["CREATE TYPE address AS (street TEXT, city TEXT);"]).unwrap();
        let d = Catalog::from_ddl(&["CREATE TYPE address AS (street TEXT, city TEXT, zip INTEGER);"]).unwrap();
        assert_ne!(c.fingerprint(), d.fingerprint());
    }

    #[test]
    fn test_dialect_participates() {
        let pg = Catalog::from_ddl_with_dialect(&["CREATE TABLE t (id INTEGER);"], &SqlDialect::PostgreSQL).unwrap();
        let mysql = Catalog::from_ddl_with_dialect(&["CREATE TABLE t (id INT);"], &SqlDialect::MySQL).unwrap();

        assert_ne!(pg.fingerprint(), mysql.fingerprint());
    }

    #[test]
    fn test_mysql_and_mariadb_alias_share_one_dialect_variant() {
        // ~keep `mariadb` is not a distinct `SqlDialect` variant; `from_str` folds
        // it into `SqlDialect::MySQL`, so there is nothing further for the
        // fingerprint to distinguish -- this test documents that guarantee
        // at the alias-resolution boundary the fingerprint depends on.
        assert_eq!(SqlDialect::from_str("mysql"), SqlDialect::from_str("mariadb"));
    }

    #[test]
    fn test_public_prefix_stripped_when_no_collision() {
        let a = Catalog::from_ddl_with_dialect(&["CREATE TABLE public.users (id INTEGER);"], &SqlDialect::PostgreSQL)
            .unwrap();
        let b = Catalog::from_ddl_with_dialect(&["CREATE TABLE users (id INTEGER);"], &SqlDialect::PostgreSQL).unwrap();

        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn test_public_schema_collision_falls_back_to_raw_keys() {
        let base = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE public.users (id INTEGER); CREATE TABLE users (name TEXT);"],
            &SqlDialect::PostgreSQL,
        )
        .unwrap();
        let fp_base = base.fingerprint();

        let changed_bare = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE public.users (id INTEGER); CREATE TABLE users (name TEXT, extra BOOLEAN);"],
            &SqlDialect::PostgreSQL,
        )
        .unwrap();
        assert_ne!(
            fp_base,
            changed_bare.fingerprint(),
            "changing the bare `users` table must change the hash even though `public.users` also exists"
        );

        let changed_qualified = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE public.users (id INTEGER, extra BOOLEAN); CREATE TABLE users (name TEXT);"],
            &SqlDialect::PostgreSQL,
        )
        .unwrap();
        assert_ne!(
            fp_base,
            changed_qualified.fingerprint(),
            "changing `public.users` must change the hash even though bare `users` also exists"
        );
    }

    #[test]
    fn test_schema_qualifier_stripped_on_non_postgresql_dialect() {
        // ~keep `get_table` (catalog/mod.rs) never checks `self.dialect` before
        // splitting on `.` -- MSSQL's `dbo.` prefix resolves to the bare
        // table exactly like PostgreSQL's `public.` does. Gating the strip
        // on `dialect == SqlDialect::PostgreSQL` (the pre-fix behavior)
        // would fingerprint `dbo.users` and `users` differently even though
        // `get_table` treats them as the same table.
        let qualified =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE dbo.users (id INTEGER);"], &SqlDialect::MsSql).unwrap();
        let bare = Catalog::from_ddl_with_dialect(&["CREATE TABLE users (id INTEGER);"], &SqlDialect::MsSql).unwrap();

        assert_eq!(
            qualified.fingerprint(),
            bare.fingerprint(),
            "a schema-qualified table must fingerprint the same as its bare name on every dialect, not just PostgreSQL"
        );
    }

    #[test]
    fn test_non_public_schema_qualifier_stripped_under_postgresql() {
        // The pre-fix strip only recognized the literal `public.` prefix.
        // `get_table` strips *any* single leading qualifier, so
        // `myschema.users` and `users` must fingerprint the same too.
        let qualified =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE myschema.users (id INTEGER);"], &SqlDialect::PostgreSQL)
                .unwrap();
        let bare =
            Catalog::from_ddl_with_dialect(&["CREATE TABLE users (id INTEGER);"], &SqlDialect::PostgreSQL).unwrap();

        assert_eq!(
            qualified.fingerprint(),
            bare.fingerprint(),
            "any leading schema qualifier must be stripped under PostgreSQL, not only the literal `public.`"
        );
    }

    #[test]
    fn test_non_public_schema_collision_falls_back_to_raw_keys_on_other_dialects() {
        // Mirrors `test_public_schema_collision_falls_back_to_raw_keys`, but
        // exercises the collision fallback for a non-`public`, non-PostgreSQL
        // qualifier -- the fallback is now reachable from every dialect and
        // every qualifier text, not just PostgreSQL's `public.`.
        let base = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE dbo.users (id INTEGER); CREATE TABLE users (name TEXT);"],
            &SqlDialect::MsSql,
        )
        .unwrap();
        let fp_base = base.fingerprint();

        let changed_bare = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE dbo.users (id INTEGER); CREATE TABLE users (name TEXT, extra BOOLEAN);"],
            &SqlDialect::MsSql,
        )
        .unwrap();
        assert_ne!(
            fp_base,
            changed_bare.fingerprint(),
            "changing the bare `users` table must change the hash even though `dbo.users` also exists"
        );

        let changed_qualified = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE dbo.users (id INTEGER, extra BOOLEAN); CREATE TABLE users (name TEXT);"],
            &SqlDialect::MsSql,
        )
        .unwrap();
        assert_ne!(
            fp_base,
            changed_qualified.fingerprint(),
            "changing `dbo.users` must change the hash even though bare `users` also exists"
        );
    }

    #[test]
    fn test_enum_pipe_value_does_not_collide_with_split_values() {
        // ~keep Pre-fix, `values.join("|")` rendered a single value `"a|b"` and
        // two values `"a"`, `"b"` as the byte-identical string `a|b` -- a
        // real drift (one value became two, or vice versa) that produced no
        // change in the fingerprint at all. This is the collision the issue
        // calls out as the most important case: it makes SC-PRV01 silently
        // blind, not merely a false positive.
        let one_value_with_pipe = Catalog::from_ddl(&["CREATE TYPE t AS ENUM ('a|b');"]).unwrap();
        let two_values = Catalog::from_ddl(&["CREATE TYPE t AS ENUM ('a', 'b');"]).unwrap();

        assert_ne!(
            one_value_with_pipe.fingerprint(),
            two_values.fingerprint(),
            "an enum with one value containing a literal `|` must not fingerprint the same as \
             a different enum with two values split at that `|`"
        );
    }

    #[test]
    fn test_composite_field_colon_does_not_collide_across_name_type_boundary() {
        // Pre-fix, a composite field rendered as `{name}:{sql_type}` with an
        // unescaped `:`. A field named `a` typed the (quoted, unregistered)
        // custom type `"b:c"` and a field named the (quoted) `"a:b"` typed
        // `c` both rendered to the identical string `a:b:c` -- two
        // differently-shaped composites, one fingerprint.
        let colon_in_type = Catalog::from_ddl(&["CREATE TYPE addr AS (a \"b:c\");"]).unwrap();
        let colon_in_name = Catalog::from_ddl(&["CREATE TYPE addr AS (\"a:b\" c);"]).unwrap();

        assert_ne!(
            colon_in_type.fingerprint(),
            colon_in_name.fingerprint(),
            "a composite field name or type containing `:` must not fingerprint the same as a \
             differently-shaped field whose unescaped `name:type` rendering is byte-identical"
        );
    }

    #[test]
    fn test_enum_value_with_tab_and_newline_cannot_forge_a_composite_line() {
        // The sharpest version of "a value containing a tab or newline can
        // forge an entire extra record": an enum value containing `\n` and
        // `\t` is crafted so that, rendered unescaped, it reads as the
        // enum's own line followed by a second, syntactically valid
        // `composite\t...` line for a composite that does not exist in this
        // catalog at all. A schema with that one enum must not fingerprint
        // the same as an unrelated schema that genuinely declares both the
        // enum and the composite.
        let smuggled_value = "z\ncomposite\taddr\tstreet:text";
        let forged_ddl = format!("CREATE TYPE e AS ENUM ('{smuggled_value}');");
        let forged = Catalog::from_ddl(&[forged_ddl.as_str()]).unwrap();

        let genuine =
            Catalog::from_ddl(&["CREATE TYPE e AS ENUM ('z');", "CREATE TYPE addr AS (street TEXT);"]).unwrap();

        assert_ne!(
            forged.fingerprint(),
            genuine.fingerprint(),
            "a tab/newline-bearing enum value must not be able to forge what looks like an \
             unrelated composite's canonical-form line"
        );
    }

    #[test]
    fn test_domain_base_type_change_produces_different_hash() {
        // `Catalog.domains` feeds `normalize_data_type` but, pre-fix, never
        // appeared in `canonical_form` itself. A domain used only as a query
        // cast target, or declared in a different file than the table that
        // uses it, never surfaces through any table's resolved column type
        // -- so without its own line, changing its base type moved nothing.
        let a = Catalog::from_ddl(&["CREATE DOMAIN email AS TEXT;"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE DOMAIN email AS VARCHAR(255);"]).unwrap();

        assert_ne!(
            a.fingerprint(),
            b.fingerprint(),
            "a domain's base type must participate in the fingerprint even when no table column resolves through it"
        );
    }

    /// Golden fingerprints, pinned at the values 0.14.0 shipped.
    ///
    /// Every other test in this module is *relative* — it asserts that two
    /// catalogs hash the same, or differently, and would keep passing if the
    /// canonical form changed shape and moved every emitted value at once.
    /// That is exactly the change this pin exists to catch.
    ///
    /// `verify_provenance` compares the algorithm tag as part of an opaque string. So a
    /// canonical-form edit that alters the emitted hash for a schema that did
    /// not change hands every existing user a `scythe check` failure reporting
    /// schema drift that is not real, until they regenerate. These values are
    /// the contract that says the emitted bytes did not move.
    ///
    /// Each case covers one construct the canonical form renders, so a change
    /// to any single line format is localized by which case fails:
    /// - `table`/`column` lines, including nullability and primary-key flags
    /// - `enum` lines, whose values are joined with `|`
    /// - `composite` lines, whose fields are joined with `|` and `:`
    /// - the `dialect` line
    /// - the PostgreSQL `public.` qualifier strip
    ///
    /// **Updating these values is never routine.** A deliberate algorithm
    /// change must bump `FINGERPRINT_ALGORITHM_TAG` in the same commit, so old
    /// and new fingerprints are never mistaken for one another — that is what
    /// the tag is for. If you are here because a change you did not intend to
    /// be observable moved a value, the change was observable; fix the change.
    #[test]
    fn test_fingerprints_are_pinned_to_their_released_values() {
        let cases: &[(&str, SqlDialect, &str)] = &[
            (
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, bio TEXT);",
                SqlDialect::PostgreSQL,
                "sch2:4bf6bb703d5818da",
            ),
            (
                "CREATE TYPE status AS ENUM ('active', 'inactive', 'banned');",
                SqlDialect::PostgreSQL,
                "sch2:b23bd2728dc1df1c",
            ),
            (
                "CREATE TYPE address AS (street TEXT, city TEXT, zip INTEGER);",
                SqlDialect::PostgreSQL,
                "sch2:08277bec474dde8b",
            ),
            (
                "CREATE TABLE public.users (id INTEGER PRIMARY KEY);",
                SqlDialect::PostgreSQL,
                "sch2:83901ff72944cf53",
            ),
            (
                "CREATE TABLE t (id INT NOT NULL, note VARCHAR(255));",
                SqlDialect::MySQL,
                "sch2:d1b6623bd34edc4b",
            ),
            (
                "CREATE TABLE t (id INTEGER NOT NULL, note TEXT);",
                SqlDialect::SQLite,
                "sch2:4eb52891d7937ab9",
            ),
        ];

        for (ddl, dialect, expected) in cases {
            let catalog = Catalog::from_ddl_with_dialect(&[ddl], dialect).unwrap_or_else(|e| panic!("{ddl}: {e}"));
            assert_eq!(
                catalog.fingerprint(),
                *expected,
                "pinned fingerprint moved for {ddl:?} -- see this test's doc comment before updating it"
            );
        }
    }

    /// Line prefix the child half prints its fingerprint behind. Shared by
    /// both halves so the writer and the reader cannot drift.
    const CHILD_FINGERPRINT_PREFIX: &str = "SCH2_CHILD_FINGERPRINT=";

    /// Fully qualified name of [`cross_process_child_prints_fingerprint`],
    /// as libtest's `--exact` filter spells it.
    const CHILD_TEST_NAME: &str = "catalog::fingerprint::tests::cross_process_child_prints_fingerprint";

    /// The child half of [`test_cross_process_ordering_independence`]:
    /// prints this process's fingerprint of the shared sample catalog.
    ///
    /// Split out into its own `#[test]` rather than living behind an
    /// environment-variable branch inside the parent. With an env-gated
    /// branch, a `SCYTHE_FINGERPRINT_CHILD` that happened to already be set
    /// in the ambient environment sent the *parent* run down the child path:
    /// it printed a fingerprint, returned, and asserted nothing at all,
    /// while still reporting as a pass. Which process is the child is now a
    /// property of which test libtest was asked to run — something only the
    /// parent decides, via `--exact`, and something no inherited environment
    /// can forge.
    ///
    /// Running this on its own (as a normal `cargo test` sweep does) is
    /// harmless and asserts nothing; it exists to be spawned.
    #[test]
    fn cross_process_child_prints_fingerprint() {
        println!(
            "{CHILD_FINGERPRINT_PREFIX}{}",
            cross_process_sample_catalog().fingerprint()
        );
    }

    /// Cross-process ordering independence.
    ///
    /// The failure this guards against is a `canonical_form` that leaked
    /// `AHashMap` iteration order into the hash, making the same unchanged
    /// schema fingerprint differently from one run to the next — which would
    /// report as permanent, unfixable SC-PRV01 drift in CI.
    ///
    /// A second process is required to catch it. `ahash`'s `RandomState`
    /// draws on two sources, and both differ across processes: a set of
    /// fixed seeds initialized once per process, and a per-instance value
    /// from `DefaultRandomSource::gen_hasher_seed`, which is a static
    /// counter seeded from a static's (ASLR-dependent) address. Note the
    /// per-instance half means the claim that a single process's maps share
    /// one iteration order is false — two `AHashMap`s built in the same
    /// process already disagree, so an in-process comparison would be
    /// *probabilistic*: it could catch an order-dependent `canonical_form`,
    /// but only by luck, and it would say nothing about the per-process
    /// fixed seeds that are the actual run-to-run variable. Spawning a real
    /// second process exercises both sources at once, which is why the
    /// design is right regardless.
    ///
    /// This re-invokes the current test binary (`current_exe()`), asking it
    /// to run [`cross_process_child_prints_fingerprint`] and nothing else,
    /// then compares what the child printed against its own in-process
    /// fingerprint.
    #[test]
    fn test_cross_process_ordering_independence() {
        let parent_fingerprint = cross_process_sample_catalog().fingerprint();

        let exe = std::env::current_exe().expect("current test binary path");
        let output = std::process::Command::new(exe)
            .arg("--exact")
            .arg(CHILD_TEST_NAME)
            .arg("--nocapture")
            .output()
            .expect("failed to spawn child test process");

        assert!(
            output.status.success(),
            "child process failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let child_fingerprint = stdout
            .lines()
            .find_map(|line| line.strip_prefix(CHILD_FINGERPRINT_PREFIX))
            .unwrap_or_else(|| {
                panic!(
                    "child process did not print a fingerprint -- if `{CHILD_TEST_NAME}` was renamed, \
                     update CHILD_TEST_NAME. stdout was:\n{stdout}"
                )
            });

        assert_eq!(
            parent_fingerprint, child_fingerprint,
            "fingerprint must be identical across independently seeded processes"
        );
    }

    /// A catalog with enough distinct map keys (tables, enums, composites)
    /// that two independently seeded `AHashMap` random states are very
    /// likely to disagree on raw iteration order -- giving the
    /// cross-process test something real to catch.
    fn cross_process_sample_catalog() -> Catalog {
        Catalog::from_ddl(&["CREATE TABLE alpha (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE bravo (id INTEGER PRIMARY KEY, alpha_id INTEGER NOT NULL);
             CREATE TABLE charlie (id INTEGER PRIMARY KEY, note TEXT);
             CREATE TABLE delta (id INTEGER PRIMARY KEY, amount NUMERIC(10,2));
             CREATE TABLE echo (id INTEGER PRIMARY KEY, active BOOLEAN NOT NULL);
             CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
             CREATE TYPE status AS ENUM ('pending', 'done');
             CREATE TYPE address AS (street TEXT, city TEXT, zip INTEGER);
             CREATE TYPE point AS (x INTEGER, y INTEGER);"])
        .unwrap()
    }
}
