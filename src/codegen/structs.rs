use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{row_struct_name, to_pascal_case, to_snake_case};

use crate::analyzer::{AnalyzedQuery, CompositeInfo};
use crate::errors::{ErrorCode, ScytheError};

use super::resolve_col_type;

pub(super) fn generate_row_struct(
    analyzed: &AnalyzedQuery,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    let struct_name = row_struct_name(&analyzed.name, &manifest.naming);
    let mut out = String::new();

    writeln!(out, "#[derive(Debug, sqlx::FromRow)]").unwrap();
    writeln!(out, "pub struct {} {{", struct_name).unwrap();

    for col in &analyzed.columns {
        let field_name = to_snake_case(&col.name);
        let rust_type = resolve_col_type(col, manifest)?;
        writeln!(out, "    pub {}: {},", field_name, rust_type).unwrap();
    }

    write!(out, "}}").unwrap();

    Ok(out)
}

pub(super) fn generate_model_struct(
    analyzed: &AnalyzedQuery,
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    let table_name = analyzed.source_table.as_deref().unwrap_or(&analyzed.name);
    // Singularize table name for model struct: "users" -> "User"
    let singular = singularize(table_name);
    let struct_name = to_pascal_case(&singular).into_owned();
    let mut out = String::new();

    writeln!(out, "#[derive(Debug, sqlx::FromRow)]").unwrap();
    writeln!(out, "pub struct {} {{", struct_name).unwrap();

    for col in &analyzed.columns {
        let field_name = to_snake_case(&col.name);
        let rust_type = resolve_col_type(col, manifest)?;
        writeln!(out, "    pub {}: {},", field_name, rust_type).unwrap();
    }

    write!(out, "}}").unwrap();

    Ok(out)
}

pub(super) fn generate_composite_defs(
    composites: &[CompositeInfo],
    manifest: &BackendManifest,
) -> Result<String, ScytheError> {
    use scythe_backend::types::resolve_type;

    let mut out = String::new();
    for (i, comp) in composites.iter().enumerate() {
        if i > 0 {
            writeln!(out).unwrap();
            writeln!(out).unwrap();
        }
        let struct_name = to_pascal_case(&comp.sql_name).into_owned();
        writeln!(out, "#[derive(Debug, Clone, sqlx::Type)]").unwrap();
        writeln!(out, "#[sqlx(type_name = \"{}\")]", comp.sql_name).unwrap();
        writeln!(out, "pub struct {} {{", struct_name).unwrap();
        for field in &comp.fields {
            let rust_type = resolve_type(&field.neutral_type, manifest, false)
                .map(|t| t.into_owned())
                .map_err(|e| {
                    ScytheError::new(
                        ErrorCode::InternalError,
                        format!("composite field type error: {}", e),
                    )
                })?;
            writeln!(
                out,
                "    pub {}: {},",
                to_snake_case(&field.name),
                rust_type
            )
            .unwrap();
        }
        write!(out, "}}").unwrap();
    }
    Ok(out)
}

/// Simple singularization: remove trailing 's'
pub(super) fn singularize(name: &str) -> String {
    if let Some(stem) = name.strip_suffix("ies") {
        format!("{stem}y")
    } else if name.ends_with("sses")
        || name.ends_with("shes")
        || name.ends_with("ches")
        || name.ends_with("xes")
        || name.ends_with("zes")
        || name.ends_with("ses")
    {
        name[..name.len() - 2].to_string()
    } else if name.ends_with('s') && !name.ends_with("ss") {
        name[..name.len() - 1].to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_singularize_basic() {
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("orders"), "order");
        assert_eq!(singularize("posts"), "post");
    }

    #[test]
    fn test_singularize_ies() {
        assert_eq!(singularize("categories"), "category");
        assert_eq!(singularize("entries"), "entry");
    }

    #[test]
    fn test_singularize_sses() {
        assert_eq!(singularize("addresses"), "address");
        assert_eq!(singularize("classes"), "class");
    }

    #[test]
    fn test_singularize_no_change() {
        assert_eq!(singularize("status"), "statu");
        // Words ending in "ss" should not be trimmed
        assert_eq!(singularize("boss"), "boss");
        assert_eq!(singularize("address"), "address");
    }

    #[test]
    fn test_singularize_shes_ches_xes() {
        assert_eq!(singularize("batches"), "batch");
        assert_eq!(singularize("boxes"), "box");
        assert_eq!(singularize("wishes"), "wish");
    }
}
