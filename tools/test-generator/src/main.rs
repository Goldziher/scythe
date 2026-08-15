mod fixture;

use clap::Parser;
use fixture::{Command, ExpectedCatalog, ExpectedGeneratedCode, ExpectedQuery, Fixture};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;

/// The candidate backend names probed for codegen verification. Kept as the generator's
/// original fixed list -- widening it to the full ~55-name `scythe_codegen::get_backend`
/// registry (to add real MySQL/SQLite/etc. coverage) is separate follow-up work from the defect
/// this list exists to fix: which of these actually applies to a *given* fixture's engine. See
/// #156.
const CANDIDATE_BACKENDS: &[&str] = &[
    "rust-sqlx",
    "rust-tokio-postgres",
    "python-psycopg3",
    "python-asyncpg",
    "typescript-postgres",
    "typescript-pg",
    "typescript-kysely",
    "go-pgx",
    "java-jdbc",
    "java-r2dbc",
    "kotlin-exposed",
    "kotlin-jdbc",
    "kotlin-r2dbc",
    "csharp-npgsql",
    "elixir-postgrex",
    "elixir-ecto",
    "ruby-pg",
    "ruby-trilogy",
    "php-pdo",
    "php-amphp",
];

/// The engine string a fixture targets, defaulting to `"postgresql"` when unset -- the same
/// default `catalog_expr` and `Engine::dialect_path` use.
fn fixture_engine_str(fixture: &Fixture) -> &'static str {
    fixture
        .config
        .as_ref()
        .and_then(|c| c.engine)
        .map(fixture::Engine::as_str)
        .unwrap_or("postgresql")
}

/// Ask the real `get_backend` registry which of `CANDIDATE_BACKENDS` accept `engine`, at
/// generation time. Before this, the generated test embedded the *full* candidate list for
/// every fixture regardless of engine and silently `continue`d past whichever ones didn't apply
/// -- `ruby-trilogy` (MySQL/MariaDB only) appeared, and contributed zero assertions, in every
/// one of the ~370 non-error query fixtures, all of which target PostgreSQL. Filtering here
/// means the list embedded in a generated test is exactly the backends expected to construct,
/// so a construction failure in the generated test is a real regression rather than an expected
/// engine mismatch. See #156.
fn applicable_backends(engine: &str) -> Vec<&'static str> {
    CANDIDATE_BACKENDS
        .iter()
        .copied()
        .filter(|name| scythe_codegen::get_backend(name, engine).is_ok())
        .collect()
}

/// A fixture with `query_sql`, `expected.success == true`, and a row-returning command
/// (`one`/`many`/`grouped`) that declares no `expected.query.columns` would generate a test
/// asserting name, command and params only -- not one column type or nullability. A
/// one-character key typo (`columns` -> `column`) used to produce exactly this silently, because
/// the field is `#[serde(default)]`. `deny_unknown_fields` now catches the typo itself at parse
/// time; this catches the remaining case where the key is simply never written. See #156.
fn validate_fixtures(fixtures: &[Fixture]) -> Result<(), String> {
    for fixture in fixtures {
        if fixture.query_sql.is_none() || !fixture.expected.success {
            continue;
        }
        let Some(ref query) = fixture.expected.query else {
            continue;
        };
        let row_returning = matches!(query.command, Command::One | Command::Many | Command::Grouped);
        if row_returning && query.columns.is_empty() {
            return Err(format!(
                "{}: query command `{}` returns rows but declares no `expected.query.columns`",
                fixture.file_path.as_deref().unwrap_or(&fixture.name),
                query.command,
            ));
        }
    }
    Ok(())
}

/// Emit a bool assertion that satisfies clippy's `bool_assert_comparison` lint.
///
/// `assert_eq!(expr, true/false, msg)` triggers the lint; use `assert!` instead.
fn bool_assert(expr: &str, value: bool, msg: &str) -> String {
    if value {
        format!("    assert!({expr}, \"{msg}\");\n")
    } else {
        format!("    assert!(!{expr}, \"{msg}\");\n")
    }
}

/// Scythe test generator -- turns JSON fixture files into Rust integration tests.
#[derive(Parser, Debug)]
#[command(name = "test-generator", version, about)]
struct Cli {
    /// Directory containing JSON fixture files.
    #[arg(long, default_value = "testing_data")]
    fixtures: PathBuf,

    /// Output directory for generated test files.
    #[arg(long, default_value = "tests/generated")]
    output: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if !cli.fixtures.is_dir() {
        return Err(format!("fixtures directory does not exist: {}", cli.fixtures.display()).into());
    }

    let fixtures = fixture::load_fixtures(&cli.fixtures)?;
    if fixtures.is_empty() {
        eprintln!("warning: no fixture files found in {}", cli.fixtures.display());
        return Ok(());
    }
    validate_fixtures(&fixtures)?;

    let mut groups: BTreeMap<String, Vec<&Fixture>> = BTreeMap::new();
    for f in &fixtures {
        let top = f.category.split('/').next().unwrap_or(&f.category).to_string();
        groups.entry(top).or_default().push(f);
    }

    fs::create_dir_all(&cli.output)?;

    let mut module_names: Vec<String> = Vec::new();

    for (category, category_fixtures) in &groups {
        let module_name = sanitize_module_name(category);
        let file_name = format!("test_{}.rs", module_name);
        let file_path = cli.output.join(&file_name);

        let code = generate_test_file(category, category_fixtures);
        fs::write(&file_path, &code)?;

        println!("wrote {}", file_path.display());
        module_names.push(module_name);
    }

    let mod_path = cli.output.join("mod.rs");
    let mod_code = generate_mod_rs(&module_names);
    fs::write(&mod_path, &mod_code)?;
    println!("wrote {}", mod_path.display());

    println!(
        "generated {} test file(s) from {} fixture(s)",
        module_names.len(),
        fixtures.len()
    );

    if !fixtures.is_empty() && module_names.is_empty() {
        return Err("fixtures were loaded but no tests were generated".into());
    }

    Ok(())
}

fn generate_mod_rs(module_names: &[String]) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("// Auto-generated by test-generator. Do not edit.\n\n");
    for name in module_names {
        let _ = writeln!(out, "mod test_{};", name);
    }
    out
}

