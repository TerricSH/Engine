use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    Arc, Mutex,
};

use glow::HasContext;
use render_core::*;

use crate::device::{
    gl_binding_point, BufferSlot, OpenGlDevice, PipelineBlendState, PipelineSlot,
    PushUniformBinding, PushUniformKind, TextureSlot,
};

pub(crate) struct FrameState {
    draw_calls: AtomicU32,
    triangles: AtomicU64,
    error: Mutex<Option<String>>,
}

impl FrameState {
    fn record_error(&self, detail: impl Into<String>) {
        let detail = detail.into();
        tracing::error!(target: "opengl", %detail, "OpenGL command validation failed");
        let mut error = self
            .error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if error.is_none() {
            *error = Some(detail);
        }
    }

    fn record_draw(&self, topology: u32, primitive_vertices: u32, instance_count: u32) {
        self.draw_calls.fetch_add(1, Ordering::Relaxed);
        let primitives =
            primitive_count(topology, primitive_vertices).saturating_mul(u64::from(instance_count));
        self.triangles.fetch_add(primitives, Ordering::Relaxed);
    }
}

pub(crate) fn primitive_count(topology: u32, vertex_count: u32) -> u64 {
    match topology {
        glow::TRIANGLES => u64::from(vertex_count / 3),
        glow::TRIANGLE_STRIP | glow::TRIANGLE_FAN => u64::from(vertex_count.saturating_sub(2)),
        _ => 0,
    }
}

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
    pub(crate) current_render_pass: Option<RenderPassHandle>,
    pub(crate) current_pipeline: Option<PipelineSlot>,
    pub(crate) current_vertex_buffer: Option<(glow::Buffer, u64, u64)>,
    pub(crate) current_index_buffer: Option<glow::Buffer>,
    pub(crate) current_index_buffer_size: u64,
    pub(crate) current_index_format: IndexFormat,
    pub(crate) current_index_offset: u64,
    pub(crate) frame_state: Arc<FrameState>,
}

impl Drop for OpenGlCommandEncoder {
    fn drop(&mut self) {
        if self.current_render_pass.is_some() {
            self.frame_state
                .record_error("OpenGL command encoder was dropped with an active render pass");
        }
    }
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

    /// Resolve a pipeline handle to an immutable GL binding snapshot.
    fn resolve_pipeline(&self, handle: PipelineHandle) -> Option<PipelineSlot> {
        // SAFETY: Same as buffer_slot — device_ptr is valid for the encoder's
        // lifetime (single-frame scope, dropped before the device).
        let device = unsafe { &*self.device_ptr.0 };
        device
            .pipelines
            .get(handle.index)
            .filter(|s| s.generation == handle.generation)
            .map(|s| s.value.clone())
    }

    fn texture_slot(&self, handle: TextureHandle) -> Option<&TextureSlot> {
        // SAFETY: Same lifetime invariant as `buffer_slot`.
        let device = unsafe { &*self.device_ptr.0 };
        device
            .textures
            .get(handle.index)
            .filter(|slot| slot.generation == handle.generation)
            .map(|slot| &slot.value)
    }

    fn resolve_layout(&self, handle: PipelineLayoutHandle) -> Option<&PipelineLayoutDescriptor> {
        // SAFETY: Same lifetime invariant as `buffer_slot`.
        let device = unsafe { &*self.device_ptr.0 };
        device
            .pipeline_layouts
            .get(handle.index)
            .filter(|slot| slot.generation == handle.generation)
            .map(|slot| &slot.value.descriptor)
    }

    /// Resolve a framebuffer handle to its GL framebuffer.
    fn resolve_framebuffer(
        &self,
        handle: FramebufferHandle,
    ) -> Option<(glow::Framebuffer, RenderPassHandle)> {
        // SAFETY: Same as buffer_slot — device_ptr is valid for the encoder's
        // lifetime (single-frame scope, dropped before the device).
        let device = unsafe { &*self.device_ptr.0 };
        device
            .framebuffers
            .get(handle.index)
            .filter(|s| s.generation == handle.generation)
            .map(|s| (s.value.gl_framebuffer, s.value.render_pass))
    }

    fn record_error(&self, detail: impl Into<String>) {
        self.frame_state.record_error(detail);
    }

