#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::{
    Win32::Foundation::RECT, Win32::Graphics::Direct3D::*, Win32::Graphics::Direct3D12::*,
    Win32::Graphics::Dxgi::Common::*,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use render_core::{
    BufferHandle, CommandEncoder, FramebufferHandle, IndexFormat, PipelineHandle,
    PipelineLayoutHandle, RenderPassHandle, TextureHandle,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::device::Dx12Device;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(crate) struct Dx12CommandEncoder {
    pub(crate) cmd_list: ID3D12GraphicsCommandList,
    device: *const Dx12Device,
    main_rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    main_dsv: D3D12_CPU_DESCRIPTOR_HANDLE,
    active_depth_attachment: Option<ID3D12Resource>,
    pub(crate) draws: u32,
    pub(crate) triangles: u64,
}

// SAFETY: The device outlives the encoder — the encoder is created in
// begin_frame and consumed in end_frame. The raw pointer is only dereferenced
// within method calls and the device is never deallocated while the encoder
// is alive.
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
unsafe impl Send for Dx12CommandEncoder {}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12CommandEncoder {
    pub(crate) fn new(
        cmd_list: ID3D12GraphicsCommandList,
        device: *const Dx12Device,
        main_rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
        main_dsv: D3D12_CPU_DESCRIPTOR_HANDLE,
    ) -> Self {
        Self {
            cmd_list,
            device,
            main_rtv,
            main_dsv,
            active_depth_attachment: None,
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
        framebuffer: FramebufferHandle,
        area: (u32, u32, u32, u32),
        clear_color: [f32; 4],
        clear_depth: Option<f32>,
    ) {
        unsafe {
            let device = &*self.device;
            let (_, framebuffer_index) = Dx12Device::decode_handle(framebuffer.index);
            let Some(framebuffer) = device.framebuffers.get(framebuffer_index) else {
                return;
            };
            if let Some(depth) = framebuffer.depth_resource.as_ref() {
                if framebuffer.depth_is_sampled {
                    Dx12Device::transition_resource(
                        &self.cmd_list,
                        depth,
                        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                        D3D12_RESOURCE_STATE_DEPTH_WRITE,
                    );
                }
                self.active_depth_attachment = Some(depth.clone());
            }
            for rtv in &framebuffer.rtv_descriptors {
                self.cmd_list
                    .ClearRenderTargetView(*rtv, &clear_color, None);
            }
            if let (Some(dsv), Some(depth)) = (framebuffer.dsv_descriptor, clear_depth) {
                self.cmd_list
                    .ClearDepthStencilView(dsv, D3D12_CLEAR_FLAG_DEPTH, depth, 0, &[]);
            }
            self.cmd_list.OMSetRenderTargets(
                framebuffer.rtv_descriptors.len() as u32,
                if framebuffer.rtv_descriptors.is_empty() {
                    None
                } else {
                    Some(framebuffer.rtv_descriptors.as_ptr())
                },
                false,
                framebuffer
                    .dsv_descriptor
                    .as_ref()
                    .map(|handle| handle as *const D3D12_CPU_DESCRIPTOR_HANDLE),
            );
            self.set_viewport(
                area.0 as f32,
                area.1 as f32,
                area.2 as f32,
                area.3 as f32,
                0.0,
                1.0,
            );
            self.set_scissor(area.0 as i32, area.1 as i32, area.2, area.3);
        }
    }

    fn bind_pipeline(&mut self, pipeline: PipelineHandle) {
        unsafe {
            let device = &*self.device;
            let (_, table_idx) = Dx12Device::decode_handle(pipeline.index);
            if let Some(inner) = device.pipelines.get(table_idx) {
                self.cmd_list.SetPipelineState(&inner.pso);
                self.cmd_list
                    .IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY(inner.topology as i32));
            }
        }
    }

    fn bind_vertex_buffers(&mut self, buffers: &[BufferHandle], offsets: &[u64]) {
        unsafe {
            let device = &*self.device;
            let views: Vec<D3D12_VERTEX_BUFFER_VIEW> = buffers
                .iter()
                .zip(offsets.iter().chain(std::iter::repeat(&0u64)))
                .filter_map(|(bh, &off)| {
                    let (_, table_idx) = Dx12Device::decode_handle(bh.index);
                    device.buffers.get(table_idx).map(|inner| {
                        let gpu_addr = inner.resource.GetGPUVirtualAddress() + off;
                        D3D12_VERTEX_BUFFER_VIEW {
                            BufferLocation: gpu_addr,
                            SizeInBytes: (inner.size - off) as u32,
                            StrideInBytes: inner.vertex_stride,
                        }
                    })
                })
                .collect();
            if !views.is_empty() {
                self.cmd_list.IASetVertexBuffers(0, Some(&views));
            }
        }
    }

    fn bind_index_buffer(&mut self, buffer: BufferHandle, offset: u64, index_format: IndexFormat) {
        unsafe {
            let device = &*self.device;
            let (_, table_idx) = Dx12Device::decode_handle(buffer.index);
            if let Some(inner) = device.buffers.get(table_idx) {
                let fmt = match index_format {
                    IndexFormat::U16 => DXGI_FORMAT_R16_UINT,
                    IndexFormat::U32 => DXGI_FORMAT_R32_UINT,
                };
                let gpu_addr = inner.resource.GetGPUVirtualAddress() + offset;
                let ibv = D3D12_INDEX_BUFFER_VIEW {
                    BufferLocation: gpu_addr,
                    SizeInBytes: (inner.size - offset) as u32,
                    Format: fmt,
                };
                self.cmd_list.IASetIndexBuffer(Some(&ibv));
            }
        }
    }

    fn bind_descriptor_sets(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _first_set: u32,
        sets: &[render_core::DescriptorSetHandle],
        _dynamic_offsets: &[u32],
    ) -> Result<(), render_core::RhiError> {
        if sets.is_empty() {
            // Binding zero sets is a valid no-op. Scene data uses the
            // explicit root-CBV and sampled-texture bridge methods below.
            return Ok(());
        }
        Err(render_core::RhiError::UnsupportedFeature {
            feature: "portable descriptor-set handles on the DX12 backend".to_owned(),
        })
    }

    fn bind_sampled_texture(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        texture: TextureHandle,
    ) -> bool {
        unsafe {
            let device = &*self.device;
            let (_, layout_index) = Dx12Device::decode_handle(pipeline_layout.index);
            let Some(layout) = device.pipeline_layouts.get(layout_index) else {
                return false;
            };
            let (_, texture_index) = Dx12Device::decode_handle(texture.index);
            let Some(texture) = device.textures.get(texture_index) else {
                return false;
            };
            let (Some(srv_parameter), Some(sampler_parameter)) =
                (layout.sampled_texture_parameter, layout.sampler_parameter)
            else {
                return false;
            };
            let (Some(srv_heap), Some(sampler_heap)) = (
                texture.sampled_srv_heap.as_ref(),
                texture.sampled_sampler_heap.as_ref(),
            ) else {
                return false;
            };
            let heaps = [Some(srv_heap.clone()), Some(sampler_heap.clone())];
            self.cmd_list.SetDescriptorHeaps(&heaps);
            self.cmd_list.SetGraphicsRootDescriptorTable(
                srv_parameter,
                srv_heap.GetGPUDescriptorHandleForHeapStart(),
            );
            self.cmd_list.SetGraphicsRootDescriptorTable(
                sampler_parameter,
                sampler_heap.GetGPUDescriptorHandleForHeapStart(),
            );
            true
        }
    }

    fn bind_sampled_texture_pair(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        base_color: TextureHandle,
        shadow_map: TextureHandle,
    ) -> bool {
        unsafe {
            let device = &*self.device;
            let (_, layout_index) = Dx12Device::decode_handle(pipeline_layout.index);
            let Some(layout) = device.pipeline_layouts.get(layout_index) else {
                return false;
            };
            let Some(srv_parameter) = layout.sampled_texture_parameter else {
                return false;
            };
            let Some(sampler_parameter) = layout.sampler_parameter else {
                return false;
            };
            let texture = |handle: TextureHandle| {
                let (_, index) = Dx12Device::decode_handle(handle.index);
                device.textures.get(index)
            };
            let (Some(base), Some(shadow)) = (texture(base_color), texture(shadow_map)) else {
                return false;
            };
            let (Some(base_srv), Some(base_sampler), Some(shadow_srv), Some(shadow_sampler)) = (
                base.sampled_srv_heap.as_ref(),
                base.sampled_sampler_heap.as_ref(),
                shadow.sampled_srv_heap.as_ref(),
                shadow.sampled_sampler_heap.as_ref(),
            ) else {
                return false;
            };
            let srv_heap: ID3D12DescriptorHeap =
                match device
                    .device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        NumDescriptors: 2,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    }) {
                    Ok(heap) => heap,
                    Err(_) => return false,
                };
            let sampler_heap: ID3D12DescriptorHeap =
                match device
                    .device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
                        NumDescriptors: 2,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    }) {
                    Ok(heap) => heap,
                    Err(_) => return false,
                };
            let srv_size = device
                .device
                .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)
                as usize;
            let sampler_size = device
                .device
                .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER)
                as usize;
            let srv_start = srv_heap.GetCPUDescriptorHandleForHeapStart();
            let sampler_start = sampler_heap.GetCPUDescriptorHandleForHeapStart();
            device.device.CopyDescriptorsSimple(
                1,
                srv_start,
                base_srv.GetCPUDescriptorHandleForHeapStart(),
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            );
            device.device.CopyDescriptorsSimple(
                1,
                D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: srv_start.ptr + srv_size,
                },
                shadow_srv.GetCPUDescriptorHandleForHeapStart(),
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            );
            device.device.CopyDescriptorsSimple(
                1,
                sampler_start,
                base_sampler.GetCPUDescriptorHandleForHeapStart(),
                D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
            );
            device.device.CopyDescriptorsSimple(
                1,
                D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: sampler_start.ptr + sampler_size,
                },
                shadow_sampler.GetCPUDescriptorHandleForHeapStart(),
                D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
            );
            self.cmd_list
                .SetDescriptorHeaps(&[Some(srv_heap.clone()), Some(sampler_heap.clone())]);
            self.cmd_list.SetGraphicsRootDescriptorTable(
                srv_parameter,
                srv_heap.GetGPUDescriptorHandleForHeapStart(),
            );
            self.cmd_list.SetGraphicsRootDescriptorTable(
                sampler_parameter,
                sampler_heap.GetGPUDescriptorHandleForHeapStart(),
            );
            let Ok(mut in_flight) = device.descriptor_heaps_in_flight.lock() else {
                return false;
            };
            in_flight.push(srv_heap);
            in_flight.push(sampler_heap);
            true
        }
    }

    fn bind_sampled_texture_set(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        textures: &[TextureHandle],
    ) -> bool {
        if textures.is_empty() {
            return false;
        }
        unsafe {
            let device = &*self.device;
            let (_, layout_index) = Dx12Device::decode_handle(pipeline_layout.index);
            let Some(layout) = device.pipeline_layouts.get(layout_index) else {
                return false;
            };
            let (Some(srv_parameter), Some(sampler_parameter)) =
                (layout.sampled_texture_parameter, layout.sampler_parameter)
            else {
                return false;
            };
            let mut sources = Vec::with_capacity(textures.len());
            for handle in textures {
                let (_, index) = Dx12Device::decode_handle(handle.index);
                let Some(texture) = device.textures.get(index) else {
                    return false;
                };
                let (Some(srv), Some(sampler)) = (
                    texture.sampled_srv_heap.as_ref(),
                    texture.sampled_sampler_heap.as_ref(),
                ) else {
                    return false;
                };
                sources.push((srv, sampler));
            }
            let descriptor_count = textures.len() as u32;
            let srv_heap: ID3D12DescriptorHeap =
                match device
                    .device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                        NumDescriptors: descriptor_count,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    }) {
                    Ok(heap) => heap,
                    Err(_) => return false,
                };
            let sampler_heap: ID3D12DescriptorHeap =
                match device
                    .device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
                        NumDescriptors: descriptor_count,
                        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                        NodeMask: 0,
                    }) {
                    Ok(heap) => heap,
                    Err(_) => return false,
                };
            let srv_size = device
                .device
                .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)
                as usize;
            let sampler_size = device
                .device
                .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER)
                as usize;
            let srv_start = srv_heap.GetCPUDescriptorHandleForHeapStart();
            let sampler_start = sampler_heap.GetCPUDescriptorHandleForHeapStart();
            for (index, (srv, sampler)) in sources.into_iter().enumerate() {
                device.device.CopyDescriptorsSimple(
                    1,
                    D3D12_CPU_DESCRIPTOR_HANDLE {
                        ptr: srv_start.ptr + index * srv_size,
                    },
                    srv.GetCPUDescriptorHandleForHeapStart(),
                    D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                );
                device.device.CopyDescriptorsSimple(
                    1,
                    D3D12_CPU_DESCRIPTOR_HANDLE {
                        ptr: sampler_start.ptr + index * sampler_size,
                    },
                    sampler.GetCPUDescriptorHandleForHeapStart(),
                    D3D12_DESCRIPTOR_HEAP_TYPE_SAMPLER,
                );
            }
            self.cmd_list
                .SetDescriptorHeaps(&[Some(srv_heap.clone()), Some(sampler_heap.clone())]);
            self.cmd_list.SetGraphicsRootDescriptorTable(
                srv_parameter,
                srv_heap.GetGPUDescriptorHandleForHeapStart(),
            );
            self.cmd_list.SetGraphicsRootDescriptorTable(
                sampler_parameter,
                sampler_heap.GetGPUDescriptorHandleForHeapStart(),
            );
            let Ok(mut in_flight) = device.descriptor_heaps_in_flight.lock() else {
                return false;
            };
            in_flight.push(srv_heap);
            in_flight.push(sampler_heap);
            true
        }
    }

    fn bind_uniform_buffer(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        buffer: BufferHandle,
    ) -> bool {
        unsafe {
            let device = &*self.device;
            let (_, layout_index) = Dx12Device::decode_handle(pipeline_layout.index);
            let Some(layout) = device.pipeline_layouts.get(layout_index) else {
                return false;
            };
            let Some(parameter) = layout.uniform_buffer_parameter else {
                return false;
            };
            let (_, buffer_index) = Dx12Device::decode_handle(buffer.index);
            let Some(buffer) = device.buffers.get(buffer_index) else {
                return false;
            };
            self.cmd_list.SetGraphicsRootConstantBufferView(
                parameter,
                buffer.resource.GetGPUVirtualAddress(),
            );
            true
        }
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
        first_vertex: u32,
        first_instance: u32,
    ) {
        unsafe {
            self.cmd_list
                .DrawInstanced(vertex_count, instance_count, first_vertex, first_instance);
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
        first_instance: u32,
    ) {
        unsafe {
            self.cmd_list.DrawIndexedInstanced(
                index_count,
                instance_count,
                first_index,
                vertex_offset,
                first_instance,
            );
        }
        self.draws += 1;
        self.triangles += index_count as u64 / 3 * instance_count as u64;
    }

    fn end_render_pass(&mut self) {
        unsafe {
            if let Some(depth) = self.active_depth_attachment.take() {
                Dx12Device::transition_resource(
                    &self.cmd_list,
                    &depth,
                    D3D12_RESOURCE_STATE_DEPTH_WRITE,
                    D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                );
                self.cmd_list.OMSetRenderTargets(
                    1,
                    Some(&self.main_rtv as *const _ as *const _),
                    false,
                    Some(&self.main_dsv),
                );
            }
        }
    }

    fn push_constants(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        _stage_flags: u32,
        offset: u32,
        data: &[u8],
    ) {
        unsafe {
            let device = &*self.device;
            let (_, table_idx) = Dx12Device::decode_handle(pipeline_layout.index);
            let Some(inner) = device.pipeline_layouts.get(table_idx) else {
                return;
            };
            let Some(root_parameter) = inner.root_constants_parameter else {
                return;
            };
            let num_constants = (data.len() / 4) as u32;
            if num_constants > 0 {
                let u32_data: Vec<u32> = data
                    .chunks_exact(4)
                    .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                self.cmd_list.SetGraphicsRoot32BitConstants(
                    root_parameter,
                    num_constants,
                    u32_data.as_ptr() as *const _,
                    offset / 4,
                );
            }
        }
    }
}
