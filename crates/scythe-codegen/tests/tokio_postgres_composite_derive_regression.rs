//! Regression test (unfiled -- found by the torture gate on 2026-08-14): `rust-tokio-postgres`
//! could neither bind nor read a composite column. `generate_composite_def` emitted only
//! `#[derive(Debug, Clone)]` on the struct, which implements neither `ToSql` nor `FromSql` --
//! the two traits `tokio-postgres` uses to move a value across the wire (`postgres-types`'s
//! `Row::get`/parameter binding are both bounded on them). A real `cargo build` against
//! `sql/torture/schema.sql` (which has a composite column) failed with:
//!
//! ```text
//! error[E0277]: the trait bound `TortureAddress: FromSql<'_>` is not satisfied
//!    --> src/queries.rs:109:31  |  home_address: row.get("home_address"),
//! help: the trait `FromSql<'_>` is not implemented for `TortureAddress`
//!    --> src/queries.rs:69:1  |  pub struct TortureAddress {
//! ```
//!
//! The fix derives `postgres_types::{ToSql, FromSql}` (re-exported from `postgres-derive`
//! behind `postgres-types`'s `derive` feature -- see that crate's own composite/naming doc
//! examples) on the generated struct, plus `#[postgres(name = "...")]` so the derive's
//! exact-name-match requirement lines up with the SQL type name when it differs from the
//! PascalCase Rust identifier. Before the fix, every assertion below that checks for
//! `postgres_types::ToSql`/`postgres_types::FromSql`/`#[postgres(name = ...)]` in the output
//! failed -- the emitted text was just `#[derive(Debug, Clone)]` with no attribute line.
//!
//! The enum case does not share this mechanism: `generate_enum_def` already hand-writes its
//! own `impl tokio_postgres::types::{FromSql, ToSql}` (string round-trip through `Display`/
//! `FromStr`), never routes through `postgres-derive`, and was not part of this defect --
//! `enum_def_still_hand_writes_fromsql_tosql_not_the_derive_macro` pins that so a future change
//! to the enum path does not silently start relying on the composite fix's derive machinery.

use scythe_codegen::{CodegenBackend, get_backend};
use scythe_core::analyzer::{CompositeFieldInfo, CompositeInfo, EnumInfo};

fn tokio_postgres_backend() -> Box<dyn CodegenBackend> {
    get_backend("rust-tokio-postgres", "postgresql").expect("rust-tokio-postgres must support postgresql")
}

/// `sql_name` deliberately differs from the PascalCase Rust identifier `TortureAddress` would
/// derive from ("torture_address"), so a struct-level `#[postgres(name = "...")]` is load-bearing,
/// not incidental -- without it postgres-derive's exact-name-match would look for a Postgres type
/// literally named `TortureAddress`.
fn torture_address() -> CompositeInfo {
    CompositeInfo {
        sql_name: "torture_address".to_string(),
        fields: vec![
            CompositeFieldInfo {
                name: "street".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            },
            CompositeFieldInfo {
                name: "unit_count".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
            },
        ],
    }
}

#[test]
fn composite_derives_postgres_types_tosql_and_fromsql() {
    let backend = tokio_postgres_backend();
    let code = backend
        .generate_composite_def(&torture_address())
        .expect("composite def must generate");

    assert!(
        code.starts_with("#[derive(Debug, Clone, postgres_types::ToSql, postgres_types::FromSql)]\n"),
        "composite struct must derive postgres_types::ToSql and postgres_types::FromSql \
         (postgres-derive, re-exported by postgres-types behind its `derive` feature) so \
         tokio-postgres can bind and read it; got:\n{code}"
    );
}

#[test]
fn composite_carries_a_postgres_name_attribute_matching_the_sql_type() {
    let backend = tokio_postgres_backend();
    let code = backend
        .generate_composite_def(&torture_address())
        .expect("composite def must generate");

    assert!(
        code.contains("#[postgres(name = \"torture_address\")]\n"),
        "postgres-derive requires an exact name match between the Rust type and the Postgres \
         type by default; the SQL name (torture_address) differs from the PascalCase Rust \
         identifier (TortureAddress), so #[postgres(name = \"torture_address\")] is required; \
         got:\n{code}"
    );

    let derive_pos = code.find("#[derive(").expect("derive line must be present");
    let attr_pos = code
        .find("#[postgres(name = \"torture_address\")]")
        .expect("postgres name attribute must be present");
    let struct_pos = code
        .find("pub struct TortureAddress {")
        .expect("struct declaration must be present");
    assert!(
        derive_pos < attr_pos && attr_pos < struct_pos,
        "derive, then #[postgres(name = ...)], then the struct itself, in that order; got:\n{code}"
    );
}

#[test]
fn composite_full_shape_matches_exactly() {
    let backend = tokio_postgres_backend();
    let code = backend
        .generate_composite_def(&torture_address())
        .expect("composite def must generate");

    let expected = "#[derive(Debug, Clone, postgres_types::ToSql, postgres_types::FromSql)]\n\
        #[postgres(name = \"torture_address\")]\n\
        pub struct TortureAddress {\n    \
        pub street: String,\n    \
        pub unit_count: i32,\n\
        }";
    assert_eq!(code, expected, "composite definition must match exactly; got:\n{code}");
}

#[test]
fn nested_composite_keeps_tosql_fromsql_alongside_serde() {
    // json_agg/row_to_json nesting adds serde::Serialize/Deserialize (the field is decoded
    // through postgres_types::Json<T>, bounded on Deserialize) but must not drop ToSql/FromSql:
    // the same CompositeInfo can also be selected as a plain top-level column in another query
    // in the same file, and that path still needs the wire-format traits.
    let backend = tokio_postgres_backend();
    let code = backend
        .generate_composite_def_for_nested(&torture_address())
        .expect("nested composite def must generate");

    assert!(
        code.starts_with(
            "#[derive(Debug, Clone, postgres_types::ToSql, postgres_types::FromSql, \
             serde::Serialize, serde::Deserialize)]\n"
        ),
        "nested composite must keep ToSql/FromSql and add both serde traits; got:\n{code}"
    );
}

#[test]
fn enum_def_still_hand_writes_fromsql_tosql_not_the_derive_macro() {
    let backend = tokio_postgres_backend();
    let enum_info = EnumInfo {
        sql_name: "mood".to_string(),
        values: vec!["sad".to_string(), "ok".to_string(), "happy".to_string()],
    };
    let code = backend.generate_enum_def(&enum_info).expect("enum def must generate");

    assert!(
        code.contains("impl<'a> tokio_postgres::types::FromSql<'a> for Mood {"),
        "enum FromSql must stay hand-written (string round-trip via FromStr), unaffected by the \
         composite fix; got:\n{code}"
    );
    assert!(
        code.contains("impl tokio_postgres::types::ToSql for Mood {"),
        "enum ToSql must stay hand-written (string round-trip via Display), unaffected by the \
         composite fix; got:\n{code}"
    );
    assert!(
        !code.contains("postgres_types::FromSql") && !code.contains("postgres_types::ToSql"),
        "the enum path must not start deriving postgres_types::{{ToSql, FromSql}} -- it never \
         shared the composite's mechanism, and enum variant names do not exact-match SQL \
         labels the way the derive would require; got:\n{code}"
    );
}
