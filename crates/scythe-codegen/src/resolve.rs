use scythe_backend::manifest::BackendManifest;
use scythe_backend::naming::{NamingConfig, field_name};
use scythe_backend::types::resolve_type_pair;

use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam};
use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::{ResolvedColumn, ResolvedParam};
use crate::overrides::{TypeOverride, find_override};

/// Resolve analyzed columns into resolved columns using a backend manifest.
///
/// When `overrides` is non-empty, each column is checked against the override
/// list before the normal type-resolution path. The first matching override
/// replaces the column's neutral type.
pub fn resolve_columns(
    columns: &[AnalyzedColumn],
    manifest: &BackendManifest,
    overrides: &[TypeOverride],
    source_table: &str,
) -> Result<Vec<ResolvedColumn>, ScytheError> {
    let resolved: Vec<ResolvedColumn> = columns
        .iter()
        .map(|col| {
            let column_match = if source_table.is_empty() {
                String::new()
            } else {
                format!("{}.{}", source_table, col.name)
            };
            let effective_neutral_type =
                find_override(overrides, &column_match, &col.neutral_type).unwrap_or(&col.neutral_type);

            let (full_type, lang_type) = resolve_type_pair(effective_neutral_type, manifest, col.nullable)
                .map(|(f, l)| (f.into_owned(), l.into_owned()))
                .map_err(|e| {
                    ScytheError::new(
                        ErrorCode::InternalError,
                        format!("type resolution failed for column '{}': {}", col.name, e),
                    )
                })?;
            Ok(ResolvedColumn {
                name: col.name.clone(),
                field_name: field_name(&col.name, &manifest.naming).into_owned(),
                lang_type,
                full_type,
                neutral_type: effective_neutral_type.to_string(),
                nullable: col.nullable,
                join_group: col.join_group.clone(),
                nullable_before_join: col.nullable_before_join,
                sql_type: col.sql_type.clone(),
            })
        })
        .collect::<Result<_, ScytheError>>()?;

    check_field_name_collisions(
        resolved.iter().map(|c| (c.name.as_str(), c.field_name.as_str())),
        "columns",
        &manifest.naming,
    )?;

    Ok(resolved)
}

/// Resolve analyzed params into resolved params using a backend manifest.
///
/// When `overrides` is non-empty, each param is checked against the override
/// list before the normal type-resolution path.
pub fn resolve_params(
    params: &[AnalyzedParam],
    manifest: &BackendManifest,
    overrides: &[TypeOverride],
    source_table: &str,
) -> Result<Vec<ResolvedParam>, ScytheError> {
    let resolved: Vec<ResolvedParam> = params
        .iter()
        .map(|param| {
            let column_match = if source_table.is_empty() {
                String::new()
            } else {
                format!("{}.{}", source_table, param.name)
            };
            let effective_neutral_type =
                find_override(overrides, &column_match, &param.neutral_type).unwrap_or(&param.neutral_type);

            let (full_type, lang_type) = resolve_type_pair(effective_neutral_type, manifest, param.nullable)
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
                field_name: field_name(&param.name, &manifest.naming).into_owned(),
                lang_type,
                full_type,
                borrowed_type,
                neutral_type: effective_neutral_type.to_string(),
                nullable: param.nullable,
            })
        })
        .collect::<Result<_, ScytheError>>()?;

    check_field_name_collisions(
        resolved.iter().map(|p| (p.name.as_str(), p.field_name.as_str())),
        "params",
        &manifest.naming,
    )?;

    Ok(resolved)
}

