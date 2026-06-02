#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::{
    Win32::Graphics::Dxgi::Common::*,
    Win32::Graphics::Direct3D12::*,
};

use render_core::{ShaderFormat, TextureFormat};

// ============================================================================
// Internal resource types
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12BufferInner {
    pub(crate) resource: ID3D12Resource,
    pub(crate) upload_resource: Option<ID3D12Resource>,
    pub(crate) size: u64,
    pub(crate) state: D3D12_RESOURCE_STATES,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12TextureInner {
    pub(crate) resource: ID3D12Resource,
    pub(crate) format: TextureFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) state: D3D12_RESOURCE_STATES,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12ShaderModuleInner {
    pub(crate) format: ShaderFormat,
    pub(crate) entry_points: Vec<String>,
    pub(crate) source_hash: [u8; 32],
    pub(crate) bytecode: Vec<u8>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12RenderPassInner {
    pub(crate) color_formats: Vec<DXGI_FORMAT>,
    pub(crate) depth_format: Option<DXGI_FORMAT>,
    pub(crate) sample_count: u8,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12FramebufferInner {
    pub(crate) rtv_descriptors: Vec<D3D12_CPU_DESCRIPTOR_HANDLE>,
    pub(crate) dsv_descriptor: Option<D3D12_CPU_DESCRIPTOR_HANDLE>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}
