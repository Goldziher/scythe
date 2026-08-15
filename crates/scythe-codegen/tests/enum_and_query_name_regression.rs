//! Regression tests for GH #136: three enum/query naming defects that made
//! `scythe generate` emit syntactically invalid code while reporting success.
//!
//! 1. A schema-qualified enum's `.` reached `enum_type_name` unsanitized:
//!    `enum_type_name("public.status", ..)` returned `"Public.status"`,
//!    landing in every backend's declaration line (`pub enum Public.status`,
//!    `type Public.status string`, `class Public.status(str, Enum):`).
//!    `struct_case = "PascalCase"` in every one of the 102 manifests, so this
//!    hit all ten target languages identically.
//! 2. An enum label that sanitizes to a run of underscores (`"!!!"` via
//!    `sanitize_for_identifier`) made `to_pascal_case` return `""`: every
//!    `_`-delimited part was itself empty, and the old loop contributed
//!    nothing for any of them. This only reaches an empty variant under
//!    `enum_variant_case = "PascalCase"` (C#, Go, Rust, TypeScript) --
//!    `to_snake_case`/`to_screaming_snake_case` do not collapse underscores,
//!    so Java/Kotlin/PHP/Python/Ruby (`SCREAMING_SNAKE_CASE`) and Elixir
//!    (`snake_case`) already produced a non-empty, if unconventional, `"___"`
//!    before this fix.
//! 3. Two enums colliding under `struct_case`, or an enum colliding with the
//!    query's own row/model type, went undetected: `generate_enum_defs_via_backend`
//!    only deduplicated by raw SQL name, never checked the *generated* name
//!    against anything else, so `scythe generate` wrote two type
//!    declarations under one name and exited 0.
//! 4. Two SQL values of the *same* enum colliding under `enum_variant_case`
//!    went undetected the same way: `'gpt-3.5-turbo'` and `'gpt_3_5_turbo'`
//!    both sanitize and case-convert to `Gpt35Turbo`, and nothing compared
//!    the rendered variant names against each other before `scythe generate`
//!    wrote `pub enum Model { Gpt35Turbo, Gpt35Turbo, }` -- `E0428` in Rust,
//!    a redeclaration in every other target -- and exited 0.

