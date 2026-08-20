use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name};
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeFieldInfo, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::GeneratedCode;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, RbsGenerationContext, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/ruby-pg.toml");
const DEFAULT_MANIFEST_REDSHIFT: &str = include_str!("../../manifests/ruby-pg.redshift.toml");

pub struct RubyPgBackend {
    manifest: BackendManifest,
}

impl RubyPgBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        let default_toml = match engine {
            "postgresql" | "postgres" | "pg" => DEFAULT_MANIFEST_TOML,
            "redshift" => DEFAULT_MANIFEST_REDSHIFT,
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("ruby-pg only supports PostgreSQL/Redshift, got engine '{}'", engine),
                ));
            }
        };
        let manifest = super::parse_manifest(default_toml)?;
        Ok(Self { manifest })
    }
}

/// A neutral SQL type's `pg` decode step: how to turn `pg`'s raw wire-format text into the
/// manifest-declared Ruby type.
///
/// Two shapes are needed because they compose differently with the nullable path's
/// `&.then { |v| ... }` block (see every `ruby_coercion(...)` call site below): `Suffix` is
/// appended after the raw expression (`row["col"].to_i`, or `v.to_i` inside the block), but a
/// JSON decode has to surround the expression instead -- `row["col"].JSON.parse` is not valid
/// Ruby. Kept as a two-variant enum rather than a second `&'static str` table so every call site
/// goes through one `apply`, instead of each needing its own suffix-vs-wrap branch (GH #147).
enum RubyCoercion {
    /// The raw `pg` text is already the manifest-declared type; no conversion needed.
    None,
    /// Appended directly after the raw expression, e.g. `.to_i`.
    Suffix(&'static str),
    /// Wraps the raw expression, e.g. `JSON.parse(...)`.
    Wrap(&'static str),
}

impl RubyCoercion {
    fn is_none(&self) -> bool {
        matches!(self, RubyCoercion::None)
    }

    /// Apply this coercion to a Ruby expression -- either the raw column access
    /// (`row["col"]`) or the nullable block's `v`.
    fn apply(&self, expr: &str) -> String {
        match self {
            RubyCoercion::None => expr.to_string(),
            RubyCoercion::Suffix(suffix) => format!("{expr}{suffix}"),
            RubyCoercion::Wrap(func) => format!("{func}({expr})"),
        }
    }
}

/// Map a neutral type to a Ruby type coercion for pg.
///
/// Derived from the manifest's own declared Ruby type for `neutral_type`
/// (`manifest.types.scalars`) rather than matching `neutral_type` directly, so this table
/// can never silently diverge from what `ruby-pg.toml` (or a `manifest = "..."` overlay)
/// actually declares -- the failure mode from #198: the old table had no `"decimal"` arm at
/// all, so a `decimal` column came back as `pg`'s raw wire `String` while the `.rbs` this
/// same manifest drives said `BigDecimal`. `pg` does no client-side type casting of its own
/// -- every column value is the wire-format text `pg` received from the server -- so every
/// declared type below needs an explicit conversion; `BigDecimal` uses `.to_d`
/// (`require "bigdecimal/util"`, added to the file header by `file_header_for_results`
/// only when a generated file actually calls it). `Hash` (the manifest's declared type for
/// `json`) and `Array` (declared for `json_array`, the `json_agg` array shape -- see
/// `degrade_unsupported_nested_structs`) are the same bug (GH #147, the other live half of
/// #198): both need `JSON.parse`, gated the same way behind `require "json"`.
fn ruby_coercion(neutral_type: &str, manifest: &BackendManifest) -> RubyCoercion {
    match manifest.types.scalars.get(neutral_type).map(String::as_str) {
        Some("Integer") => RubyCoercion::Suffix(".to_i"),
        Some("Float") => RubyCoercion::Suffix(".to_f"),
        Some("BigDecimal") => RubyCoercion::Suffix(".to_d"),
        Some("Boolean") => RubyCoercion::Suffix(" == \"t\""),
        Some("Hash") | Some("Array") => RubyCoercion::Wrap("JSON.parse"),
        _ => RubyCoercion::None,
    }
}

/// Board #219: the `pg` gem does no client-side decoding of a user-defined composite -- a
/// column typed `composite::address` comes back exactly like every other `pg` value, the
/// driver's raw text form, but `ruby_coercion` has no table entry for it (`composite::*` is
/// never a manifest scalar), so it fell through to the empty-string arm and the raw text
/// reached the row struct untouched, silently wrong against the declared `Address` type.
/// `{Struct}.from_text` (emitted by `generate_composite_def`) parses that text and is already
/// nil-safe, so it replaces the coercion entirely rather than composing with it.
fn ruby_composite_column_expr(col: &ResolvedColumn, raw: &str) -> Option<String> {
    col.neutral_type
        .starts_with("composite::")
        .then(|| format!("{}.from_text({})", col.lang_type, raw))
}

/// The Ruby expression converting one composite field's raw text token (`raw`, already
/// unescaped by `_parse_composite_fields`) into the field's declared value.
///
/// Reuses `ruby_coercion` -- the same table every top-level column already goes through --
/// rather than a bespoke per-field table, so a scalar composite field behaves identically to a
/// column of that same neutral type (including the existing gaps in that table, e.g. no
/// `date`/`time` parsing: not something this fix introduces or can close from here). A nested
/// `composite::` field recurses through that nested type's own `from_text`, which is already
/// nil-safe on a genuinely NULL sub-field.
fn ruby_composite_field_from_text(field: &CompositeFieldInfo, raw: &str, manifest: &BackendManifest) -> String {
    if let Some(sql_name) = field.neutral_type.strip_prefix("composite::") {
        return format!("{}.from_text({})", composite_type_name(sql_name, &manifest.naming), raw);
    }
    ruby_coercion(&field.neutral_type, manifest).apply(raw)
}

/// Splits a PostgreSQL composite's text form (`"(a,b,c)"`) into its raw field tokens, honoring
/// its escaping rules -- an empty unquoted field is SQL NULL, and a field containing a comma,
/// paren, quote, backslash, or leading/trailing space (or the empty string) is double-quoted,
/// with an inner `"` **doubled** and an inner `\` backslash-escaped. See
/// `tests/composite_text_escaping_regression.rs` for the PostgreSQL 16 output this was read off.
const RUBY_PARSE_COMPOSITE_FIELDS_METHOD: &str = r#"    def self._parse_composite_fields(text)
      fields = []
      inner = text[1..-2]
      i = 0
      n = inner.length
      loop do
        chars = []
        is_null = false
        if i < n && inner[i] == '"'
          i += 1
          while i < n
            c = inner[i]
            if c == "\\" && i + 1 < n
              chars << inner[i + 1]
              i += 2
            elsif c == '"' && i + 1 < n && inner[i + 1] == '"'
              chars << '"'
              i += 2
            elsif c == '"'
              i += 1
              break
            else
              chars << c
              i += 1
            end
          end
        else
          start = i
          i += 1 while i < n && inner[i] != ','
          chars = inner[start...i].chars
          is_null = chars.empty?
        end
        fields << (is_null ? nil : chars.join)
        if i < n && inner[i] == ','
          i += 1
          next
        end
        break
      end
      fields
    end
"#;

impl CodegenBackend for RubyPgBackend {
    fn name(&self) -> &str {
        "ruby-pg"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["postgresql", "redshift"]
    }

    fn generate_rbs_file(&self, context: &RbsGenerationContext) -> Option<String> {
        Some(super::ruby_rbs::generate_rbs_content(
            context,
            "PG::Connection",
            &self.manifest,
        ))
    }

    fn file_preamble(&self) -> String {
        "# frozen_string_literal: true\n".to_string()
    }

    fn file_header(&self) -> String {
        format!("module Queries\n{}", super::ruby_rbs::RECORD_NOT_FOUND_CLASS)
    }

    fn file_header_for_results(&self, generated: &[GeneratedCode]) -> String {
        // ~keep Each `require` is added only when this file's generated code actually calls
        // the coercion it backs -- most files never touch `BigDecimal` or `JSON` at all, and
        // an unconditional `require` here would add a stdlib dependency the file never uses.
        // The `.rbs` file needs no counterpart -- see `ruby_rbs.rs`'s `generate_rbs_content`
        // for why a signature naming `BigDecimal` or `Hash`/`Array` carries no directive at
        // all.
        let mut requires = String::new();
        if super::ruby_rbs::ruby_generated_code_needs_bigdecimal_util(generated) {
            requires.push_str("require \"bigdecimal/util\"\n");
        }
        if super::ruby_rbs::ruby_generated_code_needs_json(generated) {
            requires.push_str("require \"json\"\n");
        }
        if requires.is_empty() {
            self.file_header()
        } else {
            format!(
                "{requires}\nmodule Queries\n{}",
                super::ruby_rbs::RECORD_NOT_FOUND_CLASS
            )
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
        let cleaned_sql = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let (rewritten_sql, _) =
            super::rewrite_placeholders_indexed(&cleaned_sql, scythe_core::SqlDialect::PostgreSQL, |position| {
                let placeholder = format!("${position}");
                let param = super::resolved_param_for_position(&analyzed.params, params, position);
                param
                    .neutral_type
                    .strip_prefix("composite::")
                    .map_or(placeholder.clone(), |sql_type| {
                        format!("{placeholder}::text::{sql_type}")
                    })
            });
        let sql = crate::sql_literal::escape_ruby_double_quoted(&rewritten_sql);
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

        let param_array = if params.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                params
                    .iter()
                    .map(|p| {
                        if p.neutral_type.starts_with("composite::") {
                            format!("{}&.to_pg_text", p.field_name)
                        } else {
                            p.field_name.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "    result = conn.exec_params(\"{}\", {})", sql, param_array);
                let _ = writeln!(
                    out,
                    "    raise RecordNotFound, \"{}: no row found\" if result.ntuples.zero?",
                    func_name
                );
                let _ = writeln!(out, "    row = result[0]");

                let fields = columns
                    .iter()
                    .map(|c| {
                        let raw = format!("row[\"{}\"]", c.name);
                        if let Some(expr) = ruby_composite_column_expr(c, &raw) {
                            return format!("{}: {}", c.field_name, expr);
                        }
                        let coercion = ruby_coercion(&c.neutral_type, &self.manifest);
                        if c.nullable {
                            format!("{}: {}&.then {{ |v| {} }}", c.field_name, raw, coercion.apply("v"))
                        } else {
                            format!("{}: {}", c.field_name, coercion.apply(&raw))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "    {}.new({})", struct_name, fields);
            }
            QueryCommand::Opt => {
                let _ = writeln!(out, "    result = conn.exec_params(\"{}\", {})", sql, param_array);
                let _ = writeln!(out, "    return nil if result.ntuples.zero?");
                let _ = writeln!(out, "    row = result[0]");

                let fields = columns
                    .iter()
                    .map(|c| {
                        let raw = format!("row[\"{}\"]", c.name);
                        if let Some(expr) = ruby_composite_column_expr(c, &raw) {
                            return format!("{}: {}", c.field_name, expr);
                        }
                        let coercion = ruby_coercion(&c.neutral_type, &self.manifest);
                        if c.nullable {
                            format!("{}: {}&.then {{ |v| {} }}", c.field_name, raw, coercion.apply("v"))
                        } else {
                            format!("{}: {}", c.field_name, coercion.apply(&raw))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "    {}.new({})", struct_name, fields);
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let _ = writeln!(out, "  def self.{}(conn, items)", batch_fn_name);
                let _ = writeln!(out, "    conn.transaction do");
                let _ = writeln!(out, "      items.each do |item|");
                if params.len() > 1 {
                    let item_params = params
                        .iter()
                        .enumerate()
                        .map(|(index, param)| {
                            if param.neutral_type.starts_with("composite::") {
                                format!("item[{index}]&.to_pg_text")
                            } else {
                                format!("item[{index}]")
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(out, "        conn.exec_params(\"{}\", [{}])", sql, item_params);
                } else if params.len() == 1 {
                    let item = if params[0].neutral_type.starts_with("composite::") {
                        "item&.to_pg_text"
                    } else {
                        "item"
                    };
                    let _ = writeln!(out, "        conn.exec_params(\"{}\", [{}])", sql, item);
                } else {
                    let _ = writeln!(out, "        conn.exec_params(\"{}\", [])", sql);
                }
                let _ = writeln!(out, "      end");
                let _ = writeln!(out, "    end");
                let _ = write!(out, "  end");
                return Ok(out);
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "    result = conn.exec_params(\"{}\", {})", sql, param_array);
                let _ = writeln!(out, "    result.map do |row|");
                let fields = columns
                    .iter()
                    .map(|c| {
                        let raw = format!("row[\"{}\"]", c.name);
                        if let Some(expr) = ruby_composite_column_expr(c, &raw) {
                            return format!("{}: {}", c.field_name, expr);
                        }
                        let coercion = ruby_coercion(&c.neutral_type, &self.manifest);
                        if c.nullable {
                            format!("{}: {}&.then {{ |v| {} }}", c.field_name, raw, coercion.apply("v"))
                        } else {
                            format!("{}: {}", c.field_name, coercion.apply(&raw))
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "      {}.new({})", struct_name, fields);
                let _ = writeln!(out, "    end");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "    conn.exec_params(\"{}\", {})", sql, param_array);
                let _ = writeln!(out, "    nil");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "    result = conn.exec_params(\"{}\", {})", sql, param_array);
                let _ = writeln!(out, "    result.cmd_tuples.to_i");
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
        let cleaned_sql = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let (rewritten_sql, _) =
            super::rewrite_placeholders_indexed(&cleaned_sql, scythe_core::SqlDialect::PostgreSQL, |position| {
                let placeholder = format!("${position}");
                let param = super::resolved_param_for_position(&analyzed.params, params, position);
                param
                    .neutral_type
                    .strip_prefix("composite::")
                    .map_or(placeholder.clone(), |sql_type| {
                        format!("{placeholder}::text::{sql_type}")
                    })
            });
        let sql = crate::sql_literal::escape_ruby_double_quoted(&rewritten_sql);

        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };
        let param_array = if params.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                params
                    .iter()
                    .map(|p| {
                        if p.neutral_type.starts_with("composite::") {
                            format!("{}&.to_pg_text", p.field_name)
                        } else {
                            p.field_name.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let key_neutral_type = all_columns
            .iter()
            .find(|c| c.name == key_column)
            .map(|c| c.neutral_type.as_str())
            .unwrap_or("string");
        let key_coercion = ruby_coercion(key_neutral_type, &self.manifest);

        let mut out = String::new();
        let _ = writeln!(out, "  def self.{}(conn{}{})", func_name, sep, param_list);
        let _ = writeln!(out, "    result = conn.exec_params(\"{}\", {})", sql, param_array);
        let _ = writeln!(out, "    _index = {{}}");
        let _ = writeln!(out, "    _entries = []");
        let _ = writeln!(out, "    result.each do |row|");
        let key_expr = key_coercion.apply(&format!("row[\"{}\"]", key_column));
        let _ = writeln!(out, "      key = {}", key_expr);
        let _ = writeln!(out, "      unless _index.key?(key)");
        let _ = writeln!(out, "        _index[key] = _entries.size");
        let _ = writeln!(out, "        _entries << {{");
        for col in parent_columns {
            let raw = format!("row[\"{}\"]", col.name);
            if let Some(expr) = ruby_composite_column_expr(col, &raw) {
                let _ = writeln!(out, "          {}: {},", col.field_name, expr);
                continue;
            }
            let coercion = ruby_coercion(&col.neutral_type, &self.manifest);
            if col.nullable && !coercion.is_none() {
                let _ = writeln!(
                    out,
                    "          {}: {}&.then {{ |v| {} }},",
                    col.field_name,
                    raw,
                    coercion.apply("v")
                );
            } else {
                let _ = writeln!(out, "          {}: {},", col.field_name, coercion.apply(&raw));
            }
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
            let raw = format!("row[\"{}\"]", col.name);
            if let Some(expr) = ruby_composite_column_expr(col, &raw) {
                let _ = writeln!(out, "        {}: {},", col.field_name, expr);
                continue;
            }
            let coercion = ruby_coercion(&col.neutral_type, &self.manifest);
            if col.nullable && !coercion.is_none() {
                let _ = writeln!(
                    out,
                    "        {}: {}&.then {{ |v| {} }},",
                    col.field_name,
                    raw,
                    coercion.apply("v")
                );
            } else {
                let _ = writeln!(out, "        {}: {},", col.field_name, coercion.apply(&raw));
            }
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
        // ~keep board #219: a composite with zero fields cannot exist in PostgreSQL
        // (`CREATE TYPE ... AS ()` is rejected), so there is no reachable runtime value that
        // would need `from_text` here. Left as the bare `Data.define()` it always was.
        if composite.fields.is_empty() {
            let _ = writeln!(out, "  {} = Data.define()", name);
            return Ok(out);
        }
        let fields = composite
            .fields
            .iter()
            .map(|f| format!(":{}", f.name))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  {} = Data.define({}) do", name, fields);
        let _ = writeln!(
            out,
            "    # ~keep board #219: pg hands back a PostgreSQL composite as its raw text form,"
        );
        let _ = writeln!(out, "    # not this generated type; parse it here instead.");
        let _ = writeln!(out, "    def self.from_text(text)");
        let _ = writeln!(out, "      return nil if text.nil?");
        let _ = writeln!(out);
        let _ = writeln!(out, "      f = _parse_composite_fields(text)");
        let _ = writeln!(out, "      new(");
        for (i, field) in composite.fields.iter().enumerate() {
            let raw = format!("f[{}]", i);
            let expr = ruby_composite_field_from_text(field, &raw, &self.manifest);
            let sep = if i + 1 < composite.fields.len() { "," } else { "" };
            let _ = writeln!(out, "        {}: {}{}", field.name, expr, sep);
        }
        let _ = writeln!(out, "      )");
        let _ = writeln!(out, "    end");
        let _ = writeln!(out);
        let encoded_fields = composite
            .fields
            .iter()
            .map(|field| format!("self.class._encode_composite_field({})", field.name))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "    def to_pg_text");
        let _ = writeln!(out, "      \"(\" + [{}].join(\",\") + \")\"", encoded_fields);
        let _ = writeln!(out, "    end");
        let _ = writeln!(out);
        let _ = writeln!(out, "    def self._encode_composite_field(value)");
        let _ = writeln!(out, "      return \"\" if value.nil?");
        let _ = writeln!(out, "      raw = if value.respond_to?(:to_pg_text)");
        let _ = writeln!(out, "        value.to_pg_text");
        let _ = writeln!(out, "      elsif value.respond_to?(:value)");
        let _ = writeln!(out, "        value.value.to_s");
        let _ = writeln!(out, "      else");
        let _ = writeln!(out, "        value.to_s");
        let _ = writeln!(out, "      end");
        let _ = writeln!(
            out,
            "      return raw unless raw.empty? || raw.match?(/[(),\\\"\\\\]/) || raw != raw.strip"
        );
        let _ = writeln!(
            out,
            "      '\"' + raw.gsub('\\\\', '\\\\\\\\').gsub('\"', '\"\"') + '\"'"
        );
        let _ = writeln!(out, "    end");
        let _ = writeln!(out);
        out.push_str(RUBY_PARSE_COMPOSITE_FIELDS_METHOD);
        let _ = writeln!(out, "  end");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_trait::RbsQueryInfo;
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

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
    fn test_grouped_ruby_pg_structs() {
        let backend = RubyPgBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();
        assert!(
            row_struct.contains("GetUsersWithOrdersChildRow = Data.define(:order_id, :total, :order_date)"),
            "missing child struct; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("GetUsersWithOrdersRow = Data.define(:id, :name, :email, :children)"),
            "missing parent struct with :children; got:\n{row_struct}"
        );
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow").unwrap();
        let parent_pos = row_struct.find("GetUsersWithOrdersRow = Data.define").unwrap();
        assert!(child_pos < parent_pos, "child struct must appear before parent struct");
    }

    #[test]
    fn test_grouped_ruby_pg_query_fn() {
        let backend = RubyPgBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("def self.get_users_with_orders(conn)"),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("exec_params("),
            "must use exec_params; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("_index = {}"),
            "must use _index hash; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("_entries = []"),
            "must use _entries array; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersChildRow.new("),
            "must construct child struct; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("GetUsersWithOrdersRow.new(**e)"),
            "must fold into parent via **e; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("_index[key]"),
            "must look up key in _index; got:\n{query_fn}"
        );
    }

    /// GH #147: `ruby-pg`'s `json = "Hash"` manifest declaration was never backed by a real
    /// decode -- `ruby_coercion` had no arm for it, so a `json`/`jsonb` column reached the row
    /// struct as `pg`'s raw wire-format `String` while the `.rbs` this same manifest drives
    /// said `Hash[String, untyped]` (the `json` sibling of #198's `decimal` bug -- see
    /// `ruby_coercion`'s doc comment). This pins the fix end to end: the `.rb` field
    /// expression must call `JSON.parse`, not read the bare `row["..."]`, the nullable path
    /// must still guard against `nil` before calling it, `file_header_for_results` must add
    /// `require "json"` once any generated code calls it, and the `.rbs` signature this
    /// backend generates alongside the `.rb` must agree with what it actually produces.
    ///
    /// Reverting `Some("Hash") | Some("Array") => RubyCoercion::Wrap("JSON.parse")` (back to
    /// the old `_ => RubyCoercion::None` fallback) in `ruby_coercion` fails the first three
    /// assertions below. Reverting `manifest_type_to_rbs`'s `"Hash" =>
    /// "Hash[String, untyped]".to_string()` arm (back to passing `"Hash"` through unchanged)
    /// fails the last two.
    #[test]
    fn test_json_column_decodes_via_json_parse_and_agrees_with_rbs() {
        let backend = RubyPgBackend::new("postgresql").unwrap();
        let query = AnalyzedQuery::build(|aq| {
            aq.name = "GetSettings".to_string();
            aq.command = QueryCommand::One;
            aq.sql = "SELECT id, payload, extra FROM settings LIMIT 1".to_string();
            aq.columns = vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "payload".to_string(),
                    neutral_type: "json".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "extra".to_string(),
                    neutral_type: "json".to_string(),
                    nullable: true,
                    ..Default::default()
                },
            ];
        });

        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("payload: JSON.parse(row[\"payload\"])"),
            "a non-nullable `json` column must decode via JSON.parse; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("payload: row[\"payload\"]"),
            "a `json` column must not reach the row struct as pg's bare wire-format text; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("extra: row[\"extra\"]&.then { |v| JSON.parse(v) }"),
            "a nullable `json` column must still guard nil before calling JSON.parse; got:\n{query_fn}"
        );

        let header = backend.file_header_for_results(std::slice::from_ref(&result));
        assert!(
            header.contains("require \"json\"\n"),
            "a file whose generated code calls JSON.parse must require \"json\"; got:\n{header}"
        );

        let rbs_context = RbsGenerationContext {
            queries: vec![RbsQueryInfo {
                func_name: "get_settings".to_string(),
                struct_name: Some("GetSettingsRow".to_string()),
                columns: vec![
                    ResolvedColumn {
                        name: "payload".to_string(),
                        field_name: "payload".to_string(),
                        neutral_type: "json".to_string(),
                        nullable: false,
                        ..Default::default()
                    },
                    ResolvedColumn {
                        name: "extra".to_string(),
                        field_name: "extra".to_string(),
                        neutral_type: "json".to_string(),
                        nullable: true,
                        ..Default::default()
                    },
                ],
                child_columns: Vec::new(),
                params: vec![],
                command: QueryCommand::One,
            }],
            enums: vec![],
        };
        let rbs = backend.generate_rbs_file(&rbs_context).unwrap();
        assert!(
            rbs.contains("attr_reader payload: Hash[String, untyped]"),
            "the .rbs type for `payload` must agree with the Hash JSON.parse actually produces; got:\n{rbs}"
        );
        assert!(
            rbs.contains("attr_reader extra: Hash[String, untyped]?"),
            "the .rbs type for the nullable `extra` must agree with the .rb code; got:\n{rbs}"
        );
    }
}
