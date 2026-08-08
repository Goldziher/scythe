//! Read a live PostgreSQL schema out of `pg_catalog` into a
//! [`SchemaDescription`].
//!
//! All of the drift check's I/O is here, and none of its logic. Fetching and
//! comparing are separate so that [`diff`](super::diff::diff) can be an
//! ordinary pure function — every rule, every skip and every edge case is
//! covered by unit tests that never open a socket, and the live tests only
//! have to prove that this module reads the catalog correctly.
//!
//! ## Why `pg_catalog` and not `information_schema`
//!
//! `information_schema.columns` reports `USER-DEFINED` as the data type of
//! every enum column and never names the type, which makes enum drift
//! (SC-DRF07) and any enum-typed column mismatch (SC-DRF05) undetectable.
//! `pg_catalog` carries the type OID, so the type can be resolved properly.

use std::collections::HashMap;

use tokio_postgres::Client;
use tokio_postgres::types::{Kind, Type};

use crate::error::InspectError;
use crate::verify::pg_types::neutral_type_for;

use super::model::{ColumnDescription, EnumDescription, SchemaDescription, TableDescription, object_key};

/// Label used for the `check_id` of any [`InspectError::Query`] raised here.
const DRIFT_CHECK_ID: &str = "schema-drift";

/// Relation kinds the drift check considers a "table".
///
/// Views (`v`) and materialized views (`m`) are included because scythe's
/// catalog stores views in the same table map as ordinary tables; restricting
/// this to `r` would report every view in the DDL as missing from the
/// database. Partitioned tables (`p`) and foreign tables (`f`) are included
/// for the same reason — they are ordinary tables as far as a query is
/// concerned.
///
/// Inlined into the SQL below rather than bound as a parameter: it is a
/// compile-time constant, never user input, and `"char"[]` is awkward to bind
/// from Rust.
const RELATION_KINDS: &str = "'r', 'p', 'v', 'm', 'f'";

/// Relation kinds whose `attnotnull` flags describe the relation itself.
///
/// PostgreSQL stores `attnotnull = false` for every column of a view or
/// materialized view, whatever the underlying table declares. Trusting that
/// would report a nullability mismatch on every non-null column of every view.
fn nullability_is_authoritative(relation_kind: &str) -> bool {
    !matches!(relation_kind, "v" | "m")
}

/// A row of `pg_type`, kept so a type OID can be resolved into a
/// [`tokio_postgres::types::Type`] without a second round trip per column.
#[derive(Debug, Clone)]
struct PgTypeRow {
    name: String,
    schema: String,
    /// `pg_type.typtype`: `b` base, `e` enum, `d` domain, `c` composite, …
    kind: String,
    /// `pg_type.typbasetype`, non-zero only for domains.
    base_type_oid: u32,
    /// `pg_type.typelem`, the element type of an array.
    element_type_oid: u32,
    /// `pg_type.typcategory`; `A` marks an array type.
    category: String,
}

/// How many levels of domain and array nesting a type may be resolved through.
///
/// PostgreSQL cannot actually produce a cycle here, but resolution recurses
/// through catalog rows this process does not control, and an unbounded
/// recursion over corrupt or unexpected catalog contents would blow the stack
/// rather than skip one column.
///
/// Counted as levels *entered*: the outermost type is depth 0, so resolution
/// runs exactly this many levels and no more.
const MAX_TYPE_RESOLUTION_DEPTH: usize = 16;

/// Fetch the schemas on the connection's `search_path`, then everything they
/// contain that drift compares.
///
/// Scoped to `search_path` rather than to every non-system schema because that
/// is exactly the set a query on this connection resolves against. Scanning
/// every schema would report every table of every unrelated tenant, extension
/// or staging schema as SC-DRF02.
pub async fn fetch_live_schema(client: &Client) -> Result<SchemaDescription, InspectError> {
    let types = fetch_types(client).await?;
    let enum_labels = fetch_enum_labels(client).await?;
    let search_path = fetch_search_path(client).await?;

    let mut description = SchemaDescription::new();
    fetch_tables(client, &types, &enum_labels, &mut description).await?;
    collect_enums(&types, &enum_labels, &search_path, &mut description);

    Ok(description)
}

