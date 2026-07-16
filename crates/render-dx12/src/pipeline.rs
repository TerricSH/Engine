#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::Win32::Graphics::Direct3D12::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12PipelineLayoutInner {
    pub(crate) root_signature: ID3D12RootSignature,
    pub(crate) root_constants_parameter: Option<u32>,
    pub(crate) sampled_texture_parameter: Option<u32>,
    pub(crate) sampler_parameter: Option<u32>,
    pub(crate) uniform_buffer_parameter: Option<u32>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12PipelineInner {
    pub(crate) pso: ID3D12PipelineState,
    pub(crate) topology: u32,
}