fn generate_test_file(category: &str, fixtures: &[&Fixture]) -> String {
    let mut out = String::with_capacity(64 * 1024);
    out.push_str("// Auto-generated by test-generator. Do not edit.\n");
    let _ = writeln!(out, "// Category: {}\n", category);
    out.push_str("#[allow(unused_imports)]\nuse scythe_codegen as _codegen;\n");
    out.push_str("#[allow(unused_imports)]\nuse syn as _syn;\n\n");
    out.push_str("#[allow(dead_code)]\n");
    out.push_str("fn normalize_whitespace(s: &str) -> String {\n");
    out.push_str("    s.split_whitespace().collect::<Vec<_>>().join(\" \")\n");
    out.push_str("}\n\n");

    for fixture in fixtures {
        out.push_str(&generate_single_test(fixture));
        out.push('\n');
    }

    out
}

fn generate_single_test(fixture: &Fixture) -> String {
    let file_path_comment = fixture.file_path.as_deref().unwrap_or("<unknown>");

    let expected = &fixture.expected;
    let is_error = !expected.success;
    let is_lint = fixture.category.starts_with("lint/");

    if is_error {
        generate_error_test(fixture, file_path_comment)
    } else if is_lint {
        generate_lint_test(fixture, file_path_comment)
    } else if fixture.query_sql.is_some() {
        generate_query_test(fixture, file_path_comment)
    } else {
        generate_catalog_test(fixture, file_path_comment)
    }
}

/// The `let catalog = ...;` line for a query test, honouring the fixture's
/// declared engine.
///
/// A fixture that declares no engine, or declares PostgreSQL, keeps the
/// plain `Catalog::from_ddl` call this generator has always emitted — so
/// adding engine awareness leaves every existing PostgreSQL fixture's
/// generated test byte-identical.
///
/// Two axes, because `SqlDialect` collapses PostgreSQL-compatible engines
/// onto one variant: the dialect drives parsing and type resolution, while
/// `with_engine` carries the engine name that gates capabilities the dialect
/// cannot express (Redshift parses as PostgreSQL but has no `json_agg`).
fn catalog_expr(fixture: &Fixture) -> String {
    let Some(engine) = fixture.config.as_ref().and_then(|c| c.engine) else {
        return "    let catalog = scythe_core::catalog::Catalog::from_ddl(schema_sql).unwrap();\n".to_string();
    };

    let base = match engine.dialect_path() {
        Some(dialect) => format!(
            "scythe_core::catalog::Catalog::from_ddl_with_dialect(\n        schema_sql,\n        \
             &scythe_core::dialect::SqlDialect::{dialect},\n    )\n    .unwrap()"
        ),
        None => "scythe_core::catalog::Catalog::from_ddl(schema_sql).unwrap()".to_string(),
    };

    if matches!(engine, fixture::Engine::Postgresql) {
        format!("    let catalog = {base};\n")
    } else {
        format!("    let catalog = {base}.with_engine({:?});\n", engine.as_str())
    }
}

fn generate_catalog_test(fixture: &Fixture, file_path: &str) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("#[test]\n");
    let _ = writeln!(out, "fn test_{}() {{", fixture.name);
    let _ = writeln!(out, "    // From: {}", file_path);
    let _ = writeln!(out, "    // {:?}", fixture.description);

    out.push_str(&format_schema_sql(&fixture.schema_sql));

    // Always bind `catalog` (not `_catalog`) and always run
    // `generate_catalog_assertions`, even when every expected map is empty:
    // that path used to emit nothing but `Catalog::from_ddl(...).unwrap()`,
    // so a fixture declaring zero tables/enums/composites asserted nothing
    // about that -- a regression that spuriously created one would pass
    // silently. `generate_catalog_assertions` now emits a total-count
    // assertion unconditionally, which is a real (if trivially "== 0")
    // assertion for the empty case and closes that gap. See #161.
    out.push_str("    let catalog = scythe_core::catalog::Catalog::from_ddl(schema_sql).unwrap();\n\n");
    if let Some(ref catalog) = fixture.expected.catalog {
        out.push_str(&generate_catalog_assertions(catalog));
    } else {
        out.push_str("    let _ = &catalog;\n");
    }

    out.push_str("}\n");
    out
}

