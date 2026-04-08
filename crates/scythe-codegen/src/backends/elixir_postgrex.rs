use std::fmt::Write;
use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case, to_snake_case,
};

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/elixir-postgrex.toml");

pub struct ElixirPostgrexBackend {
    manifest: BackendManifest,
}

impl ElixirPostgrexBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "postgresql" | "postgres" | "pg" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!(
                        "elixir-postgrex only supports PostgreSQL, got engine '{}'",
                        engine
                    ),
                ));
            }
        }
        let manifest_path = Path::new("backends/elixir-postgrex/manifest.toml");
        let manifest = if manifest_path.exists() {
            load_manifest(manifest_path)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        } else {
            toml::from_str(DEFAULT_MANIFEST_TOML)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        };
        Ok(Self { manifest })
    }
}

impl CodegenBackend for ElixirPostgrexBackend {
    fn name(&self) -> &str {
        "elixir-postgrex"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
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

    fn generate_model_struct(
        &self,
        table_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
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
        let sql = super::clean_sql_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        );
        let mut out = String::new();

        // Parameter list
        let param_list = params
            .iter()
            .map(|p| p.field_name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        // Build the params array for Postgrex.query
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
            QueryCommand::One => {
                let _ = writeln!(
                    out,
                    "@spec {}(Postgrex.conn(){}) :: {{:ok, %{}{{}}}} | {{:error, term()}}",
                    func_name, param_specs, struct_name
                );
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "@spec {}(Postgrex.conn(){}) :: {{:ok, [%{}{{}}]}} | {{:error, term()}}",
                    func_name, param_specs, struct_name
                );
            }
            QueryCommand::Batch => {
                let batch_fn_name = format!("{}_batch", func_name);
                let _ = writeln!(
                    out,
                    "@spec {}(Postgrex.conn(), list()) :: :ok | {{:error, term()}}",
                    batch_fn_name
                );
                let _ = writeln!(out, "def {}(conn, items) do", batch_fn_name);
                let _ = writeln!(out, "  Postgrex.transaction(conn, fn tx_conn ->");
                let _ = writeln!(out, "    Enum.each(items, fn item ->");
                if params.len() > 1 {
                    let _ = writeln!(
                        out,
                        "      Postgrex.query!(tx_conn, \"{}\", Tuple.to_list(item))",
                        sql
                    );
                } else if params.len() == 1 {
                    let _ = writeln!(out, "      Postgrex.query!(tx_conn, \"{}\", [item])", sql);
                } else {
                    let _ = writeln!(out, "      Postgrex.query!(tx_conn, \"{}\", [])", sql);
                }
                let _ = writeln!(out, "    end)");
                let _ = writeln!(out, "  end)");
                let _ = write!(out, "end");
                return Ok(out);
            }
            QueryCommand::Exec => {
                let _ = writeln!(
                    out,
                    "@spec {}(Postgrex.conn(){}) :: :ok | {{:error, term()}}",
                    func_name, param_specs
                );
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "@spec {}(Postgrex.conn(){}) :: {{:ok, non_neg_integer()}} | {{:error, term()}}",
                    func_name, param_specs
                );
            }
            QueryCommand::Grouped => {
                unreachable!("Grouped is rewritten to Many before codegen")
            }
        }
        let _ = writeln!(out, "def {}(conn{}{}) do", func_name, sep, param_list);

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(
                    out,
                    "  case Postgrex.query(conn, \"{}\", {}) do",
                    sql, param_args
                );
                let _ = writeln!(out, "    {{:ok, %{{rows: [row]}}}} ->");

                // Destructure row
                let field_vars = columns
                    .iter()
                    .map(|c| c.field_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(out, "      [{}] = row", field_vars);

                // Build struct
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
                let _ = writeln!(
                    out,
                    "  case Postgrex.query(conn, \"{}\", {}) do",
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
                let _ = writeln!(
                    out,
                    "  case Postgrex.query(conn, \"{}\", {}) do",
                    sql, param_args
                );
                let _ = writeln!(out, "    {{:ok, _}} -> :ok");
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "  case Postgrex.query(conn, \"{}\", {}) do",
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
        let _ = writeln!(
            out,
            "  @moduledoc \"Enum type for {}.\"",
            enum_info.sql_name
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "  @type t :: String.t()");
        let _ = writeln!(out);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "  @spec {}() :: String.t()", to_snake_case(&variant));
            let _ = writeln!(
                out,
                "  def {}(), do: \"{}\"",
                to_snake_case(&variant),
                value
            );
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
        let _ = writeln!(
            out,
            "  @moduledoc \"Composite type for {}.\"",
            composite.sql_name
        );
        let _ = writeln!(out);
        // Generate @type definition
        if composite.fields.is_empty() {
            let _ = writeln!(out, "  @type t :: %__MODULE__{{}}");
        } else {
            let _ = writeln!(out, "  @type t :: %__MODULE__{{");
            for (i, f) in composite.fields.iter().enumerate() {
                let sep = if i + 1 < composite.fields.len() {
                    ","
                } else {
                    ""
                };
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
}
