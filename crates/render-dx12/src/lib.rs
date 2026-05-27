#![forbid(unsafe_code)]

//! Stub DirectX 12 backend implementation.
//!
//! This crate provides a compile-safe placeholder for the DirectX 12 backend.
//! No platform-specific FFI or windows-rs dependency is used at this point.
//! All resource-creation methods return an error explaining that the DX12
//! backend requires the windows/d3d12 crates to be added to the workspace.
//!
//! # Enabling real DX12 support
//!
//! To enable actual DirectX 12 functionality, add the following to the
//! workspace `Cargo.toml`:
//!
//! ```toml
//! windows = { version = "0.58", features = [
//!     "Win32_Graphics_Direct3D12",
//!     "Win32_Graphics_Dxgi",
//!     "Win32_System_Threading",
//! ] }
//! ```
//!
//! Then implement `Dx12Device` methods using the `d3d12` and `dxgi` crates
//! or the raw `windows` crate bindings.

mod device;
mod error;

pub use device::{DirectX12Backend, Dx12Adapter, Dx12Device, backend, is_available};
pub use error::Dx12Error;

#[cfg(test)]
mod tests {
    use super::*;
    use render_core::Backend;

    // ── Dx12Error tests ──────────────────────────────────────────────────

    #[test]
    fn dx12_error_adapter_not_found_display() {
        let err = Dx12Error::AdapterNotFound;
        assert_eq!(err.to_string(), "no suitable DirectX 12 adapter found");
    }

    #[test]
    fn dx12_error_device_creation_failed_display() {
        let err = Dx12Error::DeviceCreationFailed("driver not found".to_string());
        assert_eq!(
            err.to_string(),
            "device creation failed: driver not found"
        );
    }

    #[test]
    fn dx12_error_unsupported_format_display() {
        let err = Dx12Error::UnsupportedFormat("Rgba32Float".to_string());
        assert_eq!(err.to_string(), "unsupported format: Rgba32Float");
    }

    #[test]
    fn dx12_error_debug() {
        let err = Dx12Error::AdapterNotFound;
        let debug = format!("{:?}", err);
        assert!(debug.contains("AdapterNotFound"));
    }

    // ── Dx12Adapter tests ────────────────────────────────────────────────

    #[test]
    fn dx12_adapter_construction() {
        let adapter = Dx12Adapter {
            name: "NVIDIA GeForce RTX 4090".to_string(),
            vendor_id: 0x10DE,
            device_id: 0x2684,
            dedicated_memory: 24 * 1024 * 1024 * 1024,
        };
        assert_eq!(adapter.name, "NVIDIA GeForce RTX 4090");
        assert_eq!(adapter.vendor_id, 0x10DE);
        assert_eq!(adapter.device_id, 0x2684);
        assert_eq!(adapter.dedicated_memory, 24 * 1024 * 1024 * 1024);
    }

    #[test]
    fn dx12_adapter_debug() {
        let adapter = Dx12Adapter {
            name: "Test GPU".to_string(),
            vendor_id: 0,
            device_id: 0,
            dedicated_memory: 0,
        };
        let debug = format!("{:?}", adapter);
        assert!(debug.contains("Dx12Adapter"));
        assert!(debug.contains("Test GPU"));
    }

    // ── is_available tests ───────────────────────────────────────────────

    #[test]
    fn dx12_is_available_returns_false() {
        assert!(!is_available());
    }

    // ── DirectX12Backend tests ───────────────────────────────────────────

    #[test]
    fn dx12_backend_kind() {
        let backend = DirectX12Backend::new();
        assert_eq!(backend.kind(), render_core::BackendKind::DirectX12);
    }

    #[test]
    fn dx12_backend_enumerate_adapters_returns_one() {
        let backend = DirectX12Backend::new();
        let adapters = backend.enumerate_adapters().unwrap();
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].backend, render_core::BackendKind::DirectX12);
    }

    #[test]
    fn dx12_backend_create_device_fails() {
        let backend = DirectX12Backend::new();
        let descriptor = render_core::DeviceDescriptor {
            adapter: render_core::AdapterInfo {
                backend: render_core::BackendKind::DirectX12,
                name: "test".to_string(),
                vendor_id: None,
                device_id: None,
                driver_version: None,
                capabilities: render_core::BackendCapabilities::default(),
            },
            required_features: vec![],
            required_limits: render_core::ResourceLimits::default(),
            debug_label: None,
            validation_mode: render_core::ValidationMode::Disabled,
        };
        let result = backend.create_device(&descriptor);
        assert!(result.is_err());
    }

    #[test]
    fn dx12_backend_debug() {
        let backend = DirectX12Backend::new();
        let debug = format!("{:?}", backend);
        assert!(debug.contains("DirectX12Backend"));
    }

    // ── backend() helper test ────────────────────────────────────────────

    #[test]
    fn dx12_backend_helper_creates_backend() {
        let b = backend();
        assert_eq!(b.kind(), render_core::BackendKind::DirectX12);
    }
}
