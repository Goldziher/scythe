use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::to_snake_case;
use scythe_backend::types::resolve_type_pair;

use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam};
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::{ResolvedColumn, ResolvedParam};

/// Resolve analyzed columns into resolved columns using a backend manifest.
pub fn resolve_columns(
    columns: &[AnalyzedColumn],
    manifest: &BackendManifest,
) -> Result<Vec<ResolvedColumn>, ScytheError> {
    columns
        .iter()
        .map(|col| {
            let (full_type, lang_type) =
                resolve_type_pair(&col.neutral_type, manifest, col.nullable)
                    .map(|(f, l)| (f.into_owned(), l.into_owned()))
                    .map_err(|e| {
                        ScytheError::new(
                            ErrorCode::InternalError,
                            format!("type resolution failed for column '{}': {}", col.name, e),
                        )
                    })?;
            Ok(ResolvedColumn {
                name: col.name.clone(),
                field_name: to_snake_case(&col.name).into_owned(),
                lang_type,
                full_type,
                neutral_type: col.neutral_type.clone(),
                nullable: col.nullable,
            })
        })
        .collect()
}

/// Resolve analyzed params into resolved params using a backend manifest.
pub fn resolve_params(
    params: &[AnalyzedParam],
    manifest: &BackendManifest,
) -> Result<Vec<ResolvedParam>, ScytheError> {
    params
        .iter()
        .map(|param| {
            let (full_type, lang_type) =
                resolve_type_pair(&param.neutral_type, manifest, param.nullable)
                    .map(|(f, l)| (f.into_owned(), l.into_owned()))
                    .map_err(|e| {
                        ScytheError::new(
                            ErrorCode::InternalError,
                            format!("type resolution failed for param '{}': {}", param.name, e),
                        )
                    })?;
            let borrowed_type = param_type_to_borrowed(&full_type);
            Ok(ResolvedParam {
                name: param.name.clone(),
                field_name: to_snake_case(&param.name).into_owned(),
                lang_type,
                full_type,
                borrowed_type,
                neutral_type: param.neutral_type.clone(),
                nullable: param.nullable,
            })
        })
        .collect()
}

/// Convert a resolved Rust type to its borrowed form for function parameters.
/// Copy types (primitives) stay as-is; String becomes &str; other non-Copy types get a & prefix.
pub fn param_type_to_borrowed(rust_type: &str) -> String {
    // Copy types that should stay owned in function params
    let copy_types = ["bool", "i16", "i32", "i64", "f32", "f64", "u64"];
    if copy_types.contains(&rust_type) {
        return rust_type.to_string();
    }
    // String -> &str
    if rust_type == "String" {
        return "&str".to_string();
    }
    // Option<T> wrapping: Option<String> -> Option<&str>, Option<Copy> stays, Option<Other> -> Option<&Other>
    if let Some(inner) = rust_type
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let borrowed_inner = param_type_to_borrowed(inner);
        return format!("Option<{}>", borrowed_inner);
    }
    // Vec<T> -> &[T] (slice reference)
    if let Some(inner) = rust_type
        .strip_prefix("Vec<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return format!("&[{}]", inner);
    }
    // Everything else gets a & prefix
    format!("&{}", rust_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_type_to_borrowed_string() {
        assert_eq!(param_type_to_borrowed("String"), "&str");
    }

    #[test]
    fn test_param_type_to_borrowed_vec() {
        assert_eq!(param_type_to_borrowed("Vec<i32>"), "&[i32]");
        assert_eq!(param_type_to_borrowed("Vec<String>"), "&[String]");
    }

    #[test]
    fn test_param_type_to_borrowed_passthrough() {
        assert_eq!(param_type_to_borrowed("i32"), "i32");
        assert_eq!(param_type_to_borrowed("i64"), "i64");
        assert_eq!(param_type_to_borrowed("bool"), "bool");
        assert_eq!(param_type_to_borrowed("f64"), "f64");
    }

    #[test]
    fn test_param_type_to_borrowed_option_string() {
        assert_eq!(param_type_to_borrowed("Option<String>"), "Option<&str>");
    }

    #[test]
    fn test_param_type_to_borrowed_option_copy() {
        assert_eq!(param_type_to_borrowed("Option<i32>"), "Option<i32>");
    }

    #[test]
    fn test_param_type_to_borrowed_other() {
        assert_eq!(param_type_to_borrowed("Uuid"), "&Uuid");
        assert_eq!(param_type_to_borrowed("NaiveDateTime"), "&NaiveDateTime");
    }
}