/// Reject two SQL identifiers in the same column list or param list that
/// collapse onto the same generated field name.
///
/// Not just a `camelCase` problem: `field_name` always runs `to_snake_case`
/// as its base transform (see `naming::apply_case`), and `to_snake_case` is
/// not the identity function for mixed- or upper-case identifiers, so
/// quoted SQL like `SELECT "USER_ID", user_id FROM t` folds both onto
/// `user_id` even under the default `snake_case`. The analyzer's
/// `duplicate_alias` check cannot catch this in any case -- it runs on the
/// raw SQL names, before this conversion. So this runs unconditionally,
/// regardless of `field_case`: it is O(n^2) over one query's column or
/// param list, typically well under thirty items, so the cost is
/// negligible next to the type resolution this function already does.
///
/// `pub(crate)` rather than private: `lib.rs`'s `:grouped` path reuses this
/// same check against the parent column list *plus* the synthesized
/// `children` field a grouped backend injects, so that collision is caught
/// by the same single rule instead of a second, independent one (#188).
pub(crate) fn check_field_name_collisions<'a>(
    items: impl Iterator<Item = (&'a str, &'a str)>,
    kind: &str,
    naming: &NamingConfig,
) -> Result<(), ScytheError> {
    let items: Vec<(&str, &str)> = items.collect();
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            let (sql_a, field_a) = items[i];
            let (sql_b, field_b) = items[j];
            if field_a == field_b {
                let alternative = if naming.field_case == "snake_case" {
                    String::new()
                } else {
                    ", or set field_case = \"snake_case\"".to_string()
                };
                return Err(ScytheError::new(
                    ErrorCode::DuplicateAlias,
                    format!(
                        "{kind} '{sql_a}' and '{sql_b}' both resolve to field name '{field_a}' under \
                         field_case = \"{}\" -- alias one of them in SQL{alternative}",
                        naming.field_case
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Convert a resolved Rust type to its borrowed form for function parameters.
/// Copy types (primitives) stay as-is; String becomes &str; other non-Copy types get a & prefix.
pub fn param_type_to_borrowed(rust_type: &str) -> String {
    let copy_types = ["bool", "i16", "i32", "i64", "f32", "f64", "u64"];
    if copy_types.contains(&rust_type) {
        return rust_type.to_string();
    }
    if rust_type == "String" {
        return "&str".to_string();
    }
    if let Some(inner) = rust_type.strip_prefix("Option<").and_then(|s| s.strip_suffix('>')) {
        let borrowed_inner = param_type_to_borrowed(inner);
        return format!("Option<{}>", borrowed_inner);
    }
    if let Some(inner) = rust_type.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
        return format!("&[{}]", inner);
    }
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

    fn test_manifest(field_case: &str) -> BackendManifest {
        use ahash::AHashMap;
        use scythe_backend::manifest::{BackendMeta, TypeMappings};

        let mut scalars = AHashMap::new();
        scalars.insert("string".to_string(), "String".to_string());
        scalars.insert("int32".to_string(), "i32".to_string());

        BackendManifest {
            backend: BackendMeta {
                name: "test".to_string(),
                language: "rust".to_string(),
                file_extension: "rs".to_string(),
                engine: "postgresql".to_string(),
                description: None,
            },
            types: TypeMappings {
                scalars,
                containers: AHashMap::new(),
            },
            naming: NamingConfig {
                struct_case: "PascalCase".to_string(),
                fn_case: "snake_case".to_string(),
                enum_variant_case: "PascalCase".to_string(),
                row_suffix: "Row".to_string(),
                field_case: field_case.to_string(),
            },
            imports: None,
        }
    }

    fn test_column(name: &str) -> AnalyzedColumn {
        AnalyzedColumn {
            name: name.to_string(),
            neutral_type: "string".to_string(),
            ..Default::default()
        }
    }

    fn test_param(name: &str, position: i64) -> AnalyzedParam {
        AnalyzedParam {
            name: name.to_string(),
            neutral_type: "string".to_string(),
            nullable: false,
            position,
        }
    }

    #[test]
    fn test_resolve_columns_defaults_to_snake_case_field_names() {
        let manifest = test_manifest("snake_case");
        let resolved = resolve_columns(&[test_column("UserId")], &manifest, &[], "").unwrap();
        assert_eq!(resolved[0].field_name, "user_id");
    }

    #[test]
    fn test_resolve_columns_honors_camel_case_field_case() {
        let manifest = test_manifest("camelCase");
        let resolved = resolve_columns(&[test_column("user_id")], &manifest, &[], "").unwrap();
        assert_eq!(resolved[0].field_name, "userId");
    }

    #[test]
    fn test_resolve_params_honors_camel_case_field_case() {
        let manifest = test_manifest("camelCase");
        let resolved = resolve_params(&[test_param("user_id", 1)], &manifest, &[], "").unwrap();
        assert_eq!(resolved[0].field_name, "userId");
    }

    /// This must fail before the fix: `field_name` always runs
    /// `to_snake_case` as its base transform, and `to_snake_case` is not the
    /// identity function for mixed-/upper-case identifiers --
    /// `to_snake_case("USER_ID") == "user_id"`. Quoted SQL like
    /// `SELECT "USER_ID", user_id FROM t` is legal and passes the
    /// analyzer's case-sensitive `duplicate_alias` check, so this collision
    /// must be caught here, even under the default `field_case`. Skipping
    /// the check when `field_case == "snake_case"` (the previous behavior)
    /// let it through and produced two struct fields both named `user_id`.
    #[test]
    fn test_resolve_columns_rejects_collision_under_snake_case() {
        let manifest = test_manifest("snake_case");
        let err = resolve_columns(&[test_column("user_id"), test_column("USER_ID")], &manifest, &[], "")
            .expect_err("user_id and USER_ID must collide under snake_case too");
        let message = err.to_string();
        assert!(message.contains("user_id"), "{message}");
        assert!(message.contains("USER_ID"), "{message}");
        assert!(
            !message.contains("or set field_case"),
            "already snake_case -- switching field_case cannot fix this: {message}"
        );
    }

    #[test]
    fn test_resolve_columns_rejects_collision_under_camel_case() {
        let manifest = test_manifest("camelCase");
        let err = resolve_columns(&[test_column("user_id"), test_column("userId")], &manifest, &[], "")
            .expect_err("user_id and userId must collide under camelCase");
        let message = err.to_string();
        assert!(message.contains("user_id"), "{message}");
        assert!(message.contains("userId"), "{message}");
        assert!(message.contains("field_case"), "{message}");
    }

    #[test]
    fn test_resolve_columns_rejects_collision_across_four_spellings() {
        let manifest = test_manifest("camelCase");
        for (a, b) in [
            ("user_id", "USER_ID"),
            ("user_id", "UserId"),
            ("USER_ID", "userId"),
            ("UserId", "userId"),
        ] {
            resolve_columns(&[test_column(a), test_column(b)], &manifest, &[], "")
                .expect_err(&format!("{a} and {b} must collide under camelCase"));
        }
    }

    #[test]
    fn test_resolve_columns_rejects_double_underscore_collision() {
        let manifest = test_manifest("camelCase");
        resolve_columns(&[test_column("a_b"), test_column("a__b")], &manifest, &[], "")
            .expect_err("a_b and a__b must both collapse to aB under camelCase");
    }

    #[test]
    fn test_resolve_columns_rejects_leading_underscore_collision() {
        let manifest = test_manifest("camelCase");
        resolve_columns(&[test_column("id"), test_column("_id")], &manifest, &[], "")
            .expect_err("id and _id must both collapse to id under camelCase");
    }

    #[test]
    fn test_resolve_params_rejects_collision_under_camel_case() {
        let manifest = test_manifest("camelCase");
        let err = resolve_params(&[test_param("user_id", 1), test_param("userId", 2)], &manifest, &[], "")
            .expect_err("user_id and userId must collide under camelCase");
        assert!(err.to_string().contains("params"), "{err}");
    }

    #[test]
    fn test_resolve_columns_no_collision_when_names_differ() {
        let manifest = test_manifest("camelCase");
        let resolved = resolve_columns(&[test_column("user_id"), test_column("order_id")], &manifest, &[], "").unwrap();
        assert_eq!(resolved[0].field_name, "userId");
        assert_eq!(resolved[1].field_name, "orderId");
    }
}
