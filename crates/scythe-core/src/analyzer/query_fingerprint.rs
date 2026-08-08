//! Deterministic fingerprinting of an *analyzed query set*, used for the
//! `queries=` field of provenance headers (#94).
//!
//! [`Catalog::fingerprint`](crate::catalog::Catalog::fingerprint) answers
//! "was this generated from this schema?" but says nothing about the query
//! files that produced the generated functions themselves -- editing a
//! `.sql` query file without touching the schema produces no drift signal
//! at all under the schema fingerprint alone. This module closes that gap
//! the same way: [`AnalyzedQuery::fingerprint_set`] reduces a set of
//! analyzed queries to a short, stable tag that changes if and only if
//! something that affects *generated output* changes.
//!
//! Deliberately modeled on `catalog::fingerprint` as closely as possible --
//! same line-oriented, tab-separated canonical form, same sorted-by-name
//! determinism, same truncated-SHA-256 tag scheme -- so the two fingerprints
//! read as one family. The one structural difference is per-field escaping
//! (see [`escape_field`]): every value that lands in this canonical form is
//! user-controlled (query names, column names), and unlike the schema
//! fingerprint's `"|"`-joined enum/composite lists, nothing here is joined
//! by a delimiter that could be forged by a value containing it.
//!
//! # Why the *analyzed* query, not the raw SQL text
//!
//! Hashing raw SQL text would make every whitespace edit or comment tweak
//! report drift -- exactly the churn the schema fingerprint was designed to
//! avoid, and precisely why `Catalog::fingerprint` hashes shape rather than
//! DDL text. Hashing the analyzed form instead means only changes that can
//! actually change generated code (a renamed query, a reordered or
//! retyped result column, a parameter whose name or type changed, a query
//! added or removed) move the fingerprint.

use sha2::{Digest, Sha256};

use super::types::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery};

/// Version tag for the query fingerprint algorithm itself, mirroring
/// `FINGERPRINT_ALGORITHM_TAG`'s role for the schema fingerprint in
/// `catalog::fingerprint`. Bump this if the canonical form or hash
/// truncation ever changes, so old and new fingerprints are never mistaken
/// for one another.
const QUERY_FINGERPRINT_ALGORITHM_TAG: &str = "q1";

/// Number of leading hash bytes kept (rendered as `2 * TRUNCATED_BYTES` hex
/// characters). Matches the schema fingerprint's truncation so the two tags
/// read the same length at a glance.
const TRUNCATED_BYTES: usize = 8;

impl AnalyzedQuery {
    /// Compute a deterministic fingerprint of an analyzed query set.
    ///
    /// The result is a short tag of the form `q1:<16 hex chars>`. Two query
    /// sets produce the same fingerprint if and only if they have the same
    /// queries (by name), and each query has the same return kind
    /// (`:one`/`:many`/`:exec`/...), the same parameter types in positional
    /// order, and the same result columns (name, resolved type, and
    /// nullability) in positional order.
    ///
    /// # Stability guarantees
    ///
    /// - **Reformat-invariant**: raw SQL text, comments, and whitespace are
    ///   never hashed -- only the *analyzed* shape participates. Two query
    ///   files that differ solely in formatting or comments produce
    ///   identical `AnalyzedQuery` values for every field this fingerprint
    ///   reads, so they fingerprint identically.
    /// - **Query-order-invariant**: queries are sorted by name before
    ///   hashing, so splitting or reordering query files (without changing
    ///   any query's shape) does not change the fingerprint.
    /// - **Injective encoding**: every user-controlled value (query name,
    ///   parameter name, column name) is escaped via [`escape_field`]
    ///   before being embedded
    ///   in the canonical form, so a name containing a tab, newline, or
    ///   backslash cannot be crafted to collide with a differently-shaped
    ///   query set. See that function's doc comment.
    ///
    /// # What participates
    ///
    /// - The query's `name`.
    /// - The query's return kind (`command`), via [`QueryCommand`]'s own
    ///   `Display`/`FromStr` round-trip strings (`"one"`, `"many"`,
    ///   `"exec"`, ...) -- already a stable, tested contract, since
    ///   `@returns :one` annotations parse through the exact inverse of
    ///   this `Display` impl.
    /// - Each parameter's name and resolved (`neutral_type`) type, **in
    ///   positional order** -- positional order is semantic and is never
    ///   sorted. The name participates because backends emit it as the
    ///   generated function's argument name; see [`param_line`].
    /// - Each result column's name, resolved (`neutral_type`) type, and
    ///   nullability, **in positional (declared) order** -- likewise never
    ///   sorted.
    ///
    /// [`QueryCommand`]: crate::parser::QueryCommand
    ///
    /// # What is excluded, deliberately
    ///
    /// - The query's raw `sql` text, and therefore all comments and
    ///   whitespace within it.
    /// - The source `.sql` file path a query was read from.
    /// - scythe's own version -- exactly as the schema fingerprint excludes
    ///   it, and for the same reason: it is reported as an independent
    ///   `v=` field in the provenance header, not folded into this hash.
    /// - Metadata unrelated to the generated signature or row type
    ///   (`deprecated`, `source_table`, `composites`, `enums`,
    ///   `optional_params`, `group_by`, `custom`, `nested_structs`): none of
    ///   these change the *shape* of the generated query function's
    ///   signature or row type in a way not already captured by the
    ///   parameter and column lists above.
    ///
    ///   Parameter *names* are deliberately **not** in this list -- see
    ///   [`param_line`] for why excluding them is a false negative.
    #[must_use]
    pub fn fingerprint_set<'a, I>(queries: I) -> String
    where
        I: IntoIterator<Item = &'a AnalyzedQuery>,
    {
        let canonical = canonical_form(queries);
        let digest = Sha256::digest(canonical.as_bytes());
        let hex: String = digest[..TRUNCATED_BYTES].iter().map(|b| format!("{b:02x}")).collect();
        format!("{QUERY_FINGERPRINT_ALGORITHM_TAG}:{hex}")
    }
}