fn generate_query_test(fixture: &Fixture, file_path: &str) -> String {
    let mut out = String::with_capacity(8192);
    out.push_str("#[test]\n");
    let _ = writeln!(out, "fn test_{}() {{", fixture.name);
    let _ = writeln!(out, "    // From: {}", file_path);
    let _ = writeln!(out, "    // {:?}", fixture.description);

    out.push_str(&format_schema_sql(&fixture.schema_sql));

    let query_sql = fixture.query_sql.as_deref().unwrap_or("");
    let _ = writeln!(out, "    let query_sql = {:?};\n", query_sql);

    out.push_str(&catalog_expr(fixture));
    out.push_str("    let query = scythe_core::parser::parse_query(query_sql).unwrap();\n");
    out.push_str("    let analyzed = scythe_core::analyzer::analyze(&catalog, &query).unwrap();\n\n");

    if let Some(ref catalog) = fixture.expected.catalog {
        out.push_str(&generate_catalog_assertions(catalog));
    }

    if let Some(ref query) = fixture.expected.query {
        out.push_str(&generate_query_assertions(query));
    }

    let command = fixture
        .expected
        .query
        .as_ref()
        .map(|q| q.command.to_string())
        .unwrap_or_default();

    let engine = fixture_engine_str(fixture);
    let applicable = applicable_backends(engine);

    out.push_str("    // Codegen verification: every backend that supports this fixture's engine\n");
    out.push_str("    // should produce valid output. The list below is derived at generation\n");
    out.push_str("    // time from the fixture's engine via the real `get_backend` registry, so\n");
    out.push_str("    // every entry here is expected to construct -- a construction failure\n");
    out.push_str("    // below is a real regression, not an expected engine mismatch. See #156.\n");
    let _ = writeln!(out, "    let engine = {:?};", engine);
    out.push_str("    let all_backends = [\n");
    for name in &applicable {
        let _ = writeln!(out, "        {:?},", name);
    }
    out.push_str("    ];\n");
    out.push_str("    for backend_name in &all_backends {\n");
    out.push_str("        let backend = match scythe_codegen::get_backend(backend_name, engine) {\n");
    out.push_str("            Ok(b) => b,\n");
    let _ = writeln!(
        out,
        "            Err(e) => panic!(\"backend {{}} failed to construct for engine {{}} in fixture {{}}: {{}}\", backend_name, engine, {:?}, e),",
        fixture.name
    );
    out.push_str("        };\n");
    // ~keep Emit a bare `None` rather than a one-arm match when the fixture declares no
    // `expected.codegen_errors`. `match *backend_name { _ => None }` is what clippy's
    // `match_single_binding` rejects, and `-D warnings` makes that a build failure on the
    // 440-odd fixtures that declare nothing.
    let match_arms = codegen_error_match_arms(fixture);
    if match_arms.is_empty() {
        out.push_str("        let declared_codegen_failure: Option<&str> = None;\n");
    } else {
        out.push_str("        let declared_codegen_failure: Option<&str> = match *backend_name {\n");
        out.push_str(&match_arms);
        out.push_str("            _ => None,\n");
        out.push_str("        };\n");
    }
    out.push_str("        match scythe_codegen::generate_with_backend(&analyzed, &*backend) {\n");
    out.push_str("            Ok(generated) => {\n");
    out.push_str("                if let Some(expected_message) = declared_codegen_failure {\n");
    let _ = writeln!(
        out,
        "                    panic!(\n                        \"backend {{}} was declared under expected.codegen_errors to fail codegen for fixture {{}} (declared message {{:?}}), but codegen succeeded -- delete the stale entry\",\n                        backend_name, {:?}, expected_message\n                    );",
        fixture.name
    );
    out.push_str("                }\n");
    // ~keep Assembled through `provenance::assemble_file`, exactly as `scythe
    // generate` assembles a real file: preamble, then the provenance header
    // line, then the body. Concatenating preamble + header directly (as this
    // generator used to) produced a file no user ever gets, so the `syn` and
    // structural assertions below were validating a shape that does not
    // ship — and in particular never saw the provenance comment.
    out.push_str("                let preamble = backend.file_preamble();\n");
    out.push_str("                let header = backend.file_header();\n");
    out.push_str("                let mut body = String::new();\n");
    out.push_str("                if header.is_empty() {\n");
    out.push_str("                    body.push_str(\"#![allow(dead_code, unused_imports)]\\n\");\n");
    out.push_str("                } else {\n");
    out.push_str("                    body.push_str(&header);\n");
    out.push_str("                    body.push('\\n');\n");
    out.push_str("                }\n");
    out.push_str("                if let Some(ref s) = generated.enum_def { body.push_str(s); body.push('\\n'); }\n");
    out.push_str(
        "                for def in &generated.nested_struct_defs { body.push_str(&def.code); body.push('\\n'); }\n",
    );
    out.push_str(
        "                if let Some(ref s) = generated.model_struct { body.push_str(s); body.push('\\n'); }\n",
    );
    out.push_str("                if let Some(ref s) = generated.row_struct { body.push_str(s); body.push('\\n'); }\n");
    out.push_str("                if let Some(ref s) = generated.query_fn { body.push_str(s); body.push('\\n'); }\n");
    out.push_str("                let code = scythe_codegen::provenance::assemble_file(\n");
    out.push_str("                    &preamble,\n");
    out.push_str("                    &scythe_codegen::provenance::header_line(\n");
    out.push_str("                        &*backend,\n");
    out.push_str("                        env!(\"CARGO_PKG_VERSION\"),\n");
    out.push_str("                        engine,\n");
    out.push_str("                        \"sch1:0123456789abcdef\",\n");
    out.push_str("                        \"q1:fedcba9876543210\",\n");
    out.push_str("                    ),\n");
    out.push_str("                    &body,\n");
    out.push_str("                );\n");
    // Counted on the body, not on `code`: the preamble and the provenance
    // line are constants present for every backend, so including them would
    // turn this "did the backend emit anything?" guard into a tautology.
    out.push_str("                if body.lines().count() > 1 {\n");
    out.push_str("                    // Only validate Rust syntax with syn for Rust backends\n");
    out.push_str(
        "                    if *backend_name == \"rust-sqlx\" || *backend_name == \"rust-tokio-postgres\" {\n",
    );
    out.push_str("                        assert!(\n");
    out.push_str("                            syn::parse_file(&code).is_ok(),\n");
    let _ = writeln!(
        out,
        "                            \"backend {{}} generated invalid Rust for {{}}\", backend_name, {:?}",
        fixture.name
    );
    out.push_str("                        );\n");
    out.push_str("                    } else {\n");
    out.push_str("                        // Structural validation for non-Rust backends\n");
    out.push_str(
        "                        let errors = scythe_codegen::validation::validate_structural(&code, backend_name);\n",
    );
    out.push_str("                        assert!(\n");
    out.push_str("                            errors.is_empty(),\n");
    let _ = writeln!(
        out,
        "                            \"backend {{}} structural validation failed for {{}}: {{:?}}\", backend_name, {:?}, errors",
        fixture.name
    );
    out.push_str("                        );\n");
    out.push_str("                    }\n");
    out.push_str("                }\n");

    if command == "one" || command == "many" || command == "grouped" {
        out.push_str("                assert!(\n");
        out.push_str("                    generated.row_struct.is_some() || generated.model_struct.is_some(),\n");
        let _ = writeln!(
            out,
            "                    \"backend {{}} should produce a struct for {{}}\", backend_name, {:?}",
            fixture.name
        );
        out.push_str("                );\n");
    }
    out.push_str("                assert!(\n");
    out.push_str("                    generated.query_fn.is_some(),\n");
    let _ = writeln!(
        out,
        "                    \"backend {{}} should produce query_fn for {{}}\", backend_name, {:?}",
        fixture.name
    );
    out.push_str("                );\n");
    out.push_str(&generate_generated_code_assertions(fixture));
    out.push_str("            }\n");
    out.push_str("            Err(e) => match declared_codegen_failure {\n");
    out.push_str("                Some(expected_message) => {\n");
    let _ = writeln!(
        out,
        "                    let message = e.to_string();\n                    assert!(\n                        message.contains(expected_message),\n                        \"backend {{}} codegen error for fixture {{}} did not match the declared expected.codegen_errors message\\n--- expected (substring) ---\\n{{}}\\n--- actual ---\\n{{}}\",\n                        backend_name, {:?}, expected_message, message\n                    );",
        fixture.name
    );
    out.push_str("                }\n");
    let _ = writeln!(
        out,
        "                None => panic!(\"backend {{}} failed to generate code for engine {{}} in fixture {{}}: {{}}\", backend_name, engine, {:?}, e),",
        fixture.name
    );
    out.push_str("            },\n");
    out.push_str("        }\n");
    out.push_str("    }\n");

    out.push_str("}\n");
    out
}

