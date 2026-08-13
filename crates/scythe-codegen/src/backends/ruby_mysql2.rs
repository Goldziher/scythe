use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, to_pascal_case};
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::GeneratedCode;
use crate::backend_trait::{CodegenBackend, GroupedQueryFn, RbsGenerationContext, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/ruby-mysql2.toml");

pub struct RubyMysql2Backend {
    manifest: BackendManifest,
}

impl RubyMysql2Backend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "mysql" | "mariadb" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("ruby-mysql2 only supports MySQL, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self { manifest })
    }
}

/// Map a neutral type to a Ruby type coercion method for mysql2.
///
/// Derived from the manifest's own declared Ruby type for `neutral_type`, not a parallel
/// hardcoded table -- see `ruby_pg.rs`'s `ruby_coercion` for why (#198). The mysql2 C
/// extension already casts `DECIMAL`/`NEWDECIMAL` columns to `BigDecimal` itself (verified
/// against the installed mysql2 0.5.7 gem's `ext/mysql2/result.c`, which calls
/// `Kernel#BigDecimal` on the raw column bytes for both types), so `.to_d` below is a
/// same-type no-op here (`BigDecimal#to_d` returns `self`) rather than a real conversion --
/// unlike `pg` and `trilogy`, where the driver hands back a `String` and `.to_d` does the
/// actual work. Applying it unconditionally anyway keeps this function agreeing with the
/// manifest by construction instead of by a second per-driver judgment call, matching how
/// `.to_i`/`.to_f` were already applied unconditionally here even though mysql2 also
/// natively returns `Integer`/`Float` for those.
fn ruby_coercion(neutral_type: &str, manifest: &BackendManifest) -> &'static str {
    match manifest.types.scalars.get(neutral_type).map(String::as_str) {
        Some("Integer") => ".to_i",
        Some("Float") => ".to_f",
        Some("BigDecimal") => ".to_d",
        Some("Boolean") => " == 1",
        _ => "",
    }
}

