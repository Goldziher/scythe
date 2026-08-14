use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name};
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::GeneratedCode;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, RbsGenerationContext, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/ruby-oci8.toml");

/// Board #225: oci8 hands back a lazy `OCI8::CLOB`/`OCI8::NCLOB`/`OCI8::BLOB`/`OCI8::BFILE`
/// locator for a LOB column, not the `String` the manifest declares (`clob`/`nclob` -> the
/// scalar `string`, `blob`/`bfile` -> `bytes`, both mapped to Ruby `String` in
/// `ruby-oci8.toml`) -- the row struct promises `String` but a raw `row[N]`/`cursor[N]` read
/// hands the caller the locator object untouched.
///
/// Which columns need this is decided statically, by `is_lob_sql_type` matching the schema's
/// declared `sql_type` (see that function) -- not by a runtime `respond_to?(:read)` duck type
/// on every string/bytes column. The helper itself still needs a runtime nil check, since a
/// NULL LOB column is legitimate and must stay `nil`, not become a `NoMethodError`.
///
/// Verified against the vendored `ruby-oci8` 2.2.14 gem source
/// (`~/.gem/gems/ruby-oci8-2.2.14/ext/oci8/lob.c`), not documentation alone:
/// - `OCI8::CLOB`, `OCI8::NCLOB`, `OCI8::BLOB`, and `OCI8::BFILE` all subclass `OCI8::LOB`
///   (`lob.c`, `oci8_Init_OCI8LOB`: `rb_define_class_under(cOCI8, "CLOB", cOCI8LOB)` etc.),
///   and only `OCI8::LOB` defines `#read` (`rb_define_method(cOCI8LOB, "read", ...)`). oci8
///   converts every other column type client-side to `String`, `Integer`, `Float`,
///   `BigDecimal`, `Time`, or `Date`, and none of those classes define `#read` either -- so
///   even a `respond_to?(:read)` duck type (which this code does not use) would not be overly
///   broad; the static `sql_type` match is simply the more precise, zero-overhead choice,
///   already established for the identical CLOB-vs-VARCHAR2 problem in the sibling
///   `rust_sibyl.rs` backend.
/// - `oci8_lob_read`'s rdoc (`lob.c`, `@overload read`) and its implementation: called with
///   no argument (`length` `nil`), it reads from the current position until EOF and returns
///   the full contents as a `String` -- an empty (but non-null) LOB returns `""` rather than
///   `nil`, which only signals EOF-at-start on a second read.
/// - A NULL LOB column is never handed to the LOB wrapper at all: `oci8_bind_get`
///   (`ext/oci8/bind.c`) checks the OCI null indicator and returns `Qnil` directly, before
///   `bind_lob_get`'s `oci8_lob_clone` ever runs. `cursor.fetch`/`cursor[key]` both resolve
///   through this same `get_data` -> `get` path (`lib/oci8/cursor.rb`, `ext/oci8/bind.c`), so
///   a NULL CLOB/BLOB is plain Ruby `nil`, not a handle whose `#read` returns `nil` -- the
///   helper must pass `nil` through untouched rather than calling `#read` on it.
///
/// This cannot be verified against a live OCI8 driver on this machine (no Oracle Instant
/// Client for macOS ARM64); the source evidence above stands in for a driver run.
const READ_LOB_METHOD: &str = "  def self.read_lob(value)\n    value.nil? ? nil : value.read\n  end";

/// Whether a column's declared SQL type is an Oracle LOB that oci8 returns as a lazy locator
/// rather than a materialized value. `neutral_type` alone can't distinguish this -- CLOB and
/// VARCHAR2 both resolve to `"string"`, BLOB and RAW both resolve to `"bytes"` -- so this
/// matches `sql_type`, the raw source SQL type (see `AnalyzedColumn::sql_type`), the same
/// seam `rust_sibyl.rs::emit_row_get` uses for the identical problem in the sibling Oracle
/// backend.
fn is_lob_sql_type(sql_type: &str) -> bool {
    matches!(sql_type, "clob" | "nclob" | "blob" | "bfile")
}

