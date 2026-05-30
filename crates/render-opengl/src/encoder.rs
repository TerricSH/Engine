use std::sync::Arc;

use glow::HasContext;
use render_core::*;

use crate::device::{BufferSlot, OpenGlDevice};

/// Raw-pointer wrapper so we can resolve handle to GL object inside the
/// encoder without borrowing the encoder (which would conflict with the
/// mutable GL accessor).
pub(crate) struct DeviceRef(pub(crate) *const OpenGlDevice);

// SAFETY: The encoder is created per-frame in begin_frame() and consumed in
// end_frame(), before the owning OpenGlDevice is dropped. The raw pointer in
// device_ptr points to the OpenGlDevice that outlives this encoder. Encoder
// methods only read from the device (handle resolution), never mutate it,
// so no data races are possible within the single-frame scope.
unsafe impl Send for OpenGlCommandEncoder {}

pub struct OpenGlCommandEncoder {
    pub(crate) gl: Arc<glow::Context>,
    pub(crate) device_ptr: DeviceRef,
    pub(crate) current_program: Option<glow::Program>,
    pub(crate) current_framebuffer: Option<glow::Framebuffer>,
}

impl OpenGlCommandEncoder {
    /// Resolve a buffer handle.
    fn buffer_slot(&self, handle: BufferHandle) -> Option<&BufferSlot> {
        // SAFETY: device_ptr points to the OpenGlDevice that outlives this
        // encoder — the encoder is created and used within a single frame
        // and dropped before the device.
        let device = unsafe { &*self.device_ptr.0 };
        device
            .buffers
            .get(handle.index)
            .filter(|s| s.generation == handle.generation)
            .map(|s| &s.value)
    }

    /// Resolve a pipeline handle to its GL program.
    fn resolve_pipeline(&self, handle: PipelineHandle) -> Option<glow::Program> {
        // SAFETY: Same as buffer_slot — device_ptr is valid for the encoder's
        // lifetime (single-frame scope, dropped before the device).
        let device = unsafe { &*self.device_ptr.0 };
        device
            .pipelines
            .get(handle.index)
            .filter(|s| s.generation == handle.generation)
            .map(|s| s.value.gl_program)
    }

    /// Resolve a framebuffer handle to its GL framebuffer.
    fn resolve_framebuffer(&self, handle: FramebufferHandle) -> Option<glow::Framebuffer> {
        // SAFETY: Same as buffer_slot — device_ptr is valid for the encoder's
        // lifetime (single-frame scope, dropped before the device).
        let device = unsafe { &*self.device_ptr.0 };
        device
            .framebuffers
            .get(handle.index)
            .filter(|s| s.generation == handle.generation)
            .map(|s| s.value.gl_framebuffer)
    }
}

impl CommandEncoder for OpenGlCommandEncoder {
    fn begin_render_pass(
        &mut self,
        _render_pass: RenderPassHandle,
        framebuffer: FramebufferHandle,
        area: (u32, u32, u32, u32),
        clear_color: [f32; 4],
        clear_depth: Option<f32>,
    ) {
        let fb = self.resolve_framebuffer(framebuffer);

        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, fb);
            self.gl.clear_color(
                clear_color[0],
                clear_color[1],
                clear_color[2],
                clear_color[3],
            );
            if let Some(d) = clear_depth {
                self.gl.clear_depth_f64(d as f64);
            }
            self.gl
                .viewport(area.0 as i32, area.1 as i32, area.2 as i32, area.3 as i32);
            self.gl
                .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        }

        self.current_framebuffer = fb;
    }

    fn bind_pipeline(&mut self, pipeline: PipelineHandle) {
        if let Some(program) = self.resolve_pipeline(pipeline) {
            unsafe {
                self.gl.use_program(Some(program));
            }
            self.current_program = Some(program);
        }
    }

    fn bind_vertex_buffers(&mut self, buffers: &[BufferHandle], _offsets: &[u64]) {
        for &handle in buffers {
            if let Some(slot) = self.buffer_slot(handle) {
                unsafe {
                    self.gl
                        .bind_buffer(glow::ARRAY_BUFFER, Some(slot.gl_buffer));
                }
            }
        }
    }

    fn bind_index_buffer(
        &mut self,
        buffer: BufferHandle,
        _offset: u64,
        _index_format: IndexFormat,
    ) {
        if let Some(slot) = self.buffer_slot(buffer) {
            unsafe {
                self.gl
                    .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(slot.gl_buffer));
            }
        }
    }

    fn bind_descriptor_sets(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _first_set: u32,
        _sets: &[DescriptorSetHandle],
        _dynamic_offsets: &[u32],
    ) {
        tracing::trace!(target: "opengl", "bind_descriptor_sets (stub)");
    }

    fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, _min_depth: f32, _max_depth: f32) {
        unsafe {
            self.gl.viewport(x as i32, y as i32, w as i32, h as i32);
        }
    }

    fn set_scissor(&mut self, x: i32, y: i32, w: u32, h: u32) {
        unsafe {
            self.gl.scissor(x, y, w as i32, h as i32);
        }
    }

    fn draw(
        &mut self,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        _first_instance: u32,
    ) {
        unsafe {
            if instance_count > 1 {
                self.gl.draw_arrays_instanced(
                    glow::TRIANGLES,
                    first_vertex as i32,
                    vertex_count as i32,
                    instance_count as i32,
                );
            } else {
                self.gl
                    .draw_arrays(glow::TRIANGLES, first_vertex as i32, vertex_count as i32);
            }
        }
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
            let offset_bytes = (first_index as i32) * 4 + vertex_offset * 4;
            if instance_count > 1 {
                self.gl.draw_elements_instanced(
                    glow::TRIANGLES,
                    index_count as i32,
                    glow::UNSIGNED_INT,
                    offset_bytes,
                    instance_count as i32,
                );
            } else {
                self.gl.draw_elements(
                    glow::TRIANGLES,
                    index_count as i32,
                    glow::UNSIGNED_INT,
                    offset_bytes,
                );
            }
        }
    }

    fn push_constants(
        &mut self,
        _layout: PipelineLayoutHandle,
        _stage_flags: u32,
        _offset: u32,
        data: &[u8],
    ) {
        tracing::trace!(target: "opengl", "push_constants {} bytes (stub)", data.len());
    }

    fn end_render_pass(&mut self) {
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        self.current_framebuffer = None;
    }
}
