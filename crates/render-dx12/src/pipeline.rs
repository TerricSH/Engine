#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::Win32::Graphics::Direct3D12::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12PipelineLayoutInner {
    pub(crate) root_signature: ID3D12RootSignature,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
pub(crate) struct Dx12PipelineInner {
    pub(crate) pso: ID3D12PipelineState,
    pub(crate) topology: u32,
}