/// Wrap a failed catalog query, naming the step that failed.
///
/// The four queries fail for different reasons — a role without `SELECT` on
/// `pg_enum` is a different problem from one that cannot read `pg_class` — and
/// a report that says only "schema-drift failed" leaves the user guessing
/// which grant is missing.
fn query_error(step: &'static str, error: tokio_postgres::Error) -> InspectError {
    InspectError::Query {
        engine: "postgres",
        check_id: format!("{DRIFT_CHECK_ID}/{step}"),
        source: Box::new(error),
    }
}

async fn fetch_search_path(client: &Client) -> Result<Vec<String>, InspectError> {
    let rows = client
        .query(
            "SELECT schema_name::text AS schema_name \
             FROM unnest(current_schemas(false)) AS schema_name",
            &[],
        )
        .await
        .map_err(|e| query_error("search-path", e))?;

    Ok(rows.iter().map(|row| row.get::<_, String>("schema_name")).collect())
}

async fn fetch_types(client: &Client) -> Result<HashMap<u32, PgTypeRow>, InspectError> {
    let rows = client
        .query(
            "SELECT t.oid                AS type_oid, \
                    t.typname::text      AS type_name, \
                    n.nspname::text      AS type_schema, \
                    t.typtype::text      AS type_kind, \
                    t.typbasetype        AS base_type_oid, \
                    t.typelem            AS element_type_oid, \
                    t.typcategory::text  AS type_category \
             FROM pg_catalog.pg_type t \
             JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace",
            &[],
        )
        .await
        .map_err(|e| query_error("pg_type", e))?;

    Ok(rows
        .iter()
        .map(|row| {
            (
                row.get::<_, u32>("type_oid"),
                PgTypeRow {
                    name: row.get("type_name"),
                    schema: row.get("type_schema"),
                    kind: row.get("type_kind"),
                    base_type_oid: row.get("base_type_oid"),
                    element_type_oid: row.get("element_type_oid"),
                    category: row.get("type_category"),
                },
            )
        })
        .collect())
}

async fn fetch_enum_labels(client: &Client) -> Result<HashMap<u32, Vec<String>>, InspectError> {
    let rows = client
        .query(
            "SELECT e.enumtypid       AS type_oid, \
                    e.enumlabel::text AS label \
             FROM pg_catalog.pg_enum e \
             ORDER BY e.enumtypid, e.enumsortorder",
            &[],
        )
        .await
        .map_err(|e| query_error("pg_enum", e))?;

    let mut labels: HashMap<u32, Vec<String>> = HashMap::new();
    for row in &rows {
        labels
            .entry(row.get::<_, u32>("type_oid"))
            .or_default()
            .push(row.get("label"));
    }
    Ok(labels)
}

