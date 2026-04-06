use std::fmt::Write;
use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_camel_case, to_pascal_case,
};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};
use crate::singularize;

const DEFAULT_MANIFEST_TOML: &str =
    include_str!("../../../../backends/typescript-pg/manifest.toml");

pub struct TypescriptPgBackend {
    manifest: BackendManifest,
}

impl TypescriptPgBackend {
    pub fn new() -> Result<Self, ScytheError> {
        let manifest_path = Path::new("backends/typescript-pg/manifest.toml");
        let manifest = if manifest_path.exists() {
            load_manifest(manifest_path)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        } else {
            toml::from_str(DEFAULT_MANIFEST_TOML)
                .map_err(|e| ScytheError::new(ErrorCode::InternalError, format!("manifest: {e}")))?
        };
        Ok(Self { manifest })
    }

    pub fn manifest(&self) -> &BackendManifest {
        &self.manifest
    }
}

/// Strip SQL comments, trailing semicolons, and excess whitespace.
fn clean_sql(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string()
}

impl CodegenBackend for TypescriptPgBackend {
    fn name(&self) -> &str {
        "typescript-pg"
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "export interface {} {{", struct_name);
        for col in columns {
            let _ = writeln!(out, "  {}: {};", col.field_name, col.full_type);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_model_struct(
        &self,
        table_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let singular = singularize(table_name);
        let name = to_pascal_case(&singular);
        self.generate_row_struct(&name, columns)
    }

    fn generate_query_fn(
        &self,
        analyzed: &AnalyzedQuery,
        struct_name: &str,
        _columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();

        // Build parameter list
        let param_list = params
            .iter()
            .map(|p| format!("{}: {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        // Clean SQL — pg uses $1, $2 positional params natively
        let sql = clean_sql(&analyzed.sql);

        // Build array of param values
        let param_array: String = if params.is_empty() {
            String::new()
        } else {
            let args: Vec<String> = params.iter().map(|p| p.field_name.clone()).collect();
            format!(", [{}]", args.join(", "))
        };

        match &analyzed.command {
            QueryCommand::One => {
                let _ = writeln!(
                    out,
                    "export async function {}(client: PoolClient{}{}): Promise<{} | undefined> {{",
                    func_name, sep, param_list, struct_name
                );
                let _ = writeln!(
                    out,
                    "  const {{ rows }} = await client.query<{}>(",
                    struct_name
                );
                let _ = writeln!(out, "    \"{}\"{}", sql, param_array);
                let _ = writeln!(out, "  );");
                let _ = writeln!(out, "  return rows[0];");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many | QueryCommand::Batch => {
                let _ = writeln!(
                    out,
                    "export async function {}(client: PoolClient{}{}): Promise<{}[]> {{",
                    func_name, sep, param_list, struct_name
                );
                let _ = writeln!(
                    out,
                    "  const {{ rows }} = await client.query<{}>(",
                    struct_name
                );
                let _ = writeln!(out, "    \"{}\"{}", sql, param_array);
                let _ = writeln!(out, "  );");
                let _ = writeln!(out, "  return rows;");
                let _ = write!(out, "}}");
            }
            QueryCommand::Exec | QueryCommand::ExecResult | QueryCommand::ExecRows => {
                let _ = writeln!(
                    out,
                    "export async function {}(client: PoolClient{}{}): Promise<void> {{",
                    func_name, sep, param_list
                );
                let _ = writeln!(out, "  await client.query(");
                let _ = writeln!(out, "    \"{}\"{}", sql, param_array);
                let _ = writeln!(out, "  );");
                let _ = write!(out, "}}");
            }
        }

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "export enum {} {{", type_name);
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(out, "  {} = \"{}\",", variant, value);
        }
        let _ = write!(out, "}}");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = writeln!(out, "export interface {} {{", name);
        if composite.fields.is_empty() {
            // empty interface
        } else {
            for field in &composite.fields {
                let ts_type = resolve_type(&field.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .map_err(|e| {
                        ScytheError::new(
                            ErrorCode::InternalError,
                            format!("composite field type error: {}", e),
                        )
                    })?;
                let _ = writeln!(out, "  {}: {};", to_camel_case(&field.name), ts_type);
            }
        }
        let _ = write!(out, "}}");
        Ok(out)
    }
}
