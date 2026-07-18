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

impl CommandEncoder for OpenGlCommandEncoder {
    fn begin_render_pass(
        &mut self,
        render_pass: RenderPassHandle,
        framebuffer: FramebufferHandle,
        area: (u32, u32, u32, u32),
        clear_color: [f32; 4],
        clear_depth: Option<f32>,
    ) {
        if self.current_render_pass.is_some() {
            self.record_error("nested OpenGL render passes are not supported");
            return;
        }
        let Some((fb, framebuffer_render_pass)) = self.resolve_framebuffer(framebuffer) else {
            self.record_error("begin_render_pass received an invalid framebuffer handle");
            return;
        };
        if framebuffer_render_pass != render_pass {
            self.record_error("framebuffer was created for a different render pass");
            return;
        }
        if area.2 == 0
            || area.3 == 0
            || area.0 > i32::MAX as u32
            || area.1 > i32::MAX as u32
            || area.2 > i32::MAX as u32
            || area.3 > i32::MAX as u32
        {
            self.record_error("render-pass area must be non-zero and fit OpenGL signed integers");
            return;
        }
        if clear_color.iter().any(|channel| !channel.is_finite())
            || clear_depth.is_some_and(|depth| !depth.is_finite() || !(0.0..=1.0).contains(&depth))
        {
            self.record_error("render-pass clear values must be finite and depth must be in 0..=1");
            return;
        }

        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the framebuffer handle was created by the same context, and the
        // encoder is dropped before the device.
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
            self.gl.clear_color(
                clear_color[0],
                clear_color[1],
                clear_color[2],
                clear_color[3],
            );
            if let Some(d) = clear_depth {
                self.gl.clear_depth_f64(d as f64);
                self.gl.depth_mask(true);
            }
            self.gl
                .viewport(area.0 as i32, area.1 as i32, area.2 as i32, area.3 as i32);
            let mut clear_mask = glow::COLOR_BUFFER_BIT;
            if clear_depth.is_some() {
                clear_mask |= glow::DEPTH_BUFFER_BIT;
            }
            self.gl.clear(clear_mask);
        }

