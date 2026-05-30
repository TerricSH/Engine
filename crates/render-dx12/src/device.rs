use render_core::{
    AdapterInfo, Backend, BackendCapabilities, BackendKind, BufferDescriptor, BufferHandle,
    CommandEncoder, Device, DeviceDescriptor, FramebufferDescriptor, FramebufferHandle,
    PipelineDescriptor, PipelineHandle, PipelineLayoutDescriptor, PipelineLayoutHandle,
    RenderPassDescriptor, RenderPassHandle, RendererStatistics, ResourceLimits, RhiError,
    ShaderFormat, ShaderModuleDescriptor, ShaderModuleHandle, SurfaceDescriptor, SurfaceHandle,
    SwapchainDescriptor, SwapchainHandle, TextureDescriptor, TextureFormat, TextureHandle,
};

// ============================================================================
// Adapter metadata
// ============================================================================

/// Metadata describing a DirectX 12 adapter (physical device).
#[derive(Clone, Debug)]
pub struct Dx12Adapter {
    /// Human-readable adapter name reported by the driver.
    pub name: String,
    /// PCI vendor identifier (e.g. 0x10DE = NVIDIA, 0x1002 = AMD, 0x8086 = Intel).
    pub vendor_id: u32,
    /// PCI device identifier.
    pub device_id: u32,
    /// Amount of dedicated video memory in bytes.
    pub dedicated_memory: u64,
}

// ============================================================================
// Backend
// ============================================================================

/// The DirectX 12 backend handle.
///
/// Because this is a stub, [`enumerate_adapters`](Self::enumerate_adapters)
/// returns one placeholder adapter and [`create_device`](Self::create_device)
/// always fails with a message explaining what dependencies are needed.
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

    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError> {
        // Since we cannot call DXGI without the windows-rs crate, we return a
        // single placeholder adapter with sensible defaults.  Real enumeration
        // would use IDXGIFactory::EnumAdapters.
        Ok(vec![AdapterInfo {
            backend: BackendKind::DirectX12,
            name: "DirectX 12 Adapter (placeholder)".to_string(),
            vendor_id: Some(0x1414), // Microsoft basic render driver vendor ID
            device_id: Some(0),
            driver_version: None,
            capabilities: BackendCapabilities {
                max_texture_dimension_2d: 16384,
                max_color_attachments: 8,
                supports_swapchain: false,
                supports_timestamps: false,
                supports_debug_markers: false,
                supported_shader_formats: vec![ShaderFormat::Dxil, ShaderFormat::Hlsl],
                supported_surface_formats: vec![
                    TextureFormat::Rgba8Unorm,
                    TextureFormat::Bgra8Unorm,
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
        }])
    }

    fn create_device(&self, _descriptor: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError> {
        Err(RhiError::Backend {
            detail: "\
                DirectX 12 backend requires the windows/d3d12 crate which is not yet \
                added to workspace dependencies. To enable DX12 support, add the \
                following to the workspace Cargo.toml:\n\n\
                windows = { version = \"0.58\", features = [\n    \
                    \"Win32_Graphics_Direct3D12\",\n    \
                    \"Win32_Graphics_Dxgi\",\n] }\
            "
            .to_string(),
        })
    }
}

// ============================================================================
// Device
// ============================================================================

/// A stub DirectX 12 device.
///
/// All resource-creation and frame-lifecycle methods return
/// [`RhiError::Backend`] explaining that the DX12 backend is not
/// functional without the windows/d3d12 crate.
pub struct Dx12Device {
    info: AdapterInfo,
}

impl Dx12Device {
    pub fn new(info: AdapterInfo) -> Self {
        Self { info }
    }
}

impl Device for Dx12Device {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.info
    }

    fn create_surface(
        &mut self,
        _descriptor: &SurfaceDescriptor,
    ) -> Result<SurfaceHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn create_swapchain(
        &mut self,
        _descriptor: &SwapchainDescriptor,
    ) -> Result<SwapchainHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn create_buffer(&mut self, _descriptor: &BufferDescriptor) -> Result<BufferHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn write_buffer(
        &mut self,
        _buffer: BufferHandle,
        _data: &[u8],
        _offset: u64,
    ) -> Result<(), RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn create_texture(
        &mut self,
        _descriptor: &TextureDescriptor,
    ) -> Result<TextureHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn create_shader_module(
        &mut self,
        _descriptor: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn create_render_pass(
        &mut self,
        _descriptor: &RenderPassDescriptor,
    ) -> Result<RenderPassHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn create_framebuffer(
        &mut self,
        _descriptor: &FramebufferDescriptor,
    ) -> Result<FramebufferHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn create_pipeline_layout(
        &mut self,
        _descriptor: &PipelineLayoutDescriptor,
    ) -> Result<PipelineLayoutHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn create_pipeline(
        &mut self,
        _descriptor: &PipelineDescriptor,
    ) -> Result<PipelineHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn begin_frame(
        &mut self,
        _swapchain: SwapchainHandle,
    ) -> Result<(u32, Box<dyn CommandEncoder>), RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn end_frame(
        &mut self,
        _swapchain: SwapchainHandle,
        _encoder: Box<dyn CommandEncoder>,
        _image_index: u32,
    ) -> Result<RendererStatistics, RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }

    fn recreate_swapchain(
        &mut self,
        _swapchain: SwapchainHandle,
        _width: u32,
        _height: u32,
    ) -> Result<(), RhiError> {
        Err(RhiError::Backend {
            detail: "DX12 backend: not implemented without windows-rs".to_string(),
        })
    }
}

// ============================================================================
// Public helpers
// ============================================================================

/// Returns `false` – DX12 support requires the `windows` / `d3d12` crate
/// which is not yet added to the workspace dependencies.
pub fn is_available() -> bool {
    false
}

/// Returns a new [`DirectX12Backend`] handle.
pub fn backend() -> DirectX12Backend {
    DirectX12Backend::new()
}