/// Emits the arms of a `match *backend_name { ... }` expression, one per backend this fixture
/// declares under `expected.codegen_errors`, each yielding `Some(message_contains)`. The
/// generated test then knows, purely from data baked in at generation time, which backends in
/// its loop are declared to fail codegen and what their error must say -- everything else falls
/// through to a caller-supplied `_ => None` arm. See #222.
fn codegen_error_match_arms(fixture: &Fixture) -> String {
    let mut out = String::new();
    let Some(ref codegen_errors) = fixture.expected.codegen_errors else {
        return out;
    };

    let mut backends: Vec<_> = codegen_errors.iter().collect();
    backends.sort_unstable_by_key(|(name, _)| name.as_str());

    for (backend_name, expected) in backends {
        let _ = writeln!(
            out,
            "            {:?} => Some({:?}),",
            backend_name, expected.message_contains
        );
    }
    out
}

/// Emits whitespace-normalised `contains` assertions for every backend/field a fixture declares
/// under `expected.generated_code`. Without this, the field deserialises and is silently
/// dropped: a fixture can declare exact expected output for a backend and the generated test
/// asserts nothing about it -- 45 of the 55 fixtures that declared it had already drifted from
/// the shipped output with nothing to notice. `contains`, not exact equality, and
/// whitespace-normalised: the point is to catch a body that materially diverges, not to pin
/// incidental formatting. See #156.
fn generate_generated_code_assertions(fixture: &Fixture) -> String {
    let mut out = String::new();
    let Some(ref generated_code) = fixture.expected.generated_code else {
        return out;
    };

    let mut backends: Vec<_> = generated_code.iter().collect();
    backends.sort_unstable_by_key(|(name, _)| name.as_str());

    for (backend_name, expected) in backends {
        let fields: [(&str, Option<&str>); 4] = [
            ("row_struct", expected.row_struct.as_deref()),
            ("query_fn", expected.query_fn.as_deref()),
            ("enum_def", expected.enum_def.as_deref()),
            ("model_struct", expected.model_struct.as_deref()),
        ];
        let has_field_assertion = fields.iter().any(|(_, value)| value.is_some());
        let has_degradation_assertion = expected.degraded_nested_structs.is_some();
        if !has_field_assertion && !has_degradation_assertion {
            continue;
        }

        let _ = writeln!(out, "                if *backend_name == {:?} {{", backend_name);
        for (field_name, value) in fields {
            let Some(value) = value else { continue };
            let _ = writeln!(
                out,
                "                    let actual = generated.{}.clone().unwrap_or_default();",
                field_name,
            );
            let _ = writeln!(
                out,
                "                    assert!(\n                        normalize_whitespace(&actual).contains(&normalize_whitespace({value:?})),\n                        \"backend {{}} {field} mismatch for {{}}\\n--- expected (substring) ---\\n{{}}\\n--- actual ---\\n{{}}\",\n                        backend_name, {name:?}, {value:?}, actual\n                    );",
                value = value,
                field = field_name,
                name = fixture.name,
            );
        }
        out.push_str(&generate_degraded_nested_structs_assertion(expected, fixture));
        out.push_str("                }\n");
    }

    out
}

/// Emits an `assert_eq!` against `scythe_codegen::GeneratedCode::degraded_nested_structs`
/// for one backend, when the fixture declares `expected.degraded_nested_structs` for it.
///
/// This targets the typed record `degrade_unsupported_nested_structs` produces (GH #147)
/// instead of pattern-matching a language-specific rendered field type: doing the latter
/// for every backend that declares `json_nested`/`json_array` would mean re-deriving each
/// backend's own type-resolution syntax (container templates, nullable wrapping, field
/// naming) by hand in fixture data, which is exactly the kind of drift-prone duplication
/// the manifest-driven resolver exists to avoid. `Some(vec![])` asserts the backend
/// constructed a real nested struct for every nested-aggregate column (no degradation);
/// a non-empty vec asserts exactly which column degraded and to which scalar.
fn generate_degraded_nested_structs_assertion(expected: &ExpectedGeneratedCode, fixture: &Fixture) -> String {
    let mut out = String::new();
    let Some(ref degradations) = expected.degraded_nested_structs else {
        return out;
    };

    out.push_str(
        "                    let expected_degradations: Vec<scythe_codegen::NestedStructDegradation> = vec![\n",
    );
    for degradation in degradations {
        out.push_str("                        scythe_codegen::NestedStructDegradation {\n");
        let _ = writeln!(
            out,
            "                            column: {:?}.to_string(),",
            degradation.column
        );
        let _ = writeln!(
            out,
            "                            struct_name: {:?}.to_string(),",
            degradation.struct_name
        );
        let _ = writeln!(
            out,
            "                            fallback_type: {:?},",
            degradation.fallback_type
        );
        out.push_str("                            backend: (*backend_name).to_string(),\n");
        out.push_str("                        },\n");
    }
    out.push_str("                    ];\n");
    let _ = writeln!(
        out,
        "                    assert_eq!(\n                        generated.degraded_nested_structs, expected_degradations,\n                        \"backend {{}} degraded_nested_structs mismatch for {{}}\\n--- expected ---\\n{{:?}}\\n--- actual ---\\n{{:?}}\",\n                        backend_name, {:?}, expected_degradations, generated.degraded_nested_structs\n                    );",
        fixture.name
    );
    out
}

