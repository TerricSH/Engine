// ============================================================================
// Non-Windows / no backend-dx12: fail-closed placeholder
// ============================================================================

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
use engine_renderer::{
    BackendRenderer, Diagnostic, DiagnosticSeverity, MeshUpload, RenderFrameInput, UploadReceipt,
};

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
pub struct Dx12SceneRenderer;

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
impl Dx12SceneRenderer {
    pub fn new(
        _device: crate::device::Dx12Device,
        _swapchain: render_core::SwapchainHandle,
        _width: u32,
        _height: u32,
    ) -> Self {
        Self
    }

    fn unavailable(operation: &str) -> Vec<Diagnostic> {
        vec![Diagnostic::new(
            "DX1290",
            DiagnosticSeverity::Error,
            "scene_renderer",
            format!("cannot perform {operation}: the DX12 backend is unavailable on this target"),
        )]
    }
}

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
impl BackendRenderer for Dx12SceneRenderer {
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        Err(Self::unavailable("begin_frame"))
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Err(Self::unavailable("abort_frame"))
    }

    fn upload_mesh(&mut self, _upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Err(Self::unavailable("mesh upload"))
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        if width == 0 || height == 0 {
            return Err(vec![Diagnostic::new(
                "DX1240",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("resize dimensions must be non-zero, got {width}x{height}"),
            )]);
        }
        Err(Self::unavailable("resize"))
    }
}
