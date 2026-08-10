use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, to_pascal_case};
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, GroupedQueryFn, RbsGenerationContext, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/ruby-trilogy.toml");

pub struct RubyTrilogyBackend {
    manifest: BackendManifest,
}

impl RubyTrilogyBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "mysql" | "mariadb" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("ruby-trilogy only supports MySQL, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self { manifest })
    }
}

/// Map a neutral type to a Ruby type coercion method for trilogy.
fn ruby_coercion(neutral_type: &str) -> &'static str {
    match neutral_type {
        "int16" | "int32" | "int64" => ".to_i",
        "float32" | "float64" => ".to_f",
        "bool" => " == 1",
        _ => "",
    }
}

/// Neutral types that render as bare SQL numeric literals and must not be quoted.
///
/// Every other neutral type (`string`, `uuid`, `date`, `time`, `time_tz`, `datetime`,
/// `datetime_tz`, `interval`, `json`, `inet`, `bytes`, `enum::*`, ...) is SQL-string-shaped
/// and must be quoted -- see [`ruby_sql_literal`].
fn is_unquoted_numeric_type(neutral_type: &str) -> bool {
    matches!(
        neutral_type,
        "int16" | "int32" | "int64" | "float32" | "float64" | "decimal" | "bool"
    )
}

/// Build the Ruby string-interpolation snippet substituted for a `?` placeholder in
/// generated SQL text.
///
/// The trilogy gem's `Trilogy` client has no prepared-statement / bind-parameter API: its
/// C extension defines only `query`, `query_with_flags`, `escape`, and connection-management
/// methods (verified against the trilogy 2.12.x gem -- no `prepare` method, no `Statement`
/// class). Parameters must therefore be embedded directly into the SQL text. Every value that
/// isn't a bare SQL numeric literal is formatted to its correct SQL representation, quoted,
/// and passed through `Trilogy#escape` (a binding for `trilogy_escape`, the client-side
/// equivalent of `mysql_real_escape_string`) so quotes, backslashes, and other special
/// characters in the value cannot break out of the string literal.
fn ruby_sql_literal(neutral_type: &str, field_expr: &str) -> String {
    if is_unquoted_numeric_type(neutral_type) {
        return format!("#{{{field_expr}}}");
    }
    let value_expr = match neutral_type {
        "date" => format!("{field_expr}.strftime('%Y-%m-%d')"),
        "time" | "time_tz" => format!("{field_expr}.strftime('%H:%M:%S')"),
        "datetime" | "datetime_tz" => format!("{field_expr}.strftime('%Y-%m-%d %H:%M:%S')"),
        "json" => format!("{field_expr}.to_json"),
        _ => format!("{field_expr}.to_s"),
    };
    format!("'#{{client.escape({value_expr})}}'")
}

impl CodegenBackend for RubyTrilogyBackend {
    fn name(&self) -> &str {
        "ruby-trilogy"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["mysql", "mariadb"]
    }

    fn generate_rbs_file(&self, context: &RbsGenerationContext) -> Option<String> {
        Some(super::ruby_rbs::generate_rbs_content(
            context,
            "Trilogy",
            &self.manifest,
        ))
    }

    fn file_preamble(&self) -> String {
        "# frozen_string_literal: true\n".to_string()
    }

