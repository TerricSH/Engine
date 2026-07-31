use std::fmt;
use std::io;

/// Errors that can occur during the asset cooking pipeline.
#[derive(Debug)]
pub enum AssetCookError {
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

/// Backwards-compatible short name for [`AssetCookError`].
pub type CookError = AssetCookError;

impl fmt::Display for AssetCookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetCookError::Io(e) => write!(f, "I/O error: {e}"),
            AssetCookError::Parse(msg) => write!(f, "parse error: {msg}"),
            AssetCookError::Compile(msg) => write!(f, "shader compile error: {msg}"),
            AssetCookError::Reflection(msg) => write!(f, "reflection error: {msg}"),
            AssetCookError::InvalidAsset(msg) => write!(f, "invalid asset: {msg}"),
            AssetCookError::UnsupportedFormat(msg) => write!(f, "unsupported format: {msg}"),
        }
    }
}

impl std::error::Error for AssetCookError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AssetCookError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for AssetCookError {
    fn from(e: io::Error) -> Self {
        AssetCookError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn legacy_cook_error_is_the_asset_cook_error() {
        assert_eq!(
            std::any::TypeId::of::<CookError>(),
            std::any::TypeId::of::<AssetCookError>()
        );
    }

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