fn generate_lint_test(fixture: &Fixture, file_path: &str) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("#[test]\n");
    let _ = writeln!(out, "fn test_{}() {{", fixture.name);
    let _ = writeln!(out, "    // From: {}", file_path);
    let _ = writeln!(out, "    // {:?}", fixture.description);

    out.push_str(&format_schema_sql(&fixture.schema_sql));

    out.push_str("    let catalog = scythe_core::catalog::Catalog::from_ddl(schema_sql).unwrap();\n");
    out.push_str("    let registry = scythe_lint::default_registry();\n");
    out.push_str("    let engine = scythe_lint::LintEngine::new(registry);\n");
    out.push_str("    let mut _violations: Vec<(scythe_lint::Violation, scythe_lint::Severity)> = Vec::new();\n\n");

    out.push_str("    _violations.extend(engine.check_catalog(&catalog));\n\n");

    if let Some(ref query_sql) = fixture.query_sql {
        let _ = writeln!(out, "    let query_sql = {:?};", query_sql);
        // `.expect(...)`, not `if let Ok(...) = ... && let Ok(...) = ...`: the
        // old pattern silently skipped `check_query` -- leaving `_violations`
        // empty -- on a parse or analyze failure, which every `*_clean`
        // fixture's `assert!(_violations.is_empty(), ...)` cannot distinguish
        // from "the rule genuinely found nothing". A parser or analyzer
        // regression that breaks a fixture's SQL would silently turn a real
        // assertion into a no-op instead of failing the test that exists to
        // catch it. See #161.
        out.push_str(
            "    let query = scythe_core::parser::parse_query(query_sql).expect(\"fixture SQL must parse\");\n",
        );
        out.push_str(
            "    let analyzed = scythe_core::analyzer::analyze(&catalog, &query).expect(\"fixture SQL must analyze\");\n",
        );
        out.push_str("    let ctx = scythe_lint::LintContext {\n");
        out.push_str("        sql: &query.sql,\n");
        out.push_str("        stmt: &query.stmt,\n");
        out.push_str("        analyzed: &analyzed,\n");
        out.push_str("        catalog: &catalog,\n");
        out.push_str("        annotations: &query.annotations,\n");
        out.push_str("        dialect: scythe_core::dialect::SqlDialect::PostgreSQL,\n");
        out.push_str("    };\n");
        out.push_str("    _violations.extend(engine.check_query(&ctx));\n");
    }

    if let Some(ref lint) = fixture.expected.lint {
        if lint.violations.is_empty() {
            out.push_str(
                "    assert!(_violations.is_empty(), \"expected no lint violations but got {}\", _violations.len());\n",
            );
        } else {
            let _ = writeln!(
                out,
                "    assert!(!_violations.is_empty(), \"expected lint violations for {}\");",
                fixture.name
            );
            for v in &lint.violations {
                let _ = writeln!(
                    out,
                    "    assert!(_violations.iter().any(|v| v.0.rule_id.contains({code:?})), \"expected violation {code}\");",
                    code = v.rule_code
                );
            }
        }
    }

    out.push_str("}\n");
    out
}

fn generate_error_test(fixture: &Fixture, file_path: &str) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("#[test]\n");
    let _ = writeln!(out, "fn test_{}() {{", fixture.name);
    let _ = writeln!(out, "    // From: {}", file_path);
    let _ = writeln!(out, "    // {:?}", fixture.description);

    out.push_str(&format_schema_sql(&fixture.schema_sql));

    let has_query = fixture.query_sql.is_some();

    if has_query {
        let query_sql = fixture.query_sql.as_deref().unwrap_or("");
        let _ = writeln!(out, "    let query_sql = {:?};\n", query_sql);
        out.push_str("    let catalog_result = scythe_core::catalog::Catalog::from_ddl(schema_sql);\n");
        out.push_str("    if let Ok(catalog) = catalog_result {\n");
        out.push_str("        let query_result = scythe_core::parser::parse_query(query_sql);\n");
        out.push_str("        if let Ok(query) = query_result {\n");
        out.push_str("            let result = scythe_core::analyzer::analyze(&catalog, &query);\n");
        out.push_str("            assert!(result.is_err(), \"expected analysis to fail\");\n");

        if let Some(ref error) = fixture.expected.error {
            out.push_str(&generate_error_assertions_nested("result", error, 12));
        }

        out.push_str("        } else {\n");
        out.push_str("            // Parse failed -- that counts as expected failure.\n");

        if let Some(ref error) = fixture.expected.error {
            out.push_str(&generate_error_assertions_nested("query_result", error, 12));
        }

        out.push_str("        }\n");
        out.push_str("    } else {\n");
        out.push_str("        // DDL processing failed -- that counts as expected failure.\n");

        if let Some(ref error) = fixture.expected.error {
            out.push_str(&generate_error_assertions_nested("catalog_result", error, 8));
        }

        out.push_str("    }\n");
    } else {
        out.push_str("    let result = scythe_core::catalog::Catalog::from_ddl(schema_sql);\n");
        out.push_str("    assert!(result.is_err(), \"expected DDL processing to fail\");\n");

        if let Some(ref error) = fixture.expected.error {
            out.push_str(&generate_error_assertions_nested("result", error, 4));
        }
    }

    out.push_str("}\n");
    out
}