async fn fetch_tables(
    client: &Client,
    types: &HashMap<u32, PgTypeRow>,
    enum_labels: &HashMap<u32, Vec<String>>,
    description: &mut SchemaDescription,
) -> Result<(), InspectError> {
    let sql = format!(
        "SELECT n.nspname::text AS schema_name, \
                c.relname::text  AS relation_name, \
                c.relkind::text  AS relation_kind, \
                a.attname::text  AS column_name, \
                a.attnotnull     AS not_null, \
                a.atttypid       AS type_oid, \
                array_position(current_schemas(false), n.nspname) AS schema_rank \
         FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_catalog.pg_attribute a ON a.attrelid = c.oid \
         WHERE c.relkind IN ({RELATION_KINDS}) \
           AND n.nspname = ANY (current_schemas(false)) \
           AND a.attnum > 0 \
           AND NOT a.attisdropped \
         ORDER BY schema_rank, c.relname, a.attnum"
    );

    let rows = client
        .query(sql.as_str(), &[])
        .await
        .map_err(|e| query_error("pg_class", e))?;

    // A table name can appear in more than one schema on the search path. The
    // rows are ordered by search-path position, so the first occurrence of a
    // name is the one an unqualified query would resolve to; the shadowed
    // copies are dropped rather than merged, which would otherwise invent a
    // table carrying both schemas' columns.
    let mut winning_rank: HashMap<String, i32> = HashMap::new();

    for row in &rows {
        let relation: String = row.get("relation_name");
        let rank: i32 = row.get("schema_rank");
        let key = object_key(&relation);

        match winning_rank.get(&key) {
            Some(&winner) if winner != rank => continue,
            Some(_) => {}
            None => {
                let schema: String = row.get("schema_name");
                let relation_kind: String = row.get("relation_kind");
                let mut table = TableDescription::new(format!("{schema}.{relation}"));
                if !nullability_is_authoritative(&relation_kind) {
                    table = table.without_authoritative_nullability();
                }
                winning_rank.insert(key.clone(), rank);
                description.tables.insert(key.clone(), table);
            }
        }

        let type_oid: u32 = row.get("type_oid");
        let not_null: bool = row.get("not_null");
        let column = ColumnDescription {
            name: row.get("column_name"),
            neutral_type: resolve_type(type_oid, types, enum_labels, 0)
                .as_ref()
                .and_then(neutral_type_for),
            nullable: !not_null,
        };

        if let Some(table) = description.tables.get_mut(&key) {
            table.columns.insert(column.name.to_lowercase(), column);
        }
    }

    Ok(())
}

/// Collect the enum types visible on the search path.
///
/// Two schemas on the search path may each declare an enum with the same bare
/// name, which collapses onto one comparison key. The winner is the one in the
/// earliest search-path schema — the type an unqualified reference would
/// resolve to — exactly as [`fetch_tables`] resolves the same collision for
/// relations. Without that rule the survivor would depend on `HashMap`
/// iteration order, whose seed is randomised per process: SC-DRF07 would fire
/// on some runs and swallow real enum drift on others.
fn collect_enums(
    types: &HashMap<u32, PgTypeRow>,
    enum_labels: &HashMap<u32, Vec<String>>,
    search_path: &[String],
    description: &mut SchemaDescription,
) {
    let mut winning_rank: HashMap<String, usize> = HashMap::new();

    for (oid, row) in types {
        if row.kind != "e" {
            continue;
        }
        let Some(rank) = search_path.iter().position(|schema| *schema == row.schema) else {
            continue;
        };
        let Some(values) = enum_labels.get(oid) else {
            continue;
        };

        let key = object_key(&row.name);
        // A tie is impossible: an equal rank means the same schema, and
        // PostgreSQL will not hold two types of one name in one schema.
        if winning_rank.get(&key).is_some_and(|&winner| winner <= rank) {
            continue;
        }

        winning_rank.insert(key.clone(), rank);
        description.enums.insert(
            key,
            EnumDescription::new(format!("{}.{}", row.schema, row.name), values.clone()),
        );
    }
}

