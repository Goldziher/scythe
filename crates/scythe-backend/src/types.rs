use std::borrow::Cow;

use crate::errors::BackendError;
use crate::manifest::BackendManifest;
use crate::naming::to_pascal_case;

/// Resolves a neutral type string to a language-specific type string.
///
/// Handles:
/// - Scalars: "int32" -> "i32"
/// - Containers: "array<int32>" -> "Vec<i32>" (recursive)
/// - Enums: "enum::user_status" -> "UserStatus" (via naming)
/// - Composites: "composite::address" -> "Address" (via naming)
/// - Nullable wrapping: if nullable, wraps result in the nullable container pattern
pub fn resolve_type<'a>(
    neutral: &str,
    manifest: &'a BackendManifest,
    nullable: bool,
) -> Result<Cow<'a, str>, BackendError> {
    let base = resolve_base_type(neutral, manifest)?;

    if nullable {
        Ok(Cow::Owned(wrap_nullable(&base, manifest)?))
    } else {
        Ok(base)
    }
}

/// Resolves a type and returns (full_type, lang_type).
///
/// `full_type` includes the nullable wrapper if needed.
/// `lang_type` is the base type without nullable wrapping.
pub fn resolve_type_pair<'a>(
    neutral: &str,
    manifest: &'a BackendManifest,
    nullable: bool,
) -> Result<(Cow<'a, str>, Cow<'a, str>), BackendError> {
    let lang_type = resolve_base_type(neutral, manifest)?;

    let full_type = if nullable {
        Cow::Owned(wrap_nullable(&lang_type, manifest)?)
    } else {
        lang_type.clone()
    };

    Ok((full_type, lang_type))
}

/// Resolve the base type (without nullable wrapping).
fn resolve_base_type<'a>(
    neutral: &str,
    manifest: &'a BackendManifest,
) -> Result<Cow<'a, str>, BackendError> {
    // Check for container pattern: "container_name<inner_type>"
    if let Some(resolved) = try_resolve_container(neutral, manifest)? {
        return Ok(Cow::Owned(resolved));
    }

    // Check for enum prefix
    if let Some(sql_name) = neutral.strip_prefix("enum::") {
        return Ok(Cow::Owned(to_pascal_case(sql_name).into_owned()));
    }

    // Check for composite prefix
    if let Some(sql_name) = neutral.strip_prefix("composite::") {
        return Ok(Cow::Owned(to_pascal_case(sql_name).into_owned()));
    }

    // Scalar lookup
    if let Some(lang_type) = manifest.types.scalars.get(neutral) {
        return Ok(Cow::Borrowed(lang_type.as_str()));
    }

    // Passthrough: user-defined type name (e.g., "EventData")
    // We treat it as passthrough if it starts with an uppercase letter
    if neutral.chars().next().is_some_and(|c| c.is_uppercase()) {
        return Ok(Cow::Owned(neutral.to_string()));
    }

    Err(BackendError::UnknownType(neutral.to_string()))
}

/// Try to parse and resolve a container type like "array<int32>".
/// Returns None if the input doesn't match any container pattern.
fn try_resolve_container(
    neutral: &str,
    manifest: &BackendManifest,
) -> Result<Option<String>, BackendError> {
    // Find the first '<' to detect container syntax
    let Some(angle_pos) = neutral.find('<') else {
        return Ok(None);
    };

    let container_name = &neutral[..angle_pos];

    // Check if this container is known
    let Some(pattern) = manifest.types.containers.get(container_name) else {
        return Err(BackendError::UnknownContainer(container_name.to_string()));
    };

    // Extract the inner type (strip trailing '>')
    let inner = neutral[angle_pos + 1..]
        .strip_suffix('>')
        .ok_or_else(|| BackendError::UnknownType(neutral.to_string()))?;

    // Trim whitespace from inner type
    let inner = inner.trim();

    // Recursively resolve the inner type
    let resolved_inner = resolve_base_type(inner, manifest)?;

    // Apply the container pattern
    let result = pattern.replace("{T}", &resolved_inner);
    Ok(Some(result))
}

