macro_rules! dx12_device_pipeline_methods {
    () => {
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
            let mut color_states = Vec::with_capacity(descriptor.color_attachments.len());
            let mut color_is_sampled = Vec::with_capacity(descriptor.color_attachments.len());
            for attachment in &descriptor.color_attachments {
                let (_, texture_index) = Self::decode_handle(attachment.index);
                let texture = self
                    .textures
                    .get(texture_index)
                    .ok_or(RhiError::InvalidHandle)?;
                color_resources.push(texture.resource.clone());
                color_states.push(texture.state.clone());
                color_is_sampled.push(texture.sampled_srv_heap.is_some());
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
                color_states,
                color_is_sampled,
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
                        "sampled_texture_triple" => (D3D12_DESCRIPTOR_RANGE_TYPE_SRV, 3),
                        "sampled_texture_set" => (D3D12_DESCRIPTOR_RANGE_TYPE_SRV, 6),
                        "sampled_texture_set7" => (D3D12_DESCRIPTOR_RANGE_TYPE_SRV, 7),
                        "scene_resource_set" => (D3D12_DESCRIPTOR_RANGE_TYPE_SRV, 11),
                        "sampler" => (D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, 1),
                        "sampler_pair" => (D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, 2),
                        "sampler_triple" => (D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, 3),
                        "sampler_set" => (D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, 6),
                        "sampler_set7" => (D3D12_DESCRIPTOR_RANGE_TYPE_SAMPLER, 7),
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
                    ShaderVisibility: if range_type == D3D12_DESCRIPTOR_RANGE_TYPE_SRV {
                        D3D12_SHADER_VISIBILITY_ALL
                    } else {
                        D3D12_SHADER_VISIBILITY_PIXEL
                    },
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
                    let per_instance = attr.semantic.starts_with("INSTANCE_");
                    D3D12_INPUT_ELEMENT_DESC {
                        SemanticName: windows::core::PCSTR::from_raw(
                            semantic.as_ptr().cast::<u8>(),
                        ),
                        SemanticIndex: 0,
                        Format: fmt,
                        InputSlot: u32::from(per_instance),
                        AlignedByteOffset: attr.offset_bytes,
                        InputSlotClass: if per_instance {
                            D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA
                        } else {
                            D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA
                        },
                        InstanceDataStepRate: u32::from(per_instance),
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
            let blend_mode = descriptor
                .blend_state
                .mode
                .as_deref()
                .unwrap_or("opaque")
                .to_ascii_lowercase();
            let (blend_enabled, source_color, destination_color, source_alpha, destination_alpha) =
                match blend_mode.as_str() {
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
            let disabled_blend_target = D3D12_RENDER_TARGET_BLEND_DESC {
                BlendEnable: FALSE,
                LogicOpEnable: FALSE,
                SrcBlend: D3D12_BLEND_ONE,
                DestBlend: D3D12_BLEND_ZERO,
                BlendOp: D3D12_BLEND_OP_ADD,
                SrcBlendAlpha: D3D12_BLEND_ONE,
                DestBlendAlpha: D3D12_BLEND_ZERO,
                BlendOpAlpha: D3D12_BLEND_OP_ADD,
                LogicOp: D3D12_LOGIC_OP_NOOP,
                RenderTargetWriteMask: 0,
            };
            let additive_blend_target = D3D12_RENDER_TARGET_BLEND_DESC {
                BlendEnable: TRUE,
                LogicOpEnable: FALSE,
                SrcBlend: D3D12_BLEND_ONE,
                DestBlend: D3D12_BLEND_ONE,
                BlendOp: D3D12_BLEND_OP_ADD,
                SrcBlendAlpha: D3D12_BLEND_ONE,
                DestBlendAlpha: D3D12_BLEND_ONE,
                BlendOpAlpha: D3D12_BLEND_OP_ADD,
                LogicOp: D3D12_LOGIC_OP_NOOP,
                RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let weighted_oit = blend_mode == "weighted_oit";
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
                    IndependentBlendEnable: BOOL::from(weighted_oit || rt_formats.len() > 1),
                    RenderTarget: {
                        let mut arr: [D3D12_RENDER_TARGET_BLEND_DESC; 8] = std::mem::zeroed();
                        if weighted_oit {
                            arr[0] = disabled_blend_target;
                            arr[1] = additive_blend_target;
                            arr[2] = additive_blend_target;
                        } else {
                            arr[0] = blend_target;
                            for target in arr.iter_mut().take(rt_formats.len()).skip(1) {
                                *target = disabled_blend_target;
                            }
                        }
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
    };
}

pub(super) use dx12_device_pipeline_methods;
