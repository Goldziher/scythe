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
    #[error(
        "rendered type {full_type:?} matches neither the base form {lang_type:?} nor its nullable-wrapped form for this manifest -- the manifest's \"nullable\" container pattern and the type that produced {full_type:?} have drifted apart"
    )]
    UnrecognizedNullableRendering { lang_type: String, full_type: String },
}
