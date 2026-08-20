use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{composite_type_name, enum_type_name, enum_variant_name, fn_name, to_snake_case};
use std::fmt::Write;

use scythe_core::analyzer::{AnalyzedQuery, CompositeFieldInfo, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

fn postgrex_param_expr(param: &ResolvedParam, raw: &str) -> String {
    if param.neutral_type.starts_with("composite::") {
        format!(
            "if(is_nil({raw}), do: nil, else: {param_type}.to_tuple({raw}))",
            param_type = param.lang_type
        )
    } else {
        raw.to_string()
    }
}

/// Board #219: `Ecto.Adapters.SQL.query`'s raw-SQL path runs through the same Postgrex binary
/// protocol as `elixir-postgrex` -- see the identical note there for what was verified live
/// against PostgreSQL 16. An unregistered composite column decodes to a bare positional tuple,
/// never the generated `%{Struct}{}`, so before this fix the tuple reached the row struct
/// untouched. `{Struct}.from_tuple` (emitted by `generate_composite_def`) converts it and is
/// already nil-safe.
fn elixir_composite_column_expr(col: &ResolvedColumn, var: &str) -> Option<String> {
    col.neutral_type
        .starts_with("composite::")
        .then(|| format!("{}.from_tuple({})", col.lang_type, var))
}

/// Build the `field: value` expression for one column already bound to a same-named local
/// (the destructured `[field_name] = row` variable both `generate_query_fn` and
/// `generate_grouped_query_fn` produce), routing a composite column through
/// [`elixir_composite_column_expr`] and passing every other column's variable straight through.
fn elixir_struct_field_assignment(col: &ResolvedColumn) -> String {
    let expr = elixir_composite_column_expr(col, &col.field_name).unwrap_or_else(|| col.field_name.clone());
    format!("{}: {}", col.field_name, expr)
}

/// The Elixir expression converting one composite field's already-decoded Postgrex value
/// (`var`, the positional tuple-destructured variable) into the field's declared value. See
/// `elixir_postgrex.rs`'s identical helper for why every non-composite field passes through
/// unchanged.
fn elixir_composite_field_from_tuple(field: &CompositeFieldInfo, var: &str, manifest: &BackendManifest) -> String {
    if let Some(sql_name) = field.neutral_type.strip_prefix("composite::") {
        return format!(
            "{}.from_tuple({})",
            composite_type_name(sql_name, &manifest.naming),
            var
        );
    }
    var.to_string()
}

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/elixir-ecto.toml");

pub struct ElixirEctoBackend {
    manifest: BackendManifest,
}

impl ElixirEctoBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "postgresql" | "postgres" | "pg" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InvalidConfig,
                    format!("elixir-ecto only supports PostgreSQL, got engine '{}'", engine),
                ));
            }
        }
        let manifest = super::parse_manifest(DEFAULT_MANIFEST_TOML)?;
        Ok(Self { manifest })
    }
}

