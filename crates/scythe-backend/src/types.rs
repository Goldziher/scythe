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
fn resolve_base_type<'a>(neutral: &str, manifest: &'a BackendManifest) -> Result<Cow<'a, str>, BackendError> {
    if let Some(resolved) = try_resolve_container(neutral, manifest)? {
        return Ok(Cow::Owned(resolved));
    }

    if let Some(sql_name) = neutral.strip_prefix("enum::") {
        return Ok(Cow::Owned(to_pascal_case(sql_name).into_owned()));
    }

    if let Some(sql_name) = neutral.strip_prefix("composite::") {
        return Ok(Cow::Owned(to_pascal_case(sql_name).into_owned()));
    }

    if let Some(lang_type) = manifest.types.scalars.get(neutral) {
        return Ok(Cow::Borrowed(lang_type.as_str()));
    }

    if neutral.chars().next().is_some_and(|c| c.is_uppercase()) {
        return Ok(Cow::Owned(neutral.to_string()));
    }

    Err(BackendError::UnknownType(neutral.to_string()))
}

/// Try to parse and resolve a container type like "array<int32>".
/// Returns None if the input doesn't match any container pattern.
fn try_resolve_container(neutral: &str, manifest: &BackendManifest) -> Result<Option<String>, BackendError> {
    let Some(angle_pos) = neutral.find('<') else {
        return Ok(None);
    };

    let container_name = &neutral[..angle_pos];

    let Some(pattern) = manifest.types.containers.get(container_name) else {
        return Err(BackendError::UnknownContainer(container_name.to_string()));
    };

    let inner = neutral[angle_pos + 1..]
        .strip_suffix('>')
        .ok_or_else(|| BackendError::UnknownType(neutral.to_string()))?;

    let inner = inner.trim();

    let resolved_inner = resolve_base_type(inner, manifest)?;

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

/// Parse whether `full_type` is a nullable rendering of `lang_type` under
/// `manifest`, without trusting any pre-computed `nullable` bit.
///
/// This is the inverse of [`resolve_type_pair`]: instead of taking
/// "nullable" as an input and producing `full_type`, it takes `full_type`
/// and `lang_type` (both already computed, e.g. from a
/// `scythe_codegen::backend_trait::ResolvedColumn`) and asks the manifest
/// which one it actually is. That makes it safe to use as an independent
/// check on a value that was *supposed* to be derived from `nullable`, to
/// catch call sites that got it wrong.
///
/// - `Ok(false)` when `full_type == lang_type` -- rendered as non-optional.
///   This also fires when the manifest's `nullable` container pattern has
///   degenerated to the identity mapping (`"{T}"`), which is itself a
///   manifest bug: such a manifest can never render an optional type, so
///   every column looks non-optional here regardless of what the analyzer
///   said -- exactly the drift this function exists to surface.
/// - `Ok(true)` when `full_type` matches `lang_type` wrapped by the
///   manifest's `nullable` pattern.
/// - `Err` when `full_type` matches neither shape, or the manifest has no
///   `nullable` container pattern at all -- an unrecognized rendering,
///   always a bug (a hand-built `ResolvedColumn`, a stale `full_type`, or a
///   manifest mid-migration).
pub fn parse_rendered_nullable(
    lang_type: &str,
    full_type: &str,
    manifest: &BackendManifest,
) -> Result<bool, BackendError> {
    if full_type == lang_type {
        return Ok(false);
    }
    let wrapped = wrap_nullable(lang_type, manifest)?;
    if full_type == wrapped {
        return Ok(true);
    }
    Err(BackendError::UnrecognizedNullableRendering {
        lang_type: lang_type.to_string(),
        full_type: full_type.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest() -> BackendManifest {
        let toml_str = include_str!("../test-manifests/rust-sqlx.toml");
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
        assert_eq!(resolve_type("array<string>", &m, false).unwrap(), "Vec<String>");
    }

    #[test]
    fn test_enum_type() {
        let m = test_manifest();
        assert_eq!(resolve_type("enum::user_status", &m, false).unwrap(), "UserStatus");
    }

    #[test]
    fn test_composite_type() {
        let m = test_manifest();
        assert_eq!(resolve_type("composite::address", &m, false).unwrap(), "Address");
    }

    #[test]
    fn test_nullable_scalar() {
        let m = test_manifest();
        assert_eq!(resolve_type("int32", &m, true).unwrap(), "Option<i32>");
    }

    #[test]
    fn test_nullable_container() {
        let m = test_manifest();
        assert_eq!(resolve_type("array<int32>", &m, true).unwrap(), "Option<Vec<i32>>");
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
        assert_eq!(resolve_type("array< int32 >", &m, false).unwrap(), "Vec<i32>");
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

    // -- parse_rendered_nullable ---------------------------------------

    #[test]
    fn test_parse_rendered_nullable_recognizes_wrapped_form() {
        let m = test_manifest();
        assert!(parse_rendered_nullable("i32", "Option<i32>", &m).unwrap());
    }

    #[test]
    fn test_parse_rendered_nullable_recognizes_bare_form() {
        let m = test_manifest();
        assert!(!parse_rendered_nullable("i32", "i32", &m).unwrap());
    }

    #[test]
    fn test_parse_rendered_nullable_rejects_unrecognized_rendering() {
        let m = test_manifest();
        let result = parse_rendered_nullable("i32", "Vec<i32>", &m);
        assert!(matches!(
            result,
            Err(BackendError::UnrecognizedNullableRendering { .. })
        ));
    }

    #[test]
    fn test_parse_rendered_nullable_catches_an_identity_nullable_pattern() {
        // ~keep A manifest whose "nullable" pattern doesn't actually wrap
        // anything can never render an optional type -- full_type always
        // equals lang_type regardless of what the caller intended, so this
        // must come back `Ok(false)` (never `Ok(true)`) and let the caller's
        // own analyzed-vs-rendered comparison catch the drift.
        let mut m = test_manifest();
        m.types.containers.insert("nullable".to_string(), "{T}".to_string());
        assert!(!parse_rendered_nullable("i32", "i32", &m).unwrap());
    }

    #[test]
    fn test_parse_rendered_nullable_errors_when_manifest_has_no_nullable_pattern() {
        let mut m = test_manifest();
        m.types.containers.remove("nullable");
        let result = parse_rendered_nullable("i32", "Option<i32>", &m);
        assert!(matches!(result, Err(BackendError::UnknownContainer(_))));
    }
}