/// Rebuild a [`Type`] from catalog metadata so [`neutral_type_for`] can map it.
///
/// Going through `Type` rather than mapping OIDs to neutral names directly is
/// what keeps a single type table in the codebase: `neutral_type_for` already
/// knows how to unwrap arrays, enums and domains and how to name every scalar,
/// and a second mapping here could disagree with the one `verify_queries`
/// uses.
///
/// Returns `None` for anything that cannot be reconstructed — a composite, a
/// range over a user type, a domain whose base is itself unresolvable — which
/// the caller records as "cannot compare" rather than as drift.
fn resolve_type(
    oid: u32,
    types: &HashMap<u32, PgTypeRow>,
    enum_labels: &HashMap<u32, Vec<String>>,
    depth: usize,
) -> Option<Type> {
    if depth >= MAX_TYPE_RESOLUTION_DEPTH {
        return None;
    }

    // Built-in OIDs resolve straight to the canonical `Type`, including the
    // built-in array types, so only user-defined types reach the catalog rows.
    if let Some(builtin) = Type::from_oid(oid) {
        return Some(builtin);
    }

    let row = types.get(&oid)?;

    match row.kind.as_str() {
        "e" => Some(Type::new(
            row.name.clone(),
            oid,
            Kind::Enum(enum_labels.get(&oid).cloned().unwrap_or_default()),
            row.schema.clone(),
        )),
        "d" => {
            let base = resolve_type(row.base_type_oid, types, enum_labels, depth + 1)?;
            Some(Type::new(row.name.clone(), oid, Kind::Domain(base), row.schema.clone()))
        }
        _ if row.category == "A" && row.element_type_oid != 0 => {
            let element = resolve_type(row.element_type_oid, types, enum_labels, depth + 1)?;
            Some(Type::new(
                row.name.clone(),
                oid,
                Kind::Array(element),
                row.schema.clone(),
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enum_type_row() -> PgTypeRow {
        PgTypeRow {
            name: "status".to_string(),
            schema: "public".to_string(),
            kind: "e".to_string(),
            base_type_oid: 0,
            element_type_oid: 0,
            category: "E".to_string(),
        }
    }

    /// The constraint that makes the whole check work on views: restricting to
    /// ordinary tables would report every view scythe's catalog holds as a
    /// missing table.
    #[test]
    fn relation_kinds_cover_tables_partitions_views_matviews_and_foreign_tables() {
        for kind in ["'r'", "'p'", "'v'", "'m'", "'f'"] {
            assert!(RELATION_KINDS.contains(kind), "relkind {kind} must be selected");
        }
    }

    #[test]
    fn views_and_matviews_have_no_authoritative_nullability() {
        assert!(!nullability_is_authoritative("v"));
        assert!(!nullability_is_authoritative("m"));
    }

    #[test]
    fn ordinary_partitioned_and_foreign_tables_have_authoritative_nullability() {
        assert!(nullability_is_authoritative("r"));
        assert!(nullability_is_authoritative("p"));
        assert!(nullability_is_authoritative("f"));
    }

    #[test]
    fn built_in_oids_resolve_without_consulting_the_catalog() {
        let resolved = resolve_type(Type::INT4.oid(), &HashMap::new(), &HashMap::new(), 0);
        assert_eq!(resolved.as_ref().and_then(neutral_type_for).as_deref(), Some("int32"));
    }

    #[test]
    fn an_enum_oid_resolves_to_the_enum_neutral_form() {
        let oid = 100_000;
        let types = HashMap::from([(oid, enum_type_row())]);
        let labels = HashMap::from([(oid, vec!["active".to_string(), "banned".to_string()])]);

        let resolved = resolve_type(oid, &types, &labels, 0).expect("enum resolves");
        assert_eq!(neutral_type_for(&resolved).as_deref(), Some("enum::status"));
    }

    /// A domain is a constrained alias with no wire representation of its own,
    /// so it must compare as its base type — otherwise `CREATE DOMAIN us_zip
    /// AS text` reads as drift against a `text` column.
    #[test]
    fn a_domain_resolves_to_its_base_type() {
        let oid = 100_001;
        let types = HashMap::from([(
            oid,
            PgTypeRow {
                name: "us_zip".to_string(),
                schema: "public".to_string(),
                kind: "d".to_string(),
                base_type_oid: Type::TEXT.oid(),
                element_type_oid: 0,
                category: "S".to_string(),
            },
        )]);

        let resolved = resolve_type(oid, &types, &HashMap::new(), 0).expect("domain resolves");
        assert_eq!(neutral_type_for(&resolved).as_deref(), Some("string"));
    }

    /// An array of a user-defined enum has no built-in OID, so it has to be
    /// rebuilt from `typelem` before `neutral_type_for` can name it.
    #[test]
    fn an_array_of_an_enum_resolves_to_the_array_neutral_form() {
        let enum_oid = 100_000;
        let array_oid = 100_002;
        let types = HashMap::from([
            (enum_oid, enum_type_row()),
            (
                array_oid,
                PgTypeRow {
                    name: "_status".to_string(),
                    schema: "public".to_string(),
                    kind: "b".to_string(),
                    base_type_oid: 0,
                    element_type_oid: enum_oid,
                    category: "A".to_string(),
                },
            ),
        ]);
        let labels = HashMap::from([(enum_oid, vec!["active".to_string()])]);

        let resolved = resolve_type(array_oid, &types, &labels, 0).expect("array resolves");
        assert_eq!(neutral_type_for(&resolved).as_deref(), Some("array<enum::status>"));
    }

    /// An OID that is neither built in nor present in `pg_type` yields `None`,
    /// which the caller records as "cannot compare" — never as drift.
    #[test]
    fn an_unknown_oid_resolves_to_none() {
        assert!(resolve_type(999_999, &HashMap::new(), &HashMap::new(), 0).is_none());
    }

    /// A composite type cannot be rebuilt from these columns alone, so it must
    /// skip rather than guess.
    #[test]
    fn a_composite_type_resolves_to_none() {
        let oid = 100_003;
        let types = HashMap::from([(
            oid,
            PgTypeRow {
                name: "address".to_string(),
                schema: "public".to_string(),
                kind: "c".to_string(),
                base_type_oid: 0,
                element_type_oid: 0,
                category: "C".to_string(),
            },
        )]);
        assert!(resolve_type(oid, &types, &HashMap::new(), 0).is_none());
    }

    /// Resolution recurses through catalog rows this process does not control,
    /// so a self-referential domain must stop at the depth bound instead of
    /// exhausting the stack.
    #[test]
    fn a_self_referential_domain_stops_at_the_depth_bound() {
        let oid = 100_004;
        let types = HashMap::from([(
            oid,
            PgTypeRow {
                name: "loop".to_string(),
                schema: "public".to_string(),
                kind: "d".to_string(),
                base_type_oid: oid,
                element_type_oid: 0,
                category: "S".to_string(),
            },
        )]);
        assert!(resolve_type(oid, &types, &HashMap::new(), 0).is_none());
    }

    /// Only enums on the search path become comparable enum descriptions; an
    /// enum in some unrelated schema is not part of what a query would see.
    #[test]
    fn collect_enums_keeps_only_search_path_enums() {
        let visible_oid = 100_000;
        let hidden_oid = 100_005;
        let mut hidden = enum_type_row();
        hidden.name = "other_status".to_string();
        hidden.schema = "archive".to_string();

        let types = HashMap::from([(visible_oid, enum_type_row()), (hidden_oid, hidden)]);
        let labels = HashMap::from([
            (visible_oid, vec!["active".to_string()]),
            (hidden_oid, vec!["gone".to_string()]),
        ]);

        let mut description = SchemaDescription::new();
        collect_enums(&types, &labels, &["public".to_string()], &mut description);

        assert_eq!(description.enums.len(), 1);
        assert_eq!(description.enums["status"].display_name, "public.status");
    }

    /// Two search-path schemas each declaring `status` collapse onto one
    /// comparison key. The earliest search-path schema wins — the type an
    /// unqualified reference resolves to — matching how `fetch_tables`
    /// resolves the same collision for relations. Left to `HashMap` iteration
    /// order the survivor would vary per process, making SC-DRF07 fire on some
    /// runs and swallow real enum drift on others.
    #[test]
    fn collect_enums_resolves_a_name_collision_by_search_path_position() {
        let first_oid = 100_000;
        let second_oid = 100_006;
        let mut shadowed = enum_type_row();
        shadowed.schema = "archive".to_string();

        let types = HashMap::from([(first_oid, enum_type_row()), (second_oid, shadowed)]);
        let labels = HashMap::from([
            (first_oid, vec!["active".to_string()]),
            (second_oid, vec!["gone".to_string()]),
        ]);
        let search_path = ["public".to_string(), "archive".to_string()];

        let mut description = SchemaDescription::new();
        collect_enums(&types, &labels, &search_path, &mut description);

        assert_eq!(description.enums.len(), 1);
        assert_eq!(description.enums["status"].display_name, "public.status");
        assert_eq!(description.enums["status"].values, vec!["active"]);
    }

    /// The same two enums with the search path reversed must yield the other
    /// one — proving the winner tracks search-path position and is not just
    /// whichever the map happened to visit first.
    #[test]
    fn collect_enums_follows_the_search_path_order_not_the_map_order() {
        let public_oid = 100_000;
        let archive_oid = 100_006;
        let mut archived = enum_type_row();
        archived.schema = "archive".to_string();

        let types = HashMap::from([(public_oid, enum_type_row()), (archive_oid, archived)]);
        let labels = HashMap::from([
            (public_oid, vec!["active".to_string()]),
            (archive_oid, vec!["gone".to_string()]),
        ]);

        let mut description = SchemaDescription::new();
        collect_enums(
            &types,
            &labels,
            &["archive".to_string(), "public".to_string()],
            &mut description,
        );

        assert_eq!(description.enums["status"].display_name, "archive.status");
    }

    /// Repeating the collision must give the same answer every time; the map
    /// is seeded randomly per process but the rank rule is not.
    #[test]
    fn collect_enums_is_deterministic_across_repeated_runs() {
        let public_oid = 100_000;
        let archive_oid = 100_006;
        let mut archived = enum_type_row();
        archived.schema = "archive".to_string();

        let types = HashMap::from([(public_oid, enum_type_row()), (archive_oid, archived)]);
        let labels = HashMap::from([
            (public_oid, vec!["active".to_string()]),
            (archive_oid, vec!["gone".to_string()]),
        ]);
        let search_path = ["public".to_string(), "archive".to_string()];

        for _ in 0..16 {
            let mut description = SchemaDescription::new();
            collect_enums(&types, &labels, &search_path, &mut description);
            assert_eq!(description.enums["status"].display_name, "public.status");
        }
    }

    /// Exactly `MAX_TYPE_RESOLUTION_DEPTH` levels are entered, no more.
    ///
    /// A chain of N domains over a base type occupies N + 1 levels — the
    /// domains at depths `0..N`, then the base type at depth `N` — so the
    /// longest chain that resolves has `MAX_TYPE_RESOLUTION_DEPTH - 1`
    /// domains. Pinning both sides of that boundary is what stops the constant
    /// and the guard drifting apart again.
    #[test]
    fn domain_chains_resolve_up_to_the_depth_bound_and_no_further() {
        fn domain_chain(length: u32) -> HashMap<u32, PgTypeRow> {
            let base_oid = 200_000;
            (0..length)
                .map(|step| {
                    let oid = base_oid + step;
                    let base_type_oid = if step + 1 == length { Type::TEXT.oid() } else { oid + 1 };
                    (
                        oid,
                        PgTypeRow {
                            name: format!("d{step}"),
                            schema: "public".to_string(),
                            kind: "d".to_string(),
                            base_type_oid,
                            element_type_oid: 0,
                            category: "S".to_string(),
                        },
                    )
                })
                .collect()
        }

        let longest_that_fits = MAX_TYPE_RESOLUTION_DEPTH as u32 - 1;

        let within = domain_chain(longest_that_fits);
        assert_eq!(
            resolve_type(200_000, &within, &HashMap::new(), 0)
                .as_ref()
                .and_then(neutral_type_for)
                .as_deref(),
            Some("string"),
            "a chain filling exactly the bound must still resolve to its base type"
        );

        let beyond = domain_chain(longest_that_fits + 1);
        assert!(
            resolve_type(200_000, &beyond, &HashMap::new(), 0).is_none(),
            "a chain one level past the bound must stop rather than recurse"
        );
    }
}
