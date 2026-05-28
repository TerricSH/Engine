use std::fmt;
use std::io;

/// Errors that can occur during the asset cooking pipeline.
#[derive(Debug)]
pub enum CookError {
    /// An I/O error (file not found, permission denied, etc.).
    Io(io::Error),
    /// A parse error when reading a source file (e.g. invalid JSON/glTF).
    Parse(String),
    /// A shader compilation error (GLSL → SPIR-V).
    Compile(String),
    /// A reflection extraction error.
    Reflection(String),
    /// The asset data is structurally invalid.
    InvalidAsset(String),
    /// The source format is not supported by the cooker.
    UnsupportedFormat(String),
}

impl fmt::Display for CookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CookError::Io(e) => write!(f, "I/O error: {e}"),
            CookError::Parse(msg) => write!(f, "parse error: {msg}"),
            CookError::Compile(msg) => write!(f, "shader compile error: {msg}"),
            CookError::Reflection(msg) => write!(f, "reflection error: {msg}"),
            CookError::InvalidAsset(msg) => write!(f, "invalid asset: {msg}"),
            CookError::UnsupportedFormat(msg) => write!(f, "unsupported format: {msg}"),
        }
    }
}

impl std::error::Error for CookError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CookError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for CookError {
    fn from(e: io::Error) -> Self {
        CookError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn cook_error_io_display() {
        let err = CookError::Io(io::Error::new(ErrorKind::NotFound, "file not found"));
        assert!(err.to_string().contains("I/O error"));
    }

    #[test]
    fn cook_error_parse_display() {
        let err = CookError::Parse("invalid JSON".into());
        assert_eq!(err.to_string(), "parse error: invalid JSON");
    }

    #[test]
    fn cook_error_compile_display() {
        let err = CookError::Compile("syntax error".into());
        assert_eq!(err.to_string(), "shader compile error: syntax error");
    }

    #[test]
    fn cook_error_reflection_display() {
        let err = CookError::Reflection("missing binding".into());
        assert_eq!(err.to_string(), "reflection error: missing binding");
    }

    #[test]
    fn cook_error_invalid_asset_display() {
        let err = CookError::InvalidAsset("no vertices".into());
        assert_eq!(err.to_string(), "invalid asset: no vertices");
    }

    #[test]
    fn cook_error_unsupported_format_display() {
        let err = CookError::UnsupportedFormat("unknown".into());
        assert_eq!(err.to_string(), "unsupported format: unknown");
    }

    #[test]
    fn cook_error_from_io() {
        let io_err = io::Error::new(ErrorKind::PermissionDenied, "denied");
        let cook_err: CookError = io_err.into();
        assert!(matches!(cook_err, CookError::Io(_)));
    }
}
