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
                        let desc = a.GetDesc1().map_err(|e| RhiError::Backend {
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

            let desc = adapter.GetDesc1().map_err(|e| RhiError::Backend {
                detail: format!("DX12: GetDesc1 failed: {e}"),
            })?;
            let adapter_name = String::from_utf16_lossy(&desc.Description)
                .trim_end_matches('\0')
                .to_string();

            // Create D3D12 device
            let mut device: Option<ID3D12Device> = None;
            D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device).map_err(|e| {
                RhiError::Backend {
                    detail: format!("DX12: D3D12CreateDevice failed: {e}"),
                }
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
            let queue: ID3D12CommandQueue =
                d3d12_device
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
                    .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &alloc, None)
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: CreateCommandList failed: {e}"),
                    })?;
                // Close initially — will be reset in begin_frame
                cmd_list.Close().map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Close(init) failed: {e}"),
                })?;
                allocators.push(alloc);
                cmd_lists.push(cmd_list);
            }

            // Create fence
            let fence: ID3D12Fence =
                d3d12_device
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
                next_frame_clear_color: [0.02, 0.02, 0.06, 1.0],
                descriptor_heaps_in_flight: std::sync::Mutex::new(Vec::new()),
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
            })
        }
    }

    pub(crate) fn make_handle(gen: &mut u32, index: u32) -> u32 {
        let g = *gen;
        *gen = gen.wrapping_add(1);
        (g << 16) | (index & 0xFFFF)
    }

    pub(crate) fn decode_handle(h: u32) -> (u32, u32) {
        (h >> 16, h & 0xFFFF)
    }

    pub(crate) fn set_vertex_stride(
        &mut self,
        buffer: BufferHandle,
        stride: u32,
    ) -> Result<(), RhiError> {
        if stride == 0 {
            return Err(RhiError::InvalidDescriptor {
                field: "vertex_stride".into(),
                reason: "vertex stride must be non-zero".into(),
            });
        }
        let (_, index) = Self::decode_handle(buffer.index);
        let inner = self.buffers.get_mut(index).ok_or(RhiError::InvalidHandle)?;
        inner.vertex_stride = stride;
        Ok(())
    }

    pub(crate) fn set_next_frame_clear_color(&mut self, color: [f32; 4]) {
        self.next_frame_clear_color = color;
    }

    fn create_swapchain_depth_target(
        device: &ID3D12Device,
        width: u32,
        height: u32,
    ) -> Result<(ID3D12Resource, ID3D12DescriptorHeap), RhiError> {
        unsafe {
            let heap = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            };
            let descriptor = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Alignment: 0,
                Width: width.into(),
                Height: height,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_D32_FLOAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                Flags: D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL,
            };
            let mut depth_buffer = None;
            device
                .CreateCommittedResource(
                    &heap,
                    D3D12_HEAP_FLAGS(0),
                    &descriptor,
                    D3D12_RESOURCE_STATE_DEPTH_WRITE,
                    None,
                    &mut depth_buffer,
                )
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create swapchain depth buffer failed: {error}"),
                })?;
            let depth_buffer = depth_buffer.ok_or_else(|| RhiError::Backend {
                detail: "DX12: swapchain depth buffer creation returned null".into(),
            })?;
            let dsv_heap: ID3D12DescriptorHeap = device
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
                    NumDescriptors: 1,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAGS(0),
                    NodeMask: 0,
                })
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create DSV heap failed: {error}"),
                })?;
            device.CreateDepthStencilView(
                &depth_buffer,
                None,
                dsv_heap.GetCPUDescriptorHandleForHeapStart(),
            );
            Ok((depth_buffer, dsv_heap))
        }
    }

    pub(crate) fn upload_sampled_rgba8(
        &mut self,
        width: u32,
        height: u32,
        color_space: engine_renderer::ColorSpace,
        mip_levels: &[engine_renderer::TextureMipLevel],
        sampler: engine_renderer::SamplerDescriptor,
    ) -> Result<TextureHandle, RhiError> {
        use engine_renderer::{SamplerAddressMode, SamplerFilter};
        use std::mem::ManuallyDrop;

        unsafe {
            self.wait_idle();

            let resource_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Alignment: 0,
                Width: width.into(),
                Height: height,
                DepthOrArraySize: 1,
                MipLevels: mip_levels.len() as u16,
                Format: DXGI_FORMAT_R8G8B8A8_TYPELESS,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                Flags: D3D12_RESOURCE_FLAGS(0),
            };
            let default_heap = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            };
            let mut texture = None;
            self.device
                .CreateCommittedResource(
                    &default_heap,
                    D3D12_HEAP_FLAGS(0),
                    &resource_desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut texture,
                )
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create sampled texture failed: {error}"),
                })?;
            let texture: ID3D12Resource = texture.ok_or_else(|| RhiError::Backend {
                detail: "DX12: sampled texture creation returned null".into(),
            })?;

            let mip_count = mip_levels.len() as u32;
            let mut layouts = vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); mip_levels.len()];
            let mut row_counts = vec![0_u32; mip_levels.len()];
            let mut row_sizes = vec![0_u64; mip_levels.len()];
            let mut upload_size = 0_u64;
            self.device.GetCopyableFootprints(
                &resource_desc,
                0,
                mip_count,
                0,
                Some(layouts.as_mut_ptr()),
                Some(row_counts.as_mut_ptr()),
                Some(row_sizes.as_mut_ptr()),
                Some(&mut upload_size),
            );

            let upload_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Alignment: 0,
                Width: upload_size,
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
            let upload_heap = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_UPLOAD,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            };
            let mut upload = None;
            self.device
                .CreateCommittedResource(
                    &upload_heap,
                    D3D12_HEAP_FLAGS(0),
                    &upload_desc,
                    D3D12_RESOURCE_STATE_GENERIC_READ,
                    None,
                    &mut upload,
                )
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create texture upload buffer failed: {error}"),
                })?;
            let upload: ID3D12Resource = upload.ok_or_else(|| RhiError::Backend {
                detail: "DX12: texture upload buffer creation returned null".into(),
            })?;

            let mut mapped = std::ptr::null_mut();
            upload
                .Map(0, None, Some(&mut mapped))
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: map texture upload buffer failed: {error}"),
                })?;
            for (mip, layout) in mip_levels.iter().zip(&layouts) {
                let source_row_bytes = mip.width as usize * 4;
                for row in 0..mip.height as usize {
                    let source_offset = row * source_row_bytes;
                    let destination_offset =
                        layout.Offset as usize + row * layout.Footprint.RowPitch as usize;
                    std::ptr::copy_nonoverlapping(
                        mip.bytes.as_ptr().add(source_offset),
                        (mapped as *mut u8).add(destination_offset),
                        source_row_bytes,
                    );
                }
            }
            upload.Unmap(0, None);

            let allocator: ID3D12CommandAllocator = self
                .device
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create texture upload allocator failed: {error}"),
                })?;
            let command_list: ID3D12GraphicsCommandList = self
                .device
                .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create texture upload command list failed: {error}"),
                })?;
            Self::transition_resource(
                &command_list,
                &texture,
                D3D12_RESOURCE_STATE_COMMON,
                D3D12_RESOURCE_STATE_COPY_DEST,
            );
            for (mip_index, layout) in layouts.iter().copied().enumerate() {
                let mut destination = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: ManuallyDrop::new(Some(texture.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        SubresourceIndex: mip_index as u32,
                    },
                };
                let mut source = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: ManuallyDrop::new(Some(upload.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        PlacedFootprint: layout,
                    },
                };
                command_list.CopyTextureRegion(&destination, 0, 0, 0, &source, None);
                ManuallyDrop::drop(&mut destination.pResource);
                ManuallyDrop::drop(&mut source.pResource);
            }
            Self::transition_resource(
                &command_list,
                &texture,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );
            command_list.Close().map_err(|error| RhiError::Backend {
                detail: format!("DX12: close texture upload command list failed: {error}"),
            })?;
            let executable: ID3D12CommandList =
                command_list.cast().map_err(|error| RhiError::Backend {
                    detail: format!("DX12: cast texture upload command list failed: {error}"),
                })?;
            self.queue.ExecuteCommandLists(&[Some(executable)]);
            let signal_value =
                self.fence_value
                    .checked_add(1)
                    .ok_or_else(|| RhiError::Backend {
                        detail: "DX12: fence value overflow during texture upload".into(),
                    })?;
            self.queue
                .Signal(&self.fence, signal_value)
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: signal texture upload failed: {error}"),
                })?;
            self.fence_value = signal_value;
            if self.fence.GetCompletedValue() < signal_value {
                self.fence
                    .SetEventOnCompletion(signal_value, self.fence_event)
                    .map_err(|error| RhiError::Backend {
                        detail: format!("DX12: wait for texture upload failed: {error}"),
                    })?;
                WaitForSingleObject(self.fence_event, u32::MAX);
            }

            let srv_heap: ID3D12DescriptorHeap = self
                .device
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                    NumDescriptors: 1,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                    NodeMask: 0,
                })
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create sampled-texture SRV heap failed: {error}"),
                })?;
            let srv_format = match color_space {
                engine_renderer::ColorSpace::Linear => DXGI_FORMAT_R8G8B8A8_UNORM,
                engine_renderer::ColorSpace::Srgb => DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
            };
            let srv_desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: srv_format,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_SRV {
                        MostDetailedMip: 0,
                        MipLevels: mip_count,
                        PlaneSlice: 0,
                        ResourceMinLODClamp: 0.0,
                    },
                },
            };
            self.device.CreateShaderResourceView(
                &texture,
                Some(&srv_desc),
                srv_heap.GetCPUDescriptorHandleForHeapStart(),
            );

            let sampler_heap: ID3D12DescriptorHeap = self
                .device
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
                    NumDescriptors: 1,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                    NodeMask: 0,
                })
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create sampler heap failed: {error}"),
                })?;
            let filter_bits = match sampler.min_filter {
                SamplerFilter::Nearest => 0,
                SamplerFilter::Linear => 0x10,
            } + match sampler.mag_filter {
                SamplerFilter::Nearest => 0,
                SamplerFilter::Linear => 0x4,
            } + match sampler.mip_filter {
                SamplerFilter::Nearest => 0,
                SamplerFilter::Linear => 0x1,
            };
            let address_mode = |mode| match mode {
                SamplerAddressMode::Repeat => D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                SamplerAddressMode::ClampToEdge => D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                SamplerAddressMode::MirroredRepeat => D3D12_TEXTURE_ADDRESS_MODE_MIRROR,
            };
            self.device.CreateSampler(
                &D3D12_SAMPLER_DESC {
                    Filter: D3D12_FILTER(filter_bits),
                    AddressU: address_mode(sampler.address_u),
                    AddressV: address_mode(sampler.address_v),
                    AddressW: address_mode(sampler.address_w),
                    MipLODBias: 0.0,
                    MaxAnisotropy: 1,
                    ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
                    BorderColor: [0.0; 4],
                    MinLOD: 0.0,
                    MaxLOD: f32::MAX,
                },
                sampler_heap.GetCPUDescriptorHandleForHeapStart(),
            );

            let index = self.textures.insert(Dx12TextureInner {
                resource: texture,
                format: TextureFormat::Rgba8Unorm,
                width,
                height,
                state: D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                sampled_srv_heap: Some(srv_heap),
                sampled_sampler_heap: Some(sampler_heap),
            });
            let handle = Self::make_handle(&mut self.gen_texture, index);
            Ok(TextureHandle::new(handle, self.gen_texture))
        }
    }

    pub(crate) fn transition_resource(
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
                                pResource: std::mem::ManuallyDrop::new(Some(resource.clone())),
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

    pub(crate) fn texture_format_to_dxgi(format: TextureFormat) -> DXGI_FORMAT {
        match format {
            TextureFormat::Rgba8Unorm => DXGI_FORMAT_R8G8B8A8_UNORM,
            TextureFormat::Bgra8Unorm => DXGI_FORMAT_B8G8R8A8_UNORM,
            TextureFormat::Rgba16Float => DXGI_FORMAT_R16G16B16A16_FLOAT,
            TextureFormat::Depth32Float => DXGI_FORMAT_D32_FLOAT,
            _ => DXGI_FORMAT_UNKNOWN,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn fill_hex(buf: &mut [u8; 32], bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            if i < 32 {
                buf[i] = *b;
            }
        }
    }

    fn info_queue_messages(queue: &ID3D12InfoQueue) -> String {
        unsafe {
            let mut messages = Vec::new();
            for index in 0..queue.GetNumStoredMessages() {
                let mut byte_length = 0_usize;
                if queue.GetMessage(index, None, &mut byte_length).is_err() || byte_length == 0 {
                    continue;
                }
                let word_count = byte_length.div_ceil(std::mem::size_of::<usize>());
                let mut storage = vec![0_usize; word_count];
                let message = storage.as_mut_ptr().cast::<D3D12_MESSAGE>();
                if queue
                    .GetMessage(index, Some(message), &mut byte_length)
                    .is_err()
                {
                    continue;
                }
                let message = &*message;
                if message.pDescription.is_null() || message.DescriptionByteLength == 0 {
                    continue;
                }
                let bytes =
                    std::slice::from_raw_parts(message.pDescription, message.DescriptionByteLength);
                messages.push(
                    String::from_utf8_lossy(bytes)
                        .trim_end_matches('\0')
                        .to_owned(),
                );
            }
            messages.join(" | ")
        }
    }
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Device for Dx12Device {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.info
    }

    // --- Surface ---
    fn create_surface(
        &mut self,
        descriptor: &SurfaceDescriptor,
    ) -> Result<SurfaceHandle, RhiError> {
        let hwnd = match &descriptor.window_handle {
            render_core::SurfaceTarget::RawWindowHandleToken(token) => {
                HWND(*token as *mut std::ffi::c_void)
            }
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
                .CreateSwapChainForHwnd(&self.queue, hwnd, &desc, None, None)
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
            let rtv_heap: ID3D12DescriptorHeap = self
                .device
                .CreateDescriptorHeap(&rtv_desc)
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateDescriptorHeap(RTV) failed: {e}"),
                })?;
            let rtv_size = self
                .device
                .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);

            // Get back buffers
            let mut back_buffers = Vec::new();
            for i in 0..3 {
                let bb: ID3D12Resource = swapchain.GetBuffer(i).map_err(|e| RhiError::Backend {
                    detail: format!("DX12: GetBuffer({i}) failed: {e}"),
                })?;
                let rtv = D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: rtv_heap.GetCPUDescriptorHandleForHeapStart().ptr
                        + i as usize * rtv_size as usize,
                };
                self.device.CreateRenderTargetView(&bb, None, rtv);
                back_buffers.push(bb);
            }
            let (depth_buffer, dsv_heap) = Self::create_swapchain_depth_target(
                &self.device,
                descriptor.width,
                descriptor.height,
            )?;

            let index = self.swapchains.insert(Dx12SwapchainInner {
                swapchain,
                back_buffers,
                rtv_heap,
                rtv_size,
                depth_buffer,
                dsv_heap,
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
            self.wait_idle();
            let (_, idx) = Self::decode_handle(swapchain.index);
            let sc = self
                .swapchains
                .get_mut(idx)
                .ok_or(RhiError::InvalidHandle)?;

            // Release back buffers
            sc.back_buffers.clear();

            sc.swapchain
                .ResizeBuffers(
                    0,
                    width,
                    height,
                    DXGI_FORMAT_UNKNOWN,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: ResizeBuffers failed: {e}"),
                })?;

            for i in 0..3 {
                let bb: ID3D12Resource =
                    sc.swapchain.GetBuffer(i).map_err(|e| RhiError::Backend {
                        detail: format!("DX12: GetBuffer({i}) resize failed: {e}"),
                    })?;
                let rtv = D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: sc.rtv_heap.GetCPUDescriptorHandleForHeapStart().ptr
                        + i as usize * sc.rtv_size as usize,
                };
                self.device.CreateRenderTargetView(&bb, None, rtv);
                sc.back_buffers.push(bb);
            }
            let (depth_buffer, dsv_heap) =
                Self::create_swapchain_depth_target(&self.device, width, height)?;
            sc.depth_buffer = depth_buffer;
            sc.dsv_heap = dsv_heap;
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
            self.device
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
                vertex_stride: if descriptor.usage_flags.0 & BufferUsage::VERTEX.0 != 0 {
                    32
                } else {
                    0
                },
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
            let write_end =
                offset
                    .checked_add(data.len() as u64)
                    .ok_or_else(|| RhiError::Backend {
                        detail: "DX12: buffer write range overflowed u64".to_string(),
                    })?;
            if write_end > buf.size {
                return Err(RhiError::Backend {
                    detail: format!(
                        "DX12: buffer write range {offset}..{write_end} exceeds buffer size {}",
                        buf.size
                    ),
                });
            }

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
            if descriptor.width == 0
                || descriptor.height == 0
                || descriptor.depth_or_layers == 0
                || descriptor.mip_levels == 0
                || descriptor.sample_count == 0
            {
                return Err(RhiError::InvalidDescriptor {
                    field: "texture".into(),
                    reason: "dimensions, layers, mip levels and sample count must be non-zero"
                        .into(),
                });
            }
            let is_depth = descriptor.usage_flags.0 & TextureUsage::DEPTH_ATTACHMENT.0 != 0;
            let is_sampled = descriptor.usage_flags.0 & TextureUsage::SAMPLED.0 != 0;
            let view_format = Self::texture_format_to_dxgi(descriptor.format);
            let resource_format = if is_depth && is_sampled {
                DXGI_FORMAT_R32_TYPELESS
            } else {
                view_format
            };
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
                Format: resource_format,
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

            let state = if is_depth && is_sampled {
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE
            } else if is_depth {
                D3D12_RESOURCE_STATE_DEPTH_WRITE
            } else {
                D3D12_RESOURCE_STATE_COMMON
            };

            let clear_value = is_depth.then_some(D3D12_CLEAR_VALUE {
                Format: DXGI_FORMAT_D32_FLOAT,
                Anonymous: D3D12_CLEAR_VALUE_0 {
                    DepthStencil: D3D12_DEPTH_STENCIL_VALUE {
                        Depth: 1.0,
                        Stencil: 0,
                    },
                },
            });

            let mut resource: Option<ID3D12Resource> = None;
            self.device
                .CreateCommittedResource(
                    &heap_props,
                    D3D12_HEAP_FLAGS(0),
                    &resource_desc,
                    state,
                    clear_value
                        .as_ref()
                        .map(|value| value as *const D3D12_CLEAR_VALUE),
                    &mut resource,
                )
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: CreateCommittedResource(texture) failed: {e}"),
                })?;
            let resource = resource.ok_or(RhiError::Backend {
                detail: "DX12: CreateCommittedResource(texture) returned null".to_string(),
            })?;

            let (sampled_srv_heap, sampled_sampler_heap) = if is_sampled {
                let srv_heap: ID3D12DescriptorHeap = self
                    .device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        NumDescriptors: 1,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    })
                    .map_err(|error| RhiError::Backend {
                        detail: format!("DX12: create texture SRV heap failed: {error}"),
                    })?;
                let srv_desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
                    Format: if is_depth {
                        DXGI_FORMAT_R32_FLOAT
                    } else {
                        view_format
                    },
                    ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                    Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                    Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                        Texture2D: D3D12_TEX2D_SRV {
                            MostDetailedMip: 0,
                            MipLevels: descriptor.mip_levels,
                            PlaneSlice: 0,
                            ResourceMinLODClamp: 0.0,
                        },
                    },
                };
                self.device.CreateShaderResourceView(
                    &resource,
                    Some(&srv_desc),
                    srv_heap.GetCPUDescriptorHandleForHeapStart(),
                );
                let sampler_heap: ID3D12DescriptorHeap = self
                    .device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
                        NumDescriptors: 1,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    })
                    .map_err(|error| RhiError::Backend {
                        detail: format!("DX12: create texture sampler heap failed: {error}"),
                    })?;
                self.device.CreateSampler(
                    &D3D12_SAMPLER_DESC {
                        Filter: D3D12_FILTER_MIN_MAG_LINEAR_MIP_POINT,
                        AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                        AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                        AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                        MipLODBias: 0.0,
                        MaxAnisotropy: 1,
                        ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
                        BorderColor: [1.0; 4],
                        MinLOD: 0.0,
                        MaxLOD: f32::MAX,
                    },
                    sampler_heap.GetCPUDescriptorHandleForHeapStart(),
                );
                (Some(srv_heap), Some(sampler_heap))
            } else {
                (None, None)
            };

            let index = self.textures.insert(Dx12TextureInner {
                resource,
                format: descriptor.format,
                width: descriptor.width,
                height: descriptor.height,
                state,
                sampled_srv_heap,
                sampled_sampler_heap,
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
        if descriptor.format != ShaderFormat::Dxil || descriptor.source_bytes.is_empty() {
            return Err(RhiError::InvalidDescriptor {
                field: "shader_module".into(),
                reason: "DX12 requires non-empty backend-ready Dxil/DXBC bytes".into(),
            });
        }

        let index = self.shader_modules.insert(Dx12ShaderModuleInner {
            format: descriptor.format,
            stage: descriptor.stage,
            entry_points: descriptor.entry_points.clone(),
            source_hash: descriptor.source_hash,
            bytecode: descriptor.source_bytes.clone(),
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
        unsafe {
            if descriptor.width == 0 || descriptor.height == 0 {
                return Err(RhiError::InvalidDescriptor {
                    field: "framebuffer".into(),
                    reason: "width and height must be non-zero".into(),
                });
            }
            let (_, pass_index) = Self::decode_handle(descriptor.render_pass.index);
            if self.render_passes.get(pass_index).is_none() {
                return Err(RhiError::InvalidHandle);
            }

            let mut color_resources = Vec::with_capacity(descriptor.color_attachments.len());
            for attachment in &descriptor.color_attachments {
                let (_, texture_index) = Self::decode_handle(attachment.index);
                let texture = self
                    .textures
                    .get(texture_index)
                    .ok_or(RhiError::InvalidHandle)?;
                color_resources.push(texture.resource.clone());
            }
            let rtv_heap: Option<ID3D12DescriptorHeap> = if color_resources.is_empty() {
                None
            } else {
                Some(
                    self.device
                        .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                            Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                            NumDescriptors: color_resources.len() as u32,
                            Flags: D3D12_DESCRIPTOR_HEAP_FLAGS(0),
                            NodeMask: 0,
                        })
                        .map_err(|error| RhiError::Backend {
                            detail: format!("DX12: create framebuffer RTV heap failed: {error}"),
                        })?,
                )
            };
            let mut rtv_descriptors = Vec::with_capacity(color_resources.len());
            if let Some(heap) = rtv_heap.as_ref() {
                let start = heap.GetCPUDescriptorHandleForHeapStart();
                let increment = self
                    .device
                    .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
                    as usize;
                for (index, resource) in color_resources.iter().enumerate() {
                    let handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                        ptr: start.ptr + index * increment,
                    };
                    self.device.CreateRenderTargetView(resource, None, handle);
                    rtv_descriptors.push(handle);
                }
            }

            let (depth_resource, depth_is_sampled) =
                if let Some(attachment) = descriptor.depth_stencil_attachment {
                    let (_, texture_index) = Self::decode_handle(attachment.index);
                    let texture = self
                        .textures
                        .get(texture_index)
                        .ok_or(RhiError::InvalidHandle)?;
                    (
                        Some(texture.resource.clone()),
                        texture.sampled_srv_heap.is_some(),
                    )
                } else {
                    (None, false)
                };
            let dsv_heap: Option<ID3D12DescriptorHeap> = if depth_resource.is_some() {
                Some(
                    self.device
                        .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                            Type: D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
                            NumDescriptors: 1,
                            Flags: D3D12_DESCRIPTOR_HEAP_FLAGS(0),
                            NodeMask: 0,
                        })
                        .map_err(|error| RhiError::Backend {
                            detail: format!("DX12: create framebuffer DSV heap failed: {error}"),
                        })?,
                )
            } else {
                None
            };
            let dsv_descriptor = dsv_heap
                .as_ref()
                .map(|heap| heap.GetCPUDescriptorHandleForHeapStart());
            if let (Some(resource), Some(handle)) = (depth_resource.as_ref(), dsv_descriptor) {
                self.device.CreateDepthStencilView(
                    resource,
                    Some(&D3D12_DEPTH_STENCIL_VIEW_DESC {
                        Format: DXGI_FORMAT_D32_FLOAT,
                        ViewDimension: D3D12_DSV_DIMENSION_TEXTURE2D,
                        Flags: D3D12_DSV_FLAG_NONE,
                        Anonymous: D3D12_DEPTH_STENCIL_VIEW_DESC_0 {
                            Texture2D: D3D12_TEX2D_DSV { MipSlice: 0 },
                        },
                    }),
                    handle,
                );
            }

            let index = self.framebuffers.insert(Dx12FramebufferInner {
                rtv_heap,
                dsv_heap,
                rtv_descriptors,
                dsv_descriptor,
                color_resources,
                depth_resource,
                depth_is_sampled,
                width: descriptor.width,
                height: descriptor.height,
            });
            let handle = Self::make_handle(&mut self.gen_fb, index);
            Ok(FramebufferHandle::new(handle, self.gen_fb))
        }
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
            let mut root_constant_bytes = 0_u32;
            for range in &descriptor.push_constant_ranges {
                if !range.offset.is_multiple_of(4)
                    || range.size == 0
                    || !range.size.is_multiple_of(4)
                {
                    return Err(RhiError::InvalidDescriptor {
                        field: "push_constant_ranges".into(),
                        reason: format!(
                            "DX12 root constants require non-zero 4-byte aligned ranges, got offset={} size={}",
                            range.offset, range.size
                        ),
                    });
                }
                root_constant_bytes =
                    root_constant_bytes.max(range.offset.checked_add(range.size).ok_or_else(
                        || RhiError::InvalidDescriptor {
                            field: "push_constant_ranges".into(),
                            reason: "push-constant range overflow".into(),
                        },
                    )?);
            }
            let root_constant_dwords = root_constant_bytes / 4;
            if root_constant_dwords > 64 {
                return Err(RhiError::InvalidDescriptor {
                    field: "push_constant_ranges".into(),
                    reason: format!(
                        "DX12 root signatures allow at most 64 DWORDs of root constants, requested {root_constant_dwords}"
                    ),
                });
            }

            let mut descriptor_ranges = Vec::new();
            let mut descriptor_kinds = Vec::new();
            let mut uniform_bindings = Vec::new();
            for layout in &descriptor.bind_group_layouts {
                for binding in &layout.bindings {
                    let normalized = binding.resource_kind.to_ascii_lowercase();
                    let (range_type, descriptor_count) = match normalized.as_str() {
                        "sampled_texture" | "texture" | "texture2d" => {
                            (D3D12_DESCRIPTOR_RANGE_TYPE_SRV, 1)
                        }
                        "sampled_texture_pair" => (D3D12_DESCRIPTOR_RANGE_TYPE_SRV, 2),
                        "sampled_texture_set" => (D3D12_DESCRIPTOR_RANGE_TYPE_SRV, 6),
                        "sampler" => (D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, 1),
                        "sampler_pair" => (D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, 2),
                        "sampler_set" => (D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, 6),
                        "uniform_buffer" => {
                            uniform_bindings.push((binding.binding, u32::from(layout.set_index)));
                            continue;
                        }
                        _ => {
                            return Err(RhiError::UnsupportedFeature {
                                feature: format!(
                                    "DX12 bind-group resource kind '{}'",
                                    binding.resource_kind
                                ),
                            });
                        }
                    };
                    descriptor_ranges.push(D3D12_DESCRIPTOR_RANGE {
                        RangeType: range_type,
                        NumDescriptors: descriptor_count,
                        BaseShaderRegister: binding.binding,
                        RegisterSpace: layout.set_index.into(),
                        OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
                    });
                    descriptor_kinds.push(range_type);
                }
            }

            let mut root_parameters = Vec::with_capacity(
                usize::from(root_constant_dwords > 0)
                    + descriptor_ranges.len()
                    + uniform_bindings.len(),
            );
            let root_constants_parameter = if root_constant_dwords > 0 {
                let index = root_parameters.len() as u32;
                root_parameters.push(D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        Constants: D3D12_ROOT_CONSTANTS {
                            ShaderRegister: 0,
                            RegisterSpace: 0,
                            Num32BitValues: root_constant_dwords,
                        },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                });
                Some(index)
            } else {
                None
            };
            let mut sampled_texture_parameter = None;
            let mut sampler_parameter = None;
            let mut uniform_buffer_parameter = None;
            for (range_index, range_type) in descriptor_kinds.iter().copied().enumerate() {
                let parameter_index = root_parameters.len() as u32;
                if range_type == D3D12_DESCRIPTOR_RANGE_TYPE_SRV {
                    sampled_texture_parameter = Some(parameter_index);
                } else if range_type == D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER {
                    sampler_parameter = Some(parameter_index);
                }
                root_parameters.push(D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                            NumDescriptorRanges: 1,
                            pDescriptorRanges: descriptor_ranges.as_ptr().add(range_index),
                        },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                });
            }
            for (shader_register, register_space) in uniform_bindings {
                let parameter_index = root_parameters.len() as u32;
                uniform_buffer_parameter = Some(parameter_index);
                root_parameters.push(D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_CBV,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        Descriptor: D3D12_ROOT_DESCRIPTOR {
                            ShaderRegister: shader_register,
                            RegisterSpace: register_space,
                        },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_VERTEX,
                });
            }

            let root_sig_desc = D3D12_ROOT_SIGNATURE_DESC {
                NumParameters: root_parameters.len() as u32,
                pParameters: root_parameters.as_ptr(),
                NumStaticSamplers: 0,
                pStaticSamplers: std::ptr::null(),
                Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
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

            let root_sig: ID3D12RootSignature =
                self.device
                    .CreateRootSignature(0, buf)
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: CreateRootSignature failed: {e}"),
                    })?;

            let index = self.pipeline_layouts.insert(Dx12PipelineLayoutInner {
                root_signature: root_sig,
                root_constants_parameter,
                sampled_texture_parameter,
                sampler_parameter,
                uniform_buffer_parameter,
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
            let shader_bytecode = |stage| {
                descriptor.shader_modules.iter().find_map(|handle| {
                    let (_, index) = Self::decode_handle(handle.index);
                    self.shader_modules
                        .get(index)
                        .filter(|module| module.stage == stage)
                        .map(|module| module.bytecode.clone())
                })
            };
            let vs_bytecode = shader_bytecode(ShaderStage::Vertex).unwrap_or_default();
            let ps_bytecode = shader_bytecode(ShaderStage::Fragment).unwrap_or_default();

            // Input layout
            let _vertex_size_bytes = descriptor.vertex_layout.stride_bytes;
            let semantic_names: Vec<std::ffi::CString> = descriptor
                .vertex_layout
                .attributes
                .iter()
                .map(|attribute| {
                    std::ffi::CString::new(attribute.semantic.as_str()).map_err(|_| {
                        RhiError::InvalidDescriptor {
                            field: "vertex_layout.attributes.semantic".into(),
                            reason: "semantic contains an interior NUL byte".into(),
                        }
                    })
                })
                .collect::<Result<_, _>>()?;
            let input_elements: Vec<D3D12_INPUT_ELEMENT_DESC> = descriptor
                .vertex_layout
                .attributes
                .iter()
                .zip(&semantic_names)
                .map(|(attr, semantic)| {
                    let fmt = Self::attribute_format_to_dxgi(&attr.format);
                    D3D12_INPUT_ELEMENT_DESC {
                        SemanticName: windows::core::PCSTR::from_raw(
                            semantic.as_ptr().cast::<u8>(),
                        ),
                        SemanticIndex: 0,
                        Format: fmt,
                        InputSlot: 0,
                        AlignedByteOffset: attr.offset_bytes,
                        InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    }
                })
                .collect();

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
                let blob = blob.ok_or_else(|| RhiError::Backend {
                    detail: "DX12: root signature serialization succeeded without a blob"
                        .to_string(),
                })?;
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
            let (blend_enabled, source_color, destination_color, source_alpha, destination_alpha) =
                match descriptor
                    .blend_state
                    .mode
                    .as_deref()
                    .unwrap_or("opaque")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "alpha" => (
                        true,
                        D3D12_BLEND_SRC_ALPHA,
                        D3D12_BLEND_INV_SRC_ALPHA,
                        D3D12_BLEND_ONE,
                        D3D12_BLEND_INV_SRC_ALPHA,
                    ),
                    "premultiplied" | "premultipliedalpha" => (
                        true,
                        D3D12_BLEND_ONE,
                        D3D12_BLEND_INV_SRC_ALPHA,
                        D3D12_BLEND_ONE,
                        D3D12_BLEND_INV_SRC_ALPHA,
                    ),
                    "add" | "additive" => (
                        true,
                        D3D12_BLEND_SRC_ALPHA,
                        D3D12_BLEND_ONE,
                        D3D12_BLEND_ONE,
                        D3D12_BLEND_ONE,
                    ),
                    _ => (
                        false,
                        D3D12_BLEND_ONE,
                        D3D12_BLEND_ZERO,
                        D3D12_BLEND_ONE,
                        D3D12_BLEND_ZERO,
                    ),
                };
            let blend_target = D3D12_RENDER_TARGET_BLEND_DESC {
                BlendEnable: blend_enabled.into(),
                LogicOpEnable: BOOL(0),
                SrcBlend: source_color,
                DestBlend: destination_color,
                BlendOp: D3D12_BLEND_OP_ADD,
                SrcBlendAlpha: source_alpha,
                DestBlendAlpha: destination_alpha,
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

            let info_queue = self.device.cast::<ID3D12InfoQueue>().ok();
            if let Some(queue) = info_queue.as_ref() {
                queue.ClearStoredMessages();
            }
            let pso: ID3D12PipelineState = self
                .device
                .CreateGraphicsPipelineState(&pso_desc)
                .map_err(|e| {
                    let messages = info_queue
                        .as_ref()
                        .map(Self::info_queue_messages)
                        .filter(|messages| !messages.is_empty())
                        .map(|messages| format!("; validation: {messages}"))
                        .unwrap_or_default();
                    RhiError::Backend {
                        detail: format!("DX12: CreateGraphicsPipelineState failed: {e}{messages}"),
                    }
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

            // Wait for the most recently submitted frame before reusing any
            // allocator. `fence_value` tracks submissions only; merely
            // beginning a frame must not create a fence value that can never
            // be signalled if recording is later aborted.
            let prev_value = self.fence_value;
            if prev_value > 0 && self.fence.GetCompletedValue() < prev_value {
                self.fence
                    .SetEventOnCompletion(prev_value, self.fence_event)
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: SetEventOnCompletion: {e}"),
                    })?;
                WaitForSingleObject(self.fence_event, u32::MAX);
            }
            self.descriptor_heaps_in_flight
                .get_mut()
                .map_err(|_| RhiError::Backend {
                    detail: "DX12: transient descriptor heap lock is poisoned".into(),
                })?
                .clear();

            // Reset allocator and command list
            self.allocators[fi].Reset().map_err(|e| RhiError::Backend {
                detail: format!("DX12: Reset allocator: {e}"),
            })?;
            self.cmd_lists[fi]
                .Reset(&self.allocators[fi], None)
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Reset cmd list: {e}"),
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
            let clear_color = self.next_frame_clear_color;
            self.cmd_lists[fi].ClearRenderTargetView(rtv_handle, &clear_color, None);
            let dsv_handle = sc.dsv_heap.GetCPUDescriptorHandleForHeapStart();
            self.cmd_lists[fi].ClearDepthStencilView(
                dsv_handle,
                D3D12_CLEAR_FLAG_DEPTH,
                1.0,
                0,
                &[],
            );

            // Bind the matching color/depth pair for the whole scene pass.
            self.cmd_lists[fi].OMSetRenderTargets(
                1,
                Some(&rtv_handle as *const _ as *const _),
                false,
                Some(&dsv_handle),
            );

            let encoder = Dx12CommandEncoder::new(
                self.cmd_lists[fi].clone(),
                self as *const _,
                rtv_handle,
                dsv_handle,
            );
            self.frame_index = (fi + 1) % Self::FRAMES_IN_FLIGHT;
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
            self.cmd_lists[fi].Close().map_err(|e| RhiError::Backend {
                detail: format!("DX12: Close: {e}"),
            })?;

            // Reserve the submission fence value before execution. Failing
            // after ExecuteCommandLists would leave GPU work with no fence that
            // the engine can safely wait on.
            let submitted_fence_value =
                self.fence_value
                    .checked_add(1)
                    .ok_or_else(|| RhiError::Backend {
                        detail: "DX12: fence value overflow".to_string(),
                    })?;

            // Execute
            let cmd_lists: [Option<ID3D12CommandList>; 1] =
                [Some(self.cmd_lists[fi].clone().cast().map_err(|e| {
                    RhiError::Backend {
                        detail: format!("DX12: cast to ID3D12CommandList: {e}"),
                    }
                })?)];
            self.queue.ExecuteCommandLists(&cmd_lists);

            // Signal the reserved fence value after command-list execution.
            self.queue
                .Signal(&self.fence, submitted_fence_value)
                .map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Signal: {e}"),
                })?;
            self.fence_value = submitted_fence_value;

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
            "float" | "float32" => DXGI_FORMAT_R32_FLOAT,
            "unorm8x4" | "rgba8" => DXGI_FORMAT_R8G8B8A8_UNORM,
            "unorm8x3" | "rgb8" => DXGI_FORMAT_R8G8B8A8_UNORM,
            _ => DXGI_FORMAT_R32G32B32_FLOAT,
        }
    }
}
