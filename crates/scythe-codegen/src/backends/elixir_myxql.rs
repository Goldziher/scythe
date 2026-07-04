use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case, to_snake_case,
};
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/elixir-myxql.toml");

pub struct ElixirMyxqlBackend {
    manifest: BackendManifest,
}

impl ElixirMyxqlBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "mysql" | "mariadb" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("elixir-myxql only supports MySQL, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::load_or_default_manifest("backends/elixir-myxql/manifest.toml", DEFAULT_MANIFEST_TOML)?;
        Ok(Self { manifest })
    }
}

impl CodegenBackend for ElixirMyxqlBackend {
    fn name(&self) -> &str {
        "elixir-myxql"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["mysql", "mariadb"]
    }

    fn query_class_header(&self) -> String {
        "defmodule Scythe.Queries do".to_string()
    }

    fn file_footer(&self) -> String {
        "end".to_string()
    }

    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "defmodule {} do", struct_name);
        let _ = writeln!(out, "  @moduledoc \"Row type for {} queries.\"", query_name);
        let _ = writeln!(out);

        // Generate typespec
        let _ = writeln!(out, "  @type t :: %__MODULE__{{");
        for (i, c) in columns.iter().enumerate() {
            let sep = if i + 1 < columns.len() { "," } else { "" };
            let type_ref = if c.neutral_type.starts_with("enum::") {
                format!("{}.t()", c.full_type)
            } else {
                c.full_type.clone()
            };
            let _ = writeln!(out, "    {}: {}{}", c.field_name, type_ref, sep);
        }
        let _ = writeln!(out, "  }}");

        // Generate defstruct
        let fields = columns
            .iter()
            .map(|c| format!(":{}", c.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  defstruct [{}]", fields);
        let _ = write!(out, "end");
        Ok(out)
    }

    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError> {
        let name = to_pascal_case(table_name);
        self.generate_row_struct(&name, columns)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let sql = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let mut out = String::new();

        // Parameter list
        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        // Build the params list for MyXQL.query
        let param_args = if params.is_empty() {
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

        // Build @spec
        let param_specs = if params.is_empty() {
            String::new()
        } else {
            let specs: Vec<String> = params.iter().map(|p| p.full_type.clone()).collect();
            format!(", {}", specs.join(", "))
        };
        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(
                    out,
                    "@spec {}(MyXQL.conn(){}) :: {{:ok, %{}{{}}}} | {{:error, term()}}",
                    func_name, param_specs, struct_name
                );
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "@spec {}(MyXQL.conn(){}) :: {{:ok, [%{}{{}}]}} | {{:error, term()}}",
                    func_name, param_specs, struct_name
                );
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let _ = writeln!(
                    out,
                    "@spec {}(MyXQL.conn(), list()) :: :ok | {{:error, term()}}",
                    batch_fn_name
                );
                let _ = writeln!(out, "def {}(conn, items) do", batch_fn_name);
                let _ = writeln!(out, "  Enum.reduce_while(items, :ok, fn item, :ok ->");
                if params.len() > 1 {
                    let _ = writeln!(out, "    case MyXQL.query(conn, \"{}\", Tuple.to_list(item)) do", sql);
                } else if params.len() == 1 {
                    let _ = writeln!(out, "    case MyXQL.query(conn, \"{}\", [item]) do", sql);
                } else {
                    let _ = writeln!(out, "    case MyXQL.query(conn, \"{}\", []) do", sql);
                }
                let _ = writeln!(out, "      {{:ok, _}} -> {{:cont, :ok}}");
                let _ = writeln!(out, "      {{:error, err}} -> {{:halt, {{:error, err}}}}");
                let _ = writeln!(out, "    end");
                let _ = writeln!(out, "  end)");
                let _ = write!(out, "end");
                return Ok(out);
            }
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "@spec {}(MyXQL.conn(){}) :: :ok | {{:error, term()}}",
                    func_name, param_specs
                );
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "@spec {}(MyXQL.conn(){}) :: {{:ok, non_neg_integer()}} | {{:error, term()}}",
                    func_name, param_specs
                );
            }
            QueryCommand::Grouped => {
                unreachable!("grouped queries are routed to generate_grouped_query_fn")
            }
        }
        let _ = writeln!(out, "def {}(conn{}{}) do", func_name, sep, param_list);

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(out, "  case MyXQL.query(conn, \"{}\", {}) do", sql, param_args);
                let _ = writeln!(out, "    {{:ok, %MyXQL.Result{{rows: [row]}}}} ->");

                let field_vars = columns
                    .iter()
                    .map(|c| c.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "      [{}] = row", field_vars);

                let struct_fields = columns
                    .iter()
                    .map(|c| format!("{}: {}", c.field_name, c.field_name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "      {{:ok, %{}{{{}}}}}", struct_name, struct_fields);
                let _ = writeln!(out, "    {{:ok, %MyXQL.Result{{rows: []}}}} -> {{:error, :not_found}}");
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "  case MyXQL.query(conn, \"{}\", {}) do", sql, param_args);
                let _ = writeln!(out, "    {{:ok, %MyXQL.Result{{rows: rows}}}} ->");

                let field_vars = columns
                    .iter()
                    .map(|c| c.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let struct_fields = columns
                    .iter()
                    .map(|c| format!("{}: {}", c.field_name, c.field_name))
                    .collect::<Vec<_>>()
                    .join(", ");

                let _ = writeln!(out, "      results = Enum.map(rows, fn row ->");
                let _ = writeln!(out, "        [{}] = row", field_vars);
                let _ = writeln!(out, "        %{}{{{}}}", struct_name, struct_fields);
                let _ = writeln!(out, "      end)");
                let _ = writeln!(out, "      {{:ok, results}}");
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::Exec => {
                let _ = writeln!(out, "  case MyXQL.query(conn, \"{}\", {}) do", sql, param_args);
                let _ = writeln!(out, "    {{:ok, _}} -> :ok");
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "  case MyXQL.query(conn, \"{}\", {}) do", sql, param_args);
                let _ = writeln!(out, "    {{:ok, %MyXQL.Result{{num_rows: n}}}} -> {{:ok, n}}");
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::Batch | QueryCommand::Grouped => unreachable!(),
        }

        let _ = write!(out, "end");
        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "defmodule {} do", type_name);
        let _ = writeln!(out, "  @moduledoc \"Enum type for {}.\"", enum_info.sql_name);
        let _ = writeln!(out);
        let _ = writeln!(out, "  @type t :: String.t()");
        let _ = writeln!(out);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "  @spec {}() :: String.t()", to_snake_case(&variant));
            let _ = writeln!(out, "  def {}(), do: \"{}\"", to_snake_case(&variant), value);
        }
        // values/0 function
        let values_list = enum_info
            .values
            .iter()
            .map(|v| format!("\"{}\"", v))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  @spec values() :: [String.t()]");
        let _ = writeln!(out, "  def values, do: [{}]", values_list);
        let _ = write!(out, "end");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = writeln!(out, "defmodule {} do", name);
        let _ = writeln!(out, "  @moduledoc \"Composite type for {}.\"", composite.sql_name);
        let _ = writeln!(out);
        // Generate @type definition
        if composite.fields.is_empty() {
            let _ = writeln!(out, "  @type t :: %__MODULE__{{}}");
        } else {
            let _ = writeln!(out, "  @type t :: %__MODULE__{{");
            for (i, f) in composite.fields.iter().enumerate() {
                let sep = if i + 1 < composite.fields.len() { "," } else { "" };
                let _ = writeln!(out, "    {}: term(){}", to_snake_case(&f.name), sep);
            }
            let _ = writeln!(out, "  }}");
        }
        let _ = writeln!(out);
        if composite.fields.is_empty() {
            let _ = writeln!(out, "  defstruct []");
        } else {
            let fields = composite
                .fields
                .iter()
                .map(|f| format!(":{}", to_snake_case(&f.name)))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "  defstruct [{}]", fields);
        }
        let _ = write!(out, "end");
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
        let mut out = String::new();

        // Child struct — defined first so the parent's @type can reference it.
        let _ = writeln!(out, "defmodule {} do", child_struct_name);
        let _ = writeln!(out, "  @moduledoc \"Child row type for grouped query.\"");
        let _ = writeln!(out);
        let _ = writeln!(out, "  @type t :: %__MODULE__{{");
        for (i, c) in child_columns.iter().enumerate() {
            let sep = if i + 1 < child_columns.len() { "," } else { "" };
            let type_ref = if c.neutral_type.starts_with("enum::") {
                format!("{}.t()", c.full_type)
            } else {
                c.full_type.clone()
            };
            let _ = writeln!(out, "    {}: {}{}", c.field_name, type_ref, sep);
        }
        let _ = writeln!(out, "  }}");
        let child_fields = child_columns
            .iter()
            .map(|c| format!(":{}", c.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  defstruct [{}]", child_fields);
        let _ = writeln!(out, "end");
        let _ = writeln!(out);

        // Parent struct — all parent columns plus a `children` list.
        let _ = writeln!(out, "defmodule {} do", parent_struct_name);
        let _ = writeln!(out, "  @moduledoc \"Parent row type for grouped query.\"");
        let _ = writeln!(out);
        let _ = writeln!(out, "  @type t :: %__MODULE__{{");
        for c in parent_columns.iter() {
            let type_ref = if c.neutral_type.starts_with("enum::") {
                format!("{}.t()", c.full_type)
            } else {
                c.full_type.clone()
            };
            let _ = writeln!(out, "    {}: {},", c.field_name, type_ref);
        }
        let _ = writeln!(out, "    children: [{}.t()]", child_struct_name);
        let _ = writeln!(out, "  }}");
        let parent_fields = parent_columns
            .iter()
            .map(|c| format!(":{}", c.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  defstruct [{}, :children]", parent_fields);
        let _ = write!(out, "end");
        Ok(out)
    }

    fn generate_grouped_query_fn(
        &self,
        request: &crate::backend_trait::GroupedQueryFn<'_>,
    ) -> Result<String, ScytheError> {
        let analyzed = request.analyzed;
        let parent_struct_name = request.parent_struct_name;
        let child_struct_name = request.child_struct_name;
        let all_columns = request.all_columns;
        let parent_columns = request.parent_columns;
        let child_columns = request.child_columns;
        let params = request.params;
        let key_column = request.key_column;

        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let key_field = to_snake_case(key_column);
        let sql = super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params);
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };
        let param_args = if params.is_empty() {
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
        let param_specs = if params.is_empty() {
            String::new()
        } else {
            format!(
                ", {}",
                params
                    .iter()
                    .map(|p| p.full_type.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let all_field_vars = all_columns
            .iter()
            .map(|c| c.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let child_struct_fields = child_columns
            .iter()
            .map(|c| format!("{}: {}", c.field_name, c.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let parent_struct_fields = parent_columns
            .iter()
            .map(|c| format!("{}: {}", c.field_name, c.field_name))
            .collect::<Vec<_>>()
            .join(", ");

        let child_init = format!("%{}{{{}}}", child_struct_name, child_struct_fields);
        let parent_init = format!("%{}{{{}, children: [child]}}", parent_struct_name, parent_struct_fields);

        let _ = writeln!(
            out,
            "@spec {}(MyXQL.conn(){}) :: {{:ok, [%{}{{}}]}} | {{:error, term()}}",
            func_name, param_specs, parent_struct_name
        );
        let _ = writeln!(out, "def {}(conn{}{}) do", func_name, sep, param_list);
        let _ = writeln!(out, "  case MyXQL.query(conn, \"{}\", {}) do", sql, param_args);
        let _ = writeln!(out, "    {{:ok, %MyXQL.Result{{rows: rows}}}} ->");
        let _ = writeln!(
            out,
            "      {{order, acc}} = Enum.reduce(rows, {{[], %{{}}}}, fn row, {{order, acc}} ->"
        );
        let _ = writeln!(out, "        [{}] = row", all_field_vars);
        let _ = writeln!(out, "        child = {}", child_init);
        let _ = writeln!(out, "        if Map.has_key?(acc, {}) do", key_field);
        let _ = writeln!(
            out,
            "          {{order, Map.update!(acc, {}, fn p -> %{{p | children: [child | p.children]}} end)}}",
            key_field
        );
        let _ = writeln!(out, "        else");
        let _ = writeln!(out, "          parent = {}", parent_init);
        let _ = writeln!(
            out,
            "          {{[{} | order], Map.put(acc, {}, parent)}}",
            key_field, key_field
        );
        let _ = writeln!(out, "        end");
        let _ = writeln!(out, "      end)");
        let _ = writeln!(out, "      result = Enum.map(Enum.reverse(order), fn key ->");
        let _ = writeln!(out, "        parent = Map.fetch!(acc, key)");
        let _ = writeln!(out, "        %{{parent | children: Enum.reverse(parent.children)}}");
        let _ = writeln!(out, "      end)");
        let _ = writeln!(out, "      {{:ok, result}}");
        let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
        let _ = writeln!(out, "  end");
        let _ = write!(out, "end");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
    use scythe_core::parser::QueryCommand;

    use super::ElixirMyxqlBackend;
    use crate::generate_with_backend;

    fn make_grouped_query() -> AnalyzedQuery {
        let parent_cols = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
            },
            AnalyzedColumn {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            },
            AnalyzedColumn {
                name: "email".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
            },
        ];
        let child_cols = vec![
            AnalyzedColumn {
                name: "order_id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
            },
            AnalyzedColumn {
                name: "total".to_string(),
                neutral_type: "decimal".to_string(),
                nullable: true,
            },
            AnalyzedColumn {
                name: "order_date".to_string(),
                neutral_type: "datetime".to_string(),
                nullable: false,
            },
        ];
        let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
        AnalyzedQuery {
            name: "GetUsersWithOrders".to_string(),
            command: QueryCommand::Grouped,
            sql: "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\n\
                  FROM users u JOIN orders o ON o.user_id = u.id"
                .to_string(),
            columns: all_cols,
            params: vec![],
            deprecated: None,
            source_table: None,
            composites: vec![],
            enums: vec![],
            optional_params: vec![],
            group_by: Some(GroupByConfig {
                table: "users".to_string(),
                key_column: "id".to_string(),
                parent_columns: parent_cols,
                child_columns: child_cols,
            }),
            custom: vec![],
        }
    }

    #[test]
    fn test_grouped_myxql_structs() {
        let backend = ElixirMyxqlBackend::new("mysql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &backend).unwrap();

        let row_struct = result.row_struct.as_deref().unwrap();

        assert!(
            row_struct.contains("defmodule GetUsersWithOrdersChildRow do"),
            "missing child defmodule; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("order_id: integer()"),
            "child struct missing order_id; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("total: Decimal.t() | nil"),
            "child struct missing nullable total; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("defmodule GetUsersWithOrdersRow do"),
            "missing parent defmodule; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("children: [GetUsersWithOrdersChildRow.t()]"),
            "parent struct missing children field; got:\n{row_struct}"
        );
        // Child must appear before parent.
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow do").unwrap();
        let parent_pos = row_struct.find("GetUsersWithOrdersRow do").unwrap();
        assert!(child_pos < parent_pos, "child struct must appear before parent struct");
    }

    #[test]
    fn test_grouped_myxql_query_fn() {
        let backend = ElixirMyxqlBackend::new("mysql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("def get_users_with_orders(conn) do"),
            "missing fn head; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("MyXQL.query(conn,"),
            "fn must use MyXQL.query; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Enum.reduce(rows,"),
            "fn must use Enum.reduce for fold; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Map.update!"),
            "fn must use Map.update! to append children; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("{:ok, result}"),
            "fn must return {{:ok, result}}; got:\n{query_fn}"
        );
    }
}
