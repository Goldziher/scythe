use std::path::Path;

use minijinja::Environment;
use serde::Serialize;

use crate::errors::BackendError;
use crate::manifest::{BackendManifest, load_manifest};

/// A backend renderer that loads templates and renders them with context data.
pub struct BackendRenderer {
    env: Environment<'static>,
    manifest: BackendManifest,
}

impl BackendRenderer {
    /// Load a backend from a directory path containing `manifest.toml` and `templates/`.
    pub fn load(backend_dir: &Path) -> Result<Self, BackendError> {
        let manifest_path = backend_dir.join("manifest.toml");
        let manifest = load_manifest(&manifest_path)?;

        let templates_dir = backend_dir.join("templates");
        let mut env = Environment::new();

        // Load all template files from the templates directory
        if templates_dir.exists() {
            for entry in std::fs::read_dir(&templates_dir).map_err(BackendError::Io)? {
                let entry = entry.map_err(BackendError::Io)?;
                let path = entry.path();
                if path.is_file() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or_else(|| {
                            BackendError::TemplateError("invalid template filename".to_string())
                        })?
                        .to_string();
                    let content = std::fs::read_to_string(&path).map_err(BackendError::Io)?;
                    env.add_template_owned(name, content)
                        .map_err(|e| BackendError::TemplateError(e.to_string()))?;
                }
            }
        }

        Ok(Self { env, manifest })
    }

    /// Get a reference to the loaded manifest.
    pub fn manifest(&self) -> &BackendManifest {
        &self.manifest
    }

    /// Render a named template with the given context.
    pub fn render(
        &self,
        template_name: &str,
        context: &impl Serialize,
    ) -> Result<String, BackendError> {
        let tmpl = self
            .env
            .get_template(template_name)
            .map_err(|e| BackendError::TemplateError(e.to_string()))?;
        tmpl.render(context)
            .map_err(|e| BackendError::TemplateError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rust_sqlx_backend_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../backends/rust-sqlx")
    }

    #[test]
    fn test_load_rust_sqlx_backend() {
        let renderer = BackendRenderer::load(&rust_sqlx_backend_dir()).unwrap();
        assert_eq!(renderer.manifest().backend.name, "rust-sqlx");
        assert_eq!(renderer.manifest().backend.language, "rust");
        assert_eq!(renderer.manifest().backend.file_extension, "rs");
    }

    #[test]
    fn test_render_template_with_context() {
        let renderer = BackendRenderer::load(&rust_sqlx_backend_dir()).unwrap();

        #[derive(Serialize)]
        struct Col {
            field_name: String,
            full_type: String,
        }

        #[derive(Serialize)]
        struct Query {
            row_struct_name: String,
            columns: Vec<Col>,
        }

        #[derive(Serialize)]
        struct Ctx {
            query: Query,
        }

        let ctx = Ctx {
            query: Query {
                row_struct_name: "GetUserRow".to_string(),
                columns: vec![
                    Col {
                        field_name: "id".to_string(),
                        full_type: "i32".to_string(),
                    },
                    Col {
                        field_name: "name".to_string(),
                        full_type: "String".to_string(),
                    },
                ],
            },
        };

        let result = renderer.render("row_struct.jinja", &ctx).unwrap();
        assert!(result.contains("pub struct GetUserRow"));
        assert!(result.contains("pub id: i32"));
        assert!(result.contains("pub name: String"));
    }

    #[test]
    fn test_load_nonexistent_directory_returns_error() {
        let result = BackendRenderer::load(Path::new("/nonexistent/path/to/backend"));
        assert!(result.is_err());
    }
}
