/// Errors that can occur in backend operations.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("unknown neutral type: {0}")]
    UnknownType(String),
    #[error("unknown container: {0}")]
    UnknownContainer(String),
    #[error("manifest error: {0}")]
    ManifestError(String),
    #[error("template error: {0}")]
    TemplateError(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