/// Render a set of analyzed queries into a line-oriented, tab-separated
/// canonical form suitable for hashing. Never render via `{:?}` (`Debug`)
/// -- that output is not a stable contract and can change on any dependency
/// or compiler bump.
///
/// Queries are sorted by name first, so the resulting lines are independent
/// of the order queries were parsed or which file each came from.
fn canonical_form<'a, I>(queries: I) -> String
where
    I: IntoIterator<Item = &'a AnalyzedQuery>,
{
    let mut sorted: Vec<&AnalyzedQuery> = queries.into_iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let mut lines: Vec<String> = Vec::new();

    for query in sorted {
        let name = escape_field(&query.name);
        lines.push(format!(
            "query\t{name}\t{}\t{}\t{}",
            query.command,
            query.params.len(),
            query.columns.len()
        ));

        for (idx, param) in query.params.iter().enumerate() {
            lines.push(param_line(&name, idx, param));
        }

        for (idx, column) in query.columns.iter().enumerate() {
            lines.push(column_line(&name, idx, column));
        }
    }

    lines.join("\n")
}

/// One `param` line: query name (already escaped by the caller), positional
/// index, and the parameter's name and resolved type.
///
/// The name participates because backends emit it as the generated
/// function's argument name (`sqlx.rs:205`, `python_psycopg3.rs:210`, and
/// every other backend via `resolve.rs:96`). Two parameters can share a
/// type and differ only in name: swapping `WHERE name = $1` for
/// `WHERE email = $1` leaves both `string` but rewrites the signature from
/// `name: &str` to `email: &str`, which breaks every caller. Fingerprinting
/// the type alone reports that as no drift at all.
fn param_line(escaped_query_name: &str, idx: usize, param: &AnalyzedParam) -> String {
    format!(
        "param\t{escaped_query_name}\t{idx}\t{}\t{}",
        escape_field(&param.name),
        escape_field(&param.neutral_type)
    )
}

/// One `column` line: query name (already escaped by the caller), positional
/// index, column name, resolved type, and nullability.
fn column_line(escaped_query_name: &str, idx: usize, column: &AnalyzedColumn) -> String {
    format!(
        "column\t{escaped_query_name}\t{idx}\t{}\t{}\t{}",
        escape_field(&column.name),
        escape_field(&column.neutral_type),
        column.nullable
    )
}

