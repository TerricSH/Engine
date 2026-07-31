macro_rules! dx12_device_resource_methods {
    () => {
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

                let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: DXGI factory for swapchain: {e}"),
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

                let swapchain: IDXGISwapChain3 =
                    swapchain.cast().map_err(|e| RhiError::Backend {
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
                    let bb: ID3D12Resource =
                        swapchain.GetBuffer(i).map_err(|e| RhiError::Backend {
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
        fn create_buffer(
            &mut self,
            descriptor: &BufferDescriptor,
        ) -> Result<BufferHandle, RhiError> {
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
                    state: std::sync::Arc::new(std::sync::Mutex::new(state)),
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
    };
}

pub(super) use dx12_device_resource_methods;
