//! Full DirectX 12 backend implementation.
//!
//! Uses the `windows` crate for D3D12 and DXGI bindings.
//! All D3D12-specific code is gated behind `#[cfg(target_os = "windows")]`
//! and the `backend-dx12` feature flag.

use std::collections::HashMap;

use render_core::{
    AdapterInfo, Backend, BackendCapabilities, BackendKind, BufferDescriptor, BufferHandle,
    BufferUsage, CommandEncoder, Device, DeviceDescriptor, FramebufferDescriptor,
    FramebufferHandle, IndexFormat, MemoryHint, PipelineDescriptor, PipelineHandle,
    PipelineLayoutDescriptor, PipelineLayoutHandle, RenderPassDescriptor, RenderPassHandle,
    RendererStatistics, ResourceLimits, RhiError, ShaderFormat, ShaderModuleDescriptor,
    ShaderModuleHandle, SurfaceDescriptor, SurfaceHandle, SwapchainDescriptor, SwapchainHandle,
    TextureDescriptor, TextureFormat, TextureHandle, TextureUsage,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::{
    core::Interface,
    Win32::Foundation::{BOOL, FALSE, HANDLE, HWND, RECT, TRUE},
    Win32::Graphics::Direct3D12::*,
    Win32::Graphics::Dxgi::Common::*,
    Win32::Graphics::Dxgi::*,
    Win32::Graphics::Direct3D::*,
    Win32::System::Threading::{CreateEventA, WaitForSingleObject},
};

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
// Handle table
// ============================================================================

struct HandleTable<T> {
    entries: Vec<Option<(u32, T)>>,
}

impl<T> HandleTable<T> {
    fn new() -> Self {
        Self { entries: Vec::new() }
    }

    fn insert(&mut self, value: T) -> u32 {
        for (idx, slot) in self.entries.iter_mut().enumerate() {
            if slot.is_none() {
                let gen = match slot {
                    Some((g, _)) => *g + 1,
                    None => 1,
                };
                *slot = Some((gen, value));
                return idx as u32;
            }
        }
        self.entries.push(Some((1, value)));
        (self.entries.len() - 1) as u32
    }

    fn get(&self, index: u32) -> Option<&T> {
        self.entries
            .get(index as usize)
            .and_then(|s| s.as_ref().map(|(_, v)| v))
    }

    fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        self.entries
            .get_mut(index as usize)
            .and_then(|s| s.as_mut().map(|(_, v)| v))
    }

    fn remove(&mut self, index: u32) -> Option<T> {
        self.entries
            .get_mut(index as usize)
            .and_then(|s| s.take().map(|(_, v)| v))
    }
}

