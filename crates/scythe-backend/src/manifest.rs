use std::path::Path;

use ahash::AHashMap;
use serde::Deserialize;

use crate::errors::BackendError;
use crate::naming::NamingConfig;

/// Top-level backend manifest parsed from `manifest.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendManifest {
    pub backend: BackendMeta,
    pub types: TypeMappings,
    pub naming: NamingConfig,
    pub imports: Option<ImportConfig>,
}

/// Metadata about the backend.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendMeta {
    pub name: String,
    pub language: String,
    pub file_extension: String,
    pub engine: String,
    pub description: Option<String>,
}

/// Mappings from neutral types to language-specific types.
#[derive(Debug, Clone, Deserialize)]
pub struct TypeMappings {
    /// Scalar type mappings: neutral name -> language type.
    pub scalars: AHashMap<String, String>,
    /// Container type patterns: container name -> pattern with `{T}` placeholder.
    pub containers: AHashMap<String, String>,
}

/// Import rules for generated code.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportConfig {
    /// Maps a type prefix to the import statement needed.
    pub rules: AHashMap<String, String>,
}

/// Load and parse a backend manifest from a TOML file.
pub fn load_manifest(path: &Path) -> Result<BackendManifest, BackendError> {
    let content = std::fs::read_to_string(path).map_err(BackendError::Io)?;
    toml::from_str(&content).map_err(|e| BackendError::ManifestError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_manifest_from_string() {
        let toml_str = include_str!("../test-manifests/rust-sqlx.toml");
        let manifest: BackendManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.backend.name, "rust-sqlx");
        assert_eq!(manifest.backend.language, "rust");
        assert_eq!(manifest.backend.file_extension, "rs");
        assert_eq!(manifest.types.scalars["int32"], "i32");
        assert_eq!(manifest.types.containers["array"], "Vec<{T}>");
        assert_eq!(manifest.naming.struct_case, "PascalCase");
        assert_eq!(manifest.naming.row_suffix, "Row");
    }

    #[test]
    fn test_load_tokio_postgres_manifest() {
        let toml_str = include_str!("../test-manifests/rust-tokio-postgres.toml");
        let manifest: BackendManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.backend.name, "rust-tokio-postgres");
        assert_eq!(manifest.backend.language, "rust");
        assert_eq!(manifest.backend.file_extension, "rs");
        assert_eq!(manifest.backend.engine, "postgresql");
        assert_eq!(manifest.types.scalars["int32"], "i32");
        assert_eq!(manifest.types.scalars["inet"], "std::net::IpAddr");
        assert_eq!(manifest.types.scalars["time_tz"], "chrono::NaiveTime");
        assert_eq!(manifest.types.scalars["interval"], "String");
        assert_eq!(manifest.types.containers["array"], "Vec<{T}>");
        assert_eq!(manifest.types.containers["json_typed"], "{T}");
        assert_eq!(manifest.types.containers["range"], "String");
        assert_eq!(manifest.naming.struct_case, "PascalCase");
        assert_eq!(manifest.naming.row_suffix, "Row");
        let imports = manifest.imports.unwrap();
        assert_eq!(imports.rules["std::net::"], "use std::net::IpAddr;");
    }
}