fn generate_catalog_assertions(catalog: &ExpectedCatalog) -> String {
    let mut out = String::with_capacity(4096);

    // Total-count assertions first, unconditionally -- including the `== 0`
    // case, which used to emit no assertion at all (see `generate_catalog_test`).
    // The per-item loops below only ever check that each *declared* table/
    // enum/composite exists; they never notice an *extra*, undeclared one, so
    // without this a regression that spuriously created (or failed to drop)
    // a catalog entity was invisible to every catalog fixture. See #161.
    let _ = writeln!(
        out,
        "    assert_eq!(catalog.tables_iter().count(), {}, \"total table count\");",
        catalog.tables.len(),
    );
    let _ = writeln!(
        out,
        "    assert_eq!(catalog.enums_iter().count(), {}, \"total enum count\");",
        catalog.enums.len(),
    );
    let _ = writeln!(
        out,
        "    assert_eq!(catalog.composites_iter().count(), {}, \"total composite count\");",
        catalog.composites.len(),
    );
    out.push('\n');

    let mut tables: Vec<_> = catalog.tables.iter().collect();
    tables.sort_unstable_by_key(|(name, _)| name.as_str());
    for (table_name, table) in tables {
        let _ = writeln!(out, "    // Assert table: {}", table_name);
        let _ = writeln!(
            out,
            "    let table_{clean} = catalog.get_table({name:?}).expect(\"table {name} should exist\");",
            clean = sanitize_ident(table_name),
            name = table_name,
        );
        let _ = writeln!(
            out,
            "    assert_eq!(table_{}.columns.len(), {}, \"column count for table {}\");",
            sanitize_ident(table_name),
            table.columns.len(),
            table_name,
        );

        for (i, col) in table.columns.iter().enumerate() {
            let tvar = sanitize_ident(table_name);
            let _ = writeln!(
                out,
                "    assert_eq!(table_{tvar}.columns[{i}].name, {name:?}, \"column name\");",
                tvar = tvar,
                i = i,
                name = col.name,
            );
            let _ = writeln!(
                out,
                "    assert_eq!(table_{tvar}.columns[{i}].sql_type, {sql_type:?}, \"column sql_type for {col_name}\");",
                tvar = tvar,
                i = i,
                sql_type = col.sql_type,
                col_name = col.name,
            );
            out.push_str(&bool_assert(
                &format!("table_{tvar}.columns[{i}].nullable", tvar = tvar, i = i),
                col.nullable,
                &format!("column nullable for {}", col.name),
            ));

            if let Some(ref default) = col.default {
                let _ = writeln!(
                    out,
                    "    assert_eq!(table_{tvar}.columns[{i}].default.as_deref(), Some({default:?}), \"column default for {col_name}\");",
                    tvar = tvar,
                    i = i,
                    default = default,
                    col_name = col.name,
                );
            }

            // `Column::primary_key` is a plain `bool`, so the assertion
            // asserts on it directly. The fixture field is the `Option`, and
            // it stays one: `None` means "this fixture says nothing about
            // primary keys" and emits no assertion at all, which is what
            // lets the field be added to one fixture without silently
            // asserting `primary_key == false` across every other.
            if let Some(pk) = col.primary_key {
                out.push_str(&bool_assert(
                    &format!("table_{tvar}.columns[{i}].primary_key", tvar = tvar, i = i),
                    pk,
                    &format!("column primary_key for {}", col.name),
                ));
            }
        }
        out.push('\n');
    }

    let mut enums: Vec<_> = catalog.enums.iter().collect();
    enums.sort_unstable_by_key(|(name, _)| name.as_str());
    for (enum_name, enum_def) in enums {
        let _ = writeln!(out, "    // Assert enum: {}", enum_name);
        let _ = writeln!(
            out,
            "    let enum_{clean} = catalog.get_enum({name:?}).expect(\"enum {name} should exist\");",
            clean = sanitize_ident(enum_name),
            name = enum_name,
        );
        let _ = writeln!(
            out,
            "    assert_eq!(enum_{clean}.values, vec![{values}], \"enum values for {name}\");",
            clean = sanitize_ident(enum_name),
            values = enum_def
                .values
                .iter()
                .map(|v| format!("{:?}", v))
                .collect::<Vec<_>>()
                .join(", "),
            name = enum_name,
        );
        out.push('\n');
    }

    let mut composites: Vec<_> = catalog.composites.iter().collect();
    composites.sort_unstable_by_key(|(name, _)| name.as_str());
    for (comp_name, comp) in composites {
        let _ = writeln!(out, "    // Assert composite: {}", comp_name);
        let _ = writeln!(
            out,
            "    let comp_{clean} = catalog.get_composite({name:?}).expect(\"composite {name} should exist\");",
            clean = sanitize_ident(comp_name),
            name = comp_name,
        );
        let _ = writeln!(
            out,
            "    assert_eq!(comp_{clean}.fields.len(), {len}, \"field count for composite {name}\");",
            clean = sanitize_ident(comp_name),
            len = comp.fields.len(),
            name = comp_name,
        );

        for (i, field) in comp.fields.iter().enumerate() {
            let cvar = sanitize_ident(comp_name);
            let _ = writeln!(
                out,
                "    assert_eq!(comp_{cvar}.fields[{i}].name, {name:?}, \"composite field name\");",
                cvar = cvar,
                i = i,
                name = field.name,
            );
            let _ = writeln!(
                out,
                "    assert_eq!(comp_{cvar}.fields[{i}].sql_type, {sql_type:?}, \"composite field sql_type for {field_name}\");",
                cvar = cvar,
                i = i,
                sql_type = field.sql_type,
                field_name = field.name,
            );
        }
        out.push('\n');
    }

    out
}

fn generate_query_assertions(query: &ExpectedQuery) -> String {
    let mut out = String::with_capacity(4096);

    let _ = writeln!(out, "    assert_eq!(analyzed.name, {:?}, \"query name\");", query.name,);
    let _ = writeln!(
        out,
        "    assert_eq!(analyzed.command.to_string(), {:?}, \"query command\");",
        query.command.to_string(),
    );

    if !query.params.is_empty() {
        let _ = writeln!(
            out,
            "    assert_eq!(analyzed.params.len(), {}, \"param count\");",
            query.params.len(),
        );
        for (i, param) in query.params.iter().enumerate() {
            let _ = writeln!(
                out,
                "    assert_eq!(analyzed.params[{i}].name, {name:?}, \"param name\");",
                i = i,
                name = param.name,
            );
            let _ = writeln!(
                out,
                "    assert_eq!(analyzed.params[{i}].neutral_type, {neutral_type:?}, \"param neutral_type for {pname}\");",
                i = i,
                neutral_type = param.neutral_type,
                pname = param.name,
            );
            out.push_str(&bool_assert(
                &format!("analyzed.params[{i}].nullable", i = i),
                param.nullable,
                &format!("param nullable for {}", param.name),
            ));
            if let Some(position) = param.position {
                let _ = writeln!(
                    out,
                    "    assert_eq!(analyzed.params[{i}].position, {position}, \"param position for {pname}\");",
                    i = i,
                    position = position,
                    pname = param.name,
                );
            }
        }
    }

    if !query.columns.is_empty() {
        let _ = writeln!(
            out,
            "    assert_eq!(analyzed.columns.len(), {}, \"column count\");",
            query.columns.len(),
        );
        for (i, col) in query.columns.iter().enumerate() {
            let _ = writeln!(
                out,
                "    assert_eq!(analyzed.columns[{i}].name, {name:?}, \"column name\");",
                i = i,
                name = col.name,
            );
            let _ = writeln!(
                out,
                "    assert_eq!(analyzed.columns[{i}].neutral_type, {neutral_type:?}, \"column neutral_type for {cname}\");",
                i = i,
                neutral_type = col.neutral_type,
                cname = col.name,
            );
            out.push_str(&bool_assert(
                &format!("analyzed.columns[{i}].nullable", i = i),
                col.nullable,
                &format!("column nullable for {}", col.name),
            ));
        }
    }

    if let Some(ref nested_structs) = query.nested_structs {
        let _ = writeln!(
            out,
            "    assert_eq!(analyzed.nested_structs.len(), {}, \"nested struct count\");",
            nested_structs.len(),
        );
        for (i, nested) in nested_structs.iter().enumerate() {
            let _ = writeln!(
                out,
                "    assert_eq!(analyzed.nested_structs[{i}].name, {name:?}, \"nested struct name\");",
                i = i,
                name = nested.name,
            );
            let _ = writeln!(
                out,
                "    assert_eq!(analyzed.nested_structs[{i}].fields.len(), {len}, \"nested field count for {name}\");",
                i = i,
                len = nested.fields.len(),
                name = nested.name,
            );
            for (j, field) in nested.fields.iter().enumerate() {
                let _ = writeln!(
                    out,
                    "    assert_eq!(analyzed.nested_structs[{i}].fields[{j}].name, {name:?}, \"nested field name\");",
                    i = i,
                    j = j,
                    name = field.name,
                );
                let _ = writeln!(
                    out,
                    "    assert_eq!(analyzed.nested_structs[{i}].fields[{j}].neutral_type, {ty:?}, \"nested field neutral_type for {fname}\");",
                    i = i,
                    j = j,
                    ty = field.neutral_type,
                    fname = field.name,
                );
                out.push_str(&bool_assert(
                    &format!("analyzed.nested_structs[{i}].fields[{j}].nullable", i = i, j = j),
                    field.nullable,
                    &format!("nested field nullable for {}", field.name),
                ));
            }
        }
    }

    out.push('\n');
    out
}