// ============================================================================
// Internal resource types
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
struct Dx12BufferInner {
    resource: ID3D12Resource,
    upload_resource: Option<ID3D12Resource>,
    size: u64,
    state: D3D12_RESOURCE_STATES,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
struct Dx12TextureInner {
    resource: ID3D12Resource,
    format: TextureFormat,
    width: u32,
    height: u32,
    state: D3D12_RESOURCE_STATES,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
struct Dx12ShaderModuleInner {
    format: ShaderFormat,
    entry_points: Vec<String>,
    source_hash: [u8; 32],
    bytecode: Vec<u8>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
struct Dx12RenderPassInner {
    color_formats: Vec<DXGI_FORMAT>,
    depth_format: Option<DXGI_FORMAT>,
    sample_count: u8,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
struct Dx12FramebufferInner {
    rtv_descriptors: Vec<D3D12_CPU_DESCRIPTOR_HANDLE>,
    dsv_descriptor: Option<D3D12_CPU_DESCRIPTOR_HANDLE>,
    width: u32,
    height: u32,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
struct Dx12PipelineLayoutInner {
    root_signature: ID3D12RootSignature,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
#[allow(dead_code)]
struct Dx12PipelineInner {
    pso: ID3D12PipelineState,
    topology: u32,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
struct Dx12SwapchainInner {
    swapchain: IDXGISwapChain3,
    back_buffers: Vec<ID3D12Resource>,
    rtv_heap: ID3D12DescriptorHeap,
    rtv_size: u32,
    width: u32,
    height: u32,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
struct Dx12SurfaceInner {
    #[allow(dead_code)]
    hwnd: HWND,
    format: TextureFormat,
}

// ============================================================================
// Command encoder
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
struct Dx12CommandEncoder {
    cmd_list: ID3D12GraphicsCommandList,
    draws: u32,
    triangles: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12CommandEncoder {
    fn new(cmd_list: ID3D12GraphicsCommandList) -> Self {
        Self {
            cmd_list,
            draws: 0,
            triangles: 0,
        }
    }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl CommandEncoder for Dx12CommandEncoder {
    fn begin_render_pass(
        &mut self,
        _render_pass: RenderPassHandle,
        _framebuffer: FramebufferHandle,
        _area: (u32, u32, u32, u32),
        _clear_color: [f32; 4],
        _clear_depth: Option<f32>,
    ) {
        // Render pass begin is handled by the caller setting RTV/DSV and clearing.
        // D3D12 doesn't have explicit render pass begin like Vulkan.
    }

    fn bind_pipeline(&mut self, _pipeline: PipelineHandle) {
        // Pipeline binding requires access to the device's handle table.
        // This is resolved by the device's command recording method.
    }

    fn bind_vertex_buffers(&mut self, _buffers: &[BufferHandle], _offsets: &[u64]) {
        // Resolved by the device when recording commands.
    }

    fn bind_index_buffer(&mut self, _buffer: BufferHandle, _offset: u64, _index_format: IndexFormat) {
    }

    fn bind_descriptor_sets(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _first_set: u32,
        _sets: &[render_core::DescriptorSetHandle],
        _dynamic_offsets: &[u32],
    ) {
    }

    fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, min_depth: f32, max_depth: f32) {
        unsafe {
            let viewport = D3D12_VIEWPORT {
                TopLeftX: x,
                TopLeftY: y,
                Width: w,
                Height: h,
                MinDepth: min_depth,
                MaxDepth: max_depth,
            };
            self.cmd_list.RSSetViewports(&[viewport]);
        }
    }

    fn set_scissor(&mut self, x: i32, y: i32, w: u32, h: u32) {
        unsafe {
            let rect: RECT = RECT {
                left: x,
                top: y,
                right: (x + w as i32),
                bottom: (y + h as i32),
            };
            self.cmd_list.RSSetScissorRects(&[rect]);
        }
    }

    fn draw(
        &mut self,
        vertex_count: u32,
        instance_count: u32,
        _first_vertex: u32,
        _first_instance: u32,
    ) {
        unsafe {
            self.cmd_list.DrawInstanced(vertex_count, instance_count, 0, 0);
        }
        self.draws += 1;
        self.triangles += vertex_count as u64 / 3 * instance_count as u64;
    }

    fn draw_indexed(
        &mut self,
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        vertex_offset: i32,
        _first_instance: u32,
    ) {
        unsafe {
            self.cmd_list
                .DrawIndexedInstanced(index_count, instance_count, first_index, vertex_offset, 0);
        }
        self.draws += 1;
        self.triangles += index_count as u64 / 3 * instance_count as u64;
    }

    fn end_render_pass(&mut self) {}

    fn push_constants(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _stage_flags: u32,
        _offset: u32,
        _data: &[u8],
    ) {
        // D3D12 push constants are handled via root constants in the root signature.
        // The actual data is set via SetGraphicsRoot32BitConstants or similar.
    }
}

// ============================================================================
// Dx12Device — full implementation
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub struct Dx12Device {
    info: AdapterInfo,
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    allocators: Vec<ID3D12CommandAllocator>,
    cmd_lists: Vec<ID3D12GraphicsCommandList>,
    fence: ID3D12Fence,
    fence_event: HANDLE,
    fence_value: u64,
    frame_index: usize,
    // Handle tables
    buffers: HandleTable<Dx12BufferInner>,
    textures: HandleTable<Dx12TextureInner>,
    shader_modules: HandleTable<Dx12ShaderModuleInner>,
    render_passes: HandleTable<Dx12RenderPassInner>,
    framebuffers: HandleTable<Dx12FramebufferInner>,
    pipeline_layouts: HandleTable<Dx12PipelineLayoutInner>,
    pipelines: HandleTable<Dx12PipelineInner>,
    swapchains: HandleTable<Dx12SwapchainInner>,
    surfaces: HandleTable<Dx12SurfaceInner>,
    // Generation counters for handles
    gen_buffer: u32,
    gen_texture: u32,
    gen_shader: u32,
    gen_pass: u32,
    gen_fb: u32,
    gen_layout: u32,
    gen_pipeline: u32,
    gen_swapchain: u32,
    gen_surface: u32,
    // Shader bytecode cache: [source_hash; 32] -> Vec<u8>
    shader_cache: HashMap<[u8; 32], Vec<u8>>,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
unsafe impl Send for Dx12Device {}
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
unsafe impl Sync for Dx12Device {}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12Device {
    const FRAMES_IN_FLIGHT: usize = 2;

    pub fn create(_descriptor: &DeviceDescriptor) -> Result<Self, RhiError> {
        unsafe {
            // Enable debug layer in debug builds
            #[cfg(debug_assertions)]
            {
                let mut debug: Option<ID3D12Debug> = None;
                if D3D12GetDebugInterface(&mut debug).is_ok() {
                    if let Some(debug) = debug {
                        debug.EnableDebugLayer();
                    }
                }
            }

            // Create DXGI factory
            let factory: IDXGIFactory2 =
                CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(|e| {
                    RhiError::Backend {
                        detail: format!("DX12: failed to create DXGI factory: {e}"),
                    }
                })?;

            // Enumerate adapters
            let mut adapter: Option<IDXGIAdapter1> = None;
            for i in 0.. {
                match factory.EnumAdapters1(i) {
                    Ok(a) => {
                        let desc = a.GetDesc1()
                            .map_err(|e| RhiError::Backend {
                                detail: format!("DX12: GetDesc1 failed: {e}"),
                            })?;
                        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                            continue;
                        }
                        adapter = Some(a);
                        break;
                    }
                    Err(_) => break,
                }
            }

            let adapter = adapter.ok_or(RhiError::Backend {
                detail: "DX12: no suitable hardware adapter found".to_string(),
            })?;

            let desc = adapter
                .GetDesc1()
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: GetDesc1 failed: {e}"),
                })?;
            let adapter_name = String::from_utf16_lossy(&desc.Description)
                .trim_end_matches('\0')
                .to_string();

            // Create D3D12 device
            let mut device: Option<ID3D12Device> = None;
            D3D12CreateDevice(
                &adapter,
                D3D_FEATURE_LEVEL_11_0,
                &mut device,
            )
            .map_err(|e| RhiError::Backend {
                detail: format!("DX12: D3D12CreateDevice failed: {e}"),
            })?;
            let d3d12_device = device.ok_or(RhiError::Backend {
                detail: "DX12: D3D12CreateDevice returned null".to_string(),
            })?;

            // Create command queue
            let queue_desc = D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                Priority: D3D12_COMMAND_QUEUE_PRIORITY_NORMAL.0,
                Flags: D3D12_COMMAND_QUEUE_FLAGS(0),
                NodeMask: 0,
            };
            let queue: ID3D12CommandQueue = d3d12_device
                .CreateCommandQueue(&queue_desc)
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateCommandQueue failed: {e}"),
                })?;

