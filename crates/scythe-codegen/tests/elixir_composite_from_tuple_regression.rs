//! Board #219: a composite column decodes wrong on `elixir-postgrex` and `elixir-ecto`.
//!
//! Both backends run every raw-SQL query through Postgrex's binary protocol (`Ecto.Adapters.SQL`
//! delegates to the same driver as a direct `Postgrex.query`). Verified live against PostgreSQL
//! 16 (`docker exec scythe-live-pg psql ...` plus `Postgrex.query/3` from a real `mix run`
//! script): an unregistered composite column decodes to a bare positional *tuple*, with every
//! field already recursively decoded to its natural Elixir type -- an `int4` field is already an
//! `integer()`, a nested composite field is itself another tuple (or `nil` for a NULL sub-field),
//! never text.
//!
//! Before this fix, `generate_query_fn`/`generate_grouped_query_fn` assigned that tuple straight
//! into the field the row struct declares as `%{Struct}{}` (`field_name: field_name`). A caller
//! calling `.field` or pattern-matching the declared struct on that value got a `KeyError` /
//! `FunctionClauseError` at runtime instead of the generated type.

use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_with_tools};
use scythe_codegen::{CodegenBackend, GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TYPE inner_probe AS (x INTEGER, y INTEGER); \
    CREATE TYPE outer_probe AS (label TEXT, pt inner_probe); \
    CREATE TABLE things (id SERIAL PRIMARY KEY, val outer_probe NOT NULL);";

const QUERY: &str = "-- @name GetThing\n-- @returns :one\nSELECT id, val FROM things WHERE id = $1;";

fn backend_for(name: &str) -> Box<dyn CodegenBackend> {
    get_backend(name, "postgresql").expect("backend must support postgresql")
}

fn generate(backend: &dyn CodegenBackend) -> GeneratedCode {
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    generate_with_backend(&analyzed, backend).expect("codegen must succeed")
}

fn generated_model_struct(backend_name: &str) -> String {
    generate(&*backend_for(backend_name))
        .model_struct
        .expect("composite columns must produce a model_struct")
}

fn generated_query_fn(backend_name: &str) -> String {
    generate(&*backend_for(backend_name))
        .query_fn
        .expect("query must produce a query_fn")
}

/// Assembles the full generated file (header, composite `from_tuple`, row struct, query
/// function, footer) the same way `scythe-cli` would write it to disk.
fn generated_file(backend_name: &str) -> String {
    let backend = backend_for(backend_name);
    let code = generate(&*backend);
    let all = std::slice::from_ref(&code);

    let mut body = backend.file_header_for_results(all);
    body.push('\n');
    // Types first, then the query function *inside* the module `query_class_header` opens.
    // `file_footer` is the `end` that closes that module, so assembling the two without the
    // header between them leaves a stray `end` and elixirc reports
    // "unexpected reserved word: end" -- a defect in this test's own assembly rather than in
    // anything the backend emitted. `run_generate` opens the module the same way.
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

    scythe_codegen::provenance::assemble_file(
        &backend.file_preamble(),
        &scythe_codegen::provenance::header_line(
            &*backend,
            env!("CARGO_PKG_VERSION"),
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ),
        &body,
    )
}

/// Mirrors `python_composite_enum_read_regression.rs`'s `assert_file_compiles`: runs the real
/// `elixirc` compiler against the fully assembled generated file.
fn assert_file_compiles(backend_name: &str, file: String) {
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

/// Both backends must emit a nil-safe `from_tuple/1` for every composite, including one that
/// recurses into a nested composite field -- and the query function must call it rather than
/// assign the raw tuple straight into the struct field.
///
/// Before this fix `model_struct` had no `from_tuple` at all, and `query_fn` read
/// `val: val` -- the destructured tuple assigned directly, never converted.
#[test]
fn both_elixir_backends_emit_from_tuple_and_route_composite_columns_through_it() {
    for backend in ["elixir-postgrex", "elixir-ecto"] {
        let model_struct = generated_model_struct(backend);
        assert!(
            model_struct.contains("defmodule InnerProbe do"),
            "{backend}: missing InnerProbe; got:\n{model_struct}"
        );
        assert!(
            model_struct.contains("defmodule OuterProbe do"),
            "{backend}: missing OuterProbe; got:\n{model_struct}"
        );
        assert!(
            model_struct.contains("def from_tuple(nil), do: nil"),
            "{backend}: from_tuple must be nil-safe; got:\n{model_struct}"
        );
        assert!(
            model_struct.contains("pt: InnerProbe.from_tuple(pt)"),
            "{backend}: OuterProbe.from_tuple must recurse into the nested InnerProbe \
             composite rather than assign its still-a-tuple field raw; got:\n{model_struct}"
        );

        let query_fn = generated_query_fn(backend);
        assert!(
            query_fn.contains("val: OuterProbe.from_tuple(val)"),
            "{backend}: the `val` column must be routed through OuterProbe.from_tuple, not \
             assigned as the bare destructured tuple; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("val: val"),
            "{backend}: must not assign the raw Postgrex tuple straight into the struct field; \
             got:\n{query_fn}"
        );
    }
}

/// Runs the *emitted* `InnerProbe`/`OuterProbe` modules (not a hand-copy of them) against tuple
/// shapes fabricated to match exactly what `Postgrex.query/3` returns for a composite column --
/// confirmed live against PostgreSQL 16 in board #219's investigation:
///   `{:ok, %{rows: [[1, {"hi", {1, 2}}]]}}` for a fully-populated nested composite,
///   `{:ok, %{rows: [[2, {"hi2", nil}]]}}` when only the nested sub-field is NULL.
///
/// Before this fix neither module had a `from_tuple` to call -- this proves the emitted
/// definition actually compiles and recovers the right values, not just that the right
/// substrings appear in the generated text.
///
/// Skips rather than fails when `elixir` is absent, matching this suite's sibling regression
/// tests for python (`composite_text_escaping_regression.rs`) and ruby.
#[test]
fn the_emitted_elixir_from_tuple_recovers_nested_composite_values() {
    for backend in ["elixir-postgrex", "elixir-ecto"] {
        let model_struct = generated_model_struct(backend);

        let script = format!(
            "{model_struct}\n\n\
             populated = OuterProbe.from_tuple({{\"hi\", {{1, 2}}}})\n\
             nested_nil = OuterProbe.from_tuple({{\"hi2\", nil}})\n\
             top_nil = OuterProbe.from_tuple(nil)\n\
             \n\
             true = populated.label == \"hi\"\n\
             true = populated.pt.__struct__ == InnerProbe\n\
             true = populated.pt.x == 1\n\
             true = populated.pt.y == 2\n\
             true = nested_nil.label == \"hi2\"\n\
             true = nested_nil.pt == nil\n\
             true = top_nil == nil\n\
             \n\
             IO.puts(\"OK\")\n"
        );
        let dir = std::env::temp_dir().join("scythe_composite_from_tuple_probe");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join(format!("{}.exs", backend.replace('-', "_")));
        std::fs::write(&path, &script).expect("write probe");

        let output = match std::process::Command::new("elixir").arg(&path).output() {
            Ok(output) => output,
            Err(e) => {
                eprintln!(
                    "SKIP: elixir unavailable ({e}); {backend}'s from_tuple was checked by \
                     string match only"
                );
                continue;
            }
        };
        assert!(
            output.status.success(),
            "{backend}: the emitted from_tuple must run and recover the expected values: {}\n\
             script:\n{script}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "OK",
            "{backend}: from_tuple output mismatch"
        );
    }
}

#[test]
fn elixir_postgrex_composite_from_tuple_file_compiles() {
    assert_file_compiles("elixir-postgrex", generated_file("elixir-postgrex"));
}

#[test]
fn elixir_ecto_composite_from_tuple_file_compiles() {
    assert_file_compiles("elixir-ecto", generated_file("elixir-ecto"));
}
