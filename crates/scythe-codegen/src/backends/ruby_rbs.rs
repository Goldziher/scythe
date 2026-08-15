use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_core::parser::QueryCommand;

use crate::GeneratedCode;
use crate::backend_trait::{RbsGenerationContext, RbsQueryInfo, ResolvedColumn};

/// Generate child and parent `Data.define` structs for a `:grouped` Ruby query.
///
/// Emits the child struct first (convention: child before parent) then the parent
/// struct with all parent columns plus a `:children` field.
pub(crate) fn generate_grouped_structs_ruby(
    parent_struct_name: &str,
    child_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
) -> String {
    let mut out = String::new();

    let child_fields = child_columns
        .iter()
        .map(|c| format!(":{}", c.field_name))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(out, "  {child_struct_name} = Data.define({child_fields})");
    let _ = writeln!(out);

    let mut parent_field_parts: Vec<String> = parent_columns.iter().map(|c| format!(":{}", c.field_name)).collect();
    parent_field_parts.push(":children".to_string());
    let parent_fields = parent_field_parts.join(", ");
    let _ = writeln!(out, "  {parent_struct_name} = Data.define({parent_fields})");

    out
}

/// Derive a `:grouped` query's child struct name from its parent struct name.
///
/// Mirrors `crates/scythe-codegen/src/lib.rs`'s `generate_with_backend_and_overrides` (the
/// `.rb` code path, run for the same query before this RBS path): `{base}Child{row_suffix}`
/// where `base` is the parent name with the naming convention's row suffix stripped. Both
/// derivations read only the parent struct name and `manifest.naming.row_suffix`, so there
/// is nothing for the two copies to independently diverge on.
fn grouped_child_struct_name(parent_struct_name: &str, manifest: &BackendManifest) -> String {
    let suffix = &manifest.naming.row_suffix;
    let base = parent_struct_name.trim_end_matches(suffix.as_str());
    format!("{base}Child{suffix}")
}

/// Write RBS class definitions for a `:grouped` query's parent and child structs.
///
/// Mirrors the `.rb` shape [`generate_grouped_structs_ruby`] emits: the child class is
/// written first (avoiding a forward reference), then the parent class with all parent
/// columns plus a synthesized `children: Array[{child_struct_name}]` reader -- matching the
/// `:children` field every `generate_grouped_structs` implementation appends to the `.rb`
/// `Data.define` parent struct (see `ruby_pg.rs`'s literal `children: []` in
/// `generate_grouped_query_fn`).
fn write_rbs_grouped_classes(
    out: &mut String,
    parent_struct_name: &str,
    child_struct_name: &str,
    parent_columns: &[ResolvedColumn],
    child_columns: &[ResolvedColumn],
    manifest: &BackendManifest,
) {
    write_rbs_data_class(out, child_struct_name, child_columns, manifest);
    let _ = writeln!(out);

    let _ = writeln!(out, "  class {}", parent_struct_name);
    for col in parent_columns {
        let rbs_type = neutral_to_rbs(&col.neutral_type, col.nullable, manifest);
        let _ = writeln!(out, "    attr_reader {}: {}", col.field_name, rbs_type);
    }
    let _ = writeln!(out, "    attr_reader children: Array[{}]", child_struct_name);

    let mut ctor_params: Vec<String> = parent_columns
        .iter()
        .map(|col| {
            let rbs_type = neutral_to_rbs(&col.neutral_type, col.nullable, manifest);
            format!("{}: {}", col.field_name, rbs_type)
        })
        .collect();
    ctor_params.push(format!("children: Array[{}]", child_struct_name));
    let _ = writeln!(
        out,
        "    def self.new: ({}) -> {}",
        ctor_params.join(", "),
        parent_struct_name
    );
    let _ = writeln!(out, "  end");
}

/// `Queries::RecordNotFound`, raised by every ruby-* backend's `:one` query when the driver
/// returns no row.
///
/// `:one` means "exactly one row, error if absent" -- unlike `:opt`, which returns `nil` for a
/// legitimately absent row. Before this fix every ruby-* `generate_query_fn` folded
/// `QueryCommand::One | QueryCommand::Opt` into one arm that always returned `nil` on a
/// missing row, so `:one` silently inherited `:opt`'s permissiveness (see
/// `ruby_php_one_contract_regression.rs`). `StandardError` is the idiomatic Ruby base for an
/// application-specific exception -- a bare `raise "message"` raises `RuntimeError`, which a
/// caller cannot rescue selectively from unrelated runtime errors. Emitted once per generated
/// file (inside `module Queries`, alongside the row structs and query methods it is raised
/// from) rather than shipped as a separate gem dependency: every ruby-* backend already
/// generates a single self-contained `.rb` file with no runtime dependency beyond the DB
/// driver itself, and this preserves that.
pub(crate) const RECORD_NOT_FOUND_CLASS: &str = "  class RecordNotFound < StandardError; end";

