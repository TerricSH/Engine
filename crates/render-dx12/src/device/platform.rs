#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use super::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12Device {
    pub(super) const FRAMES_IN_FLIGHT: usize = 2;

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

    pub(super) fn create_swapchain_depth_target(
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
                state: std::sync::Arc::new(std::sync::Mutex::new(
                    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                )),
                sampled_srv_heap: Some(srv_heap),
                sampled_sampler_heap: Some(sampler_heap),
            });
            let handle = Self::make_handle(&mut self.gen_texture, index);
            Ok(TextureHandle::new(handle, self.gen_texture))
        }
    }

    pub(crate) fn upload_sampled_rgba16f_cube(
        &mut self,
        mip_levels: &[engine_renderer::EnvironmentCubeMip],
    ) -> Result<TextureHandle, RhiError> {
        use std::mem::ManuallyDrop;

        unsafe {
            self.wait_idle();
            let face_size = mip_levels.first().map(|mip| mip.face_size).ok_or_else(|| {
                RhiError::InvalidDescriptor {
                    field: "environment.mip_levels".into(),
                    reason: "cubemap requires at least one mip".into(),
                }
            })?;
            let mip_count = mip_levels.len() as u32;
            let subresource_count = mip_count * 6;
            let resource_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Alignment: 0,
                Width: face_size.into(),
                Height: face_size,
                DepthOrArraySize: 6,
                MipLevels: mip_count as u16,
                Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
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
                    detail: format!("DX12: create HDR cubemap failed: {error}"),
                })?;
            let texture: ID3D12Resource = texture.ok_or_else(|| RhiError::Backend {
                detail: "DX12: HDR cubemap creation returned null".into(),
            })?;

            let count = subresource_count as usize;
            let mut layouts = vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); count];
            let mut row_counts = vec![0_u32; count];
            let mut row_sizes = vec![0_u64; count];
            let mut upload_size = 0_u64;
            self.device.GetCopyableFootprints(
                &resource_desc,
                0,
                subresource_count,
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
                    detail: format!("DX12: create HDR cubemap upload buffer failed: {error}"),
                })?;
            let upload: ID3D12Resource = upload.ok_or_else(|| RhiError::Backend {
                detail: "DX12: HDR cubemap upload buffer creation returned null".into(),
            })?;
            let mut mapped = std::ptr::null_mut();
            upload
                .Map(0, None, Some(&mut mapped))
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: map HDR cubemap upload buffer failed: {error}"),
                })?;
            // D3D12 subresources are array-major: mip + face * mip_count.
            for face_index in 0..6_usize {
                for (mip_index, mip) in mip_levels.iter().enumerate() {
                    let subresource = face_index * mip_levels.len() + mip_index;
                    let layout = layouts[subresource];
                    let face = &mip.faces[face_index];
                    let source_row_bytes = mip.face_size as usize * 8;
                    for row in 0..mip.face_size as usize {
                        std::ptr::copy_nonoverlapping(
                            face.as_ptr().add(row * source_row_bytes),
                            (mapped as *mut u8).add(
                                layout.Offset as usize + row * layout.Footprint.RowPitch as usize,
                            ),
                            source_row_bytes,
                        );
                    }
                }
            }
            upload.Unmap(0, None);

            let allocator: ID3D12CommandAllocator = self
                .device
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create HDR upload allocator failed: {error}"),
                })?;
            let command_list: ID3D12GraphicsCommandList = self
                .device
                .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: create HDR upload command list failed: {error}"),
                })?;
            Self::transition_resource(
                &command_list,
                &texture,
                D3D12_RESOURCE_STATE_COMMON,
                D3D12_RESOURCE_STATE_COPY_DEST,
            );
            for (subresource, layout) in layouts.iter().copied().enumerate() {
                let mut destination = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: ManuallyDrop::new(Some(texture.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        SubresourceIndex: subresource as u32,
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
                detail: format!("DX12: close HDR upload command list failed: {error}"),
            })?;
            let executable: ID3D12CommandList =
                command_list.cast().map_err(|error| RhiError::Backend {
                    detail: format!("DX12: cast HDR upload command list failed: {error}"),
                })?;
            self.queue.ExecuteCommandLists(&[Some(executable)]);
            let signal_value =
                self.fence_value
                    .checked_add(1)
                    .ok_or_else(|| RhiError::Backend {
                        detail: "DX12: fence value overflow during HDR cubemap upload".into(),
                    })?;
            self.queue
                .Signal(&self.fence, signal_value)
                .map_err(|error| RhiError::Backend {
                    detail: format!("DX12: signal HDR cubemap upload failed: {error}"),
                })?;
            self.fence_value = signal_value;
            if self.fence.GetCompletedValue() < signal_value {
                self.fence
                    .SetEventOnCompletion(signal_value, self.fence_event)
                    .map_err(|error| RhiError::Backend {
                        detail: format!("DX12: wait for HDR cubemap upload failed: {error}"),
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
                    detail: format!("DX12: create HDR cubemap SRV heap failed: {error}"),
                })?;
            let srv_desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURECUBE,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    TextureCube: D3D12_TEXCUBE_SRV {
                        MostDetailedMip: 0,
                        MipLevels: mip_count,
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
                    detail: format!("DX12: create HDR cubemap sampler heap failed: {error}"),
                })?;
            self.device.CreateSampler(
                &D3D12_SAMPLER_DESC {
                    Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                    AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                    AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
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
                format: TextureFormat::Rgba16Float,
                width: face_size,
                height: face_size,
                state: std::sync::Arc::new(std::sync::Mutex::new(
                    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                )),
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

    pub(super) fn info_queue_messages(queue: &ID3D12InfoQueue) -> String {
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
