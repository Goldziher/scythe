//! End-to-end regression tests for #218, three independent defects in
//! `typescript-oracledb`.
//!
//! 1. **`:grouped` never compiled.** The driver result was bound to `const
//!    result`, and the shared fold body
//!    (`typescript_common::generate_ts_grouped_fold_body`) then declares its
//!    accumulator as `const result: ParentRow[] = []` in the same block --
//!    `TS2451: Cannot redeclare block-scoped variable 'result'`, twice, in
//!    every `:grouped` function the backend has ever emitted. Confirmed
//!    against real `tsc --strict`.
//! 2. **Quoted column keys were uppercased.** Oracle case-folds *unquoted*
//!    identifiers, not quoted ones, so `SELECT "first name"` comes back
//!    keyed `first name`. Reading `row["FIRST NAME"]` is a key the row never
//!    has: no compile error, every such column silently `undefined`.
//! 3. **`row_type` was ignored.** `generate_struct_decl`,
//!    `generate_grouped_structs` and `generate_enum_def` never looked at it,
//!    so `row_type = "zod"` was accepted and did nothing.
//!
//! Unit-level coverage of the same three lives in the backend's own `mod
//! tests`; these go through the real parse -> analyze -> codegen pipeline
//! and check the assembled file.

use std::collections::HashMap;

use scythe_codegen::validation::{strict_mode_enabled, validate_with_tools};
use scythe_codegen::{CodegenBackend, GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TABLE users (\
    id NUMBER(10) PRIMARY KEY, \
    name VARCHAR2(100) NOT NULL, \
    \"first name\" VARCHAR2(100) NOT NULL\
);\
CREATE TABLE orders (\
    id NUMBER(10) PRIMARY KEY, \
    user_id NUMBER(10) NOT NULL, \
    total NUMBER(10, 2) NOT NULL\
);";

const QUERY_ONE: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, name, \"first name\" FROM users WHERE id = $1;";
const QUERY_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, \"first name\" FROM users;";
const QUERY_GROUPED: &str = "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\n\
    SELECT u.id, u.name, o.total FROM users u JOIN orders o ON o.user_id = u.id WHERE u.id = $1;";

fn build(options: &HashMap<String, String>) -> Box<dyn CodegenBackend> {
    let mut backend = get_backend("typescript-oracledb", "oracle").expect("typescript-oracledb must support oracle");
    backend.apply_options(options).expect("options must apply");
    backend
}

fn generate_file(backend: &dyn CodegenBackend, queries: &[&str]) -> String {
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::Oracle).expect("schema must parse");
    let codes: Vec<GeneratedCode> = queries
        .iter()
        .map(|sql| {
            let parsed = parse_query_with_dialect(sql, &SqlDialect::Oracle).expect("query must parse");
            let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
            generate_with_backend(&analyzed, backend).expect("codegen must succeed")
        })
        .collect();

    let mut file = backend.file_header_for_results(&codes);
    file.push('\n');
    for code in &codes {
        for text in [&code.enum_def, &code.row_struct, &code.query_fn].into_iter().flatten() {
            file.push_str(text);
            file.push_str("\n\n");
        }
    }
    file
}

/// Count how many `const <name> =` bindings a function body declares.
fn const_declarations(body: &str, name: &str) -> usize {
    body.match_indices(&format!("const {name}"))
        .filter(|(index, _)| {
            let after = &body[index + format!("const {name}").len()..];
            after.starts_with(' ') || after.starts_with(':')
        })
        .count()
}

/// This must fail before the fix: two `const result` declarations in one
/// block.
#[test]
fn the_grouped_query_fn_does_not_redeclare_the_result_binding() {
    let backend = build(&HashMap::new());
    let file = generate_file(&*backend, &[QUERY_GROUPED]);

    assert_eq!(
        const_declarations(&file, "result"),
        1,
        "the driver result and the fold accumulator cannot share a name (#218, TS2451):\n{file}"
    );
    assert!(
        file.contains("const queryResult = await conn.execute("),
        "the driver result must be bound under its own name; got:\n{file}"
    );
    assert!(
        file.contains("const flatRows = (queryResult.rows ?? [])"),
        "and read back under that name; got:\n{file}"
    );
    assert!(
        file.contains("const result: GetUsersWithOrdersRow[] = [];"),
        "the fold accumulator keeps its name; got:\n{file}"
    );
}

/// This must fail before the fix: `row["FIRST NAME"]` / `row['FIRST NAME']`
/// -- a key Oracle never produces for a quoted identifier.
#[test]
fn a_quoted_column_keeps_its_case_in_the_driver_row_key() {
    let backend = build(&HashMap::new());
    let file = generate_file(&*backend, &[QUERY_ONE, QUERY_MANY, QUERY_GROUPED]);

    assert!(
        !file.contains("FIRST NAME"),
        "Oracle does not case-fold a quoted identifier (#218); got:\n{file}"
    );
    assert!(
        file.contains("row[\"first name\"]"),
        "the quoted column's own spelling is the key; got:\n{file}"
    );
    // An unquoted identifier *is* folded, so those keys must stay uppercase.
    assert!(file.contains("row[\"ID\"]"), "got:\n{file}");
    assert!(file.contains("row[\"NAME\"]"), "got:\n{file}");
    assert!(file.contains("row['ID']"), "the grouped fold too; got:\n{file}");
}

/// This must fail before the fix: `row_type = "zod"` produced byte-identical
/// output to the default, with no error and no `zod` import.
#[test]
fn row_type_zod_changes_what_is_emitted() {
    let default_file = generate_file(&*build(&HashMap::new()), &[QUERY_ONE, QUERY_GROUPED]);
    let zod_file = generate_file(
        &*build(&HashMap::from([("row_type".to_string(), "zod".to_string())])),
        &[QUERY_ONE, QUERY_GROUPED],
    );

    assert_ne!(
        default_file, zod_file,
        "row_type = \"zod\" was a silent no-op on this backend (#218)"
    );
    assert!(
        zod_file.contains("import { z } from \"zod\";"),
        "a Zod schema needs the zod import; got:\n{zod_file}"
    );
    assert!(
        zod_file.contains("export const GetUserRowSchema = z.object({"),
        "the :one row must be a schema; got:\n{zod_file}"
    );
    assert!(
        zod_file.contains("export const GetUsersWithOrdersRowSchema = z.object({"),
        "the :grouped rows must be schemas too; got:\n{zod_file}"
    );
    assert!(
        !zod_file.contains("export interface "),
        "no plain interface may survive under row_type = \"zod\"; got:\n{zod_file}"
    );
    // The quoted key must stay quoted inside the schema as well.
    assert!(zod_file.contains("\"first name\": z.string(),"), "got:\n{zod_file}");
}

/// Additive: the repository's own TypeScript checker over both modes.
#[test]
fn the_generated_oracledb_files_pass_tool_validation() {
    for options in [
        HashMap::new(),
        HashMap::from([("row_type".to_string(), "zod".to_string())]),
    ] {
        let file = generate_file(&*build(&options), &[QUERY_ONE, QUERY_MANY, QUERY_GROUPED]);
        let validation = validate_with_tools(&file, "typescript-oracledb");
        assert!(
            validation.errors().is_empty(),
            "{options:?}: {:#?}\n\nfile:\n{file}",
            validation.errors()
        );
        if strict_mode_enabled() {
            assert!(
                validation.fully_checked(),
                "strict mode requires every checker to have run, got {:?} run / {:?} missing",
                validation.tools_run(),
                validation.missing_tools()
            );
        }
    }
}
