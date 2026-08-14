//! Every generated composite text-form parser must accept PostgreSQL's *doubled* double-quote.
//!
//! `record_out` escapes a literal `"` inside a quoted composite field by doubling it, and a
//! literal `\` by backslash-escaping it. Captured from PostgreSQL 16 rather than asserted from
//! memory:
//!
//! ```text
//! # SELECT ROW('he said "hi"', 'back\slash', NULL)::esc_probe::text;
//! ("he said ""hi""","back\\slash",)
//! ```
//!
//! Every parser scythe emits handled only the backslash spelling. On a doubled quote each one
//! took the first `"` for the field's closing quote, which does two things at once: it truncates
//! that field's value, and -- because the parser then resynchronizes on the wrong character --
//! it shifts every *subsequent* field of the composite by one. A row whose text column merely
//! contains a quote came back with silently wrong values in unrelated fields.
//!
//! The five JVM parsers shipped with this defect; the python and typescript ones inherited it
//! from `java_jdbc.rs`, which board #204 named as the model to copy.

use scythe_codegen::get_backend;
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TYPE addr AS (street TEXT, city TEXT); \
    CREATE TABLE things (id SERIAL PRIMARY KEY, home addr NOT NULL);";

const QUERY: &str = "-- @name GetThing\n-- @returns :one\nSELECT id, home FROM things WHERE id = $1;";

/// The exact text PostgreSQL 16 emits for `ROW('he said "hi"', 'back\slash')::addr`, and the
/// two field values a correct parser recovers from it.
const PG_TEXT: &str = r#"("he said ""hi""","back\\slash")"#;
const EXPECTED_FIELDS: [&str; 2] = [r#"he said "hi""#, r"back\slash"];

fn generated_text(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = scythe_codegen::generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");

    let mut out = String::new();
    for text in [&code.enum_def, &code.model_struct, &code.row_struct, &code.query_fn]
        .into_iter()
        .flatten()
    {
        out.push_str(text);
        out.push('\n');
    }
    out
}

/// Every backend that emits a hand-written composite parser, paired with the language's
/// spelling of "the next character is also a quote".
///
/// Deliberately a table rather than one test per backend: the defect was identical in all
/// eight, so a new backend copying the same parser -- which is how it spread the first time --
/// should be added here rather than getting its own bespoke assertion.
const PARSER_BACKENDS: &[(&str, &str)] = &[
    ("java-jdbc", "inner.charAt(i + 1) == '\"'"),
    ("java-r2dbc", "inner.charAt(i + 1) == '\"'"),
    ("kotlin-jdbc", "inner[i + 1] == '\"'"),
    ("kotlin-r2dbc", "inner[i + 1] == '\"'"),
    ("kotlin-exposed", "inner[i + 1] == '\"'"),
    ("python-psycopg3", "inner[i + 1] == '\"'"),
    ("typescript-pg", "inner[i + 1] === '\"'"),
    ("typescript-postgres", "inner[i + 1] === '\"'"),
    ("typescript-kysely", "inner[i + 1] === '\"'"),
];

#[test]
fn every_generated_composite_parser_handles_a_doubled_quote() {
    for (backend, needle) in PARSER_BACKENDS {
        let code = generated_text(backend);
        assert!(
            code.contains("i + 1 < n") && code.contains(needle),
            "{backend}: the emitted composite parser must treat a doubled double-quote as one \
             escaped quote, not as the field's closing quote -- looked for `{needle}`;\n\
             generated:\n{code}"
        );
    }
}

/// The string-matching test above cannot tell a correct branch from a plausible-looking one, so
/// this one *runs* the emitted python parser against the exact text PostgreSQL produced and
/// checks the values that come back.
///
/// Skips rather than fails when `python3` is absent, matching how `tool_validation.rs` treats a
/// missing toolchain -- but prints that it skipped, so a CI image that quietly loses python
/// cannot turn this into a test that passes without checking anything.
#[test]
fn the_emitted_python_parser_recovers_both_fields_from_real_postgresql_output() {
    let code = generated_text("python-psycopg3");
    let start = code
        .find("    @staticmethod\n    def _parse_composite_fields")
        .expect("psycopg3 must emit _parse_composite_fields");
    let rest = &code[start..];
    let end = rest
        .find("\n    @classmethod")
        .or_else(|| rest.find("\n\n@"))
        .unwrap_or(rest.len());
    let method = &rest[..end];

    // Re-indent the class-level staticmethod to module scope so it can run standalone.
    let dedented: String = method
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");

    // U+001F between fields and a literal "<NULL>" for None: a unit separator cannot occur in
    // either expected value, so this needs no JSON dependency to be unambiguous.
    let script = format!(
        "{dedented}\n\nfields = _parse_composite_fields({PG_TEXT:?})\n\
         print(\"\\x1f\".join(\"<NULL>\" if f is None else f for f in fields), end=\"\")\n"
    );
    let dir = std::env::temp_dir().join("scythe_composite_escaping_probe");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("probe.py");
    std::fs::write(&path, &script).expect("write probe");

    let output = match std::process::Command::new("python3").arg(&path).output() {
        Ok(output) => output,
        Err(e) => {
            eprintln!("SKIP: python3 unavailable ({e}); the doubled-quote rule was checked by string match only");
            return;
        }
    };
    assert!(
        output.status.success(),
        "the emitted parser must run: {}\nscript:\n{script}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parsed = String::from_utf8(output.stdout).expect("probe output must be utf-8");
    let expected = EXPECTED_FIELDS.join("\u{1f}");
    assert_eq!(
        parsed, expected,
        "parsing {PG_TEXT} must recover both field values intact; before the fix the first field \
         truncated at the doubled quote and the second was read from the wrong offset"
    );
}
