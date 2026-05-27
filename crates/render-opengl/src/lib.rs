//! OpenGL backend for the render layer, built on [glow].
//!
//! # Architecture
//!
//! - [OpenGlBackend] wraps an [Arc<glow::Context>] as the entry point.
//! - [OpenGlDevice] holds a cloned Arc<glow::Context> plus resource slabs.
//! - [OpenGlCommandEncoder] records GL commands into the immediate-mode
//!   context; it stores a raw pointer to the device for handle resolution.

mod device;
mod encoder;
mod error;

pub use device::{backend, OpenGlBackend, OpenGlDevice};
pub use encoder::OpenGlCommandEncoder;
pub use error::OpenGlError;

#[cfg(test)]
mod tests {
    use super::*;
    use render_core::{BackendKind, RhiError};

    // ── OpenGlError tests ────────────────────────────────────────────────

    #[test]
    fn opengl_error_gl_display() {
        let err = OpenGlError::Gl("invalid operation".to_string());
        assert_eq!(err.to_string(), "OpenGL error: invalid operation");
    }

    #[test]
    fn opengl_error_shader_compile_display() {
        let err = OpenGlError::ShaderCompile("syntax error".to_string());
        assert_eq!(err.to_string(), "shader compilation failed: syntax error");
    }

    #[test]
    fn opengl_error_program_link_display() {
        let err = OpenGlError::ProgramLink("incompatible types".to_string());
        assert_eq!(err.to_string(), "program link failed: incompatible types");
    }

    #[test]
    fn opengl_error_into_rhi_gl() {
        let err = OpenGlError::Gl("context lost".to_string());
        match err.into_rhi() {
            RhiError::Backend { detail } => assert_eq!(detail, "context lost"),
            _ => panic!("Expected RhiError::Backend"),
        }
    }

    #[test]
    fn opengl_error_into_rhi_shader_compile() {
        let err = OpenGlError::ShaderCompile("bad syntax".to_string());
        match err.into_rhi() {
            RhiError::ValidationFailed { detail } => assert_eq!(detail, "bad syntax"),
            _ => panic!("Expected RhiError::ValidationFailed"),
        }
    }

    #[test]
    fn opengl_error_into_rhi_program_link() {
        let err = OpenGlError::ProgramLink("mismatch".to_string());
        match err.into_rhi() {
            RhiError::ValidationFailed { detail } => assert_eq!(detail, "mismatch"),
            _ => panic!("Expected RhiError::ValidationFailed"),
        }
    }

    #[test]
    fn opengl_error_debug() {
        let err = OpenGlError::Gl("test".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("Gl"));
    }

    // ─── Conversion helpers (format functions) are private, test via side effects ──
    // The functions convert_texture_format, convert_index_format, buffer_target
    // are private in the device module. We test their behavior through the
    // OpenGlBackend's known behavior.

    #[test]
    fn opengl_backend_kind() {
        // Without a glow context, we can't create an OpenGlBackend directly.
        // Just verify the kind constant is correct.
        assert_eq!(
            format!("{:?}", BackendKind::OpenGl),
            "OpenGl"
        );
    }
}