    fn file_header(&self) -> String {
        "require \"json\"\n\nmodule Queries".to_string()
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
        let sql = crate::sql_literal::escape_ruby_double_quoted(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(out, "  def self.{}(client{}{})", func_name, sep, param_list);
                let mut sql_interpolated = sql.clone();
                for param in params.iter() {
                    if let Some(pos) = sql_interpolated.find('?') {
                        let replacement = ruby_sql_literal(&param.neutral_type, &param.field_name);
                        sql_interpolated.replace_range(pos..pos + 1, &replacement);
                    }
                }

                if params.is_empty() {
                    let _ = writeln!(out, "    results = client.query(\"{}\")", sql);
                } else {
                    let _ = writeln!(out, "    results = client.query(\"{}\")", sql_interpolated);
                }
                let _ = writeln!(out, "    row = results.first");
                let _ = writeln!(out, "    return nil if row.nil?");

                let fields = columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let coercion = ruby_coercion(&c.neutral_type);
                        if c.nullable {
                            format!("{}: row[{}]&.then {{ |v| v{} }}", c.field_name, i, coercion)
                        } else {
                            format!("{}: row[{}]{}", c.field_name, i, coercion)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "    {}.new({})", struct_name, fields);
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let _ = writeln!(out, "  def self.{}(client, items)", batch_fn_name);
                let _ = writeln!(out, "    items.each do |item|");
                if params.is_empty() {
                    let _ = writeln!(out, "      client.query(\"{}\")", sql);
                } else if params.len() > 1 {
                    let mut sql_with_params = sql.clone();
                    for (i, param) in params.iter().enumerate() {
                        if let Some(pos) = sql_with_params.find('?') {
                            let item_expr = format!("item[{}]", i);
                            let replacement = ruby_sql_literal(&param.neutral_type, &item_expr);
                            sql_with_params.replace_range(pos..pos + 1, &replacement);
                        }
                    }
                    let _ = writeln!(out, "      client.query(\"{}\")", sql_with_params);
                } else {
                    let mut sql_with_param = sql.clone();
                    if let Some(pos) = sql_with_param.find('?') {
                        let param = &params[0];
                        let replacement = ruby_sql_literal(&param.neutral_type, "item");
                        sql_with_param.replace_range(pos..pos + 1, &replacement);
                    }
                    let _ = writeln!(out, "      client.query(\"{}\")", sql_with_param);
                }
                let _ = writeln!(out, "    end");
                let _ = write!(out, "  end");
                return Ok(out);
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "  def self.{}(client{}{})", func_name, sep, param_list);
                let mut sql_interpolated = sql.clone();
                for param in params.iter() {
                    if let Some(pos) = sql_interpolated.find('?') {
                        let replacement = ruby_sql_literal(&param.neutral_type, &param.field_name);
                        sql_interpolated.replace_range(pos..pos + 1, &replacement);
                    }
                }

                if params.is_empty() {
                    let _ = writeln!(out, "    results = client.query(\"{}\")", sql);
                } else {
                    let _ = writeln!(out, "    results = client.query(\"{}\")", sql_interpolated);
                }
                let _ = writeln!(out, "    results.map do |row|");
                let fields = columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let coercion = ruby_coercion(&c.neutral_type);
                        if c.nullable {
                            format!("{}: row[{}]&.then {{ |v| v{} }}", c.field_name, i, coercion)
                        } else {
                            format!("{}: row[{}]{}", c.field_name, i, coercion)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "      {}.new({})", struct_name, fields);
                let _ = writeln!(out, "    end");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "  def self.{}(client{}{})", func_name, sep, param_list);
                let mut sql_interpolated = sql.clone();
                for param in params.iter() {
                    if let Some(pos) = sql_interpolated.find('?') {
                        let replacement = ruby_sql_literal(&param.neutral_type, &param.field_name);
                        sql_interpolated.replace_range(pos..pos + 1, &replacement);
                    }
                }

