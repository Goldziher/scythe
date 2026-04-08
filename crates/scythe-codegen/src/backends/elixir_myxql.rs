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
        let manifest_path = Path::new("backends/elixir-myxql/manifest.toml");
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

impl CodegenBackend for ElixirMyxqlBackend {
    fn name(&self) -> &str {
        "elixir-myxql"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn supported_engines(&self) -> &[&str] {
        &["mysql"]
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
            QueryCommand::One => {
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
                    let _ = writeln!(
                        out,
                        "    case MyXQL.query(conn, \"{}\", Tuple.to_list(item)) do",
                        sql
                    );
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
                unreachable!("Grouped is rewritten to Many before codegen")
            }
        }
        let _ = writeln!(out, "def {}(conn{}{}) do", func_name, sep, param_list);

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(
                    out,
                    "  case MyXQL.query(conn, \"{}\", {}) do",
                    sql, param_args
                );
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
                let _ = writeln!(
                    out,
                    "    {{:ok, %MyXQL.Result{{rows: []}}}} -> {{:error, :not_found}}"
                );
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::Many => {
                let _ = writeln!(
                    out,
                    "  case MyXQL.query(conn, \"{}\", {}) do",
                    sql, param_args
                );
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
                let _ = writeln!(
                    out,
                    "  case MyXQL.query(conn, \"{}\", {}) do",
                    sql, param_args
                );
                let _ = writeln!(out, "    {{:ok, _}} -> :ok");
                let _ = writeln!(out, "    {{:error, err}} -> {{:error, err}}");
                let _ = writeln!(out, "  end");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "  case MyXQL.query(conn, \"{}\", {}) do",
                    sql, param_args
                );
                let _ = writeln!(
                    out,
                    "    {{:ok, %MyXQL.Result{{num_rows: n}}}} -> {{:ok, n}}"
                );
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
