use std::fmt::Write;
use std::path::Path;

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::naming::{
    enum_type_name, enum_variant_name, fn_name, row_struct_name, to_pascal_case,
};

use scythe_core::analyzer::{AnalyzedQuery, CompositeInfo, EnumInfo};
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::{CodegenBackend, ResolvedColumn, ResolvedParam};

const DEFAULT_MANIFEST_TOML: &str = include_str!("../../../../backends/go-pgx/manifest.toml");

pub struct GoPgxBackend {
    manifest: BackendManifest,
}

impl GoPgxBackend {
    pub fn new() -> Result<Self, ScytheError> {
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

    pub fn manifest(&self) -> &BackendManifest {
        &self.manifest
    }
}

impl CodegenBackend for GoPgxBackend {
    fn name(&self) -> &str {
        "go-pgx"
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
            let _ = writeln!(out, "\t{} {}", field, col.full_type);
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
        _columns: &[ResolvedColumn],
        params: &[ResolvedParam],
    ) -> Result<String, ScytheError> {
        let func_name = fn_name(&analyzed.name, &self.manifest.naming);
        let mut out = String::new();
        let param_list = params
            .iter()
            .map(|p| format!("{} {}", p.field_name, p.full_type))
            .collect::<Vec<_>>()
            .join(", ");
        let sep = if param_list.is_empty() { "" } else { ", " };
        let _ = writeln!(
            out,
            "func {}(ctx context.Context, db DBTX{}{}) ({}, error) {{",
            func_name, sep, param_list, struct_name
        );
        let _ = writeln!(out, "\t// TODO: implement");
        let _ = writeln!(out, "\treturn {}{{}}, nil", struct_name);
        let _ = write!(out, "}}");
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
                "\t{}{} {} = {:?}",
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
        let _ = writeln!(out, "\t// TODO: fields");
        let _ = write!(out, "}}");
        Ok(out)
    }
}