                if params.is_empty() {
                    let _ = writeln!(out, "    client.query(\"{}\")", sql);
                } else {
                    let _ = writeln!(out, "    client.query(\"{}\")", sql_interpolated);
                }
                let _ = writeln!(out, "    nil");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "  def self.{}(client{}{})", func_name, sep, param_list);
                let mut sql_interpolated = sql.clone();
                for param in params.iter() {
                    if let Some(pos) = sql_interpolated.find('?') {
                        let replacement = ruby_sql_literal(&param.neutral_type, &param.field_name);
                        sql_interpolated.replace_range(pos..pos + 1, &replacement);
                    }
                }

                if params.is_empty() {
                    let _ = writeln!(out, "    client.query(\"{}\")", sql);
                } else {
                    let _ = writeln!(out, "    client.query(\"{}\")", sql_interpolated);
                }
                let _ = writeln!(out, "    client.affected_rows");
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
        let sql = crate::sql_literal::escape_ruby_double_quoted(&super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        ));

        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let mut sql_interpolated = sql.clone();
        for param in params.iter() {
            if let Some(pos) = sql_interpolated.find('?') {
                let replacement = ruby_sql_literal(&param.neutral_type, &param.field_name);
                sql_interpolated.replace_range(pos..pos + 1, &replacement);
            }
        }
        let final_sql = if params.is_empty() { sql } else { sql_interpolated };

        let key_idx = all_columns.iter().position(|c| c.name == key_column).unwrap_or(0);
        let key_neutral_type = all_columns
            .iter()
            .find(|c| c.name == key_column)
            .map(|c| c.neutral_type.as_str())
            .unwrap_or("string");
        let key_coercion = ruby_coercion(key_neutral_type);

        let mut out = String::new();
        let _ = writeln!(out, "  def self.{}(client{}{})", func_name, sep, param_list);
        let _ = writeln!(out, "    results = client.query(\"{}\")", final_sql);
        let _ = writeln!(out, "    _index = {{}}");
        let _ = writeln!(out, "    _entries = []");
        let _ = writeln!(out, "    results.each do |row|");
        let _ = writeln!(out, "      key = row[{}]{}", key_idx, key_coercion);
        let _ = writeln!(out, "      unless _index.key?(key)");
        let _ = writeln!(out, "        _index[key] = _entries.size");
        let _ = writeln!(out, "        _entries << {{");
        for col in parent_columns {
            let col_idx = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
            let coercion = ruby_coercion(&col.neutral_type);
            if col.nullable && !coercion.is_empty() {
                let _ = writeln!(
                    out,
                    "          {}: row[{}]&.then {{ |v| v{} }},",
                    col.field_name, col_idx, coercion
                );
            } else {
                let _ = writeln!(out, "          {}: row[{}]{},", col.field_name, col_idx, coercion);
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
            let col_idx = all_columns.iter().position(|c| c.name == col.name).unwrap_or(0);
            let coercion = ruby_coercion(&col.neutral_type);
            if col.nullable && !coercion.is_empty() {
                let _ = writeln!(
                    out,
                    "        {}: row[{}]&.then {{ |v| v{} }},",
                    col.field_name, col_idx, coercion
                );
            } else {
                let _ = writeln!(out, "        {}: row[{}]{},", col.field_name, col_idx, coercion);
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
        let name = to_pascal_case(&composite.sql_name);
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
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    fn one_param_query(sql: &str, param_name: &str, neutral_type: &str) -> AnalyzedQuery {
        AnalyzedQuery::build(|aq| {
            aq.name = "GetById".to_string();
            aq.command = QueryCommand::One;
            aq.sql = sql.to_string();
            aq.columns = vec![
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
            ];
            aq.params = vec![AnalyzedParam {
                name: param_name.to_string(),
                neutral_type: neutral_type.to_string(),
                nullable: false,
                position: 1,
            }];
        })
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
    fn test_grouped_ruby_trilogy_structs() {
        let backend = RubyTrilogyBackend::new("mysql").unwrap();
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
    fn test_grouped_ruby_trilogy_query_fn() {
        let backend = RubyTrilogyBackend::new("mysql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("def self.get_users_with_orders(client)"),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("client.query("),
            "must use client.query; got:\n{query_fn}"
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

    /// Regression test for the CI failure `1054: Unknown column 'f3373249' in 'WHERE'`.
    ///
    /// A `uuid` param must be quoted and passed through `Trilogy#escape`, not
    /// raw-interpolated -- an unquoted UUID like `f3373249-...` is parsed by MySQL/MariaDB
    /// as a bare identifier, not a string literal.
    #[test]
    fn test_uuid_param_is_quoted_and_escaped() {
        let backend = RubyTrilogyBackend::new("mariadb").unwrap();
        let query = one_param_query("SELECT id, name FROM users WHERE id = ?", "id", "uuid");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("WHERE id = '#{client.escape(id.to_s)}'"),
            "uuid param must be quoted and escaped via client.escape; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("WHERE id = #{id}"),
            "uuid param must not be raw-interpolated without quotes; got:\n{query_fn}"
        );
    }

    /// A `datetime` param must be formatted into a MySQL-compatible literal (`Time#to_s`
    /// appends a UTC offset MySQL cannot parse as DATETIME), then quoted and escaped.
    #[test]
    fn test_datetime_param_is_formatted_quoted_and_escaped() {
        let backend = RubyTrilogyBackend::new("mariadb").unwrap();
        let query = one_param_query(
            "SELECT id, name FROM users WHERE created_at = ?",
            "created_at",
            "datetime",
        );
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("WHERE created_at = '#{client.escape(created_at.strftime('%Y-%m-%d %H:%M:%S'))}'"),
            "datetime param must be strftime-formatted, quoted, and escaped; got:\n{query_fn}"
        );
    }

    /// A `date` param uses a date-only format (no time component).
    #[test]
    fn test_date_param_is_formatted_quoted_and_escaped() {
        let backend = RubyTrilogyBackend::new("mariadb").unwrap();
        let query = one_param_query("SELECT id, name FROM users WHERE dob = ?", "dob", "date");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("WHERE dob = '#{client.escape(dob.strftime('%Y-%m-%d'))}'"),
            "date param must be strftime-formatted, quoted, and escaped; got:\n{query_fn}"
        );
    }

    /// String params (which may contain apostrophes, e.g. `O'Brien`) must be routed through
    /// `Trilogy#escape` rather than embedded raw. Trilogy has no bind-parameter API (verified
    /// against the trilogy 2.12.x gem's C extension: only `query`, `query_with_flags`, and
    /// `escape` are defined -- no `prepare`, no `Statement` class), so this is the only way to
    /// prevent a value like `O'Brien` from breaking out of the SQL string literal. This test
    /// fails if the code reverts to raw `'#{name}'` interpolation, which is exactly the shape
    /// that breaks (and is injectable) for any value containing a quote.
    #[test]
    fn test_string_param_uses_escape_not_raw_interpolation() {
        let backend = RubyTrilogyBackend::new("mariadb").unwrap();
        let query = one_param_query("SELECT id, name FROM users WHERE name = ?", "name", "string");
        let result = crate::generate_with_backend(&query, &backend).unwrap();
        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("WHERE name = '#{client.escape(name.to_s)}'"),
            "string param must be quoted and escaped via client.escape; got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("WHERE name = '#{name}'"),
            "string param must not be raw-interpolated inside quotes without escaping; got:\n{query_fn}"
        );
    }

    /// Numeric-shaped types must remain bare (unquoted, unescaped) SQL literals.
    #[test]
    fn test_numeric_and_bool_params_remain_unquoted() {
        for neutral_type in ["int32", "int64", "float64", "decimal", "bool"] {
            let replacement = ruby_sql_literal(neutral_type, "value");
            assert_eq!(
                replacement, "#{value}",
                "neutral_type '{neutral_type}' must stay a bare numeric literal; got: {replacement}"
            );
        }
    }

    /// Enum params must remain quoted and escaped, matching the pre-fix behavior for enums.
    #[test]
    fn test_enum_param_remains_quoted_and_escaped() {
        let replacement = ruby_sql_literal("enum::UsersStatus", "status");
        assert_eq!(replacement, "'#{client.escape(status.to_s)}'");
    }

    /// `json` params (Ruby `Hash`) must be serialized before quoting/escaping.
    #[test]
    fn test_json_param_is_serialized_quoted_and_escaped() {
        let replacement = ruby_sql_literal("json", "metadata");
        assert_eq!(replacement, "'#{client.escape(metadata.to_json)}'");
    }
}
