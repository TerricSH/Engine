//! DirectX 12 backend — adapter, backend struct, enumeration, helpers.

use render_core::{
    AdapterInfo, Backend, BackendCapabilities, BackendKind, Device, DeviceDescriptor, RhiError,
    ResourceLimits, ShaderFormat, TextureFormat,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::Win32::Graphics::Dxgi::*;

use crate::device::Dx12Device;

// ============================================================================
// Adapter metadata
// ============================================================================

#[derive(Clone, Debug)]
pub struct Dx12Adapter {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub dedicated_memory: u64,
}

// ============================================================================
// Backend
// ============================================================================

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectX12Backend;

impl DirectX12Backend {
    pub const fn new() -> Self {
        Self
    }
}

impl Backend for DirectX12Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::DirectX12
    }

    #[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError> {
        Ok(vec![AdapterInfo {
            backend: BackendKind::DirectX12,
            name: "DirectX 12 (disabled — not on Windows)".to_string(),
            vendor_id: None,
            device_id: None,
            driver_version: None,
            capabilities: BackendCapabilities {
                max_texture_dimension_2d: 16384,
                max_color_attachments: 8,
                supports_swapchain: false,
                supports_timestamps: false,
                supports_debug_markers: false,
                supported_shader_formats: vec![ShaderFormat::Dxil, ShaderFormat::Hlsl],
                supported_surface_formats: vec![TextureFormat::Rgba8Unorm, TextureFormat::Bgra8Unorm],
                limits: ResourceLimits {
                    max_buffer_bytes: 256 * 1024 * 1024,
                    max_texture_array_layers: 256,
                    max_bind_groups: 4,
                    max_vertex_attributes: 16,
                    max_color_attachments: 8,
                    max_sample_count: 4,
                },
            },
        }])
    }

    #[cfg(all(target_os = "windows", feature = "backend-dx12"))]
    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError> {
        enumerate_adapters_impl()
    }

    #[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
    fn create_device(&self, _: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError> {
        Err(RhiError::Backend {
            detail: "DirectX 12 backend requires Windows and the `backend-dx12` feature".to_string(),
        })
    }

    #[cfg(all(target_os = "windows", feature = "backend-dx12"))]
    fn create_device(&self, descriptor: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError> {
        Dx12Device::create(descriptor).map(|d| Box::new(d) as Box<dyn Device>)
    }
}

// ============================================================================
// Platform-specific adapter enumeration
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
fn enumerate_adapters_impl() -> Result<Vec<AdapterInfo>, RhiError> {
    unsafe {
        let flags = DXGI_CREATE_FACTORY_FLAGS(0);
        let factory: IDXGIFactory2 =
            CreateDXGIFactory2(flags).map_err(|e| RhiError::Backend {
                detail: format!("DXGI: failed to create factory: {e}"),
            })?;

        let mut adapters = Vec::new();

        for i in 0.. {
            let result = factory.EnumAdapters1(i);
            let adapter = match result {
                Ok(a) => a,
                Err(_) => break,
            };

            let desc = match adapter.GetDesc1() {
                Ok(d) => d,
                Err(_) => continue,
            };

            let name = String::from_utf16_lossy(&desc.Description)
                .trim_end_matches('\0')
                .to_string();

            adapters.push(AdapterInfo {
                backend: BackendKind::DirectX12,
                name,
                vendor_id: Some(desc.VendorId),
                device_id: Some(desc.DeviceId),
                driver_version: None,
                capabilities: BackendCapabilities {
                    max_texture_dimension_2d: 16384,
                    max_color_attachments: 8,
                    supports_swapchain: true,
                    supports_timestamps: true,
                    supports_debug_markers: true,
                    supported_shader_formats: vec![ShaderFormat::Dxil, ShaderFormat::Hlsl],
                    supported_surface_formats: vec![
                        TextureFormat::Rgba8Unorm,
                        TextureFormat::Bgra8Unorm,
                        TextureFormat::Rgba16Float,
                    ],
                    limits: ResourceLimits {
                        max_buffer_bytes: 256 * 1024 * 1024,
                        max_texture_array_layers: 256,
                        max_bind_groups: 4,
                        max_vertex_attributes: 16,
                        max_color_attachments: 8,
                        max_sample_count: 4,
                    },
                },
            });
        }

        Ok(adapters)
    }
}

// ============================================================================
// is_available helper
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub fn is_available() -> bool {
    true
}

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
pub fn is_available() -> bool {
    false
}

pub fn backend() -> DirectX12Backend {
    DirectX12Backend::new()
}
