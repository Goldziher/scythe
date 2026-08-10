use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{enum_type_name, enum_variant_name, fn_name, to_pascal_case, to_snake_case};
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/elixir-tds.toml");

pub struct ElixirTdsBackend {
    manifest: BackendManifest,
}

/// Map a neutral SQL type to its `Tds.Parameter` type atom.
///
/// Only matches neutral types — the vocabulary produced by
/// `scythe_core::analyzer::type_conversion::sql_type_to_neutral`. That
/// function collapses both the MSSQL `DATETIME` and `DATETIME2` SQL types
/// into the single neutral type `"datetime"`, and `ResolvedParam` (unlike
/// `ResolvedColumn`) does not carry the raw `sql_type` a param was inferred
/// from, so a `"datetime2"` (or `"text"`, similarly collapsed to `"string"`)
/// arm here could never match live data. Do not add either back without
/// first plumbing the raw SQL type through `AnalyzedParam`/`ResolvedParam` —
/// see `ResolvedColumn::sql_type` for the equivalent that already exists for
/// columns.
fn tds_param_type_atom(neutral_type: &str) -> &'static str {
    match neutral_type {
        "bool" => ":boolean",
        "int16" | "int32" | "int64" => ":integer",
        "float32" | "float64" => ":float",
        "decimal" => ":decimal",
        "string" => ":string",
        "date" => ":date",
        "datetime" | "datetime_tz" => ":datetime",
        "uuid" => ":uuid",
        _ => ":string",
    }
}

fn format_tds_param_args(params: &[ResolvedParam]) -> String {
    if params.is_empty() {
        return "[]".to_string();
    }
    format!(
        "[{}]",
        params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                // ~keep tds's :boolean encoder (encode_binary_type) accepts integers or
                // bitstrings, not Elixir booleans, so coerce true/false to 1/0 on
                // the wire while keeping the public API boolean()-typed. `nil` is
                // falsy in Elixir too, so guard for it explicitly and pass it
                // through unchanged — otherwise a NULL boolean param would
                // silently encode as `0` (false) instead of SQL NULL.
                let value_expr = if p.neutral_type == "bool" {
                    format!(
                        "(if is_nil({0}), do: nil, else: (if {0}, do: 1, else: 0))",
                        p.field_name
                    )
                } else {
                    p.field_name.clone()
                };
                format!(
                    "%Tds.Parameter{{name: \"@{}\", value: {}, type: {}}}",
                    i + 1,
                    value_expr,
                    tds_param_type_atom(&p.neutral_type)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    )
}

impl ElixirTdsBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "mssql" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("elixir-tds only supports MSSQL, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self { manifest })
    }
}

/// Rewrite $1, $2, ... positional params to ? positional placeholders for TDS.
impl CodegenBackend for ElixirTdsBackend {
    fn name(&self) -> &str {
        "elixir-tds"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["mssql"]
    }

    fn query_class_header(&self) -> String {
        "defmodule Scythe.Queries do".to_string()
    }

    fn file_footer(&self) -> String {
        "end".to_string()
    }