/// Escape a value before it is embedded as one tab-separated field in
/// [`canonical_form`]'s output.
///
/// Every value this fingerprint reads (query names, column names) is
/// user-controlled, and the canonical form uses literal `\t` to separate
/// fields within a line and literal `\n` to separate lines. Without
/// escaping, a query or column named e.g. `"evil\tcolumn\tfake\ttype\tfalse"`
/// could inject what looks like an extra `column` line into the hashed
/// text, forging a collision between two query sets that are not actually
/// the same shape -- the same class of defect being fixed in the sibling
/// schema-fingerprint module's `"|"`-joined enum/composite lists.
///
/// Backslash, tab, newline, and carriage return are each replaced by a
/// two-character escape (`\\`, `\t`, `\n`, `\r`) so that no raw delimiter
/// character from an escaped value can ever appear in the canonical form --
/// only literal, unescaped tabs and newlines inserted by [`canonical_form`]
/// itself act as delimiters. The common case (no character needing escape)
/// allocates nothing beyond the final owned `String`.
fn escape_field(value: &str) -> String {
    if !value.contains(['\\', '\t', '\n', '\r']) {
        return value.to_string();
    }

    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use crate::analyzer::analyze;
    use crate::catalog::Catalog;
    use crate::parser::parse_query;

    use super::AnalyzedQuery;

    fn make_catalog() -> Catalog {
        Catalog::from_ddl(&["CREATE TABLE users (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                email VARCHAR(255) NOT NULL,
                age INTEGER
            );"])
        .unwrap()
    }

    fn analyzed(sql: &str) -> AnalyzedQuery {
        let catalog = make_catalog();
        let query = parse_query(sql).unwrap();
        analyze(&catalog, &query).unwrap()
    }

    #[test]
    fn test_reformatted_and_recommented_query_produces_same_fingerprint() {
        let a = analyzed("-- @name GetUser\n-- @returns :one\nSELECT id, name, email FROM users WHERE id = $1;");
        let b = analyzed(
            "-- @name GetUser\n-- @returns :one\n-- fetches a single user by id, added a comment here\nSELECT   id,\n  name,\n  \
             email\nFROM users\nWHERE   id = $1; -- trailing comment\n",
        );

        assert_eq!(
            AnalyzedQuery::fingerprint_set([&a]),
            AnalyzedQuery::fingerprint_set([&b]),
            "whitespace and comments must not affect the query fingerprint"
        );
    }

    #[test]
    fn test_query_name_change_produces_different_fingerprint() {
        let a = analyzed("-- @name GetUser\n-- @returns :one\nSELECT id FROM users WHERE id = $1;");
        let b = analyzed("-- @name FetchUser\n-- @returns :one\nSELECT id FROM users WHERE id = $1;");

        assert_ne!(
            AnalyzedQuery::fingerprint_set([&a]),
            AnalyzedQuery::fingerprint_set([&b])
        );
    }

    #[test]
    fn test_return_kind_change_produces_different_fingerprint() {
        let a = analyzed("-- @name ListUsers\n-- @returns :many\nSELECT id FROM users;");
        let b = analyzed("-- @name ListUsers\n-- @returns :one\nSELECT id FROM users;");

        assert_ne!(
            AnalyzedQuery::fingerprint_set([&a]),
            AnalyzedQuery::fingerprint_set([&b]),
            "return kind (:one vs :many) must participate"
        );
    }

    #[test]
    fn test_param_type_change_produces_different_fingerprint() {
        let a = analyzed("-- @name GetUser\n-- @returns :one\nSELECT id FROM users WHERE id = $1;");
        let b = analyzed("-- @name GetUser\n-- @returns :one\nSELECT id FROM users WHERE email = $1;");

        assert_ne!(
            AnalyzedQuery::fingerprint_set([&a]),
            AnalyzedQuery::fingerprint_set([&b]),
            "a parameter's resolved type must participate"
        );
    }

    /// A parameter can change name while keeping its type, and that rewrites
    /// the generated function's argument name -- `find_user(name: &str)`
    /// becomes `find_user(email: &str)`, which breaks every caller. Both
    /// columns here are `TEXT`, so the type-only encoding this replaces
    /// reported the swap as no drift at all.
    ///
    /// `name` and `email` are chosen deliberately: `id` would also change
    /// the type (integer to string) and so would pass even against the
    /// broken encoding.
    #[test]
    fn test_param_name_change_at_same_type_produces_different_fingerprint() {
        let a = analyzed("-- @name FindUser\n-- @returns :one\nSELECT id FROM users WHERE name = $1;");
        let b = analyzed("-- @name FindUser\n-- @returns :one\nSELECT id FROM users WHERE email = $1;");

        assert_eq!(
            a.params[0].neutral_type, b.params[0].neutral_type,
            "guard: this test is only meaningful while both params share a type"
        );
        assert_ne!(
            a.params[0].name, b.params[0].name,
            "guard: the names must actually differ"
        );

        assert_ne!(
            AnalyzedQuery::fingerprint_set([&a]),
            AnalyzedQuery::fingerprint_set([&b]),
            "a parameter's name must participate -- it becomes the generated argument name"
        );
    }

    #[test]
    fn test_column_nullability_change_produces_different_fingerprint() {
        // Constructed directly rather than through real SQL analysis: this
        // isolates the one field under test (`AnalyzedColumn::nullable`)
        // instead of depending on which SQL shapes the analyzer happens to
        // infer nullability changes from.
        let make = |nullable: bool| {
            AnalyzedQuery::build(|q| {
                q.name = "GetUser".to_string();
                q.command = crate::parser::QueryCommand::One;
                q.columns = vec![crate::analyzer::AnalyzedColumn {
                    name: "age".to_string(),
                    neutral_type: "int".to_string(),
                    nullable,
                    ..Default::default()
                }];
            })
        };

        let not_null = make(false);
        let nullable = make(true);

        assert_ne!(
            AnalyzedQuery::fingerprint_set([&not_null]),
            AnalyzedQuery::fingerprint_set([&nullable]),
            "a result column's nullability must participate"
        );
    }

    #[test]
    fn test_column_reorder_produces_different_fingerprint() {
        let a = analyzed("-- @name GetUser\n-- @returns :one\nSELECT id, name FROM users WHERE id = $1;");
        let b = analyzed("-- @name GetUser\n-- @returns :one\nSELECT name, id FROM users WHERE id = $1;");

        assert_ne!(
            AnalyzedQuery::fingerprint_set([&a]),
            AnalyzedQuery::fingerprint_set([&b]),
            "column order is positional and must be part of the fingerprint"
        );
    }

    #[test]
    fn test_query_added_produces_different_fingerprint() {
        let a = analyzed("-- @name GetUser\n-- @returns :one\nSELECT id FROM users WHERE id = $1;");
        let b = analyzed("-- @name ListUsers\n-- @returns :many\nSELECT id FROM users;");

        let one = AnalyzedQuery::fingerprint_set([&a]);
        let two = AnalyzedQuery::fingerprint_set([&a, &b]);

        assert_ne!(one, two);
    }

    #[test]
    fn test_fingerprint_is_independent_of_input_order() {
        let a = analyzed("-- @name GetUser\n-- @returns :one\nSELECT id FROM users WHERE id = $1;");
        let b = analyzed("-- @name ListUsers\n-- @returns :many\nSELECT id FROM users;");

        assert_eq!(
            AnalyzedQuery::fingerprint_set([&a, &b]),
            AnalyzedQuery::fingerprint_set([&b, &a]),
            "queries are sorted by name before hashing, so parse/collection order must not matter"
        );
    }

    #[test]
    fn test_escape_field_prevents_delimiter_forgery() {
        // A query named to *look like* it injects a second, differently-shaped
        // query must not fingerprint the same as a query set that actually
        // has that second query.
        let forged = AnalyzedQuery::build(|q| {
            q.name = "Evil\nquery\tGetUser\tone\t1\t1\nparam\tGetUser\t0\tstring".to_string();
            q.command = crate::parser::QueryCommand::One;
        });

        let real_one = AnalyzedQuery::build(|q| {
            q.name = "Evil".to_string();
            q.command = crate::parser::QueryCommand::One;
        });

        assert_ne!(
            AnalyzedQuery::fingerprint_set([&forged]),
            AnalyzedQuery::fingerprint_set([&real_one]),
            "a name containing raw delimiters must not be able to forge extra fingerprint lines"
        );
    }

    #[test]
    fn test_empty_query_set_is_deterministic() {
        assert_eq!(
            AnalyzedQuery::fingerprint_set(std::iter::empty()),
            AnalyzedQuery::fingerprint_set(std::iter::empty())
        );
    }
}
