//! A column name that is not a valid identifier must not reach a generated
//! field declaration, in any target that cannot quote one.
//!
//! `crates/scythe-codegen/tests/ts_identifier_quoting_regression.rs` covers
//! the TypeScript answer, which is to quote: `"my col": string` and
//! `row["my col"]` are both legal, and a TypeScript row type is cast straight
//! onto the driver's rows, so its key has to stay the column's own spelling.
//! No other target has a quoted form for a field, and none of them read a
//! column back by the generated field name -- they use the position or the
//! raw SQL name -- so those manifests set `[naming] sanitize_field_names`
//! and get the name mangled instead.
//!
//! Before that flag existed every one of the backends below emitted its
//! column name verbatim:
//!
//!     pub my col: String      my col: str      My col string      String my col
//!
//! none of which parse.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TABLE items (\
    id INT PRIMARY KEY, \
    \"my col\" TEXT NOT NULL, \
    \"with-dash\" TEXT NOT NULL, \
    \"2fa\" TEXT NOT NULL\
);";

const QUERY: &str = "-- @name FindItem\n-- @returns :one\n\
    SELECT id, \"my col\", \"with-dash\", \"2fa\" FROM items WHERE id = $1;";

/// One backend per target language that declares `sanitize_field_names`.
const MANGLING_BACKENDS: [&str; 8] = [
    "rust-sqlx",
    "python-psycopg3",
    "go-pgx",
    "java-jdbc",
    "kotlin-jdbc",
    "csharp-npgsql",
    "php-pdo",
    "elixir-postgrex",
];

fn generate(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    [code.row_struct, code.query_fn, code.model_struct]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Everything outside a string literal, so an assertion about generated
/// *code* is not satisfied or defeated by the SQL text, which keeps the
/// column's real name by design.
fn outside_string_literals(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in source.chars() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == q {
                    quote = None;
                }
                out.push(' ');
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                    out.push(' ');
                } else {
                    out.push(ch);
                }
            }
        }
    }
    out
}

#[test]
fn no_backend_emits_a_space_or_dash_inside_a_generated_name() {
    for backend_name in MANGLING_BACKENDS {
        let code = outside_string_literals(&generate(backend_name));
        for bad in ["my col", "with-dash", "My col", "myCol ", "with-Dash"] {
            assert!(
                !code.contains(bad),
                "{backend_name}: `{bad}` cannot appear in an identifier:\n{code}"
            );
        }
    }
}

/// Compared with the separators and the case removed, because the mangled
/// name is spelled differently per manifest and per backend: `my_col` in
/// Rust and Python, `MyCol` wherever the backend PascalCases on top. What
/// every one of them must agree on is that the two words survived and are
/// joined by something an identifier can hold.
#[test]
fn every_backend_emits_the_mangled_names() {
    for backend_name in MANGLING_BACKENDS {
        let code = generate(backend_name);
        let normalized = outside_string_literals(&code).to_lowercase().replace('_', "");
        for expected in ["mycol", "withdash"] {
            assert!(
                normalized.contains(expected),
                "{backend_name}: expected a mangled form of `{expected}`:\n{code}"
            );
        }
    }
}

/// A leading digit needs a prefix, and the prefix has to survive the case
/// conversion the manifest asks for *and* any second conversion a backend
/// applies on top -- go-pgx and the csharp family PascalCase the field name
/// again. A bare `_` does not survive either, which is why the guard is a
/// word.
#[test]
fn a_leading_digit_never_survives_into_a_generated_name() {
    for backend_name in MANGLING_BACKENDS {
        let code = outside_string_literals(&generate(backend_name));
        for line in code.lines() {
            for (index, _) in line.match_indices("2fa") {
                let preceding = line[..index].chars().next_back();
                assert!(
                    preceding.is_some_and(|c| c.is_alphanumeric() || c == '_'),
                    "{backend_name}: `2fa` starts a name on this line, which is not a valid \
                     identifier in any target language:\n{line}"
                );
            }
        }
    }
}

/// The SQL text is not a generated name and must keep the column exactly as
/// the database spells it -- mangling there would query a column that does
/// not exist.
#[test]
fn the_sql_text_still_names_the_columns_as_the_database_does() {
    for backend_name in MANGLING_BACKENDS {
        let code = generate(backend_name);
        assert!(
            code.contains("my col"),
            "{backend_name}: the SQL must still select \"my col\":\n{code}"
        );
    }
}

/// TypeScript is the counter-case, asserted here rather than assumed: its
/// manifests deliberately leave `sanitize_field_names` off.
#[test]
fn typescript_quotes_instead_of_mangling() {
    let code = generate("typescript-pg");
    assert!(
        code.contains("\"my col\": string;"),
        "typescript-pg must quote the key, not mangle it:\n{code}"
    );
    assert!(
        !code.contains("my_col: string;"),
        "a mangled key would not match the object `pg` returns:\n{code}"
    );
}
