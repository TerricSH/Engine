#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::{Win32::Graphics::Direct3D12::*, Win32::Graphics::Dxgi::Common::*};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use render_core::{ShaderFormat, ShaderStage, TextureFormat};
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use std::sync::{Arc, Mutex};

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
    pub(crate) vertex_stride: u32,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12TextureInner {
    pub(crate) resource: ID3D12Resource,
    pub(crate) format: TextureFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) state: Arc<Mutex<D3D12_RESOURCE_STATES>>,
    pub(crate) sampled_srv_heap: Option<ID3D12DescriptorHeap>,
    pub(crate) sampled_sampler_heap: Option<ID3D12DescriptorHeap>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12ShaderModuleInner {
    pub(crate) format: ShaderFormat,
    pub(crate) stage: ShaderStage,
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
    pub(crate) rtv_heap: Option<ID3D12DescriptorHeap>,
    pub(crate) dsv_heap: Option<ID3D12DescriptorHeap>,
    pub(crate) rtv_descriptors: Vec<D3D12_CPU_DESCRIPTOR_HANDLE>,
    pub(crate) dsv_descriptor: Option<D3D12_CPU_DESCRIPTOR_HANDLE>,
    pub(crate) color_resources: Vec<ID3D12Resource>,
    pub(crate) color_states: Vec<Arc<Mutex<D3D12_RESOURCE_STATES>>>,
    pub(crate) color_is_sampled: Vec<bool>,
    pub(crate) depth_resource: Option<ID3D12Resource>,
    pub(crate) depth_is_sampled: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
}
