#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::{
    Win32::Foundation::HWND,
    Win32::Graphics::Dxgi::*,
    Win32::Graphics::Direct3D12::*,
};

use render_core::TextureFormat;

// ============================================================================
// Internal resource types
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(crate) struct Dx12SwapchainInner {
    pub(crate) swapchain: IDXGISwapChain3,
    pub(crate) back_buffers: Vec<ID3D12Resource>,
    pub(crate) rtv_heap: ID3D12DescriptorHeap,
    pub(crate) rtv_size: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(crate) struct Dx12SurfaceInner {
    #[allow(dead_code)]
    pub(crate) hwnd: HWND,
    pub(crate) format: TextureFormat,
}
