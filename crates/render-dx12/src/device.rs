//! Full DirectX 12 backend implementation.
//!
//! Uses the `windows` crate for D3D12 and DXGI bindings.
//! All D3D12-specific code is gated behind `#[cfg(target_os = "windows")]`
//! and the `backend-dx12` feature flag.

use render_core::{AdapterInfo, Device};
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use render_core::{
    BackendCapabilities, BackendKind, BufferDescriptor, BufferHandle, BufferUsage, CommandEncoder,
    DeviceDescriptor, FramebufferDescriptor, FramebufferHandle, MemoryHint, PipelineDescriptor,
    PipelineHandle, PipelineLayoutDescriptor, PipelineLayoutHandle, RenderPassDescriptor,
    RenderPassHandle, RendererStatistics, ResourceLimits, RhiError, ShaderFormat,
    ShaderModuleDescriptor, ShaderModuleHandle, ShaderStage, SurfaceDescriptor, SurfaceHandle,
    SwapchainDescriptor, SwapchainHandle, TextureDescriptor, TextureFormat, TextureHandle,
    TextureUsage,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::{
    core::Interface,
    Win32::Foundation::{BOOL, FALSE, HANDLE, HWND, TRUE},
    Win32::Graphics::Direct3D::*,
    Win32::Graphics::Direct3D12::*,
    Win32::Graphics::Dxgi::Common::*,
    Win32::Graphics::Dxgi::*,
    Win32::System::Threading::{CreateEventA, WaitForSingleObject},
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::encoder::Dx12CommandEncoder;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::handle::HandleTable;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::pipeline::{Dx12PipelineInner, Dx12PipelineLayoutInner};
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::resources::{
    Dx12BufferInner, Dx12FramebufferInner, Dx12RenderPassInner, Dx12ShaderModuleInner,
    Dx12TextureInner,
};
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::swapchain::{Dx12SurfaceInner, Dx12SwapchainInner};

// ============================================================================
// Dx12Device — full implementation
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub struct Dx12Device {
    pub(crate) info: AdapterInfo,
    pub(crate) device: ID3D12Device,
    pub(crate) queue: ID3D12CommandQueue,
    pub(crate) allocators: Vec<ID3D12CommandAllocator>,
    pub(crate) cmd_lists: Vec<ID3D12GraphicsCommandList>,
    pub(crate) fence: ID3D12Fence,
    pub(crate) fence_event: HANDLE,
    pub(crate) fence_value: u64,
    pub(crate) frame_index: usize,
    pub(crate) next_frame_clear_color: [f32; 4],
    pub(crate) descriptor_heaps_in_flight: std::sync::Mutex<Vec<ID3D12DescriptorHeap>>,
    // Handle tables
    pub(crate) buffers: HandleTable<Dx12BufferInner>,
    pub(crate) textures: HandleTable<Dx12TextureInner>,
    pub(crate) shader_modules: HandleTable<Dx12ShaderModuleInner>,
    pub(crate) render_passes: HandleTable<Dx12RenderPassInner>,
    pub(crate) framebuffers: HandleTable<Dx12FramebufferInner>,
    pub(crate) pipeline_layouts: HandleTable<Dx12PipelineLayoutInner>,
    pub(crate) pipelines: HandleTable<Dx12PipelineInner>,
    pub(crate) swapchains: HandleTable<Dx12SwapchainInner>,
    pub(crate) surfaces: HandleTable<Dx12SurfaceInner>,
    // Generation counters for handles
    pub(crate) gen_buffer: u32,
    pub(crate) gen_texture: u32,
    pub(crate) gen_shader: u32,
    pub(crate) gen_pass: u32,
    pub(crate) gen_fb: u32,
    pub(crate) gen_layout: u32,
    pub(crate) gen_pipeline: u32,
    pub(crate) gen_swapchain: u32,
    pub(crate) gen_surface: u32,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
unsafe impl Send for Dx12Device {}
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
unsafe impl Sync for Dx12Device {}

mod platform;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod trait_frame;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod trait_pipeline;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod trait_resources;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use trait_frame::dx12_device_frame_methods;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use trait_pipeline::dx12_device_pipeline_methods;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use trait_resources::dx12_device_resource_methods;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Device for Dx12Device {
    dx12_device_resource_methods!();
    dx12_device_pipeline_methods!();
    dx12_device_frame_methods!();
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12Device {
    /// Close the command list for the most recently begun frame without
    /// executing it or presenting its swapchain image.
    ///
    /// The recorded PRESENT -> RENDER_TARGET transition was never submitted,
    /// so the real back buffer remains in PRESENT state. Closing the list is
    /// required before its allocator/list can be reset by a later frame.
    pub(crate) fn abort_frame(
        &mut self,
        _encoder: Box<dyn CommandEncoder>,
    ) -> Result<(), RhiError> {
        let frame = (self.frame_index + Self::FRAMES_IN_FLIGHT - 1) % Self::FRAMES_IN_FLIGHT;
        unsafe {
            self.cmd_lists[frame]
                .Close()
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: Close aborted command list failed: {error}"),
                })?;
        }
        Ok(())
    }
}

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
pub struct Dx12Device {
    info: AdapterInfo,
}

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
impl Dx12Device {
    pub fn new(info: AdapterInfo) -> Self {
        Self { info }
    }

    fn discard_unavailable<T>(&self, _handle: T, resource: &'static str) {
        tracing::debug!(
            resource,
            "ignoring destruction request because the DX12 backend is unavailable on this target"
        );
    }
}

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
impl Device for Dx12Device {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.info
    }

    fn destroy_buffer(&mut self, handle: render_core::BufferHandle) {
        self.discard_unavailable(handle, "buffer");
    }

    fn destroy_texture(&mut self, handle: render_core::TextureHandle) {
        self.discard_unavailable(handle, "texture");
    }

    fn destroy_shader_module(&mut self, handle: render_core::ShaderModuleHandle) {
        self.discard_unavailable(handle, "shader module");
    }

    fn destroy_render_pass(&mut self, handle: render_core::RenderPassHandle) {
        self.discard_unavailable(handle, "render pass");
    }

    fn destroy_framebuffer(&mut self, handle: render_core::FramebufferHandle) {
        self.discard_unavailable(handle, "framebuffer");
    }

    fn destroy_pipeline_layout(&mut self, handle: render_core::PipelineLayoutHandle) {
        self.discard_unavailable(handle, "pipeline layout");
    }

    fn destroy_pipeline(&mut self, handle: render_core::PipelineHandle) {
        self.discard_unavailable(handle, "pipeline");
    }

    fn destroy_swapchain(&mut self, handle: render_core::SwapchainHandle) {
        self.discard_unavailable(handle, "swapchain");
    }

    fn destroy_surface(&mut self, handle: render_core::SurfaceHandle) {
        self.discard_unavailable(handle, "surface");
    }

    fn wait_idle(&self) {
        tracing::debug!("DX12 wait_idle has no work because the backend is unavailable");
    }
}

// ============================================================================
// Helper: attribute format to DXGI
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12Device {
    pub(crate) fn attribute_format_to_dxgi(format: &str) -> DXGI_FORMAT {
        match format {
            "float3" | "float32x3" => DXGI_FORMAT_R32G32B32_FLOAT,
            "float2" | "float32x2" => DXGI_FORMAT_R32G32_FLOAT,
            "float4" | "float32x4" => DXGI_FORMAT_R32G32B32A32_FLOAT,
            "uint4" | "uint32x4" => DXGI_FORMAT_R32G32B32A32_UINT,
            "uint" | "uint32" => DXGI_FORMAT_R32_UINT,
            "float" | "float32" => DXGI_FORMAT_R32_FLOAT,
            "unorm8x4" | "rgba8" => DXGI_FORMAT_R8G8B8A8_UNORM,
            "unorm8x3" | "rgb8" => DXGI_FORMAT_R8G8B8A8_UNORM,
            _ => DXGI_FORMAT_R32G32B32_FLOAT,
        }
    }
}
