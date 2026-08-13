//! End-to-end regression test for #219: the `typescript-postgres`
//! (postgres.js) `:batch` path rewrote `$N` placeholders **inside SQL string
//! literals**.
//!
//! The `:one`/`:many`/`:exec` paths all go through
//! `backends::rewrite_pg_placeholders`, which tokenises the statement and
//! only touches `Code` spans. `:batch` did not: it ran
//! `sql.replace("$1", "${item.a}")` over the raw text. Given
//!
//! ```sql
//! INSERT INTO notes (owner, body) VALUES ($1, 'ticket $1 filed')
//! ```
//!
//! both occurrences were rewritten, so the literal `'ticket $1 filed'`
//! became `'ticket ${item.owner} filed'` -- a second live postgres.js
//! binding. Unlike a syntax error this is silent: the statement compiles,
//! runs, and stores the wrong text.
//!
//! Covers the TypeScript emit mode and the `javascript-postgres` JSDoc emit
//! mode, which had a verbatim copy of the same loop.

use scythe_codegen::validation::{strict_mode_enabled, validate_with_tools};
use scythe_codegen::{CodegenBackend, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TABLE notes (\
    id SERIAL PRIMARY KEY, \
    owner TEXT NOT NULL, \
    body TEXT NOT NULL, \
    tag TEXT NOT NULL\
);";

/// Two real placeholders plus two decoys inside a string literal. `$2`
/// appears in the literal *before* `$1` does, so an implementation that
/// merely reordered its replacements still fails.
const BATCH_QUERY: &str = "-- @name AddNote\n-- @returns :batch\n\
    INSERT INTO notes (owner, body, tag) VALUES ($1, $2, 'ticket $2 for $1');";

/// A single-parameter `:batch`, which takes a different branch (`items:
/// string[]`, bound as `${item}`).
const SINGLE_PARAM_BATCH_QUERY: &str = "-- @name TagNotes\n-- @returns :batch\n\
    UPDATE notes SET tag = $1 WHERE body = 'literal $1 here';";

/// A single-parameter `:batch` whose literal contains the *rewritten*
/// spelling of the placeholder (`${tag}`, the param's field name) rather
/// than the raw `$1`. This is the shape [`SINGLE_PARAM_BATCH_QUERY`] cannot
/// exercise: `$1` inside a literal is never touched by
/// `rewrite_pg_placeholders`, so it stays `$1` in the output either way.
/// `${tag}` inside a literal is escaped to the inert `\${tag}` by
/// `escape_ts_template_literal` -- and a blind `.replace("${tag}",
/// "${item}")` over the placeholder-rewritten SQL still matches that
/// escaped tail, corrupting it to `\${item}`.
const SINGLE_PARAM_BATCH_QUERY_WITH_FIELD_NAME_IN_LITERAL: &str = "-- @name TagNotes\n-- @returns :batch\n\
    UPDATE notes SET tag = $1 WHERE body = 'literal ${tag} here';";

fn query_fn(backend_name: &str, sql: &str) -> String {
    let backend: Box<dyn CodegenBackend> =
        get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(sql, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.query_fn.expect("expected a query fn")
}

/// This must fail before the fix on both emit modes: the literal came out as
/// `'ticket ${item.body} for ${item.owner}'`.
#[test]
fn batch_placeholders_inside_a_sql_string_literal_stay_literal() {
    for backend_name in ["typescript-postgres", "javascript-postgres"] {
        let generated = query_fn(backend_name, BATCH_QUERY);

        assert!(
            generated.contains("'ticket $2 for $1'"),
            "{backend_name}: the SQL string literal must survive untouched (#219); got:\n{generated}"
        );
        assert!(
            !generated.contains("'ticket ${item"),
            "{backend_name}: a `$N` inside a literal became a live binding (#219); got:\n{generated}"
        );
        // The real placeholders still have to be rewritten.
        assert!(
            generated.contains("VALUES (${item.owner}, ${item.body},"),
            "{backend_name}: the actual placeholders must still bind; got:\n{generated}"
        );
    }
}

/// The one-parameter branch used to replace the literal text `${field}`
/// with `${item}` over the already placeholder-rewritten SQL, which cannot
/// tell a real, rewritten `$1` apart from the same text sitting inert inside
/// a string literal -- see
/// `a_single_parameter_batch_leaves_a_literal_containing_the_field_name_placeholder_alone`
/// below for the case that used to fail. This test only pins the `$1`-in-a-
/// literal shape, which was never at risk (the rewriter never touches text
/// inside a literal in the first place) -- kept so a future unification of
/// the two `:batch` branches cannot regress it either.
#[test]
fn a_single_parameter_batch_also_leaves_the_literal_alone() {
    for backend_name in ["typescript-postgres", "javascript-postgres"] {
        let generated = query_fn(backend_name, SINGLE_PARAM_BATCH_QUERY);

        assert!(
            generated.contains("'literal $1 here'"),
            "{backend_name}: got:\n{generated}"
        );
        assert!(
            generated.contains("SET tag = ${item}"),
            "{backend_name}: got:\n{generated}"
        );
    }
}

/// This must fail before the fix on both emit modes: a SQL literal
/// containing the literal text `${tag}` (the single param's field name) got
/// its escaped `\${tag}` corrupted to `\${item}` by the old blind
/// `.replace("${tag}", "${item}")`, because that replace ran over SQL that
/// already had the real `$1` rewritten to `${tag}` and could no longer tell
/// the two apart.
#[test]
fn a_single_parameter_batch_leaves_a_literal_containing_the_field_name_placeholder_alone() {
    for backend_name in ["typescript-postgres", "javascript-postgres"] {
        let generated = query_fn(backend_name, SINGLE_PARAM_BATCH_QUERY_WITH_FIELD_NAME_IN_LITERAL);

        assert!(
            generated.contains(r"'literal \${tag} here'"),
            "{backend_name}: the SQL literal's own text must survive untouched (#219 residual); got:\n{generated}"
        );
        assert!(
            !generated.contains(r"'literal \${item} here'"),
            "{backend_name}: the literal's escaped `${{tag}}` must not become a live `${{item}}` binding; \
             got:\n{generated}"
        );
        assert!(
            generated.contains("SET tag = ${item}"),
            "{backend_name}: the actual placeholder must still bind to the batch item; got:\n{generated}"
        );
    }
}

/// The non-batch paths were already literal-aware; this pins that the batch
/// fix did not change them.
#[test]
fn the_non_batch_paths_remain_literal_aware() {
    let generated = query_fn(
        "typescript-postgres",
        "-- @name FindNote\n-- @returns :many\n\
         SELECT id FROM notes WHERE tag = 'literal $1 here' AND owner = $1;",
    );

    assert!(generated.contains("'literal $1 here'"), "got:\n{generated}");
    assert!(generated.contains("owner = ${owner}"), "got:\n{generated}");
}

/// Additive: the repository's own checker over the whole generated file.
#[test]
fn the_generated_batch_file_passes_tool_validation() {
    for backend_name in ["typescript-postgres", "javascript-postgres"] {
        let backend = get_backend(backend_name, "postgresql").unwrap();
        let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).unwrap();
        let parsed = parse_query_with_dialect(BATCH_QUERY, &SqlDialect::PostgreSQL).unwrap();
        let analyzed = analyze(&catalog, &parsed).unwrap();
        let code = generate_with_backend(&analyzed, &*backend).unwrap();

        let mut file = backend.file_header_for_results(std::slice::from_ref(&code));
        file.push('\n');
        for text in [&code.row_struct, &code.query_fn].into_iter().flatten() {
            file.push_str(text);
            file.push_str("\n\n");
        }

        let validation = validate_with_tools(&file, backend_name);
        assert!(
            validation.errors().is_empty(),
            "{backend_name}: {:#?}\n\nfile:\n{file}",
            validation.errors()
        );
        if strict_mode_enabled() {
            assert!(
                validation.fully_checked(),
                "{backend_name}: strict mode requires every checker to have run, got {:?} run / {:?} missing",
                validation.tools_run(),
                validation.missing_tools()
            );
        }
    }
}
