//! Regression test for psycopg3's named paramstyle: every `%(name)s` in the
//! generated SQL must be a key of the dict handed to `execute`.
//!
//! `python-psycopg3` is the only backend that binds by name rather than by
//! position, and it derived the two halves of that contract twice: the dict
//! from the resolved param's `field_name`, the placeholder from a second
//! `to_snake_case` of the raw SQL name. The spellings agreed until anything
//! else touched `field_name`, and `[naming] reserved` mangling a param called
//! `class` to `class_` was enough -- the SQL asked for `%(class)s` while the
//! dict offered `class_`, and psycopg raises "query parameter missing" at
//! execute time. Nothing before execution can see it: the module imports, type
//! checks, and passes the torture gate, because that gate compiles generated
//! code and never runs it.
//!
//! So this asserts the invariant rather than the one keyword that exposed it.

use std::collections::BTreeSet;

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// Collect the `name`s in every `%(name)s` occurrence in `source`.
fn placeholder_names(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut rest = source;
    while let Some(start) = rest.find("%(") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find(")s") else { break };
        names.insert(rest[..end].to_string());
        rest = &rest[end + 2..];
    }
    names
}

/// Collect the `"key"` of every `"key": value` entry in the params dict.
///
/// Deliberately parsed out of the emitted text rather than read off the
/// resolved params: taking both sides from the same in-memory value is what
/// let the two spellings drift in the first place.
fn dict_keys(source: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let Some(open) = source.find("        {\"") else {
        return keys;
    };
    let dict = &source[open..];
    let Some(close) = dict.find("},") else {
        return keys;
    };
    let mut rest = &dict[..close];
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else { break };
        let key = &rest[..end];
        rest = &rest[end + 1..];
        // `"key": value` -- skip anything not immediately followed by a colon.
        if rest.starts_with(':') {
            keys.insert(key.to_string());
        }
    }
    keys
}

fn generate(query: &str) -> String {
    const SCHEMA: &str = "CREATE TABLE items (id SERIAL PRIMARY KEY, class TEXT NOT NULL, label TEXT NOT NULL);";
    let backend = get_backend("python-psycopg3", "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.query_fn.expect("expected a query fn")
}

#[test]
fn every_named_placeholder_is_a_key_of_the_params_dict() {
    const QUERY: &str = "-- @name FindItem\n-- @returns :one\n\
        SELECT id, class, label FROM items WHERE class = $1 AND label = $2;";
    let query_fn = generate(QUERY);

    let placeholders = placeholder_names(&query_fn);
    let keys = dict_keys(&query_fn);

    assert!(
        !placeholders.is_empty(),
        "no %(...)s placeholders were emitted:\n{query_fn}"
    );
    assert_eq!(
        placeholders, keys,
        "psycopg binds by name: a placeholder with no matching dict key raises \
         \"query parameter missing\" at execute time:\n{query_fn}"
    );
}

#[test]
fn a_reserved_word_param_keeps_the_placeholder_and_the_key_in_step() {
    const QUERY: &str = "-- @name FindByClass\n-- @returns :one\n\
        SELECT id, class, label FROM items WHERE class = $1;";
    let query_fn = generate(QUERY);

    // The mangled spelling on both sides -- `class` is a Python keyword, so
    // the parameter cannot be named after it, and the placeholder has to
    // follow the parameter rather than the column.
    assert!(
        query_fn.contains("%(class_)s"),
        "expected the placeholder to follow the mangled param name:\n{query_fn}"
    );
    assert!(
        query_fn.contains("{\"class_\": class_}"),
        "expected the dict to be keyed by the mangled param name:\n{query_fn}"
    );
    assert_eq!(placeholder_names(&query_fn), dict_keys(&query_fn), "got:\n{query_fn}");
}