/// Wrap a resolved type in the nullable container pattern.
fn wrap_nullable(resolved: &str, manifest: &BackendManifest) -> Result<String, BackendError> {
    let pattern = manifest
        .types
        .containers
        .get("nullable")
        .ok_or_else(|| BackendError::UnknownContainer("nullable".to_string()))?;
    Ok(pattern.replace("{T}", resolved))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest() -> BackendManifest {
        let toml_str = include_str!("../../../backends/rust-sqlx/manifest.toml");
        toml::from_str(toml_str).unwrap()
    }

    #[test]
    fn test_scalar_int32() {
        let m = test_manifest();
        assert_eq!(resolve_type("int32", &m, false).unwrap(), "i32");
    }

    #[test]
    fn test_scalar_int64() {
        let m = test_manifest();
        assert_eq!(resolve_type("int64", &m, false).unwrap(), "i64");
    }

    #[test]
    fn test_scalar_string() {
        let m = test_manifest();
        assert_eq!(resolve_type("string", &m, false).unwrap(), "String");
    }

    #[test]
    fn test_scalar_boolean() {
        let m = test_manifest();
        assert_eq!(resolve_type("bool", &m, false).unwrap(), "bool");
    }

    #[test]
    fn test_scalar_uuid() {
        let m = test_manifest();
        assert_eq!(resolve_type("uuid", &m, false).unwrap(), "uuid::Uuid");
    }

    #[test]
    fn test_container_array_int32() {
        let m = test_manifest();
        assert_eq!(resolve_type("array<int32>", &m, false).unwrap(), "Vec<i32>");
    }

    #[test]
    fn test_container_array_string() {
        let m = test_manifest();
        assert_eq!(
            resolve_type("array<string>", &m, false).unwrap(),
            "Vec<String>"
        );
    }

    #[test]
    fn test_enum_type() {
        let m = test_manifest();
        assert_eq!(
            resolve_type("enum::user_status", &m, false).unwrap(),
            "UserStatus"
        );
    }

    #[test]
    fn test_composite_type() {
        let m = test_manifest();
        assert_eq!(
            resolve_type("composite::address", &m, false).unwrap(),
            "Address"
        );
    }

    #[test]
    fn test_nullable_scalar() {
        let m = test_manifest();
        assert_eq!(resolve_type("int32", &m, true).unwrap(), "Option<i32>");
    }

    #[test]
    fn test_nullable_container() {
        let m = test_manifest();
        assert_eq!(
            resolve_type("array<int32>", &m, true).unwrap(),
            "Option<Vec<i32>>"
        );
    }

    #[test]
    fn test_range_container() {
        let m = test_manifest();
        assert_eq!(
            resolve_type("range<int32>", &m, false).unwrap(),
            "sqlx::postgres::types::PgRange<i32>"
        );
    }

    #[test]
    fn test_json_typed_container() {
        let m = test_manifest();
        assert_eq!(
            resolve_type("json_typed<EventData>", &m, false).unwrap(),
            "sqlx::types::Json<EventData>"
        );
    }

    #[test]
    fn test_passthrough_type() {
        let m = test_manifest();
        assert_eq!(resolve_type("EventData", &m, false).unwrap(), "EventData");
    }

    #[test]
    fn test_unknown_scalar_returns_error() {
        let m = test_manifest();
        let result = resolve_type("nonexistent_type", &m, false);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BackendError::UnknownType(_)));
    }

    #[test]
    fn test_resolve_type_pair_nullable() {
        let m = test_manifest();
        let (full, base) = resolve_type_pair("int32", &m, true).unwrap();
        assert_eq!(full, "Option<i32>");
        assert_eq!(base, "i32");
    }

    #[test]
    fn test_resolve_type_pair_non_nullable() {
        let m = test_manifest();
        let (full, base) = resolve_type_pair("int32", &m, false).unwrap();
        assert_eq!(full, "i32");
        assert_eq!(base, "i32");
    }

    #[test]
    fn test_range_datetime_tz() {
        let m = test_manifest();
        assert_eq!(
            resolve_type("range<datetime_tz>", &m, false).unwrap(),
            "sqlx::postgres::types::PgRange<chrono::DateTime<chrono::Utc>>"
        );
    }

    #[test]
    fn test_container_with_whitespace() {
        let m = test_manifest();
        assert_eq!(
            resolve_type("array< int32 >", &m, false).unwrap(),
            "Vec<i32>"
        );
    }

    #[test]
    fn test_empty_type_returns_error() {
        let m = test_manifest();
        let result = resolve_type("", &m, false);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BackendError::UnknownType(_)));
    }

    #[test]
    fn test_empty_container_inner_returns_error() {
        let m = test_manifest();
        let result = resolve_type("array<>", &m, false);
        assert!(result.is_err());
    }
}