    fn generate_struct_decl(
        &self,
        struct_name: &str,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let mut out = String::new();
        let _ = writeln!(out, "defmodule {} do", struct_name);
        let _ = writeln!(out, "  @moduledoc \"Row type for {} queries.\"", query_name);
        let _ = writeln!(out);

        let _ = writeln!(out, "  @type t :: %__MODULE__{{");
        for (i, c) in columns.iter().enumerate() {
            let sep = if i + 1 < columns.len() { "," } else { "" };
            let _ = writeln!(out, "    {}: {}{}", c.field_name, c.full_type, sep);
        }
        let _ = writeln!(out, "  }}");

        let fields = columns
            .iter()
            .map(|c| format!(":{}", c.field_name))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  defstruct [{}]", fields);
        let _ = write!(out, "end");
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
        let sql = crate::sql_literal::escape_elixir_double_quoted(&super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!("@{n}"),
        ));
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let param_args = format_tds_param_args(params);

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
                    "@spec {}(pid(){}) :: {{:ok, %{}{{}}}} | {{:error, :not_found}} | {{:error, term()}}",
                    func_name, param_specs, struct_name
                );
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "@spec {}(pid(){}) :: {{:ok, [%{}{{}}]}} | {{:error, term()}}",
                    func_name, param_specs, struct_name
                );
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let _ = writeln!(
                    out,
                    "@spec {}(pid(), list()) :: :ok | {{:error, term()}}",
                    batch_fn_name
                );
                let _ = writeln!(out, "def {}(conn, items) do", batch_fn_name);
                let _ = writeln!(out, "  Tds.transaction(conn, fn tx_conn ->");
                let _ = writeln!(out, "    Enum.each(items, fn item ->");
                if params.len() > 1 {
                    let _ = writeln!(out, "      Tds.query(tx_conn, \"{}\", Tuple.to_list(item))", sql);
                } else if params.len() == 1 {
                    let _ = writeln!(out, "      Tds.query(tx_conn, \"{}\", [item])", sql);
                } else {
                    let _ = writeln!(out, "      Tds.query(tx_conn, \"{}\", [])", sql);
                }
                let _ = writeln!(out, "    end)");
                let _ = writeln!(out, "  end)");
                let _ = write!(out, "end");
                return Ok(out);
            }
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "@spec {}(pid(){}) :: :ok | {{:error, term()}}",
                    func_name, param_specs
                );
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "@spec {}(pid(){}) :: {{:ok, non_neg_integer()}} | {{:error, term()}}",
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
                let _ = writeln!(out, "  case Tds.query(conn, \"{}\", {}) do", sql, param_args);
                let _ = writeln!(out, "    {{:ok, %{{rows: [row | _]}}}} ->");

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
                let _ = writeln!(out, "    {{:ok, %{{rows: []}}}} -> {{:error, :not_found}}");
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::Many => {
                let _ = writeln!(out, "  case Tds.query(conn, \"{}\", {}) do", sql, param_args);
                let _ = writeln!(out, "    {{:ok, %{{rows: rows}}}} ->");

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
                let _ = writeln!(out, "  case Tds.query(conn, \"{}\", {}) do", sql, param_args);
                let _ = writeln!(out, "    {{:ok, _}} -> :ok");
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(out, "  case Tds.query(conn, \"{}\", {}) do", sql, param_args);
                let _ = writeln!(out, "    {{:ok, %{{num_rows: n}}}} -> {{:ok, n}}");
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
        if composite.fields.is_empty() {
            let _ = writeln!(out, "  @type t :: %__MODULE__{{}}");
            let _ = writeln!(out);
            let _ = writeln!(out, "  defstruct []");
        } else {
            let _ = writeln!(out, "  @type t :: %__MODULE__{{");
            for (i, f) in composite.fields.iter().enumerate() {
                let sep = if i + 1 < composite.fields.len() { "," } else { "" };
                let _ = writeln!(out, "    {}: term(){}", to_snake_case(&f.name), sep);
            }
            let _ = writeln!(out, "  }}");
            let _ = writeln!(out);
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

        let _ = writeln!(out, "defmodule {} do", child_struct_name);
        let _ = writeln!(out, "  @moduledoc \"Child row type for grouped query.\"");
        let _ = writeln!(out);
        let _ = writeln!(out, "  @type t :: %__MODULE__{{");
        for (i, c) in child_columns.iter().enumerate() {
            let sep = if i + 1 < child_columns.len() { "," } else { "" };
            let _ = writeln!(out, "    {}: {}{}", c.field_name, c.full_type, sep);
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

        let _ = writeln!(out, "defmodule {} do", parent_struct_name);
        let _ = writeln!(out, "  @moduledoc \"Parent row type for grouped query.\"");
        let _ = writeln!(out);
        let _ = writeln!(out, "  @type t :: %__MODULE__{{");
        for c in parent_columns.iter() {
            let _ = writeln!(out, "    {}: {},", c.field_name, c.full_type);
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
        let sql = crate::sql_literal::escape_elixir_double_quoted(&super::rewrite_pg_placeholders(
            &super::clean_sql_with_optional(&analyzed.sql, &analyzed.optional_params, &analyzed.params),
            |n| format!("@{n}"),
        ));
        let mut out = String::new();

        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let param_args = format_tds_param_args(params);
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
            "@spec {}(pid(){}) :: {{:ok, [%{}{{}}]}} | {{:error, term()}}",
            func_name, param_specs, parent_struct_name
        );
        let _ = writeln!(out, "def {}(conn{}{}) do", func_name, sep, param_list);
        let _ = writeln!(out, "  case Tds.query(conn, \"{}\", {}) do", sql, param_args);
        let _ = writeln!(out, "    {{:ok, %{{rows: rows}}}} ->");
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

    use super::{ElixirTdsBackend, format_tds_param_args, tds_param_type_atom};
    use crate::backend_trait::ResolvedParam;
    use crate::generate_with_backend;

    fn param(field_name: &str, neutral_type: &str) -> ResolvedParam {
        ResolvedParam {
            name: field_name.to_string(),
            field_name: field_name.to_string(),
            lang_type: String::new(),
            full_type: String::new(),
            borrowed_type: String::new(),
            neutral_type: neutral_type.to_string(),
            nullable: false,
        }
    }

    #[test]
    fn test_tds_param_type_atom_every_neutral_type() {
        assert_eq!(tds_param_type_atom("bool"), ":boolean");
        assert_eq!(tds_param_type_atom("int16"), ":integer");
        assert_eq!(tds_param_type_atom("int32"), ":integer");
        assert_eq!(tds_param_type_atom("int64"), ":integer");
        assert_eq!(tds_param_type_atom("float32"), ":float");
        assert_eq!(tds_param_type_atom("float64"), ":float");
        assert_eq!(tds_param_type_atom("decimal"), ":decimal");
        assert_eq!(tds_param_type_atom("string"), ":string");
        assert_eq!(tds_param_type_atom("date"), ":date");
        assert_eq!(tds_param_type_atom("datetime"), ":datetime");
        assert_eq!(tds_param_type_atom("datetime_tz"), ":datetime");
        assert_eq!(tds_param_type_atom("uuid"), ":uuid");
    }

    #[test]
    fn test_tds_param_type_atom_unknown_falls_back_to_string() {
        // "datetime2" and "text" are SQL type names, not neutral types --
        // sql_type_to_neutral collapses both into "datetime"/"string" before
        // this function ever sees them, so they must fall through to the
        // default rather than getting their own (dead) match arm.
        assert_eq!(tds_param_type_atom("datetime2"), ":string");
        assert_eq!(tds_param_type_atom("text"), ":string");
        assert_eq!(tds_param_type_atom("totally_unknown"), ":string");
    }

    #[test]
    fn test_format_tds_param_args_empty() {
        assert_eq!(format_tds_param_args(&[]), "[]");
    }

    #[test]
    fn test_format_tds_param_args_boolean_guards_nil_before_coercing() {
        let params = vec![param("active", "bool")];
        let args = format_tds_param_args(&params);
        assert_eq!(
            args,
            "[%Tds.Parameter{name: \"@1\", value: (if is_nil(active), do: nil, else: (if active, do: 1, else: 0)), type: :boolean}]"
        );
    }

    #[test]
    fn test_format_tds_param_args_non_boolean_passes_value_through() {
        let params = vec![param("user_id", "int32")];
        let args = format_tds_param_args(&params);
        assert_eq!(args, "[%Tds.Parameter{name: \"@1\", value: user_id, type: :integer}]");
    }

    #[test]
    fn test_format_tds_param_args_datetime() {
        let params = vec![param("created_at", "datetime")];
        let args = format_tds_param_args(&params);
        assert_eq!(
            args,
            "[%Tds.Parameter{name: \"@1\", value: created_at, type: :datetime}]"
        );
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
            aq.sql = "SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.created_at AS order_date\n\
                  FROM users u JOIN orders o ON o.user_id = u.id"
                .to_string();
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
    fn test_grouped_tds_structs() {
        let backend = ElixirTdsBackend::new("mssql").unwrap();
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
        let child_pos = row_struct.find("GetUsersWithOrdersChildRow do").unwrap();
        let parent_pos = row_struct.find("GetUsersWithOrdersRow do").unwrap();
        assert!(child_pos < parent_pos, "child struct must appear before parent struct");
    }

    #[test]
    fn test_grouped_tds_query_fn() {
        let backend = ElixirTdsBackend::new("mssql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("def get_users_with_orders(conn) do"),
            "missing fn head; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Tds.query(conn,"),
            "fn must use Tds.query; got:\n{query_fn}"
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
