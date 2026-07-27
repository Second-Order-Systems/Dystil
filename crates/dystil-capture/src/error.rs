use thiserror::Error;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("visual capture is disabled")]
    VisualCaptureDisabled,
    #[error("visual capture mode requires a visual provider")]
    VisualProviderUnavailable,
    #[error("capture produced no accessibility or visual evidence")]
    NoEvidence,
    #[error("accessibility capture failed: {0}")]
    Accessibility(String),
    #[error("visual capture failed: {0}")]
    Visual(String),
    #[error("visual image could not be materialized: {0}")]
    ImageStore(String),
    #[error("capture persistence failed: {0}")]
    Store(String),
}