use scythe_codegen::{GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::parse_query_with_dialect;

fn generate(schema: &str, query: &str, backend_name: &str) -> Result<GeneratedCode, ScytheError> {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[schema], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    generate_with_backend(&analyzed, &*backend)
}

const SCHEMA_QUALIFIED_ENUM_SCHEMA: &str = "\
    CREATE TYPE public.status AS ENUM ('active', 'inactive'); \
    CREATE TABLE items (id INT PRIMARY KEY, status public.status NOT NULL);";

const SCHEMA_QUALIFIED_ENUM_QUERY: &str = "-- @name GetItem\n-- @returns :one\n\
    SELECT id, status FROM items WHERE id = $1;";

/// Rust: before the fix, `enum_type_name` returned `"Public.status"`, so
/// this line read `pub enum Public.status {`, which does not parse.
#[test]
fn schema_qualified_enum_sanitizes_the_dot_in_rust() {
    let code =
        generate(SCHEMA_QUALIFIED_ENUM_SCHEMA, SCHEMA_QUALIFIED_ENUM_QUERY, "rust-sqlx").expect("codegen must succeed");
    let enum_def = code.enum_def.expect("enum definition must be emitted");
    assert!(
        enum_def.contains("pub enum PublicStatus {"),
        "expected the sanitized type name:\n{enum_def}"
    );
    assert!(
        !enum_def.contains("Public.status"),
        "the unsanitized, unparseable form must not appear:\n{enum_def}"
    );
}

/// Go: before the fix this line read `type Public.status string`, which does
/// not parse -- a `.` is a selector, not part of an identifier.
#[test]
fn schema_qualified_enum_sanitizes_the_dot_in_go() {
    let code =
        generate(SCHEMA_QUALIFIED_ENUM_SCHEMA, SCHEMA_QUALIFIED_ENUM_QUERY, "go-pgx").expect("codegen must succeed");
    let enum_def = code.enum_def.expect("enum definition must be emitted");
    assert!(
        enum_def.contains("type PublicStatus string"),
        "expected the sanitized type name:\n{enum_def}"
    );
    assert!(
        !enum_def.contains("Public.status"),
        "the unsanitized, unparseable form must not appear:\n{enum_def}"
    );
}

/// Python: before the fix this line read `class Public.status(str, Enum):`,
/// which does not parse -- `.` cannot appear in a class name.
#[test]
fn schema_qualified_enum_sanitizes_the_dot_in_python() {
    let code = generate(
        SCHEMA_QUALIFIED_ENUM_SCHEMA,
        SCHEMA_QUALIFIED_ENUM_QUERY,
        "python-asyncpg",
    )
    .expect("codegen must succeed");
    let enum_def = code.enum_def.expect("enum definition must be emitted");
    assert!(
        enum_def.contains("class PublicStatus(str, Enum):"),
        "expected the sanitized type name:\n{enum_def}"
    );
    assert!(
        !enum_def.contains("Public.status"),
        "the unsanitized, unparseable form must not appear:\n{enum_def}"
    );
}

const DEGENERATE_LABEL_SCHEMA: &str = "\
    CREATE TYPE weird_status AS ENUM ('active', '!!!'); \
    CREATE TABLE items (id INT PRIMARY KEY, status weird_status NOT NULL);";

const DEGENERATE_LABEL_QUERY: &str = "-- @name GetWeirdItem\n-- @returns :one\n\
    SELECT id, status FROM items WHERE id = $1;";

/// Rust (`enum_variant_case = "PascalCase"`): before the fix, `to_pascal_case`
/// returned `""` for the sanitized label, so this line read `    ,` -- an
/// enum variant with no name, which does not parse.
#[test]
fn degenerate_label_does_not_collapse_to_an_empty_variant_in_rust() {
    let code = generate(DEGENERATE_LABEL_SCHEMA, DEGENERATE_LABEL_QUERY, "rust-sqlx").expect("codegen must succeed");
    let enum_def = code.enum_def.expect("enum definition must be emitted");
    assert!(
        enum_def.contains("    ___,"),
        "expected the non-empty fallback variant:\n{enum_def}"
    );
    assert!(
        !enum_def.contains("    ,\n"),
        "an empty variant must not appear:\n{enum_def}"
    );
}

/// Go (`enum_variant_case = "PascalCase"`): before the fix this printed
/// `\tWeirdStatus WeirdStatus = "!!!"`, colliding with the type declaration
/// itself (`type WeirdStatus string`) -- `WeirdStatus redeclared`.
#[test]
fn degenerate_label_does_not_collapse_to_an_empty_variant_in_go() {
    let code = generate(DEGENERATE_LABEL_SCHEMA, DEGENERATE_LABEL_QUERY, "go-pgx").expect("codegen must succeed");
    let enum_def = code.enum_def.expect("enum definition must be emitted");
    assert!(
        enum_def.contains("WeirdStatus___ WeirdStatus = \"!!!\""),
        "expected the non-empty fallback variant:\n{enum_def}"
    );
}

/// Python (`enum_variant_case = "SCREAMING_SNAKE_CASE"`) never hit this
/// defect: `to_snake_case`/`to_uppercase` do not collapse underscores, so
/// this line already read `    ___ = "!!!"` before and after the fix. Pinned
/// so the Rust/Go fix above is not mistaken for covering every language.
#[test]
fn degenerate_label_was_already_non_empty_under_screaming_snake_case() {
    let code =
        generate(DEGENERATE_LABEL_SCHEMA, DEGENERATE_LABEL_QUERY, "python-asyncpg").expect("codegen must succeed");
    let enum_def = code.enum_def.expect("enum definition must be emitted");
    assert!(
        enum_def.contains("    ___ = \"!!!\""),
        "expected the pre-existing non-empty member:\n{enum_def}"
    );
}

const TWO_ENUMS_COLLIDE_SCHEMA: &str = "\
    CREATE TYPE order_status AS ENUM ('open', 'closed'); \
    CREATE TYPE order__status AS ENUM ('open', 'closed'); \
    CREATE TABLE orders (id INT PRIMARY KEY, a order_status NOT NULL, b order__status NOT NULL);";

const TWO_ENUMS_COLLIDE_QUERY: &str = "-- @name GetOrder\n-- @returns :one\n\
    SELECT id, a, b FROM orders WHERE id = $1;";

/// `order_status` and `order__status` are two distinct SQL enum types --
/// legal, and the analyzer's raw-name `duplicate_alias` check never sees
/// them collide, because it does not. Both case-convert to `OrderStatus`
/// under `struct_case = "PascalCase"` (the double underscore is a single
/// empty word-split part, exactly like `a_b`/`a__b` colliding under
/// `field_case` in `resolve.rs`). Before this fix, `generate_enum_defs_via_backend`
/// only deduplicated by raw SQL name, so it silently wrote two
/// `pub enum OrderStatus` declarations into the same file and returned `Ok`.
#[test]
fn two_enums_colliding_on_the_generated_name_is_rejected_not_silently_written_twice() {
    let err = generate(TWO_ENUMS_COLLIDE_SCHEMA, TWO_ENUMS_COLLIDE_QUERY, "rust-sqlx")
        .expect_err("must not silently succeed");
    assert_eq!(err.code, ErrorCode::DuplicateAlias);
    let message = err.to_string();
    assert!(message.contains("order_status"), "{message}");
    assert!(message.contains("order__status"), "{message}");
    assert!(message.contains("OrderStatus"), "{message}");
}

/// The same collision, checked backend-agnostically: the guard lives in
/// `scythe-codegen`'s shared `generate_with_backend_and_overrides`, not in
/// any one backend, so it must reject the query under every backend, not
/// just `rust-sqlx`.
#[test]
fn two_enums_colliding_on_the_generated_name_is_rejected_under_go_too() {
    let err =
        generate(TWO_ENUMS_COLLIDE_SCHEMA, TWO_ENUMS_COLLIDE_QUERY, "go-pgx").expect_err("must not silently succeed");
    assert_eq!(err.code, ErrorCode::DuplicateAlias);
}

const ENUM_VS_QUERY_TYPE_SCHEMA: &str = "\
    CREATE TYPE get_user_row AS ENUM ('a', 'b'); \
    CREATE TABLE users (id INT PRIMARY KEY, status get_user_row NOT NULL);";

const ENUM_VS_QUERY_TYPE_QUERY: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, status FROM users WHERE id = $1;";

/// Query `GetUser` declares row type `GetUserRow`
/// (`row_struct_name("GetUser", ..)`); enum `get_user_row` case-converts to
/// the identical `GetUserRow`. Before this fix, nothing compared an enum's
/// generated name against the query's own row/model type, so this wrote a
/// `pub enum GetUserRow` alongside a `pub struct GetUserRow` in the same
/// file and returned `Ok`.
#[test]
fn enum_colliding_with_the_query_row_type_is_rejected_not_silently_written_twice() {
    let err = generate(ENUM_VS_QUERY_TYPE_SCHEMA, ENUM_VS_QUERY_TYPE_QUERY, "rust-sqlx")
        .expect_err("must not silently succeed");
    assert_eq!(err.code, ErrorCode::DuplicateAlias);
    let message = err.to_string();
    assert!(message.contains("GetUserRow"), "{message}");
}

const VARIANT_COLLISION_SCHEMA: &str = "\
    CREATE TYPE model AS ENUM ('gpt-3.5-turbo', 'gpt_3_5_turbo'); \
    CREATE TABLE items (id INT PRIMARY KEY, m model NOT NULL);";

const VARIANT_COLLISION_QUERY: &str = "-- @name GetItem\n-- @returns :one\n\
    SELECT id, m FROM items WHERE id = $1;";

/// `'gpt-3.5-turbo'` and `'gpt_3_5_turbo'` are two distinct, legal SQL enum
/// values. Both sanitize through `enum_variant_name` to the identical
/// `Gpt35Turbo` under `enum_variant_case = "PascalCase"` (Rust, C#, Go,
/// TypeScript). Before this fix, `generate_enum_defs_via_backend` handed
/// `enum_info.values` straight to `backend.generate_enum_def` with no check,
/// so `scythe generate` wrote `pub enum Model { Gpt35Turbo, Gpt35Turbo, }` --
/// `error[E0428]: the name \`Gpt35Turbo\` is defined multiple times` under a
/// real `rustc` -- and returned `Ok`.
#[test]
fn enum_variant_collision_is_rejected_not_silently_written_twice_in_rust() {
    let err = generate(VARIANT_COLLISION_SCHEMA, VARIANT_COLLISION_QUERY, "rust-sqlx")
        .expect_err("must not silently succeed");
    assert_eq!(err.code, ErrorCode::DuplicateAlias);
    let message = err.to_string();
    assert!(message.contains("gpt-3.5-turbo"), "{message}");
    assert!(message.contains("gpt_3_5_turbo"), "{message}");
    assert!(message.contains("Gpt35Turbo"), "{message}");
    assert!(message.contains("model"), "{message}");
}

/// The same collision, checked backend-agnostically: the guard lives in
/// `scythe-codegen`'s shared `generate_enum_defs_via_backend`, not in any one
/// backend, so it must reject the query under every backend whose
/// `enum_variant_case` produces the collision -- not just `rust-sqlx`.
#[test]
fn enum_variant_collision_is_rejected_under_go_too() {
    let err =
        generate(VARIANT_COLLISION_SCHEMA, VARIANT_COLLISION_QUERY, "go-pgx").expect_err("must not silently succeed");
    assert_eq!(err.code, ErrorCode::DuplicateAlias);
}

/// Two enum values that render to *different* variant names under
/// `PascalCase` must not be rejected -- the guard compares generated names,
/// never raw SQL values, so `'active'` and `'inactive'` (already exercised
/// by every other test in this file) keep working. Pinned directly here so a
/// change that over-widens the new check (e.g. comparing raw SQL values
/// instead of rendered names) fails loudly.
#[test]
fn distinct_enum_variants_are_not_rejected() {
    generate(SCHEMA_QUALIFIED_ENUM_SCHEMA, SCHEMA_QUALIFIED_ENUM_QUERY, "rust-sqlx").expect("must not be rejected");
}