/// Wrap a raw column-read expression (`row[N]` or `cursor[N]`) in `read_lob` when the
/// column's declared type is a LOB, so the emitted `String`/`bytes` field actually holds the
/// LOB's contents instead of the locator object.
///
/// ~keep Only for the `cursor.fetch` path. A `RETURNING ... INTO` output bind is declared to
/// oci8 up front as `cursor.bind_param(n, nil, String)`, and oci8 materializes the value into
/// that Ruby class rather than handing back a locator -- so `cursor[n]` there is already a
/// `String` and wrapping it raised `undefined method 'read' for an instance of String` in
/// `create_order`. Those two call sites therefore read the ordinal directly. Board #225.
fn column_read_expr(col: &ResolvedColumn, raw: &str) -> String {
    if is_lob_sql_type(&col.sql_type) {
        format!("read_lob({raw})")
    } else {
        raw.to_string()
    }
}

pub struct RubyOci8Backend {
    manifest: BackendManifest,
}

impl RubyOci8Backend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "oracle" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("ruby-oci8 only supports Oracle, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self { manifest })
    }
}

impl CodegenBackend for RubyOci8Backend {
    fn name(&self) -> &str {
        "ruby-oci8"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["oracle"]
    }

    fn generate_rbs_file(&self, context: &RbsGenerationContext) -> Option<String> {
        Some(super::ruby_rbs::generate_rbs_content(context, "OCI8", &self.manifest))
    }

    fn file_preamble(&self) -> String {
        "# frozen_string_literal: true\n".to_string()
    }

    fn file_header(&self) -> String {
        format!(
            "require 'oci8'\n\nmodule Queries\n{}",
            super::ruby_rbs::RECORD_NOT_FOUND_CLASS
        )
    }

    fn file_header_for_results(&self, generated: &[GeneratedCode]) -> String {
        // `read_lob` only when this file's generated code actually calls it -- most files
        // never touch a LOB column, and an unconditional method definition would add dead
        // code to every generated file for a case only Oracle LOB columns hit. ~keep
        let needs_read_lob = generated.iter().any(|code| {
            [
                code.row_struct.as_deref(),
                code.query_fn.as_deref(),
                code.model_struct.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|s| s.contains("read_lob("))
        });
        if needs_read_lob {
            format!(
                "require 'oci8'\n\nmodule Queries\n{}\n{}",
                super::ruby_rbs::RECORD_NOT_FOUND_CLASS,
                READ_LOB_METHOD
            )
        } else {
            self.file_header()
        }
    }

