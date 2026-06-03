//! DirectX 12 backend implementation.
//!
//! This crate provides a full D3D12 rendering backend via the `windows` crate.
//! On non-Windows platforms, the backend returns stubs with descriptive errors.
//!
//! # Feature flags
//!
//! * `backend-dx12` (default) — enables the `windows` dependency and full
//!   D3D12 device creation, swapchain, and pipeline management.
//!
//! # Enabling
//!
//! The workspace requires the `windows` crate with D3D12 and DXGI features:
//!
//! ```toml
//! windows = { version = "0.58", features = [
//!     "Win32_Graphics_Direct3D12",
//!     "Win32_Graphics_Dxgi_Common",
//!     "Win32_Graphics_Dxgi",
//!     "Win32_Graphics_Direct3D",
//!     "Win32_Foundation",
//!     "Win32_System_Threading",
//!     "Win32_UI_WindowsAndMessaging",
//! ] }
//! ```

pub mod backend;
pub mod device;
mod encoder;
pub mod error;
mod handle;
mod pipeline;
mod resources;
pub mod scene_renderer;
mod swapchain;

pub use backend::{backend, is_available, DirectX12Backend, Dx12Adapter};
pub use device::Dx12Device;
pub use error::Dx12Error;

#[cfg(test)]
mod tests {
    use super::*;
    use render_core::Backend;

    #[test]
    fn dx12_backend_kind() {
        let backend = DirectX12Backend::new();
        assert_eq!(backend.kind(), render_core::BackendKind::DirectX12);
    }

    #[test]
    fn dx12_backend_enumerate_adapters() {
        let backend = DirectX12Backend::new();
        let result = backend.enumerate_adapters();
        // Should succeed (stub on non-Windows, real on Windows)
        assert!(result.is_ok());
    }

    #[test]
    fn dx12_backend_helper_creates_backend() {
        let b = backend();
        assert_eq!(b.kind(), render_core::BackendKind::DirectX12);
    }

    #[test]
    fn dx12_error_adapter_not_found_display() {
        let err = Dx12Error::AdapterNotFound;
        assert_eq!(err.to_string(), "no suitable DirectX 12 adapter found");
    }

    #[test]
    fn dx12_error_device_creation_failed_display() {
        let err = Dx12Error::DeviceCreationFailed("driver not found".to_string());
        assert_eq!(err.to_string(), "device creation failed: driver not found");
    }

    #[test]
    fn dx12_is_available_on_current_platform() {
        // is_available returns true only on Windows with the feature enabled
        #[cfg(all(target_os = "windows", feature = "backend-dx12"))]
        assert!(is_available());
        #[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
        assert!(!is_available());
    }
}