    fn supports_base_instance(&self) -> bool {
        let version = self.gl.version();
        !version.is_embedded && (version.major > 4 || (version.major == 4 && version.minor >= 2))
    }

    fn validate_current_layout(&self, layout: PipelineLayoutHandle) -> bool {
        match &self.current_pipeline {
            Some(pipeline) if pipeline.pipeline_layout == Some(layout) => true,
            Some(_) => {
                self.record_error(
                    "binding used a pipeline layout different from the bound pipeline",
                );
                false
            }
            None => {
                self.record_error("resource binding requires a bound OpenGL pipeline");
                false
            }
        }
    }

    fn resource_binding(
        &self,
        layout: PipelineLayoutHandle,
        accepted_kinds: &[&str],
    ) -> Option<u32> {
        let descriptor = self.resolve_layout(layout)?;
        descriptor.bind_group_layouts.iter().find_map(|set| {
            set.bindings.iter().find_map(|binding| {
                let kind = binding.resource_kind.trim().to_ascii_lowercase();
                accepted_kinds
                    .iter()
                    .any(|accepted| kind == *accepted)
                    .then(|| gl_binding_point(set.set_index, binding.binding))
                    .flatten()
            })
        })
    }

    fn configure_vertex_input(&self, pipeline: &PipelineSlot, buffer: glow::Buffer, base: u64) {
        if base > i32::MAX as u64 {
            self.record_error(format!(
                "vertex buffer offset {base} exceeds OpenGL i32 range"
            ));
            return;
        }
        // SAFETY: the VAO and buffer are live objects owned by the device. All
        // attribute formats were validated before the pipeline was created.
        unsafe {
            self.gl.bind_vertex_array(Some(pipeline.gl_vertex_array));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            for attribute in &pipeline.vertex_attributes {
                let Some(offset) = base.checked_add(u64::from(attribute.offset_bytes)) else {
                    self.record_error("vertex attribute offset overflowed u64");
                    return;
                };
                if offset > i32::MAX as u64 {
                    self.record_error(format!(
                        "vertex attribute offset {offset} exceeds OpenGL i32 range"
                    ));
                    return;
                }
                if attribute.format.integer {
                    self.gl.vertex_attrib_pointer_i32(
                        attribute.location,
                        attribute.format.component_count,
                        attribute.format.gl_type,
                        pipeline.vertex_stride as i32,
                        offset as i32,
                    );
                } else {
                    self.gl.vertex_attrib_pointer_f32(
                        attribute.location,
                        attribute.format.component_count,
                        attribute.format.gl_type,
                        attribute.format.normalized,
                        pipeline.vertex_stride as i32,
                        offset as i32,
                    );
                }
            }
        }
    }

