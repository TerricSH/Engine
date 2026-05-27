use render_core::RhiError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpenGlError {
    #[error("OpenGL error: {0}")]
    Gl(String),
    #[error("shader compilation failed: {0}")]
    ShaderCompile(String),
    #[error("program link failed: {0}")]
    ProgramLink(String),
}

impl OpenGlError {
    pub fn into_rhi(self) -> RhiError {
        match self {
            Self::Gl(detail) => RhiError::Backend { detail },
            Self::ShaderCompile(detail) => RhiError::ValidationFailed { detail },
            Self::ProgramLink(detail) => RhiError::ValidationFailed { detail },
        }
    }
}
