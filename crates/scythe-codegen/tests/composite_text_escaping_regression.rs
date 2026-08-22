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
use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_with_tools};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TYPE addr AS (street TEXT, city TEXT); \
    CREATE TABLE things (id SERIAL PRIMARY KEY, home addr NOT NULL);";

const QUERY: &str = "-- @name GetThing\n-- @returns :one\nSELECT id, home FROM things WHERE id = $1;";

/// ~keep Why a missing interpreter is fatal under strict mode but not locally.
/// Each of the three probe tests below asserts on values the emitted parser
/// actually returned; without its interpreter the test degrades to the string
/// match the doc comments already call insufficient, and still reports success.
/// Nobody has all three toolchains locally, so the skip stays -- but CI sets
/// strict mode precisely so that losing one from the image fails instead of
/// quietly shrinking what is checked.
const STRICT_SKIP_REASON: &str = "strict mode requires it; without it only the string match ran";

/// The exact text PostgreSQL 16 emits for `ROW('he said "hi"', 'back\slash')::addr`, and the
/// two field values a correct parser recovers from it.
const PG_TEXT: &str = r#"("he said ""hi""","back\\slash")"#;
const EXPECTED_FIELDS: [&str; 2] = [r#"he said "hi""#, r"back\slash"];

fn generated_text_for(schema: &str, query: &str, backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[schema], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = scythe_codegen::generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");

    // ~keep `file_header_for_results` first: `go-pgx` emits its shared `parseCompositeFields` helper
    // there rather than inside `model_struct` -- a Go package cannot declare the same function
    // name once per composite the way each composite's own parser could as a private per-class
    // method in Java, so the doubled-quote fix lives in the one place it is guaranteed not to
    // collide with a second composite's copy. Every other backend's `file_header_for_results`
    // is unrelated boilerplate (imports/package line) that cannot accidentally satisfy a needle
    // below.
    let mut out = backend.file_header_for_results(std::slice::from_ref(&code));
    out.push('\n');
    for text in [&code.enum_def, &code.model_struct, &code.row_struct, &code.query_fn]
        .into_iter()
        .flatten()
    {
        out.push_str(text);
        out.push('\n');
    }
    out
}

fn generated_text(backend_name: &str) -> String {
    generated_text_for(SCHEMA, QUERY, backend_name)
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
    ("ruby-pg", "inner[i + 1] == '\"'"),
    ("go-pgx", "inner[i+1] == '\"'"),
    // ~keep csharp-npgsql fits this table's shared "i + 1 < n" bound check unmodified -- C# spells
    // both the loop bound and the doubled-quote lookahead exactly like Kotlin/TypeScript do.
    // php-pdo/php-amphp do not: PHP's `$`-sigil variables mean the bound check is always
    // "$i + 1 < $n", never the literal "i + 1 < n" this table shares across every row, so they
    // are covered by their own real-execution tests below instead of a row here.
    ("csharp-npgsql", "inner[i + 1] == '\"'"),
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

const NULLABLE_SCHEMA: &str = "CREATE TYPE mood AS ENUM ('happy', 'sad'); \
    CREATE TYPE geo AS (latitude INTEGER); \
    CREATE TYPE profile AS (age INTEGER, mood mood, geo geo); \
    CREATE TABLE profiles (id INTEGER PRIMARY KEY, details profile NOT NULL);";

const NULLABLE_QUERY: &str = "-- @name GetProfile\n-- @returns :one\nSELECT id, details FROM profiles WHERE id = $1;";

#[test]
fn nullable_composite_fields_skip_scalar_and_static_decoders_for_sql_null() {
    let expectations = [
        (
            "java-r2dbc",
            [
                "f.get(0) == null ? null : Integer.parseInt(f.get(0))",
                "f.get(1) == null ? null : Mood.fromValue(f.get(1))",
                "f.get(2) == null ? null : Geo.fromText(f.get(2))",
            ],
        ),
        (
            "php-pdo",
            [
                "$f[0] === null ? null : (int) $f[0]",
                "$f[1] === null ? null : Mood::from($f[1])",
                "$f[2] === null ? null : Geo::fromText($f[2])",
            ],
        ),
        (
            "php-amphp",
            [
                "$f[0] === null ? null : (int) $f[0]",
                "$f[1] === null ? null : Mood::from($f[1])",
                "$f[2] === null ? null : Geo::fromText($f[2])",
            ],
        ),
        (
            "ruby-pg",
            [
                "age: f[0].nil? ? nil : f[0].to_i",
                "mood: f[1].nil? ? nil : f[1]",
                "geo: f[2].nil? ? nil : Geo.from_text(f[2])",
            ],
        ),
        (
            "typescript-pg",
            [
                "age: f[0] === null ? null : Number(f[0])",
                "mood: f[1] === null ? null : f[1] as Mood",
                "geo: f[2] === null ? null : parseGeo(f[2]) as Geo",
            ],
        ),
        (
            "typescript-postgres",
            [
                "age: f[0] === null ? null : Number(f[0])",
                "mood: f[1] === null ? null : f[1] as Mood",
                "geo: f[2] === null ? null : parseGeo(f[2]) as Geo",
            ],
        ),
        (
            "typescript-kysely",
            [
                "age: f[0] === null ? null : Number(f[0])",
                "mood: f[1] === null ? null : f[1] as Mood",
                "geo: f[2] === null ? null : parseGeo(f[2]) as Geo",
            ],
        ),
    ];

    for (backend, needles) in expectations {
        let code = generated_text_for(NULLABLE_SCHEMA, NULLABLE_QUERY, backend);
        for needle in needles {
            assert!(
                code.contains(needle),
                "{backend}: missing `{needle}` in generated code:\n{code}"
            );
        }
    }
}

#[test]
fn composite_encoders_preserve_sql_null_subfields() {
    let expectations = [
        ("java-r2dbc", "if (value == null) return \"\";"),
        ("php-pdo", "if ($value === null) {"),
        ("php-amphp", "if ($value === null) {"),
        ("ruby-pg", "return \"\" if value.nil?"),
        (
            "typescript-pg",
            "if (field === null || field === undefined) return \"\";",
        ),
        (
            "typescript-kysely",
            "if (field === null || field === undefined) return \"\";",
        ),
    ];

    for (backend, needle) in expectations {
        let code = generated_text_for(NULLABLE_SCHEMA, NULLABLE_QUERY, backend);
        assert!(
            code.contains(needle),
            "{backend}: missing null-safe encoder `{needle}`:\n{code}"
        );
    }
}

/// The string-matching test above cannot tell a correct branch from a plausible-looking one, so
/// this one *runs* the emitted python parser against the exact text PostgreSQL produced and
/// checks the values that come back.
///
/// Skips rather than fails when `python3` is absent, matching how `tool_validation.rs` treats a
/// missing toolchain -- but only outside strict mode. See [`STRICT_SKIP_REASON`].
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
            assert!(
                !strict_mode_enabled(),
                "python3 unavailable ({e}): {STRICT_SKIP_REASON}"
            );
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

/// Companion to the python test above, for `ruby-pg`: runs the *emitted* `Addr.from_text`
/// (not a hand-copy of it) against the exact text PostgreSQL 16 produces, and checks the
/// values that come back. `code.model_struct` is the composite's whole `Data.define ... do
/// ... end` block, so this both proves the doubled-quote escaping rule and that a composite
/// column is actually routed through `from_text` rather than left as raw driver text.
///
/// Skips rather than fails when `ruby` is absent, matching the python test's policy.
#[test]
fn the_emitted_ruby_parser_recovers_both_fields_from_real_postgresql_output() {
    let code = generated_text("ruby-pg");
    let model_struct = code
        .find("Addr = Data.define")
        .map(|start| {
            let rest = &code[start..];
            // The block's own closing `end` is written at 2-space indent (`  end`); every
            // `end` inside the `_parse_composite_fields` body it contains is indented deeper
            // (4+ spaces), so this pattern is unambiguous.
            let end = rest
                .find("\n  end\n")
                .map(|i| i + "\n  end".len())
                .unwrap_or(rest.len());
            &rest[..end]
        })
        .expect("ruby-pg must emit the Addr composite with a from_text method");

    // U+001F between fields and the literal "<NULL>" for nil: a unit separator cannot occur
    // in either expected value, so this needs no extra dependency to be unambiguous.
    let script = format!(
        "{model_struct}\n\n\
         decoded = Addr.from_text({PG_TEXT:?})\n\
         print [decoded.street, decoded.city].map {{ |f| f.nil? ? \"<NULL>\" : f }}.join(\"\\x1f\")\n"
    );
    let dir = std::env::temp_dir().join("scythe_composite_escaping_probe");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("probe.rb");
    std::fs::write(&path, &script).expect("write probe");

    let output = match std::process::Command::new("ruby").arg(&path).output() {
        Ok(output) => output,
        Err(e) => {
            assert!(!strict_mode_enabled(), "ruby unavailable ({e}): {STRICT_SKIP_REASON}");
            eprintln!("SKIP: ruby unavailable ({e}); the doubled-quote rule was checked by string match only");
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
        "parsing {PG_TEXT} must recover both field values intact; before the fix a composite \
         column was left as `pg`'s raw text form (no `from_text` existed to call), and even a \
         hand-copied version of another backend's parser truncated the first field at the \
         doubled quote and read the second from the wrong offset"
    );
}

/// The full `ruby-pg` file (header, composite `from_text`/`_parse_composite_fields`, row struct,
/// query function, footer) must actually pass a real Ruby syntax check -- mirrors
/// `python_composite_enum_read_regression.rs`'s `assert_file_compiles`, which this suite's own
/// `SCHEMA`/`QUERY` predates. Before this fix `ruby-pg` had no `from_text`/`_parse_composite_fields`
/// output at all to check.
#[test]
fn ruby_pg_composite_escaping_generated_file_compiles() {
    let backend = get_backend("ruby-pg", "postgresql").expect("ruby-pg must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = scythe_codegen::generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    let all = std::slice::from_ref(&code);

    let mut body = backend.file_header_for_results(all);
    body.push('\n');
    // ~keep Types first, then the query function *inside* the class `query_class_header` opens.
    // `file_footer` is the `}` that closes that class, so assembling these two without the
    // header between them emits the query function at file scope and leaves the footer's brace
    // closing nothing -- `mago` then reports `Expected one of Class, found Function`, a parse
    // error in this test's own assembly rather than in anything the backend emitted.
    for text in [&code.enum_def, &code.model_struct, &code.row_struct]
        .into_iter()
        .flatten()
    {
        body.push_str(text);
        body.push('\n');
    }
    let class_header = backend.query_class_header();
    if !class_header.is_empty() {
        body.push_str(&class_header);
        body.push('\n');
    }
    if let Some(query_fn) = &code.query_fn {
        body.push_str(query_fn);
        body.push('\n');
    }
    let footer = backend.file_footer();
    if !footer.is_empty() {
        body.push_str(&footer);
        body.push('\n');
    }

    let file = scythe_codegen::provenance::assemble_file(
        &backend.file_preamble(),
        &scythe_codegen::provenance::header_line(
            &*backend,
            env!("CARGO_PKG_VERSION"),
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ),
        &body,
    );

    let validation = validate_with_tools(&file, "ruby-pg");
    assert_ne!(
        validation,
        ToolValidation::Unsupported,
        "ruby-pg lost its tool validator; the compile half of this test is now vacuous"
    );
    for tool in validation.tools_run() {
        eprintln!("  ruby-pg: `{tool}` ran against the generated file");
    }
    for tool in validation.missing_tools() {
        eprintln!("  ruby-pg: `{tool}` is not on PATH -- the compile half went unchecked");
    }
    if strict_mode_enabled() {
        assert!(
            validation.fully_checked(),
            "ruby-pg: tool validation reports nothing actually checked the code"
        );
    }
    if let Err(errors) = validation.into_result() {
        panic!("ruby-pg tool validation: {errors:?}\n\nGenerated file:\n{file}");
    }
}

/// Companion to the python/ruby tests above, for `php-pdo` and `php-amphp`: runs the *emitted*
/// `Addr::fromText` (not a hand-copy of it) against the exact text PostgreSQL 16 produces, and
/// checks the values that come back. Not folded into `PARSER_BACKENDS` (see the comment on that
/// table): PHP's `$`-sigil variables mean the shared "i + 1 < n" bound check never appears
/// literally in PHP source, so the generic string check would report a false failure against
/// correctly-fixed code even though the doubled-quote branch is present and correct.
///
/// Before this fix neither backend emitted a composite `fromText`/`parseCompositeFields` at all
/// -- board #220: `home` was declared `?Addr` on the row but constructed from the driver's raw
/// string, so PHP threw `TypeError: ... Argument #2 ($home) must be of type ?Addr, string
/// given` the moment a query touched a composite column.
///
/// Skips rather than fails when `php` is absent, matching the python/ruby tests' policy.
fn assert_php_composite_parser_recovers_both_fields(backend_name: &str) {
    let code = generated_text(backend_name);
    let start = code
        .find("readonly class Addr {")
        .unwrap_or_else(|| panic!("{backend_name} must emit the Addr composite class"));
    let rest = &code[start..];
    // ~keep The class's own closing `}` is at column 0; every `}` inside the constructor or
    // `fromText`/`parseCompositeFields` bodies it contains is indented, so this pattern is
    // unambiguous (mirrors the ruby test's identical reasoning for its `end` marker).
    let end = rest.find("\n}\n").map(|i| i + "\n}".len()).unwrap_or(rest.len());
    let class_body = &rest[..end];

    // ~keep The parser lives in its own file-level class rather than inside each composite, so
    // it has to be pulled in alongside `Addr` for the extracted pair to run standalone. Asserting
    // it is present also pins that the hoist happened: with the parser back inside `Addr`, this
    // `expect` fires rather than the script dying at runtime on a missing class.
    let parser_start = code
        .find("final class ScytheCompositeText {")
        .unwrap_or_else(|| panic!("{backend_name} must emit the shared ScytheCompositeText class"));
    let parser_rest = &code[parser_start..];
    let parser_end = parser_rest
        .find("\n}\n")
        .map(|i| i + "\n}".len())
        .unwrap_or(parser_rest.len());
    let parser_class = &parser_rest[..parser_end];

    // ~keep PHP single-quoted strings need only `\` and `'` escaped; PG_TEXT contains no `'`.
    let php_literal = format!("'{}'", PG_TEXT.replace('\\', "\\\\"));

    // ~keep U+001F between fields and the literal "<NULL>" for null: a unit separator cannot
    // occur in either expected value, so this needs no extra dependency to be unambiguous.
    let script = format!(
        "<?php\ndeclare(strict_types=1);\n\n{parser_class}\n\n{class_body}\n\n\
         $decoded = Addr::fromText({php_literal});\n\
         $parts = [$decoded->street, $decoded->city];\n\
         echo implode(\"\\x1f\", array_map(fn($f) => $f === null ? '<NULL>' : $f, $parts));\n"
    );
    let dir = std::env::temp_dir().join("scythe_composite_escaping_probe");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(format!("probe_{}.php", backend_name.replace('-', "_")));
    std::fs::write(&path, &script).expect("write probe");

    let output = match std::process::Command::new("php").arg(&path).output() {
        Ok(output) => output,
        Err(e) => {
            assert!(!strict_mode_enabled(), "php unavailable ({e}): {STRICT_SKIP_REASON}");
            eprintln!("SKIP: php unavailable ({e}); the doubled-quote rule was checked by string match only");
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
        "{backend_name}: parsing {PG_TEXT} must recover both field values intact; before the fix \
         a composite column was constructed straight from the driver's raw string (no `fromText` \
         existed to call), which is exactly the shape that threw `TypeError` for a declared \
         `?Addr` property"
    );
}

#[test]
fn the_emitted_php_pdo_parser_recovers_both_fields_from_real_postgresql_output() {
    assert_php_composite_parser_recovers_both_fields("php-pdo");
}

#[test]
fn the_emitted_php_amphp_parser_recovers_both_fields_from_real_postgresql_output() {
    assert_php_composite_parser_recovers_both_fields("php-amphp");
}

/// The full `php-pdo`/`php-amphp` file (header, composite `fromText`/`parseCompositeFields`, row
/// struct, query function, footer) must actually pass a real PHP syntax check -- mirrors
/// `ruby_pg_composite_escaping_generated_file_compiles` above. Before this fix `home` was typed
/// `?Addr` but constructed from a raw string, which `mago`/`php -l` cannot catch (it is a type
/// error PHP raises at runtime, not a parse error) -- this test instead pins that the composite
/// class and its `fromText`/`parseCompositeFields` methods are themselves well-formed PHP.
fn assert_php_composite_escaping_generated_file_compiles(backend_name: &str) {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = scythe_codegen::generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    let all = std::slice::from_ref(&code);

    let mut body = backend.file_header_for_results(all);
    body.push('\n');
    // Types first, then the query function *inside* the class `query_class_header` opens.
    // `file_footer` is the `}` that closes that class, so assembling these two without the
    // header between them emits the query function at file scope and leaves the footer's brace
    // closing nothing -- `mago` then reports `Expected one of Class, found Function`, a parse
    // error in this test's own assembly rather than in anything the backend emitted.
    for text in [&code.enum_def, &code.model_struct, &code.row_struct]
        .into_iter()
        .flatten()
    {
        body.push_str(text);
        body.push('\n');
    }
    let class_header = backend.query_class_header();
    if !class_header.is_empty() {
        body.push_str(&class_header);
        body.push('\n');
    }
    if let Some(query_fn) = &code.query_fn {
        body.push_str(query_fn);
        body.push('\n');
    }
    let footer = backend.file_footer();
    if !footer.is_empty() {
        body.push_str(&footer);
        body.push('\n');
    }

    let file = scythe_codegen::provenance::assemble_file(
        &backend.file_preamble(),
        &scythe_codegen::provenance::header_line(
            &*backend,
            env!("CARGO_PKG_VERSION"),
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ),
        &body,
    );

    let validation = validate_with_tools(&file, backend_name);
    assert_ne!(
        validation,
        ToolValidation::Unsupported,
        "{backend_name} lost its tool validator; the compile half of this test is now vacuous"
    );
    for tool in validation.tools_run() {
        eprintln!("  {backend_name}: `{tool}` ran against the generated file");
    }
    for tool in validation.missing_tools() {
        eprintln!("  {backend_name}: `{tool}` is not on PATH -- the compile half went unchecked");
    }
    if strict_mode_enabled() {
        assert!(
            validation.fully_checked(),
            "{backend_name}: tool validation reports nothing actually checked the code"
        );
    }
    if let Err(errors) = validation.into_result() {
        panic!("{backend_name} tool validation: {errors:?}\n\nGenerated file:\n{file}");
    }
}

#[test]
fn php_pdo_composite_escaping_generated_file_compiles() {
    assert_php_composite_escaping_generated_file_compiles("php-pdo");
}

#[test]
fn php_amphp_composite_escaping_generated_file_compiles() {
    assert_php_composite_escaping_generated_file_compiles("php-amphp");
}