/// Whether any of a query's generated `.rb` code (row struct, query function, or model
/// struct) applies the `.to_d` coercion that `ruby_pg.rs`, `ruby_mysql2.rs`, and
/// `ruby_trilogy.rs` emit for a `decimal` column whose manifest declares it `BigDecimal`.
///
/// Used by those three backends' `file_header_for_results` to add
/// `require "bigdecimal/util"` only to files that actually reference it. The `.rbs` side
/// needs no counterpart: see [`generate_rbs_content`] for why a signature naming
/// `BigDecimal` carries no directive at all.
pub(crate) fn ruby_generated_code_needs_bigdecimal_util(generated: &[GeneratedCode]) -> bool {
    generated.iter().any(|code| {
        [
            code.row_struct.as_deref(),
            code.query_fn.as_deref(),
            code.model_struct.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|s| s.contains(".to_d"))
    })
}

/// Whether any of a query's generated `.rb` code (row struct, query function, or model
/// struct) applies the `JSON.parse` coercion `ruby_pg.rs`'s `RubyCoercion::Wrap` emits for a
/// `json`/`json_array` column whose manifest declares it `Hash`/`Array` (GH #147).
///
/// Same purpose as [`ruby_generated_code_needs_bigdecimal_util`], for the stdlib `json`
/// require instead of `bigdecimal/util`: most generated files never touch JSON at all, so an
/// unconditional `require "json"` would add a dependency the file never uses.
pub(crate) fn ruby_generated_code_needs_json(generated: &[GeneratedCode]) -> bool {
    generated.iter().any(|code| {
        [
            code.row_struct.as_deref(),
            code.query_fn.as_deref(),
            code.model_struct.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|s| s.contains("JSON.parse("))
    })
}

/// Translate a manifest scalar's Ruby documentation name (e.g. `"Hash"`, `"Boolean"`,
/// `"Time"`) into a valid RBS type.
///
/// Manifest scalars name the Ruby type shown in docs/generated code, not an RBS type, so
/// they cannot all be read verbatim into a signature: `Hash` and `Array` alone are not valid
/// RBS (they need type arguments), and `Boolean` is not an RBS type at all (RBS spells it
/// `bool`). This table covers every distinct value that appears across
/// `crates/scythe-codegen/manifests/ruby-*.toml` `[types.scalars]`: `Boolean`, `Integer`,
/// `Float`, `String`, `BigDecimal`, `Date`, `Time`, `Hash`, `Array` (the last declared by
/// `ruby-pg.toml`'s `json_array`, GH #147). Values that are already valid RBS (`Integer`,
/// `Float`, `String`, `BigDecimal`, `Date`, `Time`) pass through unchanged; `Boolean`, `Hash`
/// and `Array` are rewritten.
fn manifest_type_to_rbs(manifest_value: &str) -> String {
    match manifest_value {
        "Hash" => "Hash[String, untyped]".to_string(),
        "Array" => "Array[untyped]".to_string(),
        "Boolean" => "bool".to_string(),
        other => other.to_string(),
    }
}

