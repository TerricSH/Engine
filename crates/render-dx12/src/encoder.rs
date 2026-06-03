#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::{
    Win32::Foundation::RECT,
    Win32::Graphics::Direct3D::*,
    Win32::Graphics::Direct3D12::*,
    Win32::Graphics::Dxgi::Common::*,
};

use render_core::{
    BufferHandle, CommandEncoder, FramebufferHandle, IndexFormat, PipelineHandle,
    PipelineLayoutHandle, RenderPassHandle,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::device::Dx12Device;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(crate) struct Dx12CommandEncoder {
    pub(crate) cmd_list: ID3D12GraphicsCommandList,
    device: *const Dx12Device,
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
    pub(crate) fn new(cmd_list: ID3D12GraphicsCommandList, device: *const Dx12Device) -> Self {
        Self {
            cmd_list,
            device,
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
        // D3D12 render pass: the device's begin_frame sets the RTV via
        // OMSetRenderTargets and clears. This method is called for API
        // compatibility with the Vulkan path but is a no-op here.
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
                .zip(
                    offsets
                        .iter()
                        .chain(std::iter::repeat(&0u64))
                )
                .filter_map(|(bh, &off)| {
                    let (_, table_idx) = Dx12Device::decode_handle(bh.index);
                    device.buffers.get(table_idx).map(|inner| {
                        let gpu_addr = inner.resource.GetGPUVirtualAddress() + off;
                        D3D12_VERTEX_BUFFER_VIEW {
                            BufferLocation: gpu_addr,
                            SizeInBytes: (inner.size - off) as u32,
                            StrideInBytes: 32,
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
        _sets: &[render_core::DescriptorSetHandle],
        _dynamic_offsets: &[u32],
    ) {
        // D3D12 root descriptor / descriptor table binding requires:
        // 1. ID3D12DescriptorHeap for CBV/SRV/UAV
        // 2. SetDescriptorHeaps on the command list
        // 3. SetGraphicsRootDescriptorTable for each root parameter
        //
        // A complete implementation requires mapping the engine's descriptor
        // set abstraction to D3D12 descriptor heaps. For now this is a
        // placeholder — the MVP path in Dx12SceneRenderer uses root
        // constants for material data instead of descriptor tables.
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
        // D3D12 does not have an explicit end-render-pass concept like Vulkan.
    }

    fn push_constants(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _stage_flags: u32,
        _offset: u32,
        _data: &[u8],
    ) {
        unsafe {
            let device = &*self.device;
            let (_, table_idx) = Dx12Device::decode_handle(_pipeline_layout.index);
            let _inner = device.pipeline_layouts.get(table_idx);
            let num_constants = (_data.len() / 4) as u32;
            if num_constants > 0 {
                let u32_data: Vec<u32> = _data
                    .chunks_exact(4)
                    .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                self.cmd_list.SetGraphicsRoot32BitConstants(
                    0,
                    num_constants,
                    u32_data.as_ptr() as *const _,
                    0,
                );
            }
        }
    }
}