fn generate_error_assertions_nested(result_var: &str, error: &fixture::ExpectedError, indent: usize) -> String {
    let mut out = String::with_capacity(4096);
    let pad = " ".repeat(indent);

    let _ = writeln!(out, "{pad}let err = {var}.unwrap_err();", pad = pad, var = result_var,);
    let _ = writeln!(out, "{pad}let err_msg = err.to_string();", pad = pad,);

    if let Some(ref code) = error.code {
        let _ = writeln!(
            out,
            "{pad}assert!(err_msg.contains({code:?}), \"error should contain code {{:?}}, got: {{}}\", {code:?}, err_msg);",
            pad = pad,
            code = code,
        );
    }

    if let Some(ref msg) = error.message_contains {
        let _ = writeln!(
            out,
            "{pad}assert!(err_msg.contains({msg:?}), \"error should contain {{:?}}, got: {{}}\", {msg:?}, err_msg);",
            pad = pad,
            msg = msg,
        );
    }

    out
}

fn format_schema_sql(stmts: &[String]) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("    let schema_sql = &[\n");
    for stmt in stmts {
        let _ = writeln!(out, "        {:?},", stmt);
    }
    out.push_str("    ];\n\n");
    out
}

fn sanitize_module_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect::<String>()
        .to_lowercase()
}