/// Map a neutral type to an RBS type string.
///
/// Every scalar is looked up in `manifest.types.scalars` and translated through
/// [`manifest_type_to_rbs`] — there is no fixed per-backend table. Scalar values differ by
/// backend for more than just `json`: `ruby-sqlite3` declares `decimal = "Float"` and all
/// five date/time scalars as `"String"` (SQLite has no native type for either, so the
/// driver returns the raw string), and `ruby-oci8` declares `date = "Time"` (OCI8 returns
/// `Time` for `DATE` columns, not `Date`). Reading straight from the manifest keeps this
/// function correct for all backends instead of re-diverging as new ones are added.
///
/// `enum::*` types are not manifest scalars (SQL enums aren't a scalar kind — they always
/// map to `String`). `array<T>` recurses into the inner type via the `manifest` lookup
/// above.
///
/// Note the two spellings differ: enums are `enum::name` but arrays are `array<inner>`,
/// matching what `type_conversion.rs` actually emits. This function used to strip an
/// `array::` prefix, which no neutral type has ever carried, so every array column fell
/// through to the scalar lookup, missed, and became `untyped`.
fn neutral_to_rbs(neutral_type: &str, nullable: bool, manifest: &BackendManifest) -> String {
    if let Some(inner) = neutral_type
        .strip_prefix("array<")
        .and_then(|rest| rest.strip_suffix('>'))
    {
        let inner_rbs = neutral_to_rbs(inner, false, manifest);
        return if nullable {
            format!("Array[{}]?", inner_rbs)
        } else {
            format!("Array[{}]", inner_rbs)
        };
    }
    if neutral_type.starts_with("enum::") {
        return if nullable {
            "String?".to_string()
        } else {
            "String".to_string()
        };
    }

    let base = manifest
        .types
        .scalars
        .get(neutral_type)
        .map(|manifest_value| manifest_type_to_rbs(manifest_value))
        .unwrap_or_else(|| "untyped".to_string());
    if nullable { format!("{}?", base) } else { base }
}

/// Map a neutral type to an RBS type for a parameter.
/// Parameters use the same mapping as columns.
fn param_neutral_to_rbs(neutral_type: &str, nullable: bool, manifest: &BackendManifest) -> String {
    neutral_to_rbs(neutral_type, nullable, manifest)
}

/// Generate a complete RBS file from the given context and connection type.
/// `connection_type` is the RBS class name for the database connection
/// (e.g., "PG::Connection", "Mysql2::Client", "SQLite3::Database", "Trilogy").
/// `manifest` is the calling backend's own manifest, used to resolve every backend-specific
/// scalar type (see [`neutral_to_rbs`]).
pub fn generate_rbs_content(
    context: &RbsGenerationContext,
    connection_type: &str,
    manifest: &BackendManifest,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "module Queries");

    for enum_info in &context.enums {
        let _ = writeln!(out, "  module {}", enum_info.type_name);
        for value in &enum_info.values {
            let _ = writeln!(out, "    {}: String", value);
        }
        let _ = writeln!(out, "    ALL: Array[String]");
        let _ = writeln!(out, "  end");
        let _ = writeln!(out);
    }

    for query in &context.queries {
        if let Some(ref struct_name) = query.struct_name
            && !query.columns.is_empty()
        {
            if query.command == QueryCommand::Grouped {
                let child_struct_name = grouped_child_struct_name(struct_name, manifest);
                write_rbs_grouped_classes(
                    &mut out,
                    struct_name,
                    &child_struct_name,
                    &query.columns,
                    &query.child_columns,
                    manifest,
                );
            } else {
                write_rbs_data_class(&mut out, struct_name, &query.columns, manifest);
            }
            let _ = writeln!(out);
        }

        write_rbs_method(&mut out, query, connection_type, manifest);
        let _ = writeln!(out);
    }

    let _ = write!(out, "end");
    out.push('\n');

    // Deliberately no `library "bigdecimal"` line here. `library` is not RBS declaration
    // syntax -- it is a Steepfile / `rbs` CLI directive -- and emitting it into the signature
    // makes the whole file unparseable: `rbs parse` rejects it with "cannot start a
    // declaration, token=`library`". Naming `BigDecimal` in a signature needs no directive at
    // all; `rbs parse` resolves it fine on its own, and which stdlib signatures are loaded is
    // the consumer's decision, made in their Steepfile or `rbs_collection.yaml`, not
    // something a generated `.rbs` can or should assert. The `.rb` side is different and does
    // still emit `require "bigdecimal/util"`, because `.to_d` genuinely is not available
    // without it -- see `ruby_generated_code_needs_bigdecimal_util`.
    out
}

/// Write an RBS class definition for a Data.define struct.
fn write_rbs_data_class(out: &mut String, struct_name: &str, columns: &[ResolvedColumn], manifest: &BackendManifest) {
    let _ = writeln!(out, "  class {}", struct_name);
    for col in columns {
        let rbs_type = neutral_to_rbs(&col.neutral_type, col.nullable, manifest);
        let _ = writeln!(out, "    attr_reader {}: {}", col.field_name, rbs_type);
    }

    let ctor_params: Vec<String> = columns
        .iter()
        .map(|col| {
            let rbs_type = neutral_to_rbs(&col.neutral_type, col.nullable, manifest);
            format!("{}: {}", col.field_name, rbs_type)
        })
        .collect();
    let _ = writeln!(out, "    def self.new: ({}) -> {}", ctor_params.join(", "), struct_name);
    let _ = writeln!(out, "  end");
}