impl CodegenBackend for ElixirEctoBackend {
    fn name(&self) -> &str {
        "elixir-ecto"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn manifest_mut(&mut self) -> &mut scythe_backend::manifest::BackendManifest {
        &mut self.manifest
    }

    // ~keep `query_class_header`, not `file_header`: `query_class_header`
    // wraps only the query functions, leaving row/enum/composite defmodules
    // top-level (matching every other Elixir driver backend, e.g.
    // `ElixirPostgrexBackend`). The previous `file_header` override wrapped
    // *everything*, including struct defmodules, producing
    // `Scythe.Queries.ListUsersRow` here against `ListUsersRow` in postgrex
    // -- a silent, undocumented API difference between two backends meant to
    // be interchangeable (#202).
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
            let type_ref = if c.neutral_type.starts_with("enum::") {
                format!("{}.t()", c.full_type)
            } else {
                c.full_type.clone()
            };
            let _ = writeln!(out, "    {}: {}{}", c.field_name, type_ref, sep);
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
        let sql = crate::sql_literal::escape_elixir_double_quoted(&super::clean_sql_with_optional(
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

        let param_args = if params.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                params
                    .iter()
                    .map(|p| postgrex_param_expr(p, &p.field_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let param_specs = if params.is_empty() {
            String::new()
        } else {
            let specs: Vec<String> = params
                .iter()
                .map(super::elixir_common::elixir_param_spec_type)
                .collect();
            format!(", {}", specs.join(", "))
        };
        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                // ~keep :one keeps erroring on a missing row ({:error, :not_found});
                // :opt returns it as an absent value instead of an error (#192).
                let return_type = if matches!(analyzed.command, QueryCommand::Opt) {
                    format!("{{:ok, %{}{{}} | nil}} | {{:error, term()}}", struct_name)
                } else {
                    format!(
                        "{{:ok, %{}{{}}}} | {{:error, :not_found}} | {{:error, term()}}",
                        struct_name
                    )
                };
                let _ = writeln!(
                    out,
                    "@spec {}(Ecto.Repo.t(){}) :: {}",
                    func_name, param_specs, return_type
                );
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "@spec {}(Ecto.Repo.t(){}) :: {{:ok, [%{}{{}}]}} | {{:error, term()}}",
                    func_name, param_specs, struct_name
                );
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let _ = writeln!(
                    out,
                    "@spec {}(Ecto.Repo.t(), list()) :: :ok | {{:error, term()}}",
                    batch_fn_name
                );
                let _ = writeln!(out, "def {}(repo, items) do", batch_fn_name);
                let _ = writeln!(out, "  repo.transaction(fn ->");
                let _ = writeln!(out, "    Enum.each(items, fn item ->");
                if params.len() > 1 {
                    let item_args = params
                        .iter()
                        .enumerate()
                        .map(|(index, param)| postgrex_param_expr(param, &format!("elem(item, {index})")))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(
                        out,
                        "      case Ecto.Adapters.SQL.query(repo, \"{}\", [{}], []) do",
                        sql, item_args
                    );
                } else if params.len() == 1 {
                    let item = postgrex_param_expr(&params[0], "item");
                    let _ = writeln!(
                        out,
                        "      case Ecto.Adapters.SQL.query(repo, \"{}\", [{}], []) do",
                        sql, item
                    );
                } else {
                    let _ = writeln!(out, "      case Ecto.Adapters.SQL.query(repo, \"{}\", [], []) do", sql);
                }
                let _ = writeln!(out, "        {{:ok, _}} -> :ok");
                let _ = writeln!(out, "        {{:error, err}} -> repo.rollback(err)");
                let _ = writeln!(out, "      end");
                let _ = writeln!(out, "    end)");
                let _ = writeln!(out, "  end)");
                let _ = write!(out, "end");
                return Ok(out);
            }
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "@spec {}(Ecto.Repo.t(){}) :: :ok | {{:error, term()}}",
                    func_name, param_specs
                );
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "@spec {}(Ecto.Repo.t(){}) :: {{:ok, non_neg_integer()}} | {{:error, term()}}",
                    func_name, param_specs
                );
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped queries are routed through generate_grouped_query_fn")
            }
        }
        let _ = writeln!(out, "def {}(repo{}{}) do", func_name, sep, param_list);

        match &analyzed.command {
            QueryCommand::One | QueryCommand::Opt => {
                let _ = writeln!(
                    out,
                    "  case Ecto.Adapters.SQL.query(repo, \"{}\", {}, []) do",
                    sql, param_args
                );
                let _ = writeln!(out, "    {{:ok, %{{rows: [row | _]}}}} ->");

                let field_vars = columns
                    .iter()
                    .map(|c| c.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "      [{}] = row", field_vars);

                let struct_fields = columns
                    .iter()
                    .map(elixir_struct_field_assignment)
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "      {{:ok, %{}{{{}}}}}", struct_name, struct_fields);
                let not_found_arm = if matches!(analyzed.command, QueryCommand::Opt) {
                    "    {:ok, %{rows: []}} -> {:ok, nil}"
                } else {
                    "    {:ok, %{rows: []}} -> {:error, :not_found}"
                };
                let _ = writeln!(out, "{}", not_found_arm);
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "  case Ecto.Adapters.SQL.query(repo, \"{}\", {}, []) do",
                    sql, param_args
                );
                let _ = writeln!(out, "    {{:ok, %{{rows: rows}}}} ->");

                let field_vars = columns
                    .iter()
                    .map(|c| c.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let struct_fields = columns
                    .iter()
                    .map(elixir_struct_field_assignment)
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
                let _ = writeln!(
                    out,
                    "  case Ecto.Adapters.SQL.query(repo, \"{}\", {}, []) do",
                    sql, param_args
                );
                let _ = writeln!(out, "    {{:ok, _}} -> :ok");
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "  case Ecto.Adapters.SQL.query(repo, \"{}\", {}, []) do",
                    sql, param_args
                );
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
        let name = composite_type_name(&composite.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "defmodule {} do", name);
        let _ = writeln!(out, "  @moduledoc \"Composite type for {}.\"", composite.sql_name);
        let _ = writeln!(out);
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
            // ~keep board #219: a composite with zero fields cannot exist in PostgreSQL
            // (`CREATE TYPE ... AS ()` is rejected), so there is no reachable runtime tuple
            // that would need `from_tuple` here.
        } else {
            let fields = composite
                .fields
                .iter()
                .map(|f| format!(":{}", to_snake_case(&f.name)))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "  defstruct [{}]", fields);
            let _ = writeln!(out);
            // ~keep board #219: Postgrex decodes an unregistered composite column into a bare
            // positional tuple (every field already its natural Elixir type), never this
            // struct -- build it from that tuple here.
            let _ = writeln!(out, "  def from_tuple(nil), do: nil");
            let _ = writeln!(out);
            let field_vars: Vec<String> = composite
                .fields
                .iter()
                .map(|f| to_snake_case(&f.name).into_owned())
                .collect();
            let _ = writeln!(out, "  def from_tuple({{{}}}) do", field_vars.join(", "));
            let _ = writeln!(out, "    %__MODULE__{{");
            for (field, var) in composite.fields.iter().zip(&field_vars) {
                let expr = elixir_composite_field_from_tuple(field, var, &self.manifest);
                let _ = writeln!(out, "      {}: {},", to_snake_case(&field.name), expr);
            }
            let _ = writeln!(out, "    }}");
            let _ = writeln!(out, "  end");
            let tuple_values = composite
                .fields
                .iter()
                .map(|field| {
                    let field_name = to_snake_case(&field.name);
                    if field.neutral_type.starts_with("composite::") {
                        let nested_type = field.neutral_type.trim_start_matches("composite::");
                        let nested_name = composite_type_name(nested_type, &self.manifest.naming);
                        format!(
                            "if(is_nil(value.{field_name}), do: nil, else: {nested_name}.to_tuple(value.{field_name}))"
                        )
                    } else {
                        format!("value.{field_name}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let tuple_suffix = if composite.fields.len() == 1 { "," } else { "" };
            let _ = writeln!(out);
            let _ = writeln!(out, "  def to_tuple(%__MODULE__{{}} = value) do");
            let _ = writeln!(out, "    {{{tuple_values}{tuple_suffix}}}");
            let _ = writeln!(out, "  end");
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
        // ~keep an alias-qualified `@group_by` (e.g. `u.id`) is accepted by the
        // core parser but can resolve to zero parent columns; without this guard
        // the join above produces an empty string and this line becomes the
        // syntactically invalid `defstruct [, :children]` (#202).
        let defstruct_fields = if parent_fields.is_empty() {
            ":children".to_string()
        } else {
            format!("{}, :children", parent_fields)
        };
        let _ = writeln!(out, "  defstruct [{}]", defstruct_fields);
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
        let sql = crate::sql_literal::escape_elixir_double_quoted(&super::clean_sql_with_optional(
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
        let param_args = if params.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                params
                    .iter()
                    .map(|p| postgrex_param_expr(p, &p.field_name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let param_specs = if params.is_empty() {
            String::new()
        } else {
            let specs: Vec<String> = params
                .iter()
                .map(super::elixir_common::elixir_param_spec_type)
                .collect();
            format!(", {}", specs.join(", "))
        };

        let all_field_vars = all_columns
            .iter()
            .map(|c| c.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let child_struct_fields = child_columns
            .iter()
            .map(elixir_struct_field_assignment)
            .collect::<Vec<_>>()
            .join(", ");
        let parent_struct_fields = parent_columns
            .iter()
            .map(elixir_struct_field_assignment)
            .collect::<Vec<_>>()
            .join(", ");

        let child_init = format!("%{}{{{}}}", child_struct_name, child_struct_fields);
        let parent_init = format!("%{}{{{}, children: [child]}}", parent_struct_name, parent_struct_fields);

        let _ = writeln!(
            out,
            "@spec {}(Ecto.Repo.t(){}) :: {{:ok, [%{}{{}}]}} | {{:error, term()}}",
            func_name, param_specs, parent_struct_name
        );
        let _ = writeln!(out, "def {}(repo{}{}) do", func_name, sep, param_list);
        let _ = writeln!(
            out,
            "  case Ecto.Adapters.SQL.query(repo, \"{}\", {}, []) do",
            sql, param_args
        );
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

    use super::ElixirEctoBackend;
    use crate::generate_with_backend;

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
    fn test_grouped_ecto_structs() {
        let backend = ElixirEctoBackend::new("postgresql").unwrap();
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
    fn test_grouped_ecto_query_fn() {
        let backend = ElixirEctoBackend::new("postgresql").unwrap();
        let query = make_grouped_query();
        let result = generate_with_backend(&query, &backend).unwrap();

        let query_fn = result.query_fn.as_deref().unwrap();

        assert!(
            query_fn.contains("def get_users_with_orders(repo) do"),
            "missing fn head; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("Ecto.Adapters.SQL.query(repo,"),
            "fn must use Ecto.Adapters.SQL.query; got:\n{query_fn}"
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