fn sanitize_ident(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect::<String>()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_from_json(json: &str) -> Fixture {
        serde_json::from_str(json).expect("test fixture JSON must parse")
    }

    #[test]
    fn validate_fixtures_rejects_a_many_query_with_no_declared_columns() {
        let fixture = fixture_from_json(
            r#"{
                "name": "missing_columns",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": {
                    "success": true,
                    "query": { "name": "GetT", "command": "many" }
                },
                "source": "original"
            }"#,
        );

        let error = validate_fixtures(&[fixture]).expect_err("a `many` query with no columns must be rejected");
        assert!(
            error.contains("returns rows but declares no `expected.query.columns`"),
            "error must name the missing field, got: {error}"
        );
    }

    #[test]
    fn validate_fixtures_accepts_an_exec_query_with_no_declared_columns() {
        let fixture = fixture_from_json(
            r#"{
                "name": "exec_no_columns",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "DELETE FROM t",
                "expected": {
                    "success": true,
                    "query": { "name": "DeleteT", "command": "exec" }
                },
                "source": "original"
            }"#,
        );

        assert_eq!(
            validate_fixtures(&[fixture]),
            Ok(()),
            "exec commands return no rows and need no declared columns"
        );
    }

    /// Regression for #156: `ruby-trilogy` only supports MySQL/MariaDB, but the generator used
    /// to embed it in every fixture's backend list regardless of engine and silently `continue`
    /// past its construction failure -- contributing zero assertions to ~370 fixtures.
    #[test]
    fn applicable_backends_excludes_ruby_trilogy_for_postgresql() {
        let backends = applicable_backends("postgresql");
        assert!(
            !backends.contains(&"ruby-trilogy"),
            "ruby-trilogy does not support postgresql and must be excluded, got: {backends:?}"
        );
        assert!(
            backends.contains(&"rust-sqlx"),
            "rust-sqlx supports postgresql and must be included, got: {backends:?}"
        );
    }

    #[test]
    fn applicable_backends_is_engine_specific() {
        let mysql_backends = applicable_backends("mysql");
        assert!(
            !mysql_backends.contains(&"csharp-npgsql"),
            "csharp-npgsql (Npgsql) has no MySQL manifest and must be excluded, got: {mysql_backends:?}"
        );
    }

    #[test]
    fn generate_generated_code_assertions_is_empty_when_the_fixture_declares_none() {
        let fixture = fixture_from_json(
            r#"{
                "name": "no_generated_code",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": { "success": true },
                "source": "original"
            }"#,
        );

        assert_eq!(generate_generated_code_assertions(&fixture), String::new());
    }

    /// Regression for #156: `expected.generated_code` used to deserialise and be dropped --
    /// 45 of 55 fixtures that declared it had already drifted from the shipped output with
    /// nothing to notice.
    #[test]
    fn generate_generated_code_assertions_emits_a_contains_check_per_declared_field() {
        let fixture = fixture_from_json(
            r#"{
                "name": "with_generated_code",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": {
                    "success": true,
                    "generated_code": {
                        "rust-sqlx": { "row_struct": "pub struct GetTRow { pub id: i32 }" }
                    }
                },
                "source": "original"
            }"#,
        );

        let code = generate_generated_code_assertions(&fixture);
        assert!(code.contains(r#"if *backend_name == "rust-sqlx""#), "got:\n{code}");
        assert!(code.contains("generated.row_struct.clone()"), "got:\n{code}");
        assert!(
            code.contains("normalize_whitespace(&actual).contains(&normalize_whitespace"),
            "got:\n{code}"
        );
    }

    /// Regression guard for GH #147's unfalsifiable gate: a fixture that declares
    /// `expected.query.columns[].type` as `json_nested<...>` but whose generated-code
    /// assertions only check `row_struct.is_some()` passes identically whether the
    /// backend rendered a real nested struct or silently collapsed the column to a
    /// bare scalar. `degraded_nested_structs` is the structural signal that
    /// distinguishes the two, so a backend entry that declares only that field --
    /// no `row_struct`/`query_fn`/`enum_def`/`model_struct` -- must still emit an
    /// `if *backend_name == ...` block, or the assertion is silently dropped exactly
    /// like the #156 defect this mechanism already fixed once.
    #[test]
    fn generate_generated_code_assertions_emits_a_block_for_degradation_only_entries() {
        let fixture = fixture_from_json(
            r#"{
                "name": "degradation_only",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": {
                    "success": true,
                    "generated_code": {
                        "python-asyncpg": { "degraded_nested_structs": [] }
                    }
                },
                "source": "original"
            }"#,
        );

        let code = generate_generated_code_assertions(&fixture);
        assert!(
            code.contains(r#"if *backend_name == "python-asyncpg""#),
            "degradation-only entry must still open its backend block, got:\n{code}"
        );
        assert!(code.contains("expected_degradations"), "got:\n{code}");
    }

    /// `Some(vec![])` asserts the backend produced a real nested struct for every
    /// nested-aggregate column -- no degradation. This is the falsifiable
    /// counterpart to the old `row_struct.is_some()` gate: on a backend that
    /// silently collapsed a `json_agg` column to a scalar, `degraded_nested_structs`
    /// is non-empty, so this `assert_eq!` fails with the actual fallback recorded.
    #[test]
    fn generate_degraded_nested_structs_assertion_asserts_empty_vec_when_declared_empty() {
        let fixture = fixture_from_json(
            r#"{
                "name": "no_degradation_expected",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": {
                    "success": true,
                    "generated_code": {
                        "rust-sqlx": { "degraded_nested_structs": [] }
                    }
                },
                "source": "original"
            }"#,
        );

        let code = generate_generated_code_assertions(&fixture);
        assert!(
            code.contains("let expected_degradations: Vec<scythe_codegen::NestedStructDegradation> = vec![\n                    ];"),
            "empty declaration must build an empty expected vec, got:\n{code}"
        );
        assert!(
            code.contains(
                "assert_eq!(\n                        generated.degraded_nested_structs, expected_degradations,"
            ),
            "got:\n{code}"
        );
    }

    /// A non-empty declaration must name the exact column, struct and fallback type,
    /// so a mismatched fallback (a backend degrading to `\"json\"` when the fixture
    /// expects the richer `\"json_array\"`, or vice versa) fails with both sides
    /// printed rather than a generic truthiness check.
    #[test]
    fn generate_degraded_nested_structs_assertion_emits_exact_fields_for_a_degradation() {
        let fixture = fixture_from_json(
            r#"{
                "name": "one_degradation",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": {
                    "success": true,
                    "generated_code": {
                        "python-asyncpg": {
                            "degraded_nested_structs": [
                                {
                                    "column": "orders",
                                    "struct_name": "GetUserOrdersRowOrders",
                                    "fallback_type": "json_array"
                                }
                            ]
                        }
                    }
                },
                "source": "original"
            }"#,
        );

        let code = generate_generated_code_assertions(&fixture);
        assert!(code.contains(r#"column: "orders".to_string(),"#), "got:\n{code}");
        assert!(
            code.contains(r#"struct_name: "GetUserOrdersRowOrders".to_string(),"#),
            "got:\n{code}"
        );
        assert!(code.contains(r#"fallback_type: "json_array","#), "got:\n{code}");
        assert!(code.contains("backend: (*backend_name).to_string(),"), "got:\n{code}");
    }

    #[test]
    fn codegen_error_match_arms_is_empty_when_the_fixture_declares_none() {
        let fixture = fixture_from_json(
            r#"{
                "name": "no_codegen_errors",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": { "success": true },
                "source": "original"
            }"#,
        );

        assert_eq!(codegen_error_match_arms(&fixture), String::new());
    }

    #[test]
    fn codegen_error_match_arms_emits_an_arm_per_declared_backend() {
        let fixture = fixture_from_json(
            r#"{
                "name": "with_codegen_errors",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": {
                    "success": true,
                    "codegen_errors": {
                        "python-asyncpg": {
                            "message_contains": "unsupported type",
                            "reason": "asyncpg has no mapping for this type; re-derive before editing"
                        }
                    }
                },
                "source": "original"
            }"#,
        );

        let arms = codegen_error_match_arms(&fixture);
        assert_eq!(arms, "            \"python-asyncpg\" => Some(\"unsupported type\"),\n");
    }

    /// Regression for #222: `generate_with_backend`'s `Err` used to be discarded by an `if let
    /// Ok(...)` guard, silently skipping every downstream assertion (including
    /// `generate_generated_code_assertions`, which #156 added specifically so assertions were
    /// not dropped). The emitted test must instead `match` the result and panic on an
    /// undeclared failure, naming the backend, engine and fixture -- the same style as the
    /// existing backend-construction panic.
    #[test]
    fn generate_query_test_panics_on_an_undeclared_codegen_failure() {
        let fixture = fixture_from_json(
            r#"{
                "name": "codegen_failure_test",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": {
                    "success": true,
                    "query": { "name": "GetT", "command": "many", "columns": [
                        { "name": "id", "type": "int4", "nullable": false }
                    ] }
                },
                "source": "original"
            }"#,
        );

        let code = generate_query_test(&fixture, "<test>");
        assert!(
            !code.contains("if let Ok(generated) ="),
            "the silent-skip guard must be gone, got:\n{code}"
        );
        assert!(
            code.contains("match scythe_codegen::generate_with_backend(&analyzed, &*backend) {"),
            "got:\n{code}"
        );
        assert!(
            code.contains(
                "None => panic!(\"backend {} failed to generate code for engine {} in fixture {}: {}\", backend_name, engine, \"codegen_failure_test\", e),"
            ),
            "an undeclared backend must panic naming the fixture on codegen failure, got:\n{code}"
        );
    }

    /// The declared-backend path must assert the error message rather than panic, and the
    /// success path must panic when a backend *declared* to fail codegen instead succeeds --
    /// the stale-allowlist direction, modeled on `scripts/check-generated-backends.py`'s
    /// both-directions check.
    #[test]
    fn generate_query_test_asserts_the_declared_message_and_flags_a_stale_entry() {
        let fixture = fixture_from_json(
            r#"{
                "name": "codegen_failure_declared",
                "description": "d",
                "category": "smoke",
                "schema_sql": ["CREATE TABLE t (id INT)"],
                "query_sql": "SELECT id FROM t",
                "expected": {
                    "success": true,
                    "query": { "name": "GetT", "command": "many", "columns": [
                        { "name": "id", "type": "int4", "nullable": false }
                    ] },
                    "codegen_errors": {
                        "python-asyncpg": {
                            "message_contains": "unsupported type",
                            "reason": "asyncpg has no mapping for this type; re-derive before editing"
                        }
                    }
                },
                "source": "original"
            }"#,
        );

        let code = generate_query_test(&fixture, "<test>");
        assert!(
            code.contains("\"python-asyncpg\" => Some(\"unsupported type\"),"),
            "got:\n{code}"
        );
        assert!(
            code.contains("message.contains(expected_message)"),
            "a declared backend's error must be checked against the declared substring, got:\n{code}"
        );
        assert!(
            code.contains("was declared under expected.codegen_errors to fail codegen for fixture"),
            "a declared backend that now succeeds must panic as a stale entry, got:\n{code}"
        );
    }
}
