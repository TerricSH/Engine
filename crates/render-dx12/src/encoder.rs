#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use windows::{
    Win32::Foundation::RECT,
    Win32::Graphics::Direct3D12::*,
};

use render_core::{
    BufferHandle, CommandEncoder, FramebufferHandle, IndexFormat, PipelineHandle,
    PipelineLayoutHandle, RenderPassHandle,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(crate) struct Dx12CommandEncoder {
    pub(crate) cmd_list: ID3D12GraphicsCommandList,
    pub(crate) draws: u32,
    pub(crate) triangles: u64,
}

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
impl Dx12CommandEncoder {
    pub(crate) fn new(cmd_list: ID3D12GraphicsCommandList) -> Self {
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