impl CodegenBackend for RubyMysql2Backend {
    fn name(&self) -> &str {
        "ruby-mysql2"
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
            "Mysql2::Client",
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
        // See `ruby_pg.rs`'s identical override: `require "bigdecimal/util"` only when this
        // file's generated code actually calls `.to_d`.
        if super::ruby_rbs::ruby_generated_code_needs_bigdecimal_util(generated) {
            format!(
                "require \"bigdecimal/util\"\n\nmodule Queries\n{}",
                super::ruby_rbs::RECORD_NOT_FOUND_CLASS
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

        let _ = writeln!(out, "  def self.{}(client{}{})", func_name, sep, param_list);

        let param_array = if params.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                params
                    .iter()
                    .map(|p| p.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(out, "    stmt = client.prepare(\"{}\")", sql);
                let _ = writeln!(
                    out,
                    "    results = stmt.execute({})",
                    param_array.trim_start_matches('[').trim_end_matches(']')
                );
                let _ = writeln!(out, "    row = results.first");
                let _ = writeln!(
                    out,
                    "    raise RecordNotFound, \"{}: no row found\" if row.nil?",
                    func_name
                );

                let fields = columns
                    .iter()
                    .map(|c| {
                        let coercion = ruby_coercion(&c.neutral_type, &self.manifest);
                        if c.nullable {
                            format!("{}: row[\"{}\"]&.then {{ |v| v{} }}", c.field_name, c.name, coercion)
                        } else {
                            format!("{}: row[\"{}\"]{}", c.field_name, c.name, coercion)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "    {}.new({})", struct_name, fields);
            }
            QueryCommand::Opt => {
                let _ = writeln!(out, "    stmt = client.prepare(\"{}\")", sql);
                let _ = writeln!(
                    out,
                    "    results = stmt.execute({})",
                    param_array.trim_start_matches('[').trim_end_matches(']')
                );
                let _ = writeln!(out, "    row = results.first");
                let _ = writeln!(out, "    return nil if row.nil?");

                let fields = columns
                    .iter()
                    .map(|c| {
                        let coercion = ruby_coercion(&c.neutral_type, &self.manifest);
                        if c.nullable {
                            format!("{}: row[\"{}\"]&.then {{ |v| v{} }}", c.field_name, c.name, coercion)
                        } else {
                            format!("{}: row[\"{}\"]{}", c.field_name, c.name, coercion)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "    {}.new({})", struct_name, fields);
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let _ = writeln!(out, "  def self.{}(client, items)", batch_fn_name);
                let _ = writeln!(out, "    stmt = client.prepare(\"{}\")", sql);
                let _ = writeln!(out, "    items.each do |item|");
                if params.len() > 1 {
                    let _ = writeln!(out, "      stmt.execute(*item)");
                } else if params.len() == 1 {
                    let _ = writeln!(out, "      stmt.execute(item)");
                } else {
                    let _ = writeln!(out, "      stmt.execute");
                }
                let _ = writeln!(out, "    end");
                let _ = write!(out, "  end");
                return Ok(out);
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "    stmt = client.prepare(\"{}\")", sql);
                let _ = writeln!(
                    out,
                    "    results = stmt.execute({})",
                    param_array.trim_start_matches('[').trim_end_matches(']')
                );
                let _ = writeln!(out, "    results.map do |row|");
                let fields = columns
                    .iter()
                    .map(|c| {
                        let coercion = ruby_coercion(&c.neutral_type, &self.manifest);
                        if c.nullable {
                            format!("{}: row[\"{}\"]&.then {{ |v| v{} }}", c.field_name, c.name, coercion)
                        } else {
                            format!("{}: row[\"{}\"]{}", c.field_name, c.name, coercion)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "      {}.new({})", struct_name, fields);
                let _ = writeln!(out, "    end");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "    stmt = client.prepare(\"{}\")", sql);
                let _ = writeln!(
                    out,
                    "    stmt.execute({})",
                    param_array.trim_start_matches('[').trim_end_matches(']')
                );
                let _ = writeln!(out, "    nil");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "    stmt = client.prepare(\"{}\")", sql);
                let _ = writeln!(
                    out,
                    "    stmt.execute({})",
                    param_array.trim_start_matches('[').trim_end_matches(']')
                );
                let _ = writeln!(out, "    stmt.affected_rows");
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
        let execute_args = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");

        let key_neutral_type = all_columns
            .iter()
            .find(|c| c.name == key_column)
            .map(|c| c.neutral_type.as_str())
            .unwrap_or("string");
        let key_coercion = ruby_coercion(key_neutral_type, &self.manifest);

        let mut out = String::new();
        let _ = writeln!(out, "  def self.{}(client{}{})", func_name, sep, param_list);
        let _ = writeln!(out, "    stmt = client.prepare(\"{}\")", sql);
        let _ = writeln!(out, "    results = stmt.execute({})", execute_args);
        let _ = writeln!(out, "    _index = {{}}");
        let _ = writeln!(out, "    _entries = []");
        let _ = writeln!(out, "    results.each do |row|");
        let _ = writeln!(out, "      key = row[\"{}\"]{}", key_column, key_coercion);
        let _ = writeln!(out, "      unless _index.key?(key)");
        let _ = writeln!(out, "        _index[key] = _entries.size");
        let _ = writeln!(out, "        _entries << {{");
        for col in parent_columns {
            let coercion = ruby_coercion(&col.neutral_type, &self.manifest);
            if col.nullable && !coercion.is_empty() {
                let _ = writeln!(
                    out,
                    "          {}: row[\"{}\"]&.then {{ |v| v{} }},",
                    col.field_name, col.name, coercion
                );
            } else {
                let _ = writeln!(out, "          {}: row[\"{}\"]{},", col.field_name, col.name, coercion);
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
            let coercion = ruby_coercion(&col.neutral_type, &self.manifest);
            if col.nullable && !coercion.is_empty() {
                let _ = writeln!(
                    out,
                    "        {}: row[\"{}\"]&.then {{ |v| v{} }},",
                    col.field_name, col.name, coercion
                );
            } else {
                let _ = writeln!(out, "        {}: row[\"{}\"]{},", col.field_name, col.name, coercion);
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
    fn test_grouped_ruby_mysql2_structs() {
        let backend = RubyMysql2Backend::new("mysql").unwrap();
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
    fn test_grouped_ruby_mysql2_query_fn() {
        let backend = RubyMysql2Backend::new("mysql").unwrap();
        let query = make_grouped_query();
        let result = crate::generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();
        assert!(
            query_fn.contains("def self.get_users_with_orders(client)"),
            "missing fn; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("client.prepare("),
            "must use prepare; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("stmt.execute("),
            "must use stmt.execute; got:\n{query_fn}"
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