        self.current_framebuffer = Some(fb);
        self.current_render_pass = Some(render_pass);
    }

    fn bind_pipeline(&mut self, pipeline: PipelineHandle) {
        let Some(pipeline) = self.resolve_pipeline(pipeline) else {
            self.record_error("bind_pipeline received an invalid pipeline handle");
            return;
        };
        if let (Some(active), Some(expected)) = (self.current_render_pass, pipeline.render_pass) {
            if active != expected {
                self.record_error("pipeline is incompatible with the active render pass");
                return;
            }
        }
        // SAFETY: all state in the snapshot was validated at pipeline creation
        // and all GL objects belong to this current context.
        unsafe {
            self.gl.use_program(Some(pipeline.gl_program));
            self.gl.bind_vertex_array(Some(pipeline.gl_vertex_array));
            if let Some(cull_face) = pipeline.raster_state.cull_face {
                self.gl.enable(glow::CULL_FACE);
                self.gl.cull_face(cull_face);
            } else {
                self.gl.disable(glow::CULL_FACE);
            }
            self.gl.front_face(pipeline.raster_state.front_face);
            if !self.gl.version().is_embedded {
                self.gl
                    .polygon_mode(glow::FRONT_AND_BACK, pipeline.raster_state.polygon_mode);
            }
            if pipeline.depth_state.enabled {
                self.gl.enable(glow::DEPTH_TEST);
                self.gl.depth_func(pipeline.depth_state.compare);
            } else {
                self.gl.disable(glow::DEPTH_TEST);
            }
            self.gl.depth_mask(pipeline.depth_state.write_enabled);
            match pipeline.blend_state {
                PipelineBlendState::Disabled => self.gl.disable(glow::BLEND),
                PipelineBlendState::Alpha => {
                    self.gl.enable(glow::BLEND);
                    self.gl.blend_equation(glow::FUNC_ADD);
                    self.gl.blend_func_separate(
                        glow::SRC_ALPHA,
                        glow::ONE_MINUS_SRC_ALPHA,
                        glow::ONE,
                        glow::ONE_MINUS_SRC_ALPHA,
                    );
                }
                PipelineBlendState::PremultipliedAlpha => {
                    self.gl.enable(glow::BLEND);
                    self.gl.blend_equation(glow::FUNC_ADD);
                    self.gl.blend_func_separate(
                        glow::ONE,
                        glow::ONE_MINUS_SRC_ALPHA,
                        glow::ONE,
                        glow::ONE_MINUS_SRC_ALPHA,
                    );
                }
                PipelineBlendState::Additive => {
                    self.gl.enable(glow::BLEND);
                    self.gl.blend_equation(glow::FUNC_ADD);
                    self.gl.blend_func(glow::ONE, glow::ONE);
                }
            }
            if pipeline.multisample {
                self.gl.enable(glow::MULTISAMPLE);
            } else {
                self.gl.disable(glow::MULTISAMPLE);
            }
            if let Some(index_buffer) = self.current_index_buffer {
                self.gl
                    .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buffer));
            }
        }
        if let Some((vertex_buffer, offset, _)) = self.current_vertex_buffer {
            self.configure_vertex_input(&pipeline, vertex_buffer, offset);
        }
        self.current_program = Some(pipeline.gl_program);
        self.current_pipeline = Some(pipeline);
    }

    fn bind_vertex_buffers(&mut self, buffers: &[BufferHandle], offsets: &[u64]) {
        if buffers.len() != offsets.len() {
            self.record_error("vertex buffer and offset counts do not match");
            return;
        }
        if buffers.len() > 1 {
            self.record_error(
                "OpenGL RHI vertex layouts currently describe one interleaved vertex buffer",
            );
            return;
        }
        let Some((&handle, &offset)) = buffers.first().zip(offsets.first()) else {
            self.current_vertex_buffer = None;
            return;
        };
        let Some(slot) = self.buffer_slot(handle) else {
            self.record_error("bind_vertex_buffers received an invalid buffer handle");
            return;
        };
        if slot.usage.0 & BufferUsage::VERTEX.0 == 0 {
            self.record_error("bind_vertex_buffers received a buffer without VERTEX usage");
            return;
        }
        if offset >= slot.size_bytes && slot.size_bytes != 0 {
            self.record_error(format!(
                "vertex buffer offset {offset} is outside {}-byte buffer",
                slot.size_bytes
            ));
            return;
        }
        let gl_buffer = slot.gl_buffer;
        let buffer_size = slot.size_bytes;
        self.current_vertex_buffer = Some((gl_buffer, offset, buffer_size));
        if let Some(pipeline) = &self.current_pipeline {
            self.configure_vertex_input(pipeline, gl_buffer, offset);
        }
    }

    fn bind_index_buffer(&mut self, buffer: BufferHandle, offset: u64, index_format: IndexFormat) {
        let Some(slot) = self.buffer_slot(buffer) else {
            self.record_error("bind_index_buffer received an invalid buffer handle");
            return;
        };
        if slot.usage.0 & BufferUsage::INDEX.0 == 0 {
            self.record_error("bind_index_buffer received a buffer without INDEX usage");
            return;
        }
        let alignment = match index_format {
            IndexFormat::U16 => 2,
            IndexFormat::U32 => 4,
        };
        if !offset.is_multiple_of(alignment) || offset >= slot.size_bytes {
            self.record_error(format!(
                "index buffer offset {offset} is invalid for {:?} in a {}-byte buffer",
                index_format, slot.size_bytes
            ));
            return;
        }
        let gl_buffer = slot.gl_buffer;
        let buffer_size = slot.size_bytes;
        if let Some(pipeline) = &self.current_pipeline {
            // ELEMENT_ARRAY_BUFFER binding is VAO state.
            unsafe {
                self.gl.bind_vertex_array(Some(pipeline.gl_vertex_array));
                self.gl
                    .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(gl_buffer));
            }
        }
        self.current_index_buffer = Some(gl_buffer);
        self.current_index_buffer_size = buffer_size;
        self.current_index_offset = offset;
        self.current_index_format = index_format;
    }

    fn bind_descriptor_sets(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _first_set: u32,
        sets: &[DescriptorSetHandle],
        dynamic_offsets: &[u32],
    ) -> Result<(), RhiError> {
        if sets.is_empty() {
            if dynamic_offsets.is_empty() {
                return Ok(());
            }
            return Err(RhiError::InvalidDescriptor {
                field: "descriptor_sets.dynamic_offsets".to_string(),
                reason: "dynamic offsets were supplied without descriptor sets".to_string(),
            });
        }
        Err(RhiError::UnsupportedFeature {
            feature: "OpenGL descriptor-set allocation; use the typed texture/uniform bridges"
                .to_string(),
        })
    }

    fn bind_sampled_texture(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        texture: TextureHandle,
    ) -> bool {
        if !self.validate_current_layout(pipeline_layout) {
            return false;
        }
        let Some(unit) = self.resource_binding(
            pipeline_layout,
            &[
                "sampled_texture",
                "texture",
                "sampled_image",
                "combined_image_sampler",
            ],
        ) else {
            self.record_error("pipeline layout does not declare a sampled-texture binding");
            return false;
        };
        self.bind_texture_unit(texture, unit, 0)
    }

    fn bind_sampled_texture_pair(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        base_color: TextureHandle,
        shadow_map: TextureHandle,
    ) -> bool {
        if !self.validate_current_layout(pipeline_layout) {
            return false;
        }
        let Some(first_unit) = self.resource_binding(
            pipeline_layout,
            &["sampled_texture_pair", "texture_pair", "sampled_image_pair"],
        ) else {
            self.record_error("pipeline layout does not declare a sampled-texture-pair binding");
            return false;
        };
        let Some(shadow_unit) = first_unit.checked_add(1) else {
            self.record_error("sampled-texture-pair unit overflowed u32");
            return false;
        };
        // Resolve both handles before changing GL state so this bridge is
        // fail-closed rather than leaving a half-updated texture pair.
        if self.texture_slot(base_color).is_none() || self.texture_slot(shadow_map).is_none() {
            self.record_error("sampled-texture-pair binding received an invalid texture handle");
            return false;
        }
        self.bind_texture_unit(base_color, first_unit, 0)
            && self.bind_texture_unit(shadow_map, shadow_unit, 1)
    }

    fn bind_uniform_buffer(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        buffer: BufferHandle,
    ) -> bool {
        if !self.validate_current_layout(pipeline_layout) {
            return false;
        }
        let Some(binding) = self.resource_binding(pipeline_layout, &["uniform_buffer", "ubo"])
        else {
            self.record_error("pipeline layout does not declare a uniform-buffer binding");
            return false;
        };
        let Some(buffer) = self.buffer_slot(buffer) else {
            self.record_error("uniform-buffer binding received an invalid buffer handle");
            return false;
        };
        if buffer.usage.0 & BufferUsage::UNIFORM.0 == 0 {
            self.record_error("uniform-buffer binding received a buffer without UNIFORM usage");
            return false;
        }
        let max_bindings = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_UNIFORM_BUFFER_BINDINGS)
                .max(0) as u32
        };
        if binding >= max_bindings {
            self.record_error(format!(
                "uniform-buffer binding {binding} exceeds OpenGL limit {max_bindings}"
            ));
            return false;
        }
        unsafe {
            self.gl
                .bind_buffer_base(glow::UNIFORM_BUFFER, binding, Some(buffer.gl_buffer));
        }
        true
    }

    fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, min_depth: f32, max_depth: f32) {
        if ![x, y, w, h, min_depth, max_depth]
            .iter()
            .all(|value| value.is_finite())
            || w <= 0.0
            || h <= 0.0
            || x < i32::MIN as f32
            || x > i32::MAX as f32
            || y < i32::MIN as f32
            || y > i32::MAX as f32
            || w > i32::MAX as f32
            || h > i32::MAX as f32
            || !(0.0..=1.0).contains(&min_depth)
            || !(0.0..=1.0).contains(&max_depth)
            || min_depth > max_depth
        {
            self.record_error("viewport dimensions/depth range are invalid for OpenGL");
            return;
        }
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the encoder is dropped before the device.
        unsafe {
            self.gl.viewport(x as i32, y as i32, w as i32, h as i32);
            self.gl.depth_range_f32(min_depth, max_depth);
        }
    }

    fn set_scissor(&mut self, x: i32, y: i32, w: u32, h: u32) {
        if w == 0 || h == 0 || w > i32::MAX as u32 || h > i32::MAX as u32 {
            self.record_error("scissor width and height must be non-zero OpenGL signed integers");
            return;
        }
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the encoder is dropped before the device.
        unsafe {
            self.gl.enable(glow::SCISSOR_TEST);
            self.gl.scissor(x, y, w as i32, h as i32);
        }
    }

    fn draw(
        &mut self,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        let Some(pipeline) = &self.current_pipeline else {
            self.record_error("draw requires a bound pipeline");
            return;
        };
        let Some(active_render_pass) = self.current_render_pass else {
            self.record_error("draw must be issued inside a render pass");
            return;
        };
        if pipeline
            .render_pass
            .is_some_and(|expected| expected != active_render_pass)
        {
            self.record_error("draw pipeline is incompatible with the active render pass");
            return;
        }
        if !pipeline.vertex_attributes.is_empty() && self.current_vertex_buffer.is_none() {
            self.record_error(
                "draw requires a bound vertex buffer for the pipeline's vertex layout",
            );
            return;
        }
        if vertex_count == 0 || instance_count == 0 {
            return;
        }
        if vertex_count > i32::MAX as u32
            || first_vertex > i32::MAX as u32
            || instance_count > i32::MAX as u32
        {
            self.record_error("draw counts and first_vertex must fit OpenGL signed integers");
            return;
        }
        if !pipeline.vertex_attributes.is_empty() {
            let Some((_, base_offset, buffer_size)) = self.current_vertex_buffer else {
                unreachable!("vertex-buffer presence checked above");
            };
            let Some(vertex_end) = u64::from(first_vertex)
                .checked_add(u64::from(vertex_count))
                .and_then(|vertices| vertices.checked_mul(u64::from(pipeline.vertex_stride)))
                .and_then(|bytes| base_offset.checked_add(bytes))
            else {
                self.record_error("vertex draw range overflowed u64");
                return;
            };
            if vertex_end > buffer_size {
                self.record_error(format!(
                    "vertex draw requires bytes through {vertex_end}, beyond {buffer_size}-byte buffer"
                ));
                return;
            }
        }
        let topology = pipeline.topology;
        if first_instance > 0 && !self.supports_base_instance() {
            self.record_error("non-zero first_instance requires desktop OpenGL 4.2 or newer");
            return;
        }
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the encoder is dropped before the device; all bound GL state was
        // set by this encoder in the same frame.
        unsafe {
            if first_instance > 0 {
                self.gl.draw_arrays_instanced_base_instance(
                    topology,
                    first_vertex as i32,
                    vertex_count as i32,
                    instance_count as i32,
                    first_instance,
                );
            } else if instance_count > 1 {
                self.gl.draw_arrays_instanced(
                    topology,
                    first_vertex as i32,
                    vertex_count as i32,
                    instance_count as i32,
                );
            } else {
                self.gl
                    .draw_arrays(topology, first_vertex as i32, vertex_count as i32);
            }
        }
        self.frame_state
            .record_draw(topology, vertex_count, instance_count);
    }

    fn draw_indexed(
        &mut self,
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        let Some(pipeline) = &self.current_pipeline else {
            self.record_error("draw_indexed requires a bound pipeline");
            return;
        };
        let Some(active_render_pass) = self.current_render_pass else {
            self.record_error("draw_indexed must be issued inside a render pass");
            return;
        };
        if pipeline
            .render_pass
            .is_some_and(|expected| expected != active_render_pass)
        {
            self.record_error("indexed draw pipeline is incompatible with the active render pass");
            return;
        }
        if !pipeline.vertex_attributes.is_empty() && self.current_vertex_buffer.is_none() {
            self.record_error(
                "draw_indexed requires a bound vertex buffer for the pipeline's vertex layout",
            );
            return;
        }
        if self.current_index_buffer.is_none() {
            self.record_error("draw_indexed requires a bound index buffer");
            return;
        }
        if index_count == 0 || instance_count == 0 {
            return;
        }
        if index_count > i32::MAX as u32 || instance_count > i32::MAX as u32 {
            self.record_error("indexed draw counts must fit OpenGL signed integers");
            return;
        }
        let (index_type, index_size) = match self.current_index_format {
            IndexFormat::U16 => (glow::UNSIGNED_SHORT, 2u64),
            IndexFormat::U32 => (glow::UNSIGNED_INT, 4u64),
        };
        let Some(offset_bytes) = u64::from(first_index)
            .checked_mul(index_size)
            .and_then(|relative| self.current_index_offset.checked_add(relative))
        else {
            self.record_error("indexed draw byte offset overflowed u64");
            return;
        };
        if offset_bytes > i32::MAX as u64 {
            self.record_error(format!(
                "indexed draw byte offset {offset_bytes} exceeds OpenGL i32 range"
            ));
            return;
        }
        let Some(index_end) = u64::from(index_count)
            .checked_mul(index_size)
            .and_then(|byte_count| offset_bytes.checked_add(byte_count))
        else {
            self.record_error("indexed draw range overflowed u64");
            return;
        };
        if index_end > self.current_index_buffer_size {
            self.record_error(format!(
                "indexed draw range {offset_bytes}..{index_end} exceeds {}-byte index buffer",
                self.current_index_buffer_size
            ));
            return;
        }
        let topology = pipeline.topology;
        if first_instance > 0 && !self.supports_base_instance() {
            self.record_error("non-zero first_instance requires desktop OpenGL 4.2 or newer");
            return;
        }
        if vertex_offset != 0 && self.gl.version().is_embedded {
            self.record_error("non-zero vertex_offset is not supported by OpenGL ES/WebGL");
            return;
        }
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the encoder is dropped before the device; all bound GL state was
        // set by this encoder in the same frame.
        unsafe {
            if first_instance > 0 {
                self.gl.draw_elements_instanced_base_vertex_base_instance(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                    instance_count as i32,
                    vertex_offset,
                    first_instance,
                );
            } else if instance_count > 1 && vertex_offset != 0 {
                self.gl.draw_elements_instanced_base_vertex(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                    instance_count as i32,
                    vertex_offset,
                );
            } else if instance_count > 1 {
                self.gl.draw_elements_instanced(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                    instance_count as i32,
                );
            } else if vertex_offset != 0 {
                self.gl.draw_elements_base_vertex(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                    vertex_offset,
                );
            } else {
                self.gl.draw_elements(
                    topology,
                    index_count as i32,
                    index_type,
                    offset_bytes as i32,
                );
            }
        }
        self.frame_state
            .record_draw(topology, index_count, instance_count);
    }

    fn push_constants(
        &mut self,
        layout: PipelineLayoutHandle,
        stage_flags: u32,
        offset: u32,
        data: &[u8],
    ) {
        if data.is_empty() {
            return;
        }
        if !offset.is_multiple_of(4) || !data.len().is_multiple_of(4) {
            self.record_error("push-constant offset and byte length must be four-byte aligned");
            return;
        }
        if stage_flags == 0 {
            self.record_error("push constants require at least one shader-stage flag");
            return;
        }
        if !self.validate_current_layout(layout) {
            return;
        }
        let Some(layout_descriptor) = self.resolve_layout(layout) else {
            self.record_error("push_constants received an invalid pipeline layout handle");
            return;
        };
        let Some(end) = offset.checked_add(data.len() as u32) else {
            self.record_error("push-constant range overflowed u32");
            return;
        };
        let covered = layout_descriptor.push_constant_ranges.iter().any(|range| {
            let Some(range_end) = range.offset.checked_add(range.size) else {
                return false;
            };
            offset >= range.offset
                && end <= range_end
                && (range.stage_flags & stage_flags) == stage_flags
        });
        if !covered {
            self.record_error(format!(
                "push-constant range {offset}..{end} for stages {stage_flags:#x} is not declared by the pipeline layout"
            ));
            return;
        }

        let Some(pipeline) = self.current_pipeline.clone() else {
            self.record_error("push constants require a bound pipeline");
            return;
        };
        if let Some(push_buffer) = pipeline.push_constant_buffer {
            if end > push_buffer.size_bytes {
                self.record_error(format!(
                    "push-constant range {offset}..{end} exceeds {}-byte OpenGL uniform buffer",
                    push_buffer.size_bytes
                ));
                return;
            }
            // SAFETY: buffer ownership and range bounds were validated above.
            unsafe {
                self.gl
                    .bind_buffer(glow::UNIFORM_BUFFER, Some(push_buffer.gl_buffer));
                self.gl
                    .buffer_sub_data_u8_slice(glow::UNIFORM_BUFFER, offset as i32, data);
                self.gl.bind_buffer_base(
                    glow::UNIFORM_BUFFER,
                    push_buffer.binding,
                    Some(push_buffer.gl_buffer),
                );
                self.gl.bind_buffer(glow::UNIFORM_BUFFER, None);
            }
            return;
        }

        let mut uploaded = 0usize;
        for uniform in &pipeline.push_uniforms {
            let uniform_end = uniform.offset.saturating_add(uniform.kind.size_bytes());
            if uniform.offset < offset || uniform_end > end {
                continue;
            }
            let start = (uniform.offset - offset) as usize;
            let finish = start + uniform.kind.size_bytes() as usize;
            if self.upload_push_uniform(uniform, &data[start..finish]) {
                uploaded += 1;
            }
        }
        if uploaded == 0 {
            self.record_error(
                "GLSL program has no push-constant UBO or compatible direct uniforms; use a `PushConstants`/`PC`/`DrawPush` std140 block or offset-named uniforms such as `u_pc_0`",
            );
        } else {
            tracing::trace!(
                target: "opengl",
                offset,
                bytes = data.len(),
                uniforms = uploaded,
                "mapped RHI push constants to GLSL uniforms"
            );
        }
    }

    fn end_render_pass(&mut self) {
        if self.current_render_pass.is_none() {
            self.record_error("end_render_pass called without an active render pass");
            return;
        }
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // binding None (the default framebuffer) is always valid regardless of
        // context state; the encoder is dropped before the device.
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
        self.current_framebuffer = None;
        self.current_render_pass = None;
    }
}

#[cfg(test)]
mod tests {
    use super::primitive_count;

    #[test]
    fn primitive_count_tracks_triangle_topologies_only() {
        assert_eq!(primitive_count(glow::TRIANGLES, 8), 2);
        assert_eq!(primitive_count(glow::TRIANGLE_STRIP, 8), 6);
        assert_eq!(primitive_count(glow::TRIANGLE_FAN, 1), 0);
        assert_eq!(primitive_count(glow::LINES, 8), 0);
    }
}