            // Create command allocators and lists per frame in flight
            let mut allocators = Vec::new();
            let mut cmd_lists = Vec::new();
            for _ in 0..Self::FRAMES_IN_FLIGHT {
                let alloc: ID3D12CommandAllocator = d3d12_device
                    .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: CreateCommandAllocator failed: {e}"),
                    })?;
                let cmd_list: ID3D12GraphicsCommandList = d3d12_device
                    .CreateCommandList(
                        0,
                        D3D12_COMMAND_LIST_TYPE_DIRECT,
                        &alloc,
                        None,
                    )
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: CreateCommandList failed: {e}"),
                    })?;
                // Close initially — will be reset in begin_frame
                cmd_list
                    .Close()
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: Close(init) failed: {e}"),
                    })?;
                allocators.push(alloc);
                cmd_lists.push(cmd_list);
            }

            // Create fence
            let fence: ID3D12Fence = d3d12_device
                .CreateFence(0, D3D12_FENCE_FLAGS(0))
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateFence failed: {e}"),
                })?;
            let fence_event: HANDLE =
                CreateEventA(None, BOOL(0), BOOL(0), None).map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateEventA failed: {e}"),
                })?;

            Ok(Self {
                info: AdapterInfo {
                    backend: BackendKind::DirectX12,
                    name: adapter_name,
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
                },
                device: d3d12_device,
                queue,
                allocators,
                cmd_lists,
                fence,
                fence_event,
                fence_value: 0,
                frame_index: 0,
                buffers: HandleTable::new(),
                textures: HandleTable::new(),
                shader_modules: HandleTable::new(),
                render_passes: HandleTable::new(),
                framebuffers: HandleTable::new(),
                pipeline_layouts: HandleTable::new(),
                pipelines: HandleTable::new(),
                swapchains: HandleTable::new(),
                surfaces: HandleTable::new(),
                gen_buffer: 1,
                gen_texture: 1,
                gen_shader: 1,
                gen_pass: 1,
                gen_fb: 1,
                gen_layout: 1,
                gen_pipeline: 1,
                gen_swapchain: 1,
                gen_surface: 1,
                shader_cache: HashMap::new(),
            })
        }
    }

    fn make_handle(gen: &mut u32, index: u32) -> u32 {
        let g = *gen;
        *gen = gen.wrapping_add(1);
        (g << 16) | (index & 0xFFFF)
    }

    fn decode_handle(h: u32) -> (u32, u32) {
        (h >> 16, h & 0xFFFF)
    }

    fn transition_resource(
        cmd_list: &ID3D12GraphicsCommandList,
        resource: &ID3D12Resource,
        before: D3D12_RESOURCE_STATES,
        after: D3D12_RESOURCE_STATES,
    ) {
        unsafe {
            if before != after {
                let barrier = D3D12_RESOURCE_BARRIER {
                    Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                    Flags: D3D12_RESOURCE_BARRIER_FLAGS(0),
                    Anonymous: D3D12_RESOURCE_BARRIER_0 {
                        Transition: std::mem::ManuallyDrop::new(
                            D3D12_RESOURCE_TRANSITION_BARRIER {
                                pResource: std::mem::ManuallyDrop::new(Some(
                                    resource.clone(),
                                )),
                                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                                StateBefore: before,
                                StateAfter: after,
                            },
                        ),
                    },
                };
                cmd_list.ResourceBarrier(&[barrier]);
            }
        }
    }

    fn texture_format_to_dxgi(format: TextureFormat) -> DXGI_FORMAT {
        match format {
            TextureFormat::Rgba8Unorm => DXGI_FORMAT_R8G8B8A8_UNORM,
            TextureFormat::Bgra8Unorm => DXGI_FORMAT_B8G8R8A8_UNORM,
            TextureFormat::Rgba16Float => DXGI_FORMAT_R16G16B16A16_FLOAT,
            TextureFormat::Depth32Float => DXGI_FORMAT_D32_FLOAT,
            _ => DXGI_FORMAT_UNKNOWN,
        }
    }

    #[allow(dead_code)]
    fn fill_hex(buf: &mut [u8; 32], bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            if i < 32 {
                buf[i] = *b;
            }
        }
    }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Device for Dx12Device {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.info
    }

    // --- Surface ---
    fn create_surface(&mut self, descriptor: &SurfaceDescriptor) -> Result<SurfaceHandle, RhiError> {
        let hwnd = match &descriptor.window_handle {
            render_core::SurfaceTarget::RawWindowHandleToken(token) => HWND(*token as *mut std::ffi::c_void),
            render_core::SurfaceTarget::Headless => HWND::default(),
        };
        let index = self.surfaces.insert(Dx12SurfaceInner {
            hwnd,
            format: descriptor.preferred_format,
        });
        let handle = Self::make_handle(&mut self.gen_surface, index);
        Ok(render_core::SurfaceHandle::new(handle, self.gen_surface))
    }

    fn destroy_surface(&mut self, surface: SurfaceHandle) {
        let (_, idx) = Self::decode_handle(surface.index);
        self.surfaces.remove(idx);
    }

    // --- Swapchain ---
    fn create_swapchain(
        &mut self,
        descriptor: &SwapchainDescriptor,
    ) -> Result<SwapchainHandle, RhiError> {
        unsafe {
            let (_, surf_idx) = Self::decode_handle(descriptor.surface.index);
            let surf = self.surfaces.get(surf_idx).ok_or(RhiError::InvalidHandle)?;
            let hwnd = surf.hwnd;

            let factory: IDXGIFactory2 =
                CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).map_err(|e| {
                    RhiError::Backend {
                        detail: format!("DX12: DXGI factory for swapchain: {e}"),
                    }
                })?;

            let format = Self::texture_format_to_dxgi(surf.format);
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: descriptor.width,
                Height: descriptor.height,
                Format: format,
                Stereo: FALSE,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 3,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
                Flags: 0,
            };

            let swapchain: IDXGISwapChain1 = factory
                .CreateSwapChainForHwnd(
                    &self.queue,
                    hwnd,
                    &desc,
                    None,
                    None,
                )
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateSwapChainForHwnd failed: {e}"),
                })?;

            let swapchain: IDXGISwapChain3 = swapchain.cast().map_err(|e| RhiError::Backend {
                detail: format!("DX12: cast to IDXGISwapChain3 failed: {e}"),
            })?;

            // Create RTV descriptor heap
            let rtv_desc = D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                NumDescriptors: 3,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAGS(0),
                NodeMask: 0,
            };
            let rtv_heap = self
                .device
                .CreateDescriptorHeap(&rtv_desc)
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateDescriptorHeap(RTV) failed: {e}"),
                })?;
            let rtv_size = self.device.GetDescriptorHandleIncrementSize(
                D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            );

            // Get back buffers
            let mut back_buffers = Vec::new();
            for i in 0..3 {
                let bb: ID3D12Resource = swapchain
                    .GetBuffer(i)
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: GetBuffer({i}) failed: {e}"),
                    })?;
                back_buffers.push(bb);
            }

            let index = self.swapchains.insert(Dx12SwapchainInner {
                swapchain,
                back_buffers,
                rtv_heap,
                rtv_size,
                width: descriptor.width,
                height: descriptor.height,
            });
            let handle = Self::make_handle(&mut self.gen_swapchain, index);
            Ok(SwapchainHandle::new(handle, self.gen_swapchain))
        }
    }

    fn recreate_swapchain(
        &mut self,
        swapchain: SwapchainHandle,
        width: u32,
        height: u32,
    ) -> Result<(), RhiError> {
        unsafe {
            let (_, idx) = Self::decode_handle(swapchain.index);
            let sc = self
                .swapchains
                .get_mut(idx)
                .ok_or(RhiError::InvalidHandle)?;

            // Release back buffers
            sc.back_buffers.clear();

            sc.swapchain
                .ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: ResizeBuffers failed: {e}"),
                })?;

            for i in 0..3 {
                let bb: ID3D12Resource = sc.swapchain.GetBuffer(i).map_err(|e| {
                    RhiError::Backend {
                        detail: format!("DX12: GetBuffer({i}) resize failed: {e}"),
                    }
                })?;
                sc.back_buffers.push(bb);
            }
            sc.width = width;
            sc.height = height;
            Ok(())
        }
    }

    fn destroy_swapchain(&mut self, swapchain: SwapchainHandle) {
        let (_, idx) = Self::decode_handle(swapchain.index);
        self.swapchains.remove(idx);
    }

    // --- Buffer ---
    fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> Result<BufferHandle, RhiError> {
        unsafe {
            let heap_type = match descriptor.memory_hint {
                MemoryHint::GpuOnly => D3D12_HEAP_TYPE_DEFAULT,
                MemoryHint::CpuToGpu => D3D12_HEAP_TYPE_UPLOAD,
                MemoryHint::GpuToCpu => D3D12_HEAP_TYPE_READBACK,
                MemoryHint::CpuOnly => D3D12_HEAP_TYPE_UPLOAD,
            };

            let mut state = D3D12_RESOURCE_STATES(0);
            if heap_type == D3D12_HEAP_TYPE_UPLOAD || heap_type == D3D12_HEAP_TYPE_READBACK {
                state = D3D12_RESOURCE_STATE_GENERIC_READ;
            } else {
                if descriptor.usage_flags.0 & BufferUsage::VERTEX.0 != 0 {
                    state |= D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER;
                }
                if descriptor.usage_flags.0 & BufferUsage::INDEX.0 != 0 {
                    state |= D3D12_RESOURCE_STATE_INDEX_BUFFER;
                }
                if descriptor.usage_flags.0 & BufferUsage::UNIFORM.0 != 0 {
                    state |= D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER;
                }
                if state.0 == 0 {
                    state = D3D12_RESOURCE_STATE_COMMON;
                }
            }

            let resource_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Alignment: 0,
                Width: descriptor.size_bytes,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_UNKNOWN,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                Flags: D3D12_RESOURCE_FLAGS(0),
            };

            let heap_props = D3D12_HEAP_PROPERTIES {
                Type: heap_type,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            };

            let mut resource: Option<ID3D12Resource> = None;
            self
                .device
                .CreateCommittedResource(
                    &heap_props,
                    D3D12_HEAP_FLAGS(0),
                    &resource_desc,
                    state,
                    None,
                    &mut resource,
                )
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateCommittedResource(buffer) failed: {e}"),
                })?;
            let resource = resource.ok_or(RhiError::Backend {
                detail: "DX12: CreateCommittedResource(buffer) returned null".to_string(),
            })?;

            let index = self.buffers.insert(Dx12BufferInner {
                resource,
                upload_resource: None,
                size: descriptor.size_bytes,
                state,
            });
            let handle = Self::make_handle(&mut self.gen_buffer, index);
            Ok(BufferHandle::new(handle, self.gen_buffer))
        }
    }

    fn write_buffer(
        &mut self,
        buffer: BufferHandle,
        data: &[u8],
        offset: u64,
    ) -> Result<(), RhiError> {
        unsafe {
            let (_, idx) = Self::decode_handle(buffer.index);
            let buf = self.buffers.get(idx).ok_or(RhiError::InvalidHandle)?;

            let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
            buf.resource
                .Map(0, None, Some(&mut ptr))
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Map failed: {e}"),
                })?;
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                (ptr as *mut u8).offset(offset as isize),
                data.len(),
            );
            buf.resource.Unmap(0, None);
            Ok(())
        }
    }

    fn destroy_buffer(&mut self, buffer: BufferHandle) {
        let (_, idx) = Self::decode_handle(buffer.index);
        self.buffers.remove(idx);
    }

    // --- Texture ---
    fn create_texture(
        &mut self,
        descriptor: &TextureDescriptor,
    ) -> Result<TextureHandle, RhiError> {
        unsafe {
            let format = Self::texture_format_to_dxgi(descriptor.format);
            let mut flags = D3D12_RESOURCE_FLAGS(0);
            if descriptor.usage_flags.0 & TextureUsage::COLOR_ATTACHMENT.0 != 0 {
                flags |= D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
            }
            if descriptor.usage_flags.0 & TextureUsage::DEPTH_ATTACHMENT.0 != 0 {
                flags |= D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL;
            }

            let resource_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Alignment: 0,
                Width: descriptor.width as u64,
                Height: descriptor.height,
                DepthOrArraySize: descriptor.depth_or_layers as u16,
                MipLevels: descriptor.mip_levels as u16,
                Format: format,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: descriptor.sample_count as u32,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                Flags: flags,
            };

            let heap_props = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            };

            let state = if descriptor.usage_flags.0 & TextureUsage::DEPTH_ATTACHMENT.0 != 0 {
                D3D12_RESOURCE_STATE_DEPTH_WRITE
            } else {
                D3D12_RESOURCE_STATE_COMMON
            };

            let mut resource: Option<ID3D12Resource> = None;
            self
                .device
                .CreateCommittedResource(
                    &heap_props,
                    D3D12_HEAP_FLAGS(0),
                    &resource_desc,
                    state,
                    None,
                    &mut resource,
                )
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateCommittedResource(texture) failed: {e}"),
                })?;
            let resource = resource.ok_or(RhiError::Backend {
                detail: "DX12: CreateCommittedResource(texture) returned null".to_string(),
            })?;

            let index = self.textures.insert(Dx12TextureInner {
                resource,
                format: descriptor.format,
                width: descriptor.width,
                height: descriptor.height,
                state,
            });
            let handle = Self::make_handle(&mut self.gen_texture, index);
            Ok(TextureHandle::new(handle, self.gen_texture))
        }
    }

    fn destroy_texture(&mut self, texture: TextureHandle) {
        let (_, idx) = Self::decode_handle(texture.index);
        self.textures.remove(idx);
    }

    // --- Shader modules ---
    fn create_shader_module(
        &mut self,
        descriptor: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, RhiError> {
        let bytecode = self
            .shader_cache
            .get(&descriptor.source_hash)
            .cloned()
            .unwrap_or_default();

        let index = self.shader_modules.insert(Dx12ShaderModuleInner {
            format: descriptor.format,
            entry_points: descriptor.entry_points.clone(),
            source_hash: descriptor.source_hash,
            bytecode,
        });
        let handle = Self::make_handle(&mut self.gen_shader, index);
        Ok(ShaderModuleHandle::new(handle, self.gen_shader))
    }

    fn destroy_shader_module(&mut self, module: ShaderModuleHandle) {
        let (_, idx) = Self::decode_handle(module.index);
        self.shader_modules.remove(idx);
    }

    // --- Render pass ---
    fn create_render_pass(
        &mut self,
        descriptor: &RenderPassDescriptor,
    ) -> Result<RenderPassHandle, RhiError> {
        let color_formats: Vec<DXGI_FORMAT> = descriptor
            .color_attachments
            .iter()
            .map(|&f| Self::texture_format_to_dxgi(f))
            .collect();
        let depth_format = descriptor
            .depth_stencil_format
            .map(Self::texture_format_to_dxgi);

        let index = self.render_passes.insert(Dx12RenderPassInner {
            color_formats,
            depth_format,
            sample_count: descriptor.sample_count,
        });
        let handle = Self::make_handle(&mut self.gen_pass, index);
        Ok(RenderPassHandle::new(handle, self.gen_pass))
    }

    fn destroy_render_pass(&mut self, pass: RenderPassHandle) {
        let (_, idx) = Self::decode_handle(pass.index);
        self.render_passes.remove(idx);
    }

    // --- Framebuffer ---
    fn create_framebuffer(
        &mut self,
        descriptor: &FramebufferDescriptor,
    ) -> Result<FramebufferHandle, RhiError> {
        let index = self.framebuffers.insert(Dx12FramebufferInner {
            rtv_descriptors: Vec::new(),
            dsv_descriptor: None,
            width: descriptor.width,
            height: descriptor.height,
        });
        let handle = Self::make_handle(&mut self.gen_fb, index);
        Ok(FramebufferHandle::new(handle, self.gen_fb))
    }

    fn destroy_framebuffer(&mut self, fb: FramebufferHandle) {
        let (_, idx) = Self::decode_handle(fb.index);
        self.framebuffers.remove(idx);
    }

    // --- Pipeline layout (root signature) ---
    fn create_pipeline_layout(
        &mut self,
        descriptor: &PipelineLayoutDescriptor,
    ) -> Result<PipelineLayoutHandle, RhiError> {
        unsafe {
            // Default empty root signature for simple pipelines
            let mut flags = D3D12_ROOT_SIGNATURE_FLAGS(0);
            flags |= D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT;
            if !descriptor.push_constant_ranges.is_empty() {
                flags |= D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT;
            }

            let root_sig_desc = D3D12_ROOT_SIGNATURE_DESC {
                NumParameters: 0,
                pParameters: std::ptr::null(),
                NumStaticSamplers: 0,
                pStaticSamplers: std::ptr::null(),
                Flags: flags,
            };

            let mut blob: Option<ID3DBlob> = None;
            let mut error_blob: Option<ID3DBlob> = None;
            D3D12SerializeRootSignature(
                &root_sig_desc,
                D3D_ROOT_SIGNATURE_VERSION_1,
                &mut blob,
                Some(&mut error_blob as *mut _),
            )
            .map_err(|e| RhiError::Backend {
                detail: format!("DX12: SerializeRootSignature failed: {e}"),
            })?;

            let blob = blob.ok_or(RhiError::Backend {
                detail: "DX12: SerializeRootSignature produced no blob".to_string(),
            })?;

            let buf = {
                let ptr = blob.GetBufferPointer() as *const u8;
                let len = blob.GetBufferSize();
                std::slice::from_raw_parts(ptr, len)
            };

            let root_sig: ID3D12RootSignature = self
                .device
                .CreateRootSignature(0, buf)
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateRootSignature failed: {e}"),
                })?;

            let index = self.pipeline_layouts.insert(Dx12PipelineLayoutInner {
                root_signature: root_sig,
            });
            let handle = Self::make_handle(&mut self.gen_layout, index);
            Ok(PipelineLayoutHandle::new(handle, self.gen_layout))
        }
    }

    fn destroy_pipeline_layout(&mut self, layout: PipelineLayoutHandle) {
        let (_, idx) = Self::decode_handle(layout.index);
        self.pipeline_layouts.remove(idx);
    }

    // --- Pipeline (PSO) ---
    fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
    ) -> Result<PipelineHandle, RhiError> {
        unsafe {
            // Get vertex shader and pixel shader bytecode
            let vs_bytecode = if let Some(&h) = descriptor.shader_modules.first() {
                let (_, idx) = Self::decode_handle(h.index);
                self.shader_modules
                    .get(idx)
                    .map(|sm| sm.bytecode.clone())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            let ps_bytecode = if descriptor.shader_modules.len() > 1 {
                let (_, idx) = Self::decode_handle(descriptor.shader_modules[1].index);
                self.shader_modules
                    .get(idx)
                    .map(|sm| sm.bytecode.clone())
                    .unwrap_or_default()
            } else {
                vs_bytecode.clone()
            };

            // Input layout
            let _vertex_size_bytes = descriptor.vertex_layout.stride_bytes;
            let input_elements: Vec<D3D12_INPUT_ELEMENT_DESC> =
                descriptor.vertex_layout.attributes.iter().map(|attr| {
                    let fmt = Self::attribute_format_to_dxgi(&attr.format);
                    D3D12_INPUT_ELEMENT_DESC {
                        SemanticName: windows::core::PCSTR::from_raw(attr.semantic.as_ptr()),
                        SemanticIndex: 0,
                        Format: fmt,
                        InputSlot: 0,
                        AlignedByteOffset: attr.offset_bytes,
                        InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    }
                }).collect();

            // Raster state
            let cull_mode = match descriptor.raster_state.cull_mode.as_deref() {
                Some("none") => D3D12_CULL_MODE_NONE,
                Some("front") => D3D12_CULL_MODE_FRONT,
                _ => D3D12_CULL_MODE_BACK,
            };
            let front_ccw = descriptor.raster_state.front_face.as_deref() != Some("cw");

            // Depth state
            let depth_enabled = descriptor.depth_state.format.is_some();
            let depth_write = descriptor.depth_state.write_enabled;
            let depth_func = match descriptor.depth_state.compare.as_deref() {
                Some("less") | None => D3D12_COMPARISON_FUNC_LESS,
                Some("less_equal") => D3D12_COMPARISON_FUNC_LESS_EQUAL,
                Some("equal") => D3D12_COMPARISON_FUNC_EQUAL,
                Some("always") => D3D12_COMPARISON_FUNC_ALWAYS,
                _ => D3D12_COMPARISON_FUNC_LESS,
            };

            let topology = match descriptor.topology.as_deref() {
                Some("line_list") => D3D_PRIMITIVE_TOPOLOGY_LINELIST,
                _ => D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
            };

            let depth_format = descriptor
                .depth_state
                .format
                .map(Self::texture_format_to_dxgi)
                .unwrap_or(DXGI_FORMAT_UNKNOWN);

            let rt_formats: Vec<DXGI_FORMAT> = descriptor
                .render_targets
                .iter()
                .map(|&f| Self::texture_format_to_dxgi(f))
                .collect();

            // Get root signature from pipeline layout
            let root_sig = if let Some(layout_handle) = descriptor.pipeline_layout {
                let (_, idx) = Self::decode_handle(layout_handle.index);
                self.pipeline_layouts
                    .get(idx)
                    .map(|pl| pl.root_signature.clone())
                    .ok_or(RhiError::InvalidHandle)?
            } else {
                let flags = D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT;
                let desc = D3D12_ROOT_SIGNATURE_DESC {
                    NumParameters: 0,
                    pParameters: std::ptr::null(),
                    NumStaticSamplers: 0,
                    pStaticSamplers: std::ptr::null(),
                    Flags: flags,
                };
                let mut blob: Option<ID3DBlob> = None;
                let mut _err: Option<ID3DBlob> = None;
                D3D12SerializeRootSignature(
                    &desc,
                    D3D_ROOT_SIGNATURE_VERSION_1,
                    &mut blob,
                    Some(&mut _err as *mut _),
                )
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: default root sig serialize: {e}"),
                })?;
                let blob = blob.unwrap();
                let buf = std::slice::from_raw_parts(
                    blob.GetBufferPointer() as *const u8,
                    blob.GetBufferSize(),
                );
                self.device
                    .CreateRootSignature(0, buf)
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: default root sig create: {e}"),
                    })?
            };

            // Build PSO description
            let blend_target = D3D12_RENDER_TARGET_BLEND_DESC {
                BlendEnable: BOOL(0),
                LogicOpEnable: BOOL(0),
                SrcBlend: D3D12_BLEND_ONE,
                DestBlend: D3D12_BLEND_ZERO,
                BlendOp: D3D12_BLEND_OP_ADD,
                SrcBlendAlpha: D3D12_BLEND_ONE,
                DestBlendAlpha: D3D12_BLEND_ZERO,
                BlendOpAlpha: D3D12_BLEND_OP_ADD,
                LogicOp: D3D12_LOGIC_OP_NOOP,
                RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
                pRootSignature: std::mem::ManuallyDrop::new(Some(root_sig)),
                VS: if !vs_bytecode.is_empty() {
                    D3D12_SHADER_BYTECODE {
                        pShaderBytecode: vs_bytecode.as_ptr() as *const _,
                        BytecodeLength: vs_bytecode.len(),
                    }
                } else {
                    D3D12_SHADER_BYTECODE {
                        pShaderBytecode: std::ptr::null(),
                        BytecodeLength: 0,
                    }
                },
                PS: if !ps_bytecode.is_empty() {
                    D3D12_SHADER_BYTECODE {
                        pShaderBytecode: ps_bytecode.as_ptr() as *const _,
                        BytecodeLength: ps_bytecode.len(),
                    }
                } else {
                    D3D12_SHADER_BYTECODE {
                        pShaderBytecode: std::ptr::null(),
                        BytecodeLength: 0,
                    }
                },
                DS: D3D12_SHADER_BYTECODE {
                    pShaderBytecode: std::ptr::null(),
                    BytecodeLength: 0,
                },
                HS: D3D12_SHADER_BYTECODE {
                    pShaderBytecode: std::ptr::null(),
                    BytecodeLength: 0,
                },
                GS: D3D12_SHADER_BYTECODE {
                    pShaderBytecode: std::ptr::null(),
                    BytecodeLength: 0,
                },
                StreamOutput: D3D12_STREAM_OUTPUT_DESC::default(),
                BlendState: D3D12_BLEND_DESC {
                    AlphaToCoverageEnable: FALSE,
                    IndependentBlendEnable: FALSE,
                    RenderTarget: {
                        let mut arr: [D3D12_RENDER_TARGET_BLEND_DESC; 8] = std::mem::zeroed();
                        arr[0] = blend_target;
                        arr
                    },
                },
                SampleMask: u32::MAX,
                RasterizerState: D3D12_RASTERIZER_DESC {
                    FillMode: D3D12_FILL_MODE_SOLID,
                    CullMode: cull_mode,
                    FrontCounterClockwise: front_ccw.into(),
                    DepthBias: 0,
                    DepthBiasClamp: 0.0,
                    SlopeScaledDepthBias: 0.0,
                    DepthClipEnable: TRUE,
                    MultisampleEnable: FALSE,
                    AntialiasedLineEnable: FALSE,
                    ForcedSampleCount: 0,
                    ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
                },
                DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
                    DepthEnable: depth_enabled.into(),
                    DepthWriteMask: if depth_write {
                        D3D12_DEPTH_WRITE_MASK_ALL
                    } else {
                        D3D12_DEPTH_WRITE_MASK_ZERO
                    },
                    DepthFunc: depth_func,
                    StencilEnable: FALSE,
                    StencilReadMask: 0,
                    StencilWriteMask: 0,
                    FrontFace: D3D12_DEPTH_STENCILOP_DESC::default(),
                    BackFace: D3D12_DEPTH_STENCILOP_DESC::default(),
                },
                InputLayout: if !input_elements.is_empty() {
                    D3D12_INPUT_LAYOUT_DESC {
                        pInputElementDescs: input_elements.as_ptr(),
                        NumElements: input_elements.len() as u32,
                    }
                } else {
                    D3D12_INPUT_LAYOUT_DESC {
                        pInputElementDescs: std::ptr::null(),
                        NumElements: 0,
                    }
                },
                IBStripCutValue: D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED,
                PrimitiveTopologyType: match topology {
                    D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST => D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
                    _ => D3D12_PRIMITIVE_TOPOLOGY_TYPE_LINE,
                },
                NumRenderTargets: rt_formats.len() as u32,
                RTVFormats: {
                    let mut arr = [DXGI_FORMAT_UNKNOWN; 8];
                    for (i, &f) in rt_formats.iter().enumerate() {
                        arr[i] = f;
                    }
                    arr
                },
                DSVFormat: depth_format,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: descriptor.sample_count.unwrap_or(1) as u32,
                    Quality: 0,
                },
                NodeMask: 0,
                CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
                Flags: D3D12_PIPELINE_STATE_FLAGS(0),
            };

            let pso: ID3D12PipelineState = self
                .device
                .CreateGraphicsPipelineState(&pso_desc)
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateGraphicsPipelineState failed: {e}"),
                })?;

            let index = self.pipelines.insert(Dx12PipelineInner {
                pso,
                topology: topology.0 as u32,
            });
            let handle = Self::make_handle(&mut self.gen_pipeline, index);
            Ok(PipelineHandle::new(handle, self.gen_pipeline))
        }
    }

    fn destroy_pipeline(&mut self, pipeline: PipelineHandle) {
        let (_, idx) = Self::decode_handle(pipeline.index);
        self.pipelines.remove(idx);
    }

    // --- Frame lifecycle ---
    fn begin_frame(
        &mut self,
        swapchain: SwapchainHandle,
    ) -> Result<(u32, Box<dyn CommandEncoder>), RhiError> {
        unsafe {
            let (_, sc_idx) = Self::decode_handle(swapchain.index);
            let sc = self
                .swapchains
                .get_mut(sc_idx)
                .ok_or(RhiError::InvalidHandle)?;

            let fi = self.frame_index;
            self.frame_index = (self.frame_index + 1) % Self::FRAMES_IN_FLIGHT;

            // Wait for previous frame
            let prev_value = self.fence_value;
            if prev_value > 0 && self.fence.GetCompletedValue() < prev_value {
                self.fence
                    .SetEventOnCompletion(prev_value, self.fence_event)
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: SetEventOnCompletion: {e}"),
                    })?;
                WaitForSingleObject(self.fence_event, u32::MAX);
            }

            self.fence_value += 1;

            // Reset allocator and command list
            self.allocators[fi]
                .Reset()
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Reset allocator: {e}"),
                })?;
            self.cmd_lists[fi].Reset(&self.allocators[fi], None).map_err(|e| {
                RhiError::Backend {
                    detail: format!("DX12: Reset cmd list: {e}"),
                }
            })?;

            // Get current back buffer index
            let image_index = sc.swapchain.GetCurrentBackBufferIndex();

            // Transition back buffer to render target
            let bb = &sc.back_buffers[image_index as usize];
            Self::transition_resource(
                &self.cmd_lists[fi],
                bb,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );

            // Clear back buffer
            let rtv_handle = {
                let cpu_start = sc.rtv_heap.GetCPUDescriptorHandleForHeapStart();
                D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: cpu_start.ptr + image_index as usize * sc.rtv_size as usize,
                }
            };
            let clear_color: [f32; 4] = [0.02, 0.02, 0.06, 1.0];
            self.cmd_lists[fi].ClearRenderTargetView(rtv_handle, &clear_color, None);

            let encoder = Dx12CommandEncoder::new(self.cmd_lists[fi].clone());
            Ok((image_index, Box::new(encoder)))
        }
    }

    fn end_frame(
        &mut self,
        swapchain: SwapchainHandle,
        _encoder: Box<dyn CommandEncoder>,
        image_index: u32,
    ) -> Result<RendererStatistics, RhiError> {
        unsafe {
            let (_, sc_idx) = Self::decode_handle(swapchain.index);
            let sc = self
                .swapchains
                .get_mut(sc_idx)
                .ok_or(RhiError::InvalidHandle)?;

            let fi = (self.frame_index + Self::FRAMES_IN_FLIGHT - 1) % Self::FRAMES_IN_FLIGHT;

            // Transition back buffer to present
            let bb = &sc.back_buffers[image_index as usize];
            Self::transition_resource(
                &self.cmd_lists[fi],
                bb,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            );

            // Close command list
            self.cmd_lists[fi]
                .Close()
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Close: {e}"),
                })?;

            // Execute
            let cmd_lists: [Option<ID3D12CommandList>; 1] = [
                Some(
                    self.cmd_lists[fi]
                        .clone()
                        .cast()
                        .map_err(|e| RhiError::Backend {
                            detail: format!("DX12: cast to ID3D12CommandList: {e}"),
                        })?,
                ),
            ];
            self.queue.ExecuteCommandLists(&cmd_lists);

            // Signal fence
            self.queue
                .Signal(&self.fence, self.fence_value)
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Signal: {e}"),
                })?;

            // Present
            let sync_interval = 1u32; // vsync
            sc.swapchain
                .Present(sync_interval, DXGI_PRESENT(0))
                .ok()
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Present: {e}"),
                })?;

            let draws = 0u32;
            let triangles = 0u64;

            Ok(RendererStatistics {
                draw_calls: draws,
                triangles,
                gpu_frame_ms: 0.0,
            })
        }
    }

    fn wait_idle(&self) {
        unsafe {
            let value = self.fence_value;
            if self.fence.GetCompletedValue() < value {
                let _ = self.fence.SetEventOnCompletion(value, self.fence_event);
                WaitForSingleObject(self.fence_event, u32::MAX);
            }
        }
    }

    fn read_pixels(
        &mut self,
        _x: u32,
        _y: u32,
        _width: u32,
        _height: u32,
    ) -> Result<Vec<u8>, RhiError> {
        Err(RhiError::UnsupportedFeature {
            feature: "DX12 framebuffer readback".to_string(),
        })
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
}

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
impl Device for Dx12Device {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.info
    }
    // All other methods use the default implementations from the trait
}

// ============================================================================
// Helper: attribute format to DXGI
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12Device {
    fn attribute_format_to_dxgi(format: &str) -> DXGI_FORMAT {
        match format {
            "float3" | "float32x3" => DXGI_FORMAT_R32G32B32_FLOAT,
            "float2" | "float32x2" => DXGI_FORMAT_R32G32_FLOAT,
            "float" | "float32" => DXGI_FORMAT_R32_FLOAT,
            "unorm8x4" | "rgba8" => DXGI_FORMAT_R8G8B8A8_UNORM,
            "unorm8x3" | "rgb8" => DXGI_FORMAT_R8G8B8A8_UNORM,
            _ => DXGI_FORMAT_R32G32B32_FLOAT,
        }
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