    fn bind_texture_unit(&self, texture: TextureHandle, unit: u32, sampler_index: usize) -> bool {
        let Some(texture) = self.texture_slot(texture) else {
            self.record_error("sampled-texture binding received an invalid texture handle");
            return false;
        };
        let Some(pipeline) = &self.current_pipeline else {
            self.record_error("sampled-texture binding requires a bound pipeline");
            return false;
        };
        let Some(sampler) = pipeline.sampler_uniforms.get(sampler_index) else {
            self.record_error(format!(
                "bound GLSL program does not expose sampled texture uniform #{sampler_index}"
            ));
            return false;
        };
        let max_units = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_COMBINED_TEXTURE_IMAGE_UNITS)
                .max(0) as u32
        };
        if unit >= max_units {
            self.record_error(format!(
                "texture unit {unit} exceeds OpenGL limit {max_units}"
            ));
            return false;
        }
        // SAFETY: the program, uniform location, and texture are live objects
        // from the same current context; the texture unit was limit-checked.
        unsafe {
            self.gl.active_texture(glow::TEXTURE0 + unit);
            self.gl
                .bind_texture(glow::TEXTURE_2D, Some(texture.gl_texture));
            self.gl.uniform_1_i32(Some(&sampler.location), unit as i32);
        }
        true
    }

    fn upload_push_uniform(&self, binding: &PushUniformBinding, bytes: &[u8]) -> bool {
        if bytes.len() != binding.kind.size_bytes() as usize {
            self.record_error(format!(
                "push uniform `{}` expected {} bytes, received {}",
                binding.name,
                binding.kind.size_bytes(),
                bytes.len()
            ));
            return false;
        }
        let floats = || {
            bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_ne_bytes(chunk.try_into().expect("four-byte chunk")))
                .collect::<Vec<_>>()
        };
        let ints = || {
            bytes
                .chunks_exact(4)
                .map(|chunk| i32::from_ne_bytes(chunk.try_into().expect("four-byte chunk")))
                .collect::<Vec<_>>()
        };
        let uints = || {
            bytes
                .chunks_exact(4)
                .map(|chunk| u32::from_ne_bytes(chunk.try_into().expect("four-byte chunk")))
                .collect::<Vec<_>>()
        };
        // SAFETY: reflection supplied the uniform's exact type and location;
        // byte counts were checked above before conversion.
        unsafe {
            match binding.kind {
                PushUniformKind::Float(1) => {
                    self.gl.uniform_1_f32(Some(&binding.location), floats()[0])
                }
                PushUniformKind::Float(2) => {
                    let values = floats();
                    self.gl
                        .uniform_2_f32(Some(&binding.location), values[0], values[1]);
                }
                PushUniformKind::Float(3) => {
                    let values = floats();
                    self.gl
                        .uniform_3_f32(Some(&binding.location), values[0], values[1], values[2]);
                }
                PushUniformKind::Float(4) => {
                    let values = floats();
                    self.gl.uniform_4_f32(
                        Some(&binding.location),
                        values[0],
                        values[1],
                        values[2],
                        values[3],
                    );
                }
                PushUniformKind::Int(1) => {
                    self.gl.uniform_1_i32(Some(&binding.location), ints()[0]);
                }
                PushUniformKind::Int(2) => {
                    let values = ints();
                    self.gl
                        .uniform_2_i32(Some(&binding.location), values[0], values[1]);
                }
                PushUniformKind::Int(3) => {
                    let values = ints();
                    self.gl
                        .uniform_3_i32(Some(&binding.location), values[0], values[1], values[2]);
                }
                PushUniformKind::Int(4) => {
                    let values = ints();
                    self.gl.uniform_4_i32(
                        Some(&binding.location),
                        values[0],
                        values[1],
                        values[2],
                        values[3],
                    );
                }
                PushUniformKind::Uint(1) => {
                    self.gl.uniform_1_u32(Some(&binding.location), uints()[0]);
                }
                PushUniformKind::Uint(2) => {
                    let values = uints();
                    self.gl
                        .uniform_2_u32(Some(&binding.location), values[0], values[1]);
                }
                PushUniformKind::Uint(3) => {
                    let values = uints();
                    self.gl
                        .uniform_3_u32(Some(&binding.location), values[0], values[1], values[2]);
                }
                PushUniformKind::Uint(4) => {
                    let values = uints();
                    self.gl.uniform_4_u32(
                        Some(&binding.location),
                        values[0],
                        values[1],
                        values[2],
                        values[3],
                    );
                }
                PushUniformKind::Mat2 => {
                    self.gl
                        .uniform_matrix_2_f32_slice(Some(&binding.location), false, &floats())
                }
                PushUniformKind::Mat3 => {
                    self.gl
                        .uniform_matrix_3_f32_slice(Some(&binding.location), false, &floats())
                }
                PushUniformKind::Mat4 => {
                    self.gl
                        .uniform_matrix_4_f32_slice(Some(&binding.location), false, &floats())
                }
                _ => {
                    self.record_error(format!(
                        "push uniform `{}` uses an unsupported vector width",
                        binding.name
                    ));
                    return false;
                }
            }
        }
        true
    }
}

mod bindings;
mod constants;
mod draw;
mod pass;

use bindings::impl_encoder_bindings;
use constants::impl_encoder_constants;
use draw::impl_encoder_draw;
use pass::impl_encoder_pass;

impl CommandEncoder for OpenGlCommandEncoder {
    impl_encoder_pass!();
    impl_encoder_bindings!();
    impl_encoder_draw!();
    impl_encoder_constants!();
}

#[cfg(test)]
#[path = "encoder/tests.rs"]
mod tests;