    fn file_footer(&self) -> String {
        "end".to_string()
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        _query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let fields = columns
            .iter()
            .map(|c| format!(":{}", c.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let mut out = String::new();
        let _ = writeln!(out, "  {} = Data.define({})", struct_name, fields);
        Ok(out)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = crate::sql_literal::escape_ruby_double_quoted(&super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!(":{n}"),
        ));
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        if !matches!(analyzed.command, QueryCommand::Batch) {
            let _ = writeln!(out, "  def self.{}(conn{}{})", func_name, sep, param_list);
        }

        let bind_vars = if params.is_empty() {
            String::new()
        } else {
            format!(
                ", {}",
                params
                    .iter()
                    .map(|p| p.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let has_returning = sql.to_uppercase().contains("RETURNING");

        match &analyzed.command {
            QueryCommand::One => {
                if has_returning {
                    let _ = writeln!(out, "    cursor = conn.parse(\"{}\")", {
                        let into_clause = columns
                            .iter()
                            .enumerate()
                            .map(|(i, _)| format!(":{}", params.len() + i + 1))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("{} INTO {}", sql, into_clause)
                    });
                    for (i, p) in params.iter().enumerate() {
                        let _ = writeln!(out, "    cursor.bind_param({}, {})", i + 1, p.field_name);
                    }
                    for (i, col) in columns.iter().enumerate() {
                        let ruby_type = match col.neutral_type.as_str() {
                            "int32" | "int64" => "Integer",
                            "float32" | "float64" | "decimal" => "Float",
                            "date" | "datetime" | "datetime_tz" | "time" | "time_tz" => "Time",
                            _ => "String",
                        };
                        let _ = writeln!(
                            out,
                            "    cursor.bind_param({}, nil, {})",
                            params.len() + i + 1,
                            ruby_type
                        );
                    }
                    let _ = writeln!(out, "    rows_affected = cursor.exec");
                    let _ = writeln!(
                        out,
                        "    raise RecordNotFound, \"{}: no row found\" if rows_affected.zero?",
                        func_name
                    );
                    let fields = columns
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let raw = format!("cursor[{}]", params.len() + i + 1);
                            format!("{}: {raw}", c.field_name)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "    {}.new({})", struct_name, fields);
                } else {
                    let _ = writeln!(out, "    cursor = conn.exec(\"{}\"{})", sql, bind_vars);
                    let _ = writeln!(out, "    row = cursor.fetch");
                    let _ = writeln!(
                        out,
                        "    raise RecordNotFound, \"{}: no row found\" if row.nil?",
                        func_name
                    );
                    let fields = columns
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let raw = format!("row[{}]", i);
                            format!("{}: {}", c.field_name, column_read_expr(c, &raw))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "    {}.new({})", struct_name, fields);
                }
            }
            QueryCommand::Opt => {
                if has_returning {
                    let _ = writeln!(out, "    cursor = conn.parse(\"{}\")", {
                        let into_clause = columns
                            .iter()
                            .enumerate()
                            .map(|(i, _)| format!(":{}", params.len() + i + 1))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("{} INTO {}", sql, into_clause)
                    });
                    for (i, p) in params.iter().enumerate() {
                        let _ = writeln!(out, "    cursor.bind_param({}, {})", i + 1, p.field_name);
                    }
                    for (i, col) in columns.iter().enumerate() {
                        let ruby_type = match col.neutral_type.as_str() {
                            "int32" | "int64" => "Integer",
                            "float32" | "float64" | "decimal" => "Float",
                            "date" | "datetime" | "datetime_tz" | "time" | "time_tz" => "Time",
                            _ => "String",
                        };
                        let _ = writeln!(
                            out,
                            "    cursor.bind_param({}, nil, {})",
                            params.len() + i + 1,
                            ruby_type
                        );
                    }
                    let _ = writeln!(out, "    cursor.exec");
                    let fields = columns
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let raw = format!("cursor[{}]", params.len() + i + 1);
                            format!("{}: {raw}", c.field_name)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "    {}.new({})", struct_name, fields);
                } else {
                    let _ = writeln!(out, "    cursor = conn.exec(\"{}\"{})", sql, bind_vars);
                    let _ = writeln!(out, "    row = cursor.fetch");
                    let _ = writeln!(out, "    return nil if row.nil?");
                    let fields = columns
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let raw = format!("row[{}]", i);
                            format!("{}: {}", c.field_name, column_read_expr(c, &raw))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "    {}.new({})", struct_name, fields);
                }
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "    cursor = conn.exec(\"{}\"{})", sql, bind_vars);
                let _ = writeln!(out, "    results = []");
                let _ = writeln!(out, "    while (row = cursor.fetch)");
                let fields = columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let raw = format!("row[{}]", i);
                        format!("{}: {}", c.field_name, column_read_expr(c, &raw))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "      results << {}.new({})", struct_name, fields);
                let _ = writeln!(out, "    end");
                let _ = writeln!(out, "    results");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "    conn.exec(\"{}\"{})", sql, bind_vars);
                let _ = writeln!(out, "    nil");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                // ~keep `OCI8#exec` is polymorphic in its return: an `OCI8::Cursor` for a SELECT,
                // but the number of rows processed -- a plain Integer -- for INSERT/UPDATE/DELETE.
                // This used to bind the result and call `.row_count` on it, which is a Cursor
                // method, and the Oracle job failed with "undefined method 'row_count' for an
                // instance of Integer" in `delete_orders_by_user`. For DML the count is already
                // the return value. Board #225.
                let _ = writeln!(out, "    conn.exec(\"{}\"{})", sql, bind_vars);
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let _ = writeln!(out, "  def self.{}(conn, items)", batch_fn_name);
                let _ = writeln!(out, "    items.each do |item|");
                if params.len() > 1 {
                    let _ = writeln!(out, "      conn.exec(\"{}\", *item)", sql);
                } else if params.len() == 1 {
                    let _ = writeln!(out, "      conn.exec(\"{}\", item)", sql);
                } else {
                    let _ = writeln!(out, "      conn.exec(\"{}\")", sql);
                }
                let _ = writeln!(out, "    end");
                let _ = write!(out, "  end");
                return Ok(out);
            }
            QueryCommand::Grouped => unreachable!("handled by generate_grouped_query_fn"),
        }

        let _ = write!(out, "  end");
        Ok(out)
    }

    fn generate_grouped_structs(
        &self,
        parent_struct_name: &str,
        child_struct_name: &str,
        parent_columns: &[ResolvedColumn],
        child_columns: &[ResolvedColumn],
        _key_column: &str,
    ) -> Result<String, ScytheError> {
        Ok(super::ruby_rbs::generate_grouped_structs_ruby(
            parent_struct_name,
            child_struct_name,
            parent_columns,
            child_columns,
        ))
    }

    fn generate_grouped_query_fn(&self, request: &GroupedQueryFn<'_>) -> Result<String, ScytheError> {
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let all_columns = request.all_columns;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = crate::sql_literal::escape_ruby_double_quoted(&super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!(":{n}"),
        ));

        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };
        let bind_vars = if params.is_empty() {
            String::new()
        } else {
            format!(
                ", {}",
                params
                    .iter()
                    .map(|p| p.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let key_idx = all_columns.iter().position(|c| c.name == key_column).unwrap_or(0);

        let mut out = String::new();
        let _ = writeln!(out, "  def self.{}(conn{}{})", func_name, sep, param_list);
        let _ = writeln!(out, "    cursor = conn.exec(\"{}\"{})", sql, bind_vars);
        let _ = writeln!(out, "    _index = {{}}");
        let _ = writeln!(out, "    _entries = []");
        let _ = writeln!(out, "    while (row = cursor.fetch)");
        // The grouping key is read raw, not through `column_read_expr` -- a LOB group key is
        // both nonsensical (grouping rows by full LOB content) and, since the key column is
        // typically also one of `parent_columns`, would call `#read` on the same handle
        // twice; a LOB's read position advances past EOF after the first call, so the second
        // read (building the parent-row field below) would silently come back empty. ~keep
        let _ = writeln!(out, "      key = row[{}]", key_idx);
        let _ = writeln!(out, "      unless _index.key?(key)");
        let _ = writeln!(out, "        _index[key] = _entries.size");
        let _ = writeln!(out, "        _entries << {{");
        for col in parent_columns {
            let col_idx = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
            let raw = format!("row[{}]", col_idx);
            let _ = writeln!(out, "          {}: {},", col.field_name, column_read_expr(col, &raw));
        }
        let _ = writeln!(out, "          children: []");
        let _ = writeln!(out, "        }}");
        let _ = writeln!(out, "      end");
        let _ = writeln!(
            out,
            "      _entries[_index[key]][:children] << {}.new(",
            child_struct_name
        );
        for col in child_columns {
            let col_idx = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
            let raw = format!("row[{}]", col_idx);
            let _ = writeln!(out, "        {}: {},", col.field_name, column_read_expr(col, &raw));
        }
        let _ = writeln!(out, "      )");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out, "    _entries.map {{ |e| {}.new(**e) }}", parent_struct_name);
        let _ = write!(out, "  end");

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "  module {}", type_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "    {} = \"{}\"", variant, value);
        }
        let all_values = enum_info
            .values
            .iter()
            .map(|v| enum_variant_name(v, &self.manifest.naming))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "    ALL = [{}].freeze", all_values);
        let _ = write!(out, "  end");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        if composite.fields.is_empty() {
            let _ = writeln!(out, "  {} = Data.define()", name);
        } else {
            let fields = composite
                .fields
                .iter()
                .map(|f| format!(":{}", f.name))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "  {} = Data.define({})", name, fields);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    use crate::backends::get_backend;
    use crate::generate_with_backend;

    /// Board #225: all four Oracle LOB `sql_type`s -- not just `clob`, the one that showed up
    /// in the CI failure -- must be routed through `read_lob`. `nclob` matches
    /// `attachments.description NCLOB` and `bfile` matches no current schema column but is
    /// covered because `sql_type_to_neutral` (scythe-core) maps it to `bytes` the same as
    /// `blob`, and `rust_sibyl.rs`'s sibling fix treats it identically.
    #[test]
    fn is_lob_sql_type_matches_all_four_lob_kinds_and_rejects_non_lob_types() {
        for lob_type in ["clob", "nclob", "blob", "bfile"] {
            assert!(is_lob_sql_type(lob_type), "{lob_type} must be treated as a LOB");
        }
        for non_lob_type in ["varchar2", "number", "integer", "string", "bytes", "date"] {
            assert!(
                !is_lob_sql_type(non_lob_type),
                "{non_lob_type} must not be treated as a LOB"
            );
        }
    }

    #[test]
    fn column_read_expr_wraps_lob_columns_and_passes_non_lob_columns_through_raw() {
        let clob_column = ResolvedColumn {
            name: "notes".to_string(),
            field_name: "notes".to_string(),
            lang_type: "String".to_string(),
            full_type: "String".to_string(),
            neutral_type: "string".to_string(),
            nullable: true,
            join_group: None,
            nullable_before_join: false,
            sql_type: "clob".to_string(),
        };
        assert_eq!(column_read_expr(&clob_column, "row[1]"), "read_lob(row[1])");

        let varchar_column = ResolvedColumn {
            sql_type: "varchar2".to_string(),
            ..clob_column.clone()
        };
        assert_eq!(
            column_read_expr(&varchar_column, "row[1]"),
            "row[1]",
            "VARCHAR2 shares CLOB's neutral_type (\"string\") but must not be wrapped"
        );
    }

    /// Board #225 regression. The LOB fix wrapped *every* CLOB read in `read_lob`, including the
    /// `RETURNING ... INTO` output binds, and CI failed with
    /// "undefined method 'read' for an instance of String" in `create_order`. oci8 materializes
    /// an output bind into the Ruby class the `bind_param(n, nil, String)` declares, so
    /// `cursor[n]` on that path is already a String and must be read raw -- while the
    /// `cursor.fetch` path still hands back a locator and must stay wrapped. Both halves are
    /// asserted here because only one of them can be checked by looking at `column_read_expr`.
    #[test]
    fn should_not_wrap_returning_output_binds_in_read_lob() {
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "CreateOrder".to_string();
            aq.command = QueryCommand::One;
            aq.sql = "-- @name CreateOrder\n-- @returns :one\nINSERT INTO orders (user_id, notes) \
                      VALUES (:1, :2) RETURNING id, notes"
                .to_string();
            aq.columns = vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    sql_type: "number".to_string(),
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "notes".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    sql_type: "clob".to_string(),
                    ..Default::default()
                },
            ];
        });
        let backend = get_backend("ruby-oci8", "oracle").unwrap();
        let code = generate_with_backend(&query, &*backend).unwrap();
        let query_fn = code.query_fn.clone().expect("a query fn must be emitted");

        assert!(
            query_fn.contains("nil, String)"),
            "the fixture must actually take the RETURNING output-bind path, which declares each \
             returned column's Ruby class up front; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("read_lob(cursor["),
            "an output bind is already materialized -- wrapping it raises NoMethodError on \
             String; got:\n{query_fn}"
        );
    }

    /// `OCI8#exec` returns an `OCI8::Cursor` for a SELECT but the processed-row count -- a plain
    /// Integer -- for DML. Calling the Cursor-only `.row_count` on that Integer is what made the
    /// Oracle job fail with "undefined method 'row_count' for an instance of Integer" in
    /// `delete_orders_by_user`. Board #225.
    #[test]
    fn exec_rows_returns_the_oci8_exec_count_directly_instead_of_calling_row_count() {
        for command in [QueryCommand::ExecRows, QueryCommand::ExecResult] {
            let query = AnalyzedQuery::build(|aq| {
                aq.name = "DeleteOrdersByUser".to_string();
                aq.command = command.clone();
                aq.sql = "DELETE FROM orders WHERE user_id = :1".to_string();
                aq.columns = vec![];
            });
            let backend = get_backend("ruby-oci8", "oracle").unwrap();
            let code = generate_with_backend(&query, &*backend).unwrap();
            let query_fn = code.query_fn.clone().expect("a query fn must be emitted");

            assert!(
                !query_fn.contains("row_count"),
                "{command:?}: `row_count` is an OCI8::Cursor method and DML returns an Integer; \
                 got:\n{query_fn}"
            );
            assert!(
                query_fn.contains("    conn.exec("),
                "{command:?}: the count must be `conn.exec`'s own return value; got:\n{query_fn}"
            );
        }
    }

    fn make_grouped_query() -> AnalyzedQuery {
        let parent_cols = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "email".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
        let child_cols = vec![
            AnalyzedColumn {
                name: "order_id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "total".to_string(),
                neutral_type: "decimal".to_string(),
                nullable: true,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "order_date".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
        AnalyzedQuery::build(|aq| {
            aq.name = "GetUsersWithOrders".to_string();
            aq.command = QueryCommand::Grouped;
            aq.sql = "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\nFROM users u\nJOIN orders o ON o.user_id = u.id".to_string();
            aq.columns = all_cols;
            aq.params = vec![];
            aq.deprecated = None;
            aq.source_table = None;
            aq.composites = vec![];
            aq.enums = vec![];
            aq.optional_params = vec![];
            aq.group_by = Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: parent_cols,
                child_columns: child_cols,
            });
            aq.custom = vec![];
        })
    }

    #[test]
    fn test_grouped_ruby_oci8_structs() {
        let backend = RubyOci8Backend::new("oracle").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();
        assert!(
            row_struct.contains("GetUsersWithOrdersChildRow = Data.define(:order_id, :total, :order_date)"),
            "missing child struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("GetUsersWithOrdersRow = Data.define(:id, :name, :email, :children)"),
            "missing parent struct; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("GetUsersWithOrdersRow = Data.define").unwrap();
        assert!(child_pos < parent_pos, "child struct must appear before parent struct");
    }

    #[test]
    fn test_grouped_ruby_oci8_query_fn() {
        let backend = RubyOci8Backend::new("oracle").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("def self.get_users_with_orders(conn)"),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("cursor = conn.exec("),
            "must use conn.exec; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("while (row = cursor.fetch)"),
            "must use while-fetch; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("key = row[0]"),
            "must access key by index; got:\n{query_fn}"
        );
        assert!(query_fn.contains("_index = {}"), "must use _index; got:\n{query_fn}");
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow.new("),
            "must construct child struct; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersRow.new(**e)"),
            "must fold into parent; got:\n{query_fn}"
        );
    }
}
