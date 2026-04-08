use std::fmt::Write;
use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case,
};
use scythe_backend::types::resolve_type;

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};
use scythe_core::parser::QueryCommand;

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../manifests/go-pgx.toml");

pub struct GoPgxBackend {
    manifest: BackendManifest,
}

impl GoPgxBackend {
    pub fn new(engine: &str) -> Result<Self, ScytheError> {
        match engine {
            "postgresql" | "postgres" | "pg" => {}
            _ => {
                return Err(ScytheError::new(
                    ErrorCode::InternalError,
                    format!("go-pgx only supports PostgreSQL, got engine '{}'", engine),
                ));
            }
        }
        let manifest_path = Path::new("backends/go-pgx/manifest.toml");
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

impl CodegenBackend for GoPgxBackend {
    fn name(&self) -> &str {
        "go-pgx"
    }

    fn manifest(&self) -> &scythe_backend::manifest::BackendManifest {
        &self.manifest
    }

    fn file_header(&self) -> String {
        "package queries\n\nimport (\n\t\"context\"\n\t\"time\"\n\n\t\"github.com/jackc/pgx/v5/pgxpool\"\n\t\"github.com/shopspring/decimal\"\n)\n"
            .to_string()
    }

    fn generate_row_struct(
        &self,
        query_name: &str,
        columns: &[ResolvedColumn],
    ) -> Result<String, ScytheError> {
        let struct_name = row_struct_name(query_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "type {} struct {{", struct_name);
        for col in columns {
            let field = to_pascal_case(&col.field_name);
            let json_tag = &col.field_name;
            let _ = writeln!(out, "\t{} {} `json:\"{}\"`", field, col.full_type, json_tag);
        }
        let _ = write!(out, "}}");
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
        let sql = super::clean_sql_oneline_with_optional(
            &analyzed.sql,
            &analyzed.optional_params,
            &analyzed.params,
        );

        let param_list = params
            .iter()
            .map(|p| {
                let field = to_pascal_case(&p.field_name);
                format!("{} {}", field, p.full_type)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };

        let args = params
            .iter()
            .map(|p| to_pascal_case(&p.field_name).into_owned())
            .collect::<Vec<_>>();

        let mut out = String::new();

        match &analyzed.command {
            QueryCommand::Exec => {
                // :exec - returns error only
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *pgxpool.Pool{}{}) error {{",
                    func_name, sep, param_list
                );
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                let _ = writeln!(out, "\t_, err := db.Exec(ctx, \"{}\"{})", sql, args_str);
                let _ = writeln!(out, "\treturn err");
                let _ = write!(out, "}}");
            }
            QueryCommand::ExecResult | QueryCommand::ExecRows => {
                // :exec_rows - returns affected row count
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *pgxpool.Pool{}{}) (int64, error) {{",
                    func_name, sep, param_list
                );
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                let _ = writeln!(
                    out,
                    "\tresult, err := db.Exec(ctx, \"{}\"{})",
                    sql, args_str
                );
                let _ = writeln!(out, "\tif err != nil {{");
                let _ = writeln!(out, "\t\treturn 0, err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn result.RowsAffected(), nil");
                let _ = write!(out, "}}");
            }
            QueryCommand::One => {
                // :one - returns single struct
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *pgxpool.Pool{}{}) ({}, error) {{",
                    func_name, sep, param_list, struct_name
                );
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                let _ = writeln!(out, "\trow := db.QueryRow(ctx, \"{}\"{})", sql, args_str);
                let _ = writeln!(out, "\tvar r {}", struct_name);
                let scan_fields: Vec<String> = columns
                    .iter()
                    .map(|c| format!("&r.{}", to_pascal_case(&c.field_name)))
                    .collect();
                let _ = writeln!(out, "\terr := row.Scan({})", scan_fields.join(", "));
                let _ = writeln!(out, "\treturn r, err");
                let _ = write!(out, "}}");
            }
            QueryCommand::Many | QueryCommand::Batch => {
                // :many - returns slice
                let _ = writeln!(
                    out,
                    "func {}(ctx context.Context, db *pgxpool.Pool{}{}) ([]{}, error) {{",
                    func_name, sep, param_list, struct_name
                );
                let args_str = if args.is_empty() {
                    String::new()
                } else {
                    format!(", {}", args.join(", "))
                };
                let _ = writeln!(out, "\trows, err := db.Query(ctx, \"{}\"{})", sql, args_str);
                let _ = writeln!(out, "\tif err != nil {{");
                let _ = writeln!(out, "\t\treturn nil, err");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\tdefer rows.Close()");
                let _ = writeln!(out, "\tvar result []{}", struct_name);
                let _ = writeln!(out, "\tfor rows.Next() {{");
                let _ = writeln!(out, "\t\tvar r {}", struct_name);
                let scan_fields: Vec<String> = columns
                    .iter()
                    .map(|c| format!("&r.{}", to_pascal_case(&c.field_name)))
                    .collect();
                let _ = writeln!(
                    out,
                    "\t\tif err := rows.Scan({}); err != nil {{",
                    scan_fields.join(", ")
                );
                let _ = writeln!(out, "\t\t\treturn nil, err");
                let _ = writeln!(out, "\t\t}}");
                let _ = writeln!(out, "\t\tresult = append(result, r)");
                let _ = writeln!(out, "\t}}");
                let _ = writeln!(out, "\treturn result, rows.Err()");
                let _ = write!(out, "}}");
            }
        }

        Ok(out)
    }

    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError> {
        let type_name = enum_type_name(&enum_info.sql_name, &self.manifest.naming);
        let mut out = String::new();
        let _ = writeln!(out, "type {} string", type_name);
        let _ = writeln!(out);
        let _ = writeln!(out, "const (");
        for value in &enum_info.values {
            let variant = enum_variant_name(value, &self.manifest.naming);
            let _ = writeln!(
                out,
                "\t{}{} {} = \"{}\"",
                type_name, variant, type_name, value
            );
        }
        let _ = write!(out, ")");
        Ok(out)
    }

    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError> {
        let name = to_pascal_case(&composite.sql_name);
        let mut out = String::new();
        let _ = writeln!(out, "type {} struct {{", name);
        if composite.fields.is_empty() {
            // empty struct
        } else {
            for field in &composite.fields {
                let field_name = to_pascal_case(&field.name);
                let go_type = resolve_type(&field.neutral_type, &self.manifest, false)
                    .map(|t| t.into_owned())
                    .unwrap_or_else(|_| "interface{}".to_string());
                let json_tag = &field.name;
                let _ = writeln!(out, "\t{} {} `json:\"{}\"`", field_name, go_type, json_tag);
            }
        }
        let _ = write!(out, "}}");
        Ok(out)
    }
}