/// Write an RBS method signature for a query function.
fn write_rbs_method(out: &mut String, query: &RbsQueryInfo, connection_type: &str, manifest: &BackendManifest) {
    let param_types: Vec<String> = query
        .params
        .iter()
        .map(|p| param_neutral_to_rbs(&p.neutral_type, p.nullable, manifest))
        .collect();

    let mut all_param_types = vec![connection_type.to_string()];
    all_param_types.extend(param_types);
    let params_str = all_param_types.join(", ");

    let return_type = match query.command {
        // `:one` raises `RecordNotFound` instead of returning `nil` on a missing row (see
        // `RECORD_NOT_FOUND_CLASS`), so its signature is non-nullable -- a `?` here would
        // describe a return value the `.rb` code beside it can no longer produce.
        QueryCommand::One => query.struct_name.clone().unwrap_or_else(|| "void".to_string()),
        QueryCommand::Opt => {
            if let Some(ref sn) = query.struct_name {
                format!("{}?", sn)
            } else {
                "void".to_string()
            }
        }
        QueryCommand::Many | QueryCommand::Grouped => {
            if let Some(ref sn) = query.struct_name {
                format!("Array[{}]", sn)
            } else {
                "Array[untyped]".to_string()
            }
        }
        QueryCommand::Exec => "void".to_string(),
        QueryCommand::ExecResult | QueryCommand::ExecRows => "Integer".to_string(),
        QueryCommand::Batch => {
            let item_type = if query.params.len() > 1 {
                let inner: Vec<String> = query
                    .params
                    .iter()
                    .map(|p| param_neutral_to_rbs(&p.neutral_type, p.nullable, manifest))
                    .collect();
                format!("Array[[{}]]", inner.join(", "))
            } else if query.params.len() == 1 {
                let p = &query.params[0];
                format!("Array[{}]", param_neutral_to_rbs(&p.neutral_type, p.nullable, manifest))
            } else {
                "Array[untyped]".to_string()
            };
            let _ = writeln!(
                out,
                "  def self.{}_batch: ({}, {}) -> void",
                query.func_name, connection_type, item_type
            );
            return;
        }
    };

    let _ = writeln!(
        out,
        "  def self.{}: ({}) -> {}",
        query.func_name, params_str, return_type
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_trait::{RbsEnumInfo, RbsGenerationContext, RbsQueryInfo, ResolvedColumn, ResolvedParam};
    use scythe_core::parser::QueryCommand;

    /// Manifest fixture for tests that don't care which backend's `json` mapping is used.
    /// Uses the `ruby-pg` manifest, whose `json = "Hash"` matches the majority of backends
    /// (pg, mysql2, oci8, pg.redshift, trilogy) and preserves this suite's prior behavior.
    fn manifest() -> BackendManifest {
        super::super::parse_manifest(include_str!("../../manifests/ruby-pg.toml")).unwrap()
    }

    fn sqlite3_manifest() -> BackendManifest {
        super::super::parse_manifest(include_str!("../../manifests/ruby-sqlite3.toml")).unwrap()
    }

    fn oci8_manifest() -> BackendManifest {
        super::super::parse_manifest(include_str!("../../manifests/ruby-oci8.toml")).unwrap()
    }

    fn mysql2_manifest() -> BackendManifest {
        super::super::parse_manifest(include_str!("../../manifests/ruby-mysql2.toml")).unwrap()
    }

    fn trilogy_manifest() -> BackendManifest {
        super::super::parse_manifest(include_str!("../../manifests/ruby-trilogy.toml")).unwrap()
    }

    fn tiny_tds_manifest() -> BackendManifest {
        super::super::parse_manifest(include_str!("../../manifests/ruby-tiny-tds.toml")).unwrap()
    }

    /// A separate manifest consumed by the same `RubyPgBackend`, so it needs its
    /// own output-neutrality coverage -- its scalars match `ruby-pg` today, but
    /// nothing enforced that.
    fn pg_redshift_manifest() -> BackendManifest {
        super::super::parse_manifest(include_str!("../../manifests/ruby-pg.redshift.toml")).unwrap()
    }

    fn col(name: &str, neutral_type: &str, nullable: bool) -> ResolvedColumn {
        ResolvedColumn {
            name: name.to_string(),
            field_name: name.to_string(),
            lang_type: String::new(),
            full_type: String::new(),
            neutral_type: neutral_type.to_string(),
            nullable,
            sql_type: neutral_type.to_string(),
            ..Default::default()
        }
    }

    fn param(name: &str, neutral_type: &str, nullable: bool) -> ResolvedParam {
        ResolvedParam {
            name: name.to_string(),
            field_name: name.to_string(),
            lang_type: String::new(),
            full_type: String::new(),
            borrowed_type: String::new(),
            neutral_type: neutral_type.to_string(),
            nullable,
        }
    }

    #[test]
    fn test_neutral_to_rbs_scalars() {
        let m = manifest();
        assert_eq!(neutral_to_rbs("int32", false, &m), "Integer");
        assert_eq!(neutral_to_rbs("int64", false, &m), "Integer");
        assert_eq!(neutral_to_rbs("string", false, &m), "String");
        assert_eq!(neutral_to_rbs("bool", false, &m), "bool");
        assert_eq!(neutral_to_rbs("float64", false, &m), "Float");
        assert_eq!(neutral_to_rbs("decimal", false, &m), "BigDecimal");
        assert_eq!(neutral_to_rbs("datetime_tz", false, &m), "Time");
        assert_eq!(neutral_to_rbs("date", false, &m), "Date");
        assert_eq!(neutral_to_rbs("uuid", false, &m), "String");
        assert_eq!(neutral_to_rbs("json", false, &m), "Hash[String, untyped]");
        assert_eq!(neutral_to_rbs("bytes", false, &m), "String");
    }

    /// Regression test for #101: `ruby-sqlite3` declares `json = "String"` in its manifest
    /// because the sqlite3 driver hands back the raw JSON text, not a decoded `Hash` — the
    /// `.rb` code never parses it. The `.rbs` signature must say `String` to match, not the
    /// `Hash[String, untyped]` that other backends use.
    #[test]
    fn test_neutral_to_rbs_json_sqlite3_matches_manifest() {
        let m = sqlite3_manifest();
        assert_eq!(neutral_to_rbs("json", false, &m), "String");
    }

    /// Companion to the sqlite3 regression test. Before GH #147's fix this comment claimed
    /// "ruby-pg decodes JSON into a Hash", but nothing did: `ruby_pg.rs`'s `ruby_coercion` had
    /// no arm for the manifest's `json = "Hash"` at all, so a `json` column reached the row
    /// struct as `pg`'s raw wire-format `String` while this very assertion claimed the `.rbs`
    /// signature `Hash[String, untyped]` was correct -- true of the declaration, false of the
    /// runtime value beside it. `ruby_pg.rs` now emits `JSON.parse(...)` for a `json` column
    /// (see its `RubyCoercion::Wrap` arm and the `ruby_pg.rs` regression test asserting on the
    /// generated `.rb` text), so this assertion's value is unchanged but its premise is now
    /// actually true: this pins the *declared* half of the contract -- that `ruby-pg`'s RBS
    /// keeps saying `Hash[String, untyped]` -- and relies on the `ruby_pg.rs` test to pin the
    /// *runtime* half that now backs it.
    #[test]
    fn test_neutral_to_rbs_json_pg_stays_hash() {
        let m = manifest();
        assert_eq!(neutral_to_rbs("json", false, &m), "Hash[String, untyped]");
    }

    /// Regression test for #106: `ruby-sqlite3` declares six scalars that disagree with the
    /// old hardcoded table (`decimal`, `date`, `time`, `time_tz`, `datetime`,
    /// `datetime_tz`), because SQLite has no native decimal or date/time storage class — the
    /// driver hands back a `Float` for `decimal` and raw `String`s for everything else. The
    /// `.rbs` must match what the `.rb` code emitted alongside it actually returns.
    #[test]
    fn test_neutral_to_rbs_sqlite3_scalars_match_manifest() {
        let m = sqlite3_manifest();
        assert_eq!(neutral_to_rbs("decimal", false, &m), "Float");
        assert_eq!(neutral_to_rbs("date", false, &m), "String");
        assert_eq!(neutral_to_rbs("time", false, &m), "String");
        assert_eq!(neutral_to_rbs("time_tz", false, &m), "String");
        assert_eq!(neutral_to_rbs("datetime", false, &m), "String");
        assert_eq!(neutral_to_rbs("datetime_tz", false, &m), "String");
    }

    /// Regression test for #106: `ruby-oci8` declares `date = "Time"` because OCI8 returns
    /// a `Time` object for Oracle `DATE` columns (Oracle's `DATE` always carries a
    /// time-of-day component), not the `Date` the old hardcoded table emitted. Unlike the
    /// sqlite3 pairs, this one was already live: the committed
    /// `integration_tests/ruby-oci8-oracle` fixtures use a `DATE` column, so the old code
    /// emitted a `.rbs` that contradicted the `.rb` generated beside it.
    #[test]
    fn test_neutral_to_rbs_oci8_date_matches_manifest() {
        let m = oci8_manifest();
        assert_eq!(neutral_to_rbs("date", false, &m), "Time");
    }

    /// `ruby-sqlite3`'s scalar divergence must also surface through `generate_rbs_content`,
    /// not just the unit-level `neutral_to_rbs` helper.
    #[test]
    fn test_generate_rbs_sqlite3_date_time_decimal_columns() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_event".to_string(),
                struct_name: Some("GetEventRow".to_string()),
                columns: vec![
                    col("amount", "decimal", false),
                    col("event_date", "date", false),
                    col("event_time", "time", false),
                    col("created_at", "datetime", false),
                ],
                child_columns: Vec::new(),
                params: vec![],
                command: QueryCommand::One,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "SQLite3::Database", &sqlite3_manifest());
        assert!(rbs.contains("attr_reader amount: Float"), "got:\n{rbs}");
        assert!(rbs.contains("attr_reader event_date: String"), "got:\n{rbs}");
        assert!(rbs.contains("attr_reader event_time: String"), "got:\n{rbs}");
        assert!(rbs.contains("attr_reader created_at: String"), "got:\n{rbs}");
    }

    /// `ruby-oci8`'s `date` divergence must also surface through `generate_rbs_content`.
    #[test]
    fn test_generate_rbs_oci8_date_column() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_event".to_string(),
                struct_name: Some("GetEventRow".to_string()),
                columns: vec![col("created_at", "date", false)],
                child_columns: Vec::new(),
                params: vec![],
                command: QueryCommand::One,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "OCI8", &oci8_manifest());
        assert!(rbs.contains("attr_reader created_at: Time"), "got:\n{rbs}");
    }

    /// The four backends whose manifests already agreed with the old hardcoded table
    /// (`ruby-pg`, `ruby-mysql2`, `ruby-trilogy`, `ruby-tiny-tds`) must keep emitting
    /// exactly what they emitted before this change — driving the lookup from the manifest
    /// must be output-neutral for them.
    #[test]
    fn test_neutral_to_rbs_agreeing_backends_unchanged() {
        for m in [
            manifest(),
            pg_redshift_manifest(),
            mysql2_manifest(),
            trilogy_manifest(),
            tiny_tds_manifest(),
        ] {
            assert_eq!(neutral_to_rbs("int16", false, &m), "Integer");
            assert_eq!(neutral_to_rbs("int32", false, &m), "Integer");
            assert_eq!(neutral_to_rbs("int64", false, &m), "Integer");
            assert_eq!(neutral_to_rbs("float32", false, &m), "Float");
            assert_eq!(neutral_to_rbs("float64", false, &m), "Float");
            assert_eq!(neutral_to_rbs("decimal", false, &m), "BigDecimal");
            assert_eq!(neutral_to_rbs("string", false, &m), "String");
            assert_eq!(neutral_to_rbs("bool", false, &m), "bool");
            assert_eq!(neutral_to_rbs("bytes", false, &m), "String");
            assert_eq!(neutral_to_rbs("uuid", false, &m), "String");
            assert_eq!(neutral_to_rbs("date", false, &m), "Date");
            assert_eq!(neutral_to_rbs("time", false, &m), "Time");
            assert_eq!(neutral_to_rbs("time_tz", false, &m), "Time");
            assert_eq!(neutral_to_rbs("datetime", false, &m), "Time");
            assert_eq!(neutral_to_rbs("datetime_tz", false, &m), "Time");
            assert_eq!(neutral_to_rbs("interval", false, &m), "String");
            assert_eq!(neutral_to_rbs("inet", false, &m), "String");
        }
    }

    #[test]
    fn test_neutral_to_rbs_nullable() {
        let m = manifest();
        assert_eq!(neutral_to_rbs("string", true, &m), "String?");
        assert_eq!(neutral_to_rbs("int32", true, &m), "Integer?");
        assert_eq!(neutral_to_rbs("bool", true, &m), "bool?");
    }

    #[test]
    fn test_neutral_to_rbs_array() {
        let m = manifest();
        assert_eq!(neutral_to_rbs("array<int32>", false, &m), "Array[Integer]");
        assert_eq!(neutral_to_rbs("array<string>", true, &m), "Array[String]?");
    }

    /// `array<...>` is the spelling `type_conversion.rs` emits for every array
    /// column (`"integer[]" => "array<int32>"`). The `array::` form this function
    /// used to strip does not occur, so asserting on it proved nothing while
    /// looking like array coverage -- a real `integer[]` column silently became
    /// `untyped`. Pin the real spelling, and pin that the dead one is not special.
    #[test]
    fn test_neutral_to_rbs_array_uses_the_spelling_the_analyzer_emits() {
        let m = manifest();
        assert_eq!(neutral_to_rbs("array<int64>", false, &m), "Array[Integer]");
        assert_eq!(
            neutral_to_rbs("array<array<int32>>", false, &m),
            "Array[Array[Integer]]"
        );
        assert_eq!(neutral_to_rbs("array::int32", false, &m), "untyped");
    }

    #[test]
    fn test_neutral_to_rbs_enum() {
        let m = manifest();
        assert_eq!(neutral_to_rbs("enum::user_status", false, &m), "String");
    }

    /// `:one` raises `RecordNotFound` on a missing row (see `RECORD_NOT_FOUND_CLASS`) rather
    /// than returning `nil`, so its signature must be non-nullable -- companion test
    /// `test_generate_rbs_opt_query_is_nullable` pins that `:opt` keeps the `?`.
    #[test]
    fn test_generate_rbs_one_query() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_user".to_string(),
                struct_name: Some("GetUserRow".to_string()),
                columns: vec![
                    col("id", "int32", false),
                    col("name", "string", false),
                    col("email", "string", true),
                ],
                child_columns: Vec::new(),
                params: vec![param("id", "int32", false)],
                command: QueryCommand::One,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "PG::Connection", &manifest());
        assert!(rbs.contains("module Queries"));
        assert!(rbs.contains("class GetUserRow"));
        assert!(rbs.contains("attr_reader id: Integer"));
        assert!(rbs.contains("attr_reader name: String"));
        assert!(rbs.contains("attr_reader email: String?"));
        assert!(rbs.contains("def self.new: (id: Integer, name: String, email: String?) -> GetUserRow"));
        assert!(
            rbs.contains("def self.get_user: (PG::Connection, Integer) -> GetUserRow\n"),
            "`:one` must return the bare row type, not `GetUserRow?`; got:\n{rbs}"
        );
        assert!(rbs.contains("end\n"));
    }

    /// Companion to `test_generate_rbs_one_query`: `:opt` is unaffected by the `:one` fix and
    /// keeps returning `{Struct}?`.
    #[test]
    fn test_generate_rbs_opt_query_is_nullable() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_user".to_string(),
                struct_name: Some("GetUserRow".to_string()),
                columns: vec![col("id", "int32", false), col("name", "string", false)],
                child_columns: Vec::new(),
                params: vec![param("id", "int32", false)],
                command: QueryCommand::Opt,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "PG::Connection", &manifest());
        assert!(
            rbs.contains("def self.get_user: (PG::Connection, Integer) -> GetUserRow?"),
            "`:opt` must keep returning `GetUserRow?`; got:\n{rbs}"
        );
    }

    #[test]
    fn test_generate_rbs_many_query() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "list_users".to_string(),
                struct_name: Some("ListUsersRow".to_string()),
                columns: vec![col("id", "int32", false), col("name", "string", false)],
                child_columns: Vec::new(),
                params: vec![],
                command: QueryCommand::Many,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "PG::Connection", &manifest());
        assert!(rbs.contains("def self.list_users: (PG::Connection) -> Array[ListUsersRow]"));
    }

    #[test]
    fn test_generate_rbs_exec_query() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "delete_user".to_string(),
                struct_name: None,
                columns: vec![],
                child_columns: Vec::new(),
                params: vec![param("id", "int32", false)],
                command: QueryCommand::Exec,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "PG::Connection", &manifest());
        assert!(rbs.contains("def self.delete_user: (PG::Connection, Integer) -> void"));
    }

    #[test]
    fn test_generate_rbs_exec_rows_query() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "delete_user".to_string(),
                struct_name: None,
                columns: vec![],
                child_columns: Vec::new(),
                params: vec![param("id", "int32", false)],
                command: QueryCommand::ExecRows,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "PG::Connection", &manifest());
        assert!(rbs.contains("def self.delete_user: (PG::Connection, Integer) -> Integer"));
    }

    #[test]
    fn test_generate_rbs_batch_query() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "insert_user".to_string(),
                struct_name: None,
                columns: vec![],
                child_columns: Vec::new(),
                params: vec![param("name", "string", false), param("email", "string", true)],
                command: QueryCommand::Batch,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "PG::Connection", &manifest());
        assert!(rbs.contains("def self.insert_user_batch: (PG::Connection, Array[[String, String?]]) -> void"));
    }

    #[test]
    fn test_generate_rbs_with_enums() {
        let context = RbsGenerationContext {
            queries: vec![],
            enums: vec![RbsEnumInfo {
                type_name: "UserStatus".to_string(),
                values: vec!["ACTIVE".to_string(), "INACTIVE".to_string()],
            }],
        };

        let rbs = generate_rbs_content(&context, "PG::Connection", &manifest());
        assert!(rbs.contains("module UserStatus"));
        assert!(rbs.contains("ACTIVE: String"));
        assert!(rbs.contains("INACTIVE: String"));
        assert!(rbs.contains("ALL: Array[String]"));
    }

    #[test]
    fn test_generate_rbs_mysql2_connection_type() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_user".to_string(),
                struct_name: Some("GetUserRow".to_string()),
                columns: vec![col("id", "int32", false)],
                child_columns: Vec::new(),
                params: vec![param("id", "int32", false)],
                command: QueryCommand::One,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "Mysql2::Client", &manifest());
        assert!(rbs.contains("def self.get_user: (Mysql2::Client, Integer) -> GetUserRow\n"));
    }

    #[test]
    fn test_generate_rbs_sqlite3_connection_type() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_user".to_string(),
                struct_name: Some("GetUserRow".to_string()),
                columns: vec![col("id", "int32", false)],
                child_columns: Vec::new(),
                params: vec![param("id", "int32", false)],
                command: QueryCommand::One,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "SQLite3::Database", &sqlite3_manifest());
        assert!(rbs.contains("def self.get_user: (SQLite3::Database, Integer) -> GetUserRow\n"));
    }

    #[test]
    fn test_generate_rbs_trilogy_connection_type() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_user".to_string(),
                struct_name: Some("GetUserRow".to_string()),
                columns: vec![col("id", "int32", false)],
                child_columns: Vec::new(),
                params: vec![param("id", "int32", false)],
                command: QueryCommand::One,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "Trilogy", &manifest());
        assert!(rbs.contains("def self.get_user: (Trilogy, Integer) -> GetUserRow\n"));
    }

    /// End-to-end regression test for #101: a `json` column's RBS type must match the
    /// runtime type the backend's own `.rb` code actually returns, per its manifest.
    /// `ruby-sqlite3` returns the raw driver string (`json = "String"` in its manifest).
    #[test]
    fn test_generate_rbs_json_column_sqlite3_matches_manifest() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_settings".to_string(),
                struct_name: Some("GetSettingsRow".to_string()),
                columns: vec![col("payload", "json", false)],
                child_columns: Vec::new(),
                params: vec![],
                command: QueryCommand::One,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "SQLite3::Database", &sqlite3_manifest());
        assert!(
            rbs.contains("attr_reader payload: String"),
            "sqlite3 json column must be String, not Hash; got:\n{rbs}"
        );
        assert!(
            !rbs.contains("Hash[String, untyped]"),
            "sqlite3 must not emit Hash for json; got:\n{rbs}"
        );
    }

    /// Companion to the sqlite3 regression test: `ruby-pg` decodes JSON into a `Hash` at
    /// runtime (`json = "Hash"` in its manifest), so its RBS must keep saying so.
    #[test]
    fn test_generate_rbs_json_column_pg_stays_hash() {
        let context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_settings".to_string(),
                struct_name: Some("GetSettingsRow".to_string()),
                columns: vec![col("payload", "json", false)],
                child_columns: Vec::new(),
                params: vec![],
                command: QueryCommand::One,
            }],
            enums: vec![],
        };

        let rbs = generate_rbs_content(&context, "PG::Connection", &manifest());
        assert!(
            rbs.contains("attr_reader payload: Hash[String, untyped]"),
            "pg json column must stay Hash[String, untyped]; got:\n{rbs}"
        );
    }
}
