use std::sync::Arc;

use glow::HasContext;
use render_core::*;

// ============================================================================
// Format conversion helpers
// ============================================================================

/// Returns (internal_format, format, pixel_type) for tex_image_2d.
fn convert_texture_format(format: TextureFormat) -> Result<(i32, u32, u32), RhiError> {
    let converted = match format {
        TextureFormat::Rgba8Unorm => (glow::RGBA8 as i32, glow::RGBA, glow::UNSIGNED_BYTE),
        TextureFormat::Bgra8Unorm => (glow::RGBA8 as i32, glow::BGRA, glow::UNSIGNED_BYTE),
        TextureFormat::Rgba16Float => (glow::RGBA16F as i32, glow::RGBA, glow::HALF_FLOAT),
        TextureFormat::Depth32Float => (
            glow::DEPTH_COMPONENT32F as i32,
            glow::DEPTH_COMPONENT,
            glow::FLOAT,
        ),
        _ => {
            return Err(RhiError::UnsupportedFeature {
                feature: format!("OpenGL texture format {format:?}"),
            });
        }
    };
    Ok(converted)
}

/// Returns the GL buffer target for a given usage.
fn buffer_target(usage: BufferUsage) -> u32 {
    if usage.0 & BufferUsage::INDEX.0 != 0 {
        glow::ELEMENT_ARRAY_BUFFER
    } else if usage.0 & BufferUsage::UNIFORM.0 != 0 {
        glow::UNIFORM_BUFFER
    } else {
        glow::ARRAY_BUFFER
    }
}

fn invalid_descriptor(field: &str, reason: impl Into<String>) -> RhiError {
    RhiError::InvalidDescriptor {
        field: field.to_string(),
        reason: reason.into(),
    }
}

fn opengl_presentation_unsupported() -> RhiError {
    RhiError::UnsupportedFeature {
        feature: "OpenGL surface presentation (no platform swap callback was provided)".into(),
    }
}

/// Decode and validate GLSL at module creation time so an invalid module can
/// never survive until pipeline creation. OpenGL only supports `main` as the
/// shader entry point; alternate entry points require source rewriting, which
/// this backend deliberately does not pretend to support.
fn decode_glsl_source(descriptor: &ShaderModuleDescriptor) -> Result<String, RhiError> {
    if descriptor.format != ShaderFormat::Glsl {
        return Err(invalid_descriptor(
            "shader_module.format",
            format!(
                "OpenGL requires GLSL source, received {:?}",
                descriptor.format
            ),
        ));
    }
    if descriptor.entry_points.as_slice() != ["main"] {
        return Err(invalid_descriptor(
            "shader_module.entry_points",
            "OpenGL shader modules must declare exactly the `main` entry point",
        ));
    }
    let source = std::str::from_utf8(&descriptor.source_bytes).map_err(|error| {
        invalid_descriptor(
            "shader_module.source_bytes",
            format!("GLSL source is not valid UTF-8: {error}"),
        )
    })?;
    if source.trim().is_empty() {
        return Err(invalid_descriptor(
            "shader_module.source_bytes",
            "GLSL source must not be empty",
        ));
    }
    if source.contains('\0') {
        return Err(invalid_descriptor(
            "shader_module.source_bytes",
            "GLSL source must not contain NUL bytes",
        ));
    }
    Ok(source.to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VertexAttributeFormat {
    pub(crate) component_count: i32,
    pub(crate) gl_type: u32,
    pub(crate) normalized: bool,
    pub(crate) integer: bool,
    pub(crate) size_bytes: u32,
}

fn vector_suffix(value: &str, prefix: &str) -> Option<i32> {
    if value == prefix {
        return Some(1);
    }
    value
        .strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_prefix('x'))
        .and_then(|suffix| suffix.parse::<i32>().ok())
        .filter(|count| (1..=4).contains(count))
}

/// Convert the portable string-based vertex format into the exact GL pointer
/// description. Kept context-free so descriptor validation is unit-testable.
pub(crate) fn parse_vertex_attribute_format(value: &str) -> Option<VertexAttributeFormat> {
    let value = value.trim().to_ascii_lowercase();
    let float = |component_count: i32| VertexAttributeFormat {
        component_count,
        gl_type: glow::FLOAT,
        normalized: false,
        integer: false,
        size_bytes: component_count as u32 * 4,
    };
    let integer =
        |component_count: i32, gl_type: u32, component_bytes: u32| VertexAttributeFormat {
            component_count,
            gl_type,
            normalized: false,
            integer: true,
            size_bytes: component_count as u32 * component_bytes,
        };
    let normalized =
        |component_count: i32, gl_type: u32, component_bytes: u32| VertexAttributeFormat {
            component_count,
            gl_type,
            normalized: true,
            integer: false,
            size_bytes: component_count as u32 * component_bytes,
        };

    if let Some(count) = vector_suffix(&value, "float32") {
        return Some(float(count));
    }
    if let Some(count) = vector_suffix(&value, "uint32") {
        return Some(integer(count, glow::UNSIGNED_INT, 4));
    }
    if let Some(count) = vector_suffix(&value, "sint32").or_else(|| vector_suffix(&value, "int32"))
    {
        return Some(integer(count, glow::INT, 4));
    }
    if let Some(count) = vector_suffix(&value, "uint16") {
        return Some(integer(count, glow::UNSIGNED_SHORT, 2));
    }
    if let Some(count) = vector_suffix(&value, "sint16").or_else(|| vector_suffix(&value, "int16"))
    {
        return Some(integer(count, glow::SHORT, 2));
    }
    if let Some(count) = vector_suffix(&value, "uint8") {
        return Some(integer(count, glow::UNSIGNED_BYTE, 1));
    }
    if let Some(count) = vector_suffix(&value, "sint8").or_else(|| vector_suffix(&value, "int8")) {
        return Some(integer(count, glow::BYTE, 1));
    }
    match value.as_str() {
        "unorm8x2" => Some(normalized(2, glow::UNSIGNED_BYTE, 1)),
        "unorm8x4" | "rgba8unorm" => Some(normalized(4, glow::UNSIGNED_BYTE, 1)),
        "snorm8x2" => Some(normalized(2, glow::BYTE, 1)),
        "snorm8x4" => Some(normalized(4, glow::BYTE, 1)),
        "unorm16x2" => Some(normalized(2, glow::UNSIGNED_SHORT, 2)),
        "unorm16x4" => Some(normalized(4, glow::UNSIGNED_SHORT, 2)),
        "snorm16x2" => Some(normalized(2, glow::SHORT, 2)),
        "snorm16x4" => Some(normalized(4, glow::SHORT, 2)),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct VertexAttributeBinding {
    pub(crate) location: u32,
    pub(crate) offset_bytes: u32,
    pub(crate) format: VertexAttributeFormat,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PipelineRasterState {
    pub(crate) cull_face: Option<u32>,
    pub(crate) front_face: u32,
    pub(crate) polygon_mode: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PipelineDepthState {
    pub(crate) enabled: bool,
    pub(crate) write_enabled: bool,
    pub(crate) compare: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PipelineBlendState {
    Disabled,
    Alpha,
    PremultipliedAlpha,
    Additive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PushUniformKind {
    Float(u8),
    Int(u8),
    Uint(u8),
    Mat2,
    Mat3,
    Mat4,
}

impl PushUniformKind {
    pub(crate) const fn size_bytes(self) -> u32 {
        match self {
            Self::Float(count) | Self::Int(count) | Self::Uint(count) => count as u32 * 4,
            Self::Mat2 => 16,
            Self::Mat3 => 36,
            Self::Mat4 => 64,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PushUniformBinding {
    pub(crate) name: String,
    pub(crate) offset: u32,
    pub(crate) kind: PushUniformKind,
    pub(crate) location: glow::UniformLocation,
}

#[derive(Clone, Debug)]
pub(crate) struct SamplerUniformBinding {
    pub(crate) name: String,
    pub(crate) location: glow::UniformLocation,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PushConstantBuffer {
    pub(crate) gl_buffer: glow::Buffer,
    pub(crate) binding: u32,
    pub(crate) size_bytes: u32,
}

// ============================================================================
// Resource slabs (generational-index storage)
// ============================================================================

pub(crate) struct Slot<T> {
    pub(crate) generation: u32,
    pub(crate) value: T,
}

/// A simple generational slab. Always appends (never reuses indices) - keeps
/// the code straightforward and generation checking trivial.
pub(crate) struct ResourceSlab<T> {
    slots: Vec<Option<Slot<T>>>,
}

impl<T> ResourceSlab<T> {
    const fn new() -> Self {
        Self { slots: Vec::new() }
    }

    fn alloc(&mut self, value: T) -> (u32, u32) {
        let idx = self.slots.len();
        self.slots.push(Some(Slot {
            generation: 1,
            value,
        }));
        (idx as u32, 1)
    }

    pub(crate) fn get(&self, idx: u32) -> Option<&Slot<T>> {
        self.slots.get(idx as usize).and_then(|s| s.as_ref())
    }

    fn free(&mut self, idx: u32) {
        if let Some(slot) = self.slots.get_mut(idx as usize) {
            *slot = None;
        }
    }
}

// ============================================================================
// Slot types for each resource category
// ============================================================================

pub(crate) struct BufferSlot {
    pub(crate) gl_buffer: glow::Buffer,
    pub(crate) size_bytes: u64,
    pub(crate) usage: BufferUsage,
}

pub(crate) struct TextureSlot {
    pub(crate) gl_texture: glow::Texture,
    pub(crate) format: TextureFormat,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) usage: TextureUsage,
}

pub(crate) struct ShaderModuleSlot {
    pub(crate) format: ShaderFormat,
    pub(crate) stage: ShaderStage,
    pub(crate) source_bytes: Vec<u8>,
    pub(crate) entry_point: String,
    pub(crate) source_hash: [u8; 32],
}

pub(crate) struct RenderPassSlot {
    pub(crate) _descriptor: RenderPassDescriptor,
}

pub(crate) struct FramebufferSlot {
    pub(crate) gl_framebuffer: glow::Framebuffer,
    pub(crate) render_pass: RenderPassHandle,
    pub(crate) _width: u32,
    pub(crate) _height: u32,
}

pub(crate) struct PipelineLayoutSlot {
    pub(crate) descriptor: PipelineLayoutDescriptor,
}

#[derive(Clone, Debug)]
pub(crate) struct PipelineSlot {
    pub(crate) gl_program: glow::Program,
    pub(crate) gl_vertex_array: glow::VertexArray,
    pub(crate) vertex_stride: u32,
    pub(crate) vertex_attributes: Vec<VertexAttributeBinding>,
    pub(crate) topology: u32,
    pub(crate) raster_state: PipelineRasterState,
    pub(crate) depth_state: PipelineDepthState,
    pub(crate) blend_state: PipelineBlendState,
    pub(crate) multisample: bool,
    pub(crate) render_pass: Option<RenderPassHandle>,
    pub(crate) pipeline_layout: Option<PipelineLayoutHandle>,
    pub(crate) push_constant_buffer: Option<PushConstantBuffer>,
    pub(crate) push_uniforms: Vec<PushUniformBinding>,
    pub(crate) sampler_uniforms: Vec<SamplerUniformBinding>,
}

fn shader_stage_to_gl(stage: ShaderStage) -> Result<u32, RhiError> {
    match stage {
        ShaderStage::Vertex => Ok(glow::VERTEX_SHADER),
        ShaderStage::Fragment => Ok(glow::FRAGMENT_SHADER),
        ShaderStage::Compute => Err(RhiError::UnsupportedFeature {
            feature: "OpenGL compute pipelines through PipelineDescriptor".to_string(),
        }),
    }
}

fn parse_vertex_layout(layout: &VertexLayout) -> Result<Vec<VertexAttributeBinding>, RhiError> {
    if !layout.attributes.is_empty() && layout.stride_bytes == 0 {
        return Err(invalid_descriptor(
            "pipeline.vertex_layout.stride_bytes",
            "a non-empty vertex layout requires a non-zero stride",
        ));
    }
    let mut bindings = Vec::with_capacity(layout.attributes.len());
    for (location, attribute) in layout.attributes.iter().enumerate() {
        let format = parse_vertex_attribute_format(&attribute.format).ok_or_else(|| {
            invalid_descriptor(
                "pipeline.vertex_layout.attributes.format",
                format!(
                    "attribute `{}` uses unsupported format `{}`",
                    attribute.semantic, attribute.format
                ),
            )
        })?;
        let end = attribute
            .offset_bytes
            .checked_add(format.size_bytes)
            .ok_or_else(|| {
                invalid_descriptor(
                    "pipeline.vertex_layout.attributes.offset_bytes",
                    format!("attribute `{}` byte range overflowed", attribute.semantic),
                )
            })?;
        if end > layout.stride_bytes {
            return Err(invalid_descriptor(
                "pipeline.vertex_layout.attributes.offset_bytes",
                format!(
                    "attribute `{}` ends at byte {end}, beyond stride {}",
                    attribute.semantic, layout.stride_bytes
                ),
            ));
        }
        bindings.push(VertexAttributeBinding {
            location: location as u32,
            offset_bytes: attribute.offset_bytes,
            format,
        });
    }
    Ok(bindings)
}

fn parse_topology(value: Option<&str>) -> Result<u32, RhiError> {
    match value
        .unwrap_or("triangle_list")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "triangle_list" | "triangles" => Ok(glow::TRIANGLES),
        "triangle_strip" => Ok(glow::TRIANGLE_STRIP),
        "triangle_fan" => Ok(glow::TRIANGLE_FAN),
        "line_list" | "lines" => Ok(glow::LINES),
        "line_strip" => Ok(glow::LINE_STRIP),
        "point_list" | "points" => Ok(glow::POINTS),
        other => Err(invalid_descriptor(
            "pipeline.topology",
            format!("unsupported topology `{other}`"),
        )),
    }
}

fn parse_raster_state(descriptor: &PipelineDescriptor) -> Result<PipelineRasterState, RhiError> {
    let cull_face = match descriptor
        .raster_state
        .cull_mode
        .as_deref()
        .unwrap_or("none")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "none" | "off" => None,
        "front" => Some(glow::FRONT),
        "back" => Some(glow::BACK),
        "front_and_back" | "both" => Some(glow::FRONT_AND_BACK),
        other => {
            return Err(invalid_descriptor(
                "pipeline.raster_state.cull_mode",
                format!("unsupported cull mode `{other}`"),
            ));
        }
    };
    let front_face = match descriptor
        .raster_state
        .front_face
        .as_deref()
        .unwrap_or("ccw")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "ccw" | "counter_clockwise" | "counter-clockwise" => glow::CCW,
        "cw" | "clockwise" => glow::CW,
        other => {
            return Err(invalid_descriptor(
                "pipeline.raster_state.front_face",
                format!("unsupported front face `{other}`"),
            ));
        }
    };
    let polygon_mode = match descriptor
        .polygon_mode
        .as_deref()
        .unwrap_or("fill")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "fill" => glow::FILL,
        "line" | "wireframe" => glow::LINE,
        "point" => glow::POINT,
        other => {
            return Err(invalid_descriptor(
                "pipeline.polygon_mode",
                format!("unsupported polygon mode `{other}`"),
            ));
        }
    };
    Ok(PipelineRasterState {
        cull_face,
        front_face,
        polygon_mode,
    })
}

fn parse_depth_state(descriptor: &PipelineDescriptor) -> Result<PipelineDepthState, RhiError> {
    let enabled = descriptor.depth_state.format.is_some()
        || descriptor.depth_state.compare.is_some()
        || descriptor.depth_state.write_enabled;
    let compare = match descriptor
        .depth_state
        .compare
        .as_deref()
        .unwrap_or("less")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "never" => glow::NEVER,
        "less" => glow::LESS,
        "equal" => glow::EQUAL,
        "less_equal" | "less_or_equal" | "lequal" => glow::LEQUAL,
        "greater" => glow::GREATER,
        "not_equal" | "not-equal" | "notequal" => glow::NOTEQUAL,
        "greater_equal" | "greater_or_equal" | "gequal" => glow::GEQUAL,
        "always" => glow::ALWAYS,
        other => {
            return Err(invalid_descriptor(
                "pipeline.depth_state.compare",
                format!("unsupported depth comparison `{other}`"),
            ));
        }
    };
    Ok(PipelineDepthState {
        enabled,
        write_enabled: descriptor.depth_state.write_enabled,
        compare,
    })
}

fn parse_blend_state(descriptor: &PipelineDescriptor) -> Result<PipelineBlendState, RhiError> {
    match descriptor
        .blend_state
        .mode
        .as_deref()
        .unwrap_or("none")
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_'], "")
        .as_str()
    {
        "none" | "off" | "opaque" => Ok(PipelineBlendState::Disabled),
        "alpha" => Ok(PipelineBlendState::Alpha),
        "premultiplied" | "premultipliedalpha" => Ok(PipelineBlendState::PremultipliedAlpha),
        "add" | "additive" => Ok(PipelineBlendState::Additive),
        other => Err(invalid_descriptor(
            "pipeline.blend_state.mode",
            format!("unsupported blend mode `{other}`"),
        )),
    }
}

fn push_uniform_kind(gl_type: u32) -> Option<PushUniformKind> {
    match gl_type {
        glow::FLOAT => Some(PushUniformKind::Float(1)),
        glow::FLOAT_VEC2 => Some(PushUniformKind::Float(2)),
        glow::FLOAT_VEC3 => Some(PushUniformKind::Float(3)),
        glow::FLOAT_VEC4 => Some(PushUniformKind::Float(4)),
        glow::INT => Some(PushUniformKind::Int(1)),
        glow::INT_VEC2 => Some(PushUniformKind::Int(2)),
        glow::INT_VEC3 => Some(PushUniformKind::Int(3)),
        glow::INT_VEC4 => Some(PushUniformKind::Int(4)),
        glow::UNSIGNED_INT => Some(PushUniformKind::Uint(1)),
        glow::UNSIGNED_INT_VEC2 => Some(PushUniformKind::Uint(2)),
        glow::UNSIGNED_INT_VEC3 => Some(PushUniformKind::Uint(3)),
        glow::UNSIGNED_INT_VEC4 => Some(PushUniformKind::Uint(4)),
        glow::FLOAT_MAT2 => Some(PushUniformKind::Mat2),
        glow::FLOAT_MAT3 => Some(PushUniformKind::Mat3),
        glow::FLOAT_MAT4 => Some(PushUniformKind::Mat4),
        _ => None,
    }
}

fn push_uniform_offset(name: &str) -> Option<u32> {
    let name = name.strip_suffix("[0]").unwrap_or(name);
    let leaf = name.rsplit('.').next().unwrap_or(name).to_ascii_lowercase();
    for prefix in ["u_push_constants_", "push_constants_", "u_pc_", "pc_"] {
        if let Some(offset) = leaf.strip_prefix(prefix) {
            if let Ok(offset) = offset.parse::<u32>() {
                return Some(offset);
            }
        }
    }
    match leaf.as_str() {
        "u_push_constants" | "push_constants" | "u_pc" | "pc" => Some(0),
        "model" | "u_model" | "mvp" | "u_mvp" | "screen_size" | "u_screen_size" => Some(0),
        "light_direction" | "u_light_direction" | "light_dir" | "u_light_dir" => Some(64),
        "light_color" | "u_light_color" => Some(80),
        "ambient" | "u_ambient" | "ambient_color" | "u_ambient_color" => Some(96),
        _ => None,
    }
}

pub(crate) fn gl_binding_point(set_index: u8, binding: u32) -> Option<u32> {
    u32::from(set_index).checked_mul(16)?.checked_add(binding)
}

fn is_sampler_type(gl_type: u32) -> bool {
    matches!(
        gl_type,
        glow::SAMPLER_2D
            | glow::SAMPLER_2D_SHADOW
            | glow::INT_SAMPLER_2D
            | glow::UNSIGNED_INT_SAMPLER_2D
    )
}

fn is_push_constant_block(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "pushconstants" | "push_constants" | "pc" | "drawpush" | "draw_push"
    )
}

fn sampler_sort_key(name: &str) -> (u8, String) {
    let normalized = name.to_ascii_lowercase();
    let priority = if normalized.contains("base")
        || normalized.contains("albedo")
        || normalized.contains("diffuse")
    {
        0
    } else if normalized.contains("shadow") {
        1
    } else {
        2
    };
    (priority, normalized)
}

// ============================================================================
// OpenGlBackend
// ============================================================================

pub struct OpenGlBackend {
    gl: Arc<glow::Context>,
}

impl OpenGlBackend {
    pub fn new(gl: glow::Context) -> Self {
        Self { gl: Arc::new(gl) }
    }
}

impl Backend for OpenGlBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::OpenGl
    }

    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError> {
        // OpenGL does not have physical-device enumeration like Vulkan.
        // Return a single generic adapter built from the driver string.
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the device is alive and not yet destroyed.
        let name = unsafe { self.gl.get_parameter_string(glow::RENDERER) };
        Ok(vec![AdapterInfo {
            backend: BackendKind::OpenGl,
            name,
            vendor_id: None,
            device_id: None,
            driver_version: None,
            capabilities: BackendCapabilities {
                max_texture_dimension_2d: unsafe {
                    // SAFETY: `self.gl` is a valid `glow::Context` created by
                    // this device; `MAX_TEXTURE_SIZE` is a valid GL parameter
                    // whose value is populated by the driver.
                    self.gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32
                },
                max_color_attachments: unsafe {
                    // SAFETY: `self.gl` is a valid `glow::Context` created by
                    // this device; `MAX_COLOR_ATTACHMENTS` is a valid GL
                    // parameter whose value is populated by the driver.
                    self.gl.get_parameter_i32(glow::MAX_COLOR_ATTACHMENTS) as u8
                },
                supports_swapchain: false,
                supports_timestamps: false,
                supports_debug_markers: false,
                supported_shader_formats: vec![ShaderFormat::Glsl],
                supported_surface_formats: Vec::new(),
                limits: ResourceLimits::default(),
            },
        }])
    }

    fn create_device(&self, descriptor: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError> {
        let version = self.gl.version();
        let supported = if version.is_embedded {
            version.major >= 3
        } else {
            version.major > 3 || (version.major == 3 && version.minor >= 3)
        };
        if !supported {
            return Err(RhiError::UnsupportedFeature {
                feature: format!(
                    "OpenGL 3.3+ or OpenGL ES 3.0+ context (received {}.{})",
                    version.major, version.minor
                ),
            });
        }
        Ok(Box::new(OpenGlDevice::new(self.gl.clone(), descriptor)))
    }
}

// ============================================================================
// OpenGlDevice
// ============================================================================

pub struct OpenGlDevice {
    adapter: AdapterInfo,
    gl: Arc<glow::Context>,

    // Resource slabs
    pub(crate) buffers: ResourceSlab<BufferSlot>,
    pub(crate) textures: ResourceSlab<TextureSlot>,
    pub(crate) shader_modules: ResourceSlab<ShaderModuleSlot>,
    pub(crate) render_passes: ResourceSlab<RenderPassSlot>,
    pub(crate) framebuffers: ResourceSlab<FramebufferSlot>,
    pub(crate) pipeline_layouts: ResourceSlab<PipelineLayoutSlot>,
    pub(crate) pipelines: ResourceSlab<PipelineSlot>,
}

impl OpenGlDevice {
    fn new(gl: Arc<glow::Context>, descriptor: &DeviceDescriptor) -> Self {
        let adapter = descriptor.adapter.clone();
        Self {
            adapter,
            gl,
            buffers: ResourceSlab::new(),
            textures: ResourceSlab::new(),
            shader_modules: ResourceSlab::new(),
            render_passes: ResourceSlab::new(),
            framebuffers: ResourceSlab::new(),
            pipeline_layouts: ResourceSlab::new(),
            pipelines: ResourceSlab::new(),
        }
    }
}

impl Device for OpenGlDevice {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.adapter
    }

    // Surface presentation stays fail-closed until construction accepts a
    // platform buffer-swap callback.

    fn create_surface(
        &mut self,
        _descriptor: &SurfaceDescriptor,
    ) -> Result<SurfaceHandle, RhiError> {
        Err(opengl_presentation_unsupported())
    }

    fn create_swapchain(
        &mut self,
        _descriptor: &SwapchainDescriptor,
    ) -> Result<SwapchainHandle, RhiError> {
        Err(opengl_presentation_unsupported())
    }

    fn destroy_surface(&mut self, handle: SurfaceHandle) {
        tracing::warn!(
            target: "opengl",
            ?handle,
            "ignored surface destruction: this backend cannot create OpenGL presentation surfaces"
        );
    }

    fn destroy_swapchain(&mut self, handle: SwapchainHandle) {
        tracing::warn!(
            target: "opengl",
            ?handle,
            "ignored swapchain destruction: this backend cannot create OpenGL presentation swapchains"
        );
    }

    // ██ buffers ████████████████████████████████████████████████████████████████████████████████

    fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> Result<BufferHandle, RhiError> {
        if descriptor.size_bytes == 0 || descriptor.size_bytes > i32::MAX as u64 {
            return Err(invalid_descriptor(
                "buffer.size_bytes",
                format!(
                    "OpenGL buffer size must be in 1..={}, received {}",
                    i32::MAX,
                    descriptor.size_bytes
                ),
            ));
        }
        // SAFETY: glow buffer creation.
        let gl_buffer = unsafe {
            self.gl
                .create_buffer()
                .map_err(|e| RhiError::Backend { detail: e })?
        };
        let target = buffer_target(descriptor.usage_flags);
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // `gl_buffer` was just created by the same context, and the device is
        // not yet destroyed.
        unsafe {
            self.gl.bind_buffer(target, Some(gl_buffer));
            self.gl
                .buffer_data_size(target, descriptor.size_bytes as i32, glow::STATIC_DRAW);
        }

        let (idx, gen) = self.buffers.alloc(BufferSlot {
            gl_buffer,
            size_bytes: descriptor.size_bytes,
            usage: descriptor.usage_flags,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn write_buffer(
        &mut self,
        buffer: BufferHandle,
        data: &[u8],
        offset: u64,
    ) -> Result<(), RhiError> {
        let slot = self
            .buffers
            .get(buffer.index)
            .filter(|s| s.generation == buffer.generation)
            .ok_or(RhiError::InvalidHandle)?;
        let end = offset.checked_add(data.len() as u64).ok_or_else(|| {
            invalid_descriptor("buffer.write", "offset plus data length overflowed")
        })?;
        if end > slot.value.size_bytes {
            return Err(invalid_descriptor(
                "buffer.write",
                format!(
                    "write range {offset}..{end} exceeds buffer size {}",
                    slot.value.size_bytes
                ),
            ));
        }
        let target = buffer_target(slot.value.usage);
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // `slot.value.gl_buffer` was created by the same context, the slot was
        // validated by generation check above, and the device is not yet destroyed.
        unsafe {
            self.gl.bind_buffer(target, Some(slot.value.gl_buffer));
            self.gl
                .buffer_sub_data_u8_slice(target, offset as i32, data);
        }
        Ok(())
    }

    fn destroy_buffer(&mut self, handle: BufferHandle) {
        let slot = self.buffers.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                // SAFETY: `self.gl` is a valid `glow::Context` created by this
                // device; `slot.value.gl_buffer` was created by the same context
                // and is not in use elsewhere (the handle generation matched).
                unsafe { self.gl.delete_buffer(slot.value.gl_buffer) };
            }
        }
        self.buffers.free(handle.index);
    }

    // ██ textures ███████████████████████████████████████████████████████████████████████████████

    fn create_texture(
        &mut self,
        descriptor: &TextureDescriptor,
    ) -> Result<TextureHandle, RhiError> {
        if descriptor.width == 0 || descriptor.height == 0 {
            return Err(invalid_descriptor(
                "texture.extent",
                "width and height must both be non-zero",
            ));
        }
        if descriptor.width > i32::MAX as u32 || descriptor.height > i32::MAX as u32 {
            return Err(invalid_descriptor(
                "texture.extent",
                "width and height must fit the OpenGL signed integer API",
            ));
        }
        if descriptor.depth_or_layers != 1 {
            return Err(RhiError::UnsupportedFeature {
                feature: "OpenGL array/3D textures through TextureDescriptor".to_string(),
            });
        }
        if descriptor.mip_levels != 1 {
            return Err(RhiError::UnsupportedFeature {
                feature: "OpenGL mipmapped texture allocation through TextureDescriptor"
                    .to_string(),
            });
        }
        if descriptor.sample_count != 1 {
            return Err(RhiError::UnsupportedFeature {
                feature: "OpenGL multisampled texture allocation through TextureDescriptor"
                    .to_string(),
            });
        }
        let (internal_fmt, fmt, pixel_type) = convert_texture_format(descriptor.format)?;
        // SAFETY: glow texture creation.
        let gl_texture = unsafe {
            self.gl
                .create_texture()
                .map_err(|e| RhiError::Backend { detail: e })?
        };
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // `gl_texture` was just created by the same context, and the device is
        // not yet destroyed.
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(gl_texture));
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal_fmt,
                descriptor.width as i32,
                descriptor.height as i32,
                0,
                fmt,
                pixel_type,
                glow::PixelUnpackData::Slice(None),
            );
        }

        let (idx, gen) = self.textures.alloc(TextureSlot {
            gl_texture,
            format: descriptor.format,
            width: descriptor.width,
            height: descriptor.height,
            usage: descriptor.usage_flags,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_texture(&mut self, handle: TextureHandle) {
        let slot = self.textures.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                // SAFETY: `self.gl` is a valid `glow::Context` created by this
                // device; `slot.value.gl_texture` was created by the same context
                // and is not in use elsewhere (the handle generation matched).
                unsafe { self.gl.delete_texture(slot.value.gl_texture) };
            }
        }
        self.textures.free(handle.index);
    }

    // ██ shader modules ██████████████████████████████████████████████████████████████████████

    fn create_shader_module(
        &mut self,
        descriptor: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, RhiError> {
        decode_glsl_source(descriptor)?;
        let (idx, gen) = self.shader_modules.alloc(ShaderModuleSlot {
            format: descriptor.format,
            stage: descriptor.stage,
            source_bytes: descriptor.source_bytes.clone(),
            entry_point: descriptor.entry_points[0].clone(),
            source_hash: descriptor.source_hash,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_shader_module(&mut self, handle: ShaderModuleHandle) {
        self.shader_modules.free(handle.index);
    }

    // ██ render passes ██████████████████████████████████████████████████████████████████████

    fn create_render_pass(
        &mut self,
        descriptor: &RenderPassDescriptor,
    ) -> Result<RenderPassHandle, RhiError> {
        if descriptor.sample_count != 1 {
            return Err(RhiError::UnsupportedFeature {
                feature: "OpenGL multisampled render passes through the generic RHI".to_string(),
            });
        }
        if descriptor.color_attachments.len() > u8::MAX as usize {
            return Err(invalid_descriptor(
                "render_pass.color_attachments",
                "attachment count exceeds the portable RHI limit",
            ));
        }
        let max_color_attachments = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_COLOR_ATTACHMENTS)
                .max(0) as usize
        };
        if descriptor.color_attachments.len() > max_color_attachments {
            return Err(RhiError::UnsupportedLimit {
                limit: "OpenGL color attachments".to_string(),
                requested: descriptor.color_attachments.len() as u64,
                available: max_color_attachments as u64,
            });
        }
        let (idx, gen) = self.render_passes.alloc(RenderPassSlot {
            _descriptor: descriptor.clone(),
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_render_pass(&mut self, handle: RenderPassHandle) {
        self.render_passes.free(handle.index);
    }

    // ██ framebuffers ███████████████████████████████████████████████████████████████████████

    fn create_framebuffer(
        &mut self,
        descriptor: &FramebufferDescriptor,
    ) -> Result<FramebufferHandle, RhiError> {
        if descriptor.width == 0 || descriptor.height == 0 {
            return Err(invalid_descriptor(
                "framebuffer.extent",
                "width and height must both be non-zero",
            ));
        }
        let render_pass = &self
            .render_passes
            .get(descriptor.render_pass.index)
            .filter(|slot| slot.generation == descriptor.render_pass.generation)
            .ok_or(RhiError::InvalidHandle)?
            .value
            ._descriptor;
        if descriptor.color_attachments.len() != render_pass.color_attachments.len() {
            return Err(invalid_descriptor(
                "framebuffer.color_attachments",
                format!(
                    "render pass declares {} color attachments but framebuffer supplies {}",
                    render_pass.color_attachments.len(),
                    descriptor.color_attachments.len()
                ),
            ));
        }

        let mut color_textures = Vec::with_capacity(descriptor.color_attachments.len());
        for (index, (&handle, &expected_format)) in descriptor
            .color_attachments
            .iter()
            .zip(render_pass.color_attachments.iter())
            .enumerate()
        {
            let texture = &self
                .textures
                .get(handle.index)
                .filter(|slot| slot.generation == handle.generation)
                .ok_or(RhiError::InvalidHandle)?
                .value;
            if texture.usage.0 & TextureUsage::COLOR_ATTACHMENT.0 == 0 {
                return Err(invalid_descriptor(
                    "framebuffer.color_attachments",
                    format!("attachment {index} was not created with COLOR_ATTACHMENT usage"),
                ));
            }
            if texture.format != expected_format {
                return Err(invalid_descriptor(
                    "framebuffer.color_attachments",
                    format!(
                        "attachment {index} format {:?} does not match render-pass format {expected_format:?}",
                        texture.format
                    ),
                ));
            }
            if texture.width != descriptor.width || texture.height != descriptor.height {
                return Err(invalid_descriptor(
                    "framebuffer.extent",
                    format!(
                        "attachment {index} is {}x{}, expected {}x{}",
                        texture.width, texture.height, descriptor.width, descriptor.height
                    ),
                ));
            }
            color_textures.push(texture.gl_texture);
        }

        let depth_texture = match (
            descriptor.depth_stencil_attachment,
            render_pass.depth_stencil_format,
        ) {
            (Some(handle), Some(expected_format)) => {
                let texture = &self
                    .textures
                    .get(handle.index)
                    .filter(|slot| slot.generation == handle.generation)
                    .ok_or(RhiError::InvalidHandle)?
                    .value;
                if texture.usage.0 & TextureUsage::DEPTH_ATTACHMENT.0 == 0 {
                    return Err(invalid_descriptor(
                        "framebuffer.depth_stencil_attachment",
                        "depth attachment was not created with DEPTH_ATTACHMENT usage",
                    ));
                }
                if texture.format != expected_format {
                    return Err(invalid_descriptor(
                        "framebuffer.depth_stencil_attachment",
                        format!(
                            "depth format {:?} does not match render-pass format {expected_format:?}",
                            texture.format
                        ),
                    ));
                }
                if texture.width != descriptor.width || texture.height != descriptor.height {
                    return Err(invalid_descriptor(
                        "framebuffer.extent",
                        format!(
                            "depth attachment is {}x{}, expected {}x{}",
                            texture.width, texture.height, descriptor.width, descriptor.height
                        ),
                    ));
                }
                Some(texture.gl_texture)
            }
            (None, None) => None,
            (Some(_), None) => {
                return Err(invalid_descriptor(
                    "framebuffer.depth_stencil_attachment",
                    "framebuffer supplies depth but render pass does not declare it",
                ));
            }
            (None, Some(_)) => {
                return Err(invalid_descriptor(
                    "framebuffer.depth_stencil_attachment",
                    "render pass declares depth but framebuffer does not supply it",
                ));
            }
        };

        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the device is not yet destroyed; the returned framebuffer handle is
        // checked for errors before use.
        let gl_framebuffer = unsafe {
            self.gl
                .create_framebuffer()
                .map_err(|e| RhiError::Backend { detail: e })?
        };

        // SAFETY: `self.gl` is a valid `glow::Context`; `gl_framebuffer` was
        // just created by the same context, and the device is not yet destroyed.
        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(gl_framebuffer));
        }

        // Attach the fully validated texture set and reject incomplete FBOs.
        unsafe {
            let mut draw_buffers = Vec::with_capacity(color_textures.len());
            for (index, texture) in color_textures.iter().enumerate() {
                let attachment = glow::COLOR_ATTACHMENT0 + index as u32;
                self.gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    attachment,
                    glow::TEXTURE_2D,
                    Some(*texture),
                    0,
                );
                draw_buffers.push(attachment);
            }
            if draw_buffers.is_empty() {
                self.gl.draw_buffer(glow::NONE);
                self.gl.read_buffer(glow::NONE);
            } else {
                self.gl.draw_buffers(&draw_buffers);
            }
            if let Some(texture) = depth_texture {
                self.gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_ATTACHMENT,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
            }
            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                self.gl.delete_framebuffer(gl_framebuffer);
                return Err(RhiError::ValidationFailed {
                    detail: format!("OpenGL framebuffer is incomplete (status {status:#x})"),
                });
            }
        }

        let (idx, gen) = self.framebuffers.alloc(FramebufferSlot {
            gl_framebuffer,
            render_pass: descriptor.render_pass,
            _width: descriptor.width,
            _height: descriptor.height,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_framebuffer(&mut self, handle: FramebufferHandle) {
        let slot = self.framebuffers.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                // SAFETY: `self.gl` is a valid `glow::Context` created by this
                // device; `slot.value.gl_framebuffer` was created by the same
                // context and is not in use elsewhere (generation matched).
                unsafe { self.gl.delete_framebuffer(slot.value.gl_framebuffer) };
            }
        }
        self.framebuffers.free(handle.index);
    }

    // ██ pipeline layouts ██████████████████████████████████████████████████████████████████

    fn create_pipeline_layout(
        &mut self,
        descriptor: &PipelineLayoutDescriptor,
    ) -> Result<PipelineLayoutHandle, RhiError> {
        let max_push_bytes = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_UNIFORM_BLOCK_SIZE)
                .max(0) as u32
        };
        for range in &descriptor.push_constant_ranges {
            if range.size == 0 {
                return Err(invalid_descriptor(
                    "pipeline_layout.push_constant_ranges.size",
                    "push-constant ranges must not be empty",
                ));
            }
            if !range.offset.is_multiple_of(4) || !range.size.is_multiple_of(4) {
                return Err(invalid_descriptor(
                    "pipeline_layout.push_constant_ranges",
                    "push-constant offsets and sizes must be four-byte aligned",
                ));
            }
            if range.stage_flags == 0 {
                return Err(invalid_descriptor(
                    "pipeline_layout.push_constant_ranges.stage_flags",
                    "at least one shader stage must be selected",
                ));
            }
            let end = range.offset.checked_add(range.size).ok_or_else(|| {
                invalid_descriptor(
                    "pipeline_layout.push_constant_ranges",
                    "push-constant range overflowed u32",
                )
            })?;
            if end > max_push_bytes {
                return Err(RhiError::UnsupportedLimit {
                    limit: "OpenGL push-constant uniform block bytes".to_string(),
                    requested: end as u64,
                    available: max_push_bytes as u64,
                });
            }
        }

        let max_uniform_bindings = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_UNIFORM_BUFFER_BINDINGS)
                .max(0) as u32
        };
        let max_texture_units = unsafe {
            self.gl
                .get_parameter_i32(glow::MAX_COMBINED_TEXTURE_IMAGE_UNITS)
                .max(0) as u32
        };
        let mut sets = std::collections::BTreeSet::new();
        for set in &descriptor.bind_group_layouts {
            if !sets.insert(set.set_index) {
                return Err(invalid_descriptor(
                    "pipeline_layout.bind_group_layouts.set_index",
                    format!("descriptor set {} is duplicated", set.set_index),
                ));
            }
            let mut bindings = std::collections::BTreeSet::new();
            for binding in &set.bindings {
                if !bindings.insert(binding.binding) {
                    return Err(invalid_descriptor(
                        "pipeline_layout.bind_group_layouts.bindings",
                        format!(
                            "descriptor set {} repeats binding {}",
                            set.set_index, binding.binding
                        ),
                    ));
                }
                let Some(gl_binding) = gl_binding_point(set.set_index, binding.binding) else {
                    return Err(invalid_descriptor(
                        "pipeline_layout.bind_group_layouts.bindings",
                        "flattened OpenGL binding point overflowed u32",
                    ));
                };
                let kind = binding.resource_kind.trim().to_ascii_lowercase();
                match kind.as_str() {
                    "uniform_buffer" | "ubo" if gl_binding >= max_uniform_bindings => {
                        return Err(RhiError::UnsupportedLimit {
                            limit: "OpenGL uniform buffer binding point".to_string(),
                            requested: gl_binding as u64 + 1,
                            available: max_uniform_bindings as u64,
                        });
                    }
                    "sampled_texture" | "texture" | "sampled_image" | "combined_image_sampler"
                        if gl_binding >= max_texture_units =>
                    {
                        return Err(RhiError::UnsupportedLimit {
                            limit: "OpenGL texture unit".to_string(),
                            requested: gl_binding as u64 + 1,
                            available: max_texture_units as u64,
                        });
                    }
                    "sampled_texture_pair" | "texture_pair" | "sampled_image_pair"
                        if gl_binding
                            .checked_add(1)
                            .is_none_or(|second| second >= max_texture_units) =>
                    {
                        return Err(RhiError::UnsupportedLimit {
                            limit: "OpenGL texture units for sampled pair".to_string(),
                            requested: gl_binding as u64 + 2,
                            available: max_texture_units as u64,
                        });
                    }
                    "uniform_buffer"
                    | "ubo"
                    | "sampled_texture"
                    | "texture"
                    | "sampled_image"
                    | "combined_image_sampler"
                    | "sampled_texture_pair"
                    | "texture_pair"
                    | "sampled_image_pair"
                    | "sampler"
                    | "sampler_pair" => {}
                    _ => {
                        return Err(RhiError::IncompatibleBindLayout {
                            reason: format!(
                                "OpenGL does not support resource kind `{}`",
                                binding.resource_kind
                            ),
                        });
                    }
                }
            }
        }
        let (idx, gen) = self.pipeline_layouts.alloc(PipelineLayoutSlot {
            descriptor: descriptor.clone(),
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_pipeline_layout(&mut self, handle: PipelineLayoutHandle) {
        self.pipeline_layouts.free(handle.index);
    }

    // ██ pipelines █████████████████████████████████████████████████████████████████████████

    fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
    ) -> Result<PipelineHandle, RhiError> {
        if descriptor.shader_modules.is_empty() {
            return Err(invalid_descriptor(
                "pipeline.shader_modules",
                "at least one vertex shader module is required",
            ));
        }
        if !descriptor.specialization.is_empty() {
            return Err(RhiError::UnsupportedFeature {
                feature: "GLSL specialization constants".to_string(),
            });
        }

        let vertex_attributes = parse_vertex_layout(&descriptor.vertex_layout)?;
        let topology = parse_topology(descriptor.topology.as_deref())?;
        let raster_state = parse_raster_state(descriptor)?;
        if self.gl.version().is_embedded && raster_state.polygon_mode != glow::FILL {
            return Err(RhiError::UnsupportedFeature {
                feature: "non-fill polygon mode on OpenGL ES/WebGL".to_string(),
            });
        }
        let depth_state = parse_depth_state(descriptor)?;
        let blend_state = parse_blend_state(descriptor)?;
        let sample_count = descriptor.sample_count.unwrap_or(1);
        if sample_count == 0 {
            return Err(invalid_descriptor(
                "pipeline.sample_count",
                "sample count must be at least one",
            ));
        }
        if let Some(render_pass_handle) = descriptor.render_pass {
            let render_pass = &self
                .render_passes
                .get(render_pass_handle.index)
                .filter(|slot| slot.generation == render_pass_handle.generation)
                .ok_or(RhiError::InvalidHandle)?
                .value
                ._descriptor;
            if descriptor.render_targets != render_pass.color_attachments {
                return Err(invalid_descriptor(
                    "pipeline.render_targets",
                    "pipeline color formats do not match the referenced render pass",
                ));
            }
            if sample_count != render_pass.sample_count {
                return Err(invalid_descriptor(
                    "pipeline.sample_count",
                    format!(
                        "pipeline requests {sample_count} samples but render pass requests {}",
                        render_pass.sample_count
                    ),
                ));
            }
            if descriptor.depth_state.format != render_pass.depth_stencil_format {
                return Err(invalid_descriptor(
                    "pipeline.depth_state.format",
                    "pipeline depth format does not match the referenced render pass",
                ));
            }
        }

        let layout_descriptor = match descriptor.pipeline_layout {
            Some(handle) => Some(
                self.pipeline_layouts
                    .get(handle.index)
                    .filter(|slot| slot.generation == handle.generation)
                    .ok_or(RhiError::InvalidHandle)?
                    .value
                    .descriptor
                    .clone(),
            ),
            None => None,
        };
        if descriptor.pipeline_layout.is_none() && !descriptor.bind_layouts.is_empty() {
            return Err(invalid_descriptor(
                "pipeline.pipeline_layout",
                "resource bind layouts require an explicit pipeline layout handle on OpenGL",
            ));
        }
        if let Some(layout) = &layout_descriptor {
            if !descriptor.bind_layouts.is_empty()
                && descriptor.bind_layouts != layout.bind_group_layouts
            {
                return Err(RhiError::IncompatibleBindLayout {
                    reason: "pipeline bind_layouts differ from the referenced pipeline layout"
                        .to_string(),
                });
            }
        }

        #[derive(Debug)]
        struct ModuleSource {
            stage: ShaderStage,
            source: String,
            label: String,
        }
        let mut modules = Vec::with_capacity(descriptor.shader_modules.len());
        let mut vertex_count = 0u32;
        let mut fragment_count = 0u32;
        for handle in &descriptor.shader_modules {
            let module = &self
                .shader_modules
                .get(handle.index)
                .filter(|slot| slot.generation == handle.generation)
                .ok_or(RhiError::InvalidHandle)?
                .value;
            if module.format != ShaderFormat::Glsl || module.entry_point != "main" {
                return Err(RhiError::ValidationFailed {
                    detail: "corrupt OpenGL shader module metadata".to_string(),
                });
            }
            shader_stage_to_gl(module.stage)?;
            match module.stage {
                ShaderStage::Vertex => vertex_count += 1,
                ShaderStage::Fragment => fragment_count += 1,
                ShaderStage::Compute => unreachable!("compute stage rejected above"),
            }
            let source = std::str::from_utf8(&module.source_bytes)
                .map_err(|error| RhiError::ValidationFailed {
                    detail: format!("stored GLSL source became invalid UTF-8: {error}"),
                })?
                .to_owned();
            let hash_prefix = module
                .source_hash
                .iter()
                .take(4)
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            modules.push(ModuleSource {
                stage: module.stage,
                source,
                label: format!("{:?}@{hash_prefix}", module.stage),
            });
        }
        if vertex_count != 1
            || fragment_count > 1
            || (!descriptor.render_targets.is_empty() && fragment_count != 1)
        {
            return Err(invalid_descriptor(
                "pipeline.shader_modules",
                format!(
                    "graphics pipelines require exactly one vertex shader and, for color output, exactly one fragment shader; got {vertex_count} vertex and {fragment_count} fragment modules"
                ),
            ));
        }

        // SAFETY: the GL context is current and all handles created below are
        // either transferred into the resource slab or deleted on every error
        // path before this function returns.
        let gl_program = unsafe {
            self.gl
                .create_program()
                .map_err(|detail| RhiError::Backend { detail })?
        };
        let mut attached = Vec::<glow::Shader>::new();
        let cleanup_program = |attached: &[glow::Shader]| {
            // SAFETY: every shader in `attached` is attached to `gl_program`
            // and all objects were created by this context.
            unsafe {
                for shader in attached {
                    self.gl.detach_shader(gl_program, *shader);
                    self.gl.delete_shader(*shader);
                }
                self.gl.delete_program(gl_program);
            }
        };

        for module in &modules {
            let gl_shader =
                match unsafe { self.gl.create_shader(shader_stage_to_gl(module.stage)?) } {
                    Ok(shader) => shader,
                    Err(detail) => {
                        cleanup_program(&attached);
                        return Err(RhiError::Backend { detail });
                    }
                };
            // SAFETY: `gl_shader` is live and belongs to this current context.
            unsafe {
                self.gl.shader_source(gl_shader, &module.source);
                self.gl.compile_shader(gl_shader);
                if !self.gl.get_shader_compile_status(gl_shader) {
                    let log = self.gl.get_shader_info_log(gl_shader);
                    self.gl.delete_shader(gl_shader);
                    cleanup_program(&attached);
                    return Err(RhiError::ValidationFailed {
                        detail: format!("{} GLSL compilation failed: {log}", module.label),
                    });
                }
                self.gl.attach_shader(gl_program, gl_shader);
            }
            attached.push(gl_shader);
        }

        // SAFETY: all attached shaders and the program are live GL objects
        // created by the same current context.
        unsafe {
            self.gl.link_program(gl_program);
            if !self.gl.get_program_link_status(gl_program) {
                let log = self.gl.get_program_info_log(gl_program);
                cleanup_program(&attached);
                return Err(RhiError::ValidationFailed {
                    detail: format!("OpenGL pipeline link failed: {log}"),
                });
            }
            for shader in &attached {
                self.gl.detach_shader(gl_program, *shader);
                self.gl.delete_shader(*shader);
            }
        }

        let gl_vertex_array = match unsafe { self.gl.create_vertex_array() } {
            Ok(vertex_array) => vertex_array,
            Err(detail) => {
                // SAFETY: `gl_program` linked successfully and is not stored yet.
                unsafe { self.gl.delete_program(gl_program) };
                return Err(RhiError::Backend { detail });
            }
        };
        // A VAO owns enable state. Pointer formats are installed when the
        // actual vertex buffer is bound because PipelineDescriptor does not
        // contain buffer handles.
        unsafe {
            self.gl.bind_vertex_array(Some(gl_vertex_array));
            for attribute in &vertex_attributes {
                self.gl.enable_vertex_attrib_array(attribute.location);
            }
            self.gl.bind_vertex_array(None);
        }

        let mut sampler_uniforms = Vec::new();
        let mut push_uniforms = Vec::new();
        // SAFETY: the linked program is valid; reflection queries do not mutate
        // application-visible resources.
        unsafe {
            let uniform_count = self.gl.get_active_uniforms(gl_program);
            for index in 0..uniform_count {
                let Some(active) = self.gl.get_active_uniform(gl_program, index) else {
                    continue;
                };
                if is_sampler_type(active.utype) {
                    if let Some(location) = self.gl.get_uniform_location(gl_program, &active.name) {
                        sampler_uniforms.push(SamplerUniformBinding {
                            name: active.name,
                            location,
                        });
                    }
                    continue;
                }
                let Some(kind) = push_uniform_kind(active.utype) else {
                    continue;
                };
                let Some(base_offset) = push_uniform_offset(&active.name) else {
                    continue;
                };
                let base_name = active.name.strip_suffix("[0]").unwrap_or(&active.name);
                for element in 0..active.size.max(1) as u32 {
                    let name = if active.size > 1 {
                        format!("{base_name}[{element}]")
                    } else {
                        active.name.clone()
                    };
                    if let Some(location) = self.gl.get_uniform_location(gl_program, &name) {
                        push_uniforms.push(PushUniformBinding {
                            name,
                            offset: base_offset + element * kind.size_bytes(),
                            kind,
                            location,
                        });
                    }
                }
            }
        }
        sampler_uniforms.sort_by_key(|uniform| sampler_sort_key(&uniform.name));
        push_uniforms.sort_by_key(|uniform| uniform.offset);

        let mut push_constant_buffer = None;
        if let Some(layout) = &layout_descriptor {
            let push_size = layout
                .push_constant_ranges
                .iter()
                .filter_map(|range| range.offset.checked_add(range.size))
                .max()
                .unwrap_or(0);
            let mut push_blocks = Vec::new();
            let mut ordinary_blocks = Vec::new();
            unsafe {
                let block_count = self
                    .gl
                    .get_program_parameter_i32(gl_program, glow::ACTIVE_UNIFORM_BLOCKS)
                    .max(0) as u32;
                for block_index in 0..block_count {
                    let name = self
                        .gl
                        .get_active_uniform_block_name(gl_program, block_index);
                    if is_push_constant_block(&name) {
                        push_blocks.push((block_index, name));
                    } else {
                        ordinary_blocks.push((block_index, name));
                    }
                }
            }
            if push_blocks.len() > 1 {
                unsafe {
                    self.gl.delete_vertex_array(gl_vertex_array);
                    self.gl.delete_program(gl_program);
                }
                return Err(invalid_descriptor(
                    "pipeline.shader_modules",
                    "linked program exposes more than one push-constant uniform block",
                ));
            }

            let mut uniform_binding_points = layout
                .bind_group_layouts
                .iter()
                .flat_map(|set| {
                    set.bindings.iter().filter_map(move |binding| {
                        let kind = binding.resource_kind.trim().to_ascii_lowercase();
                        (kind == "uniform_buffer" || kind == "ubo")
                            .then(|| gl_binding_point(set.set_index, binding.binding))
                            .flatten()
                    })
                })
                .collect::<Vec<_>>();
            uniform_binding_points.sort_unstable();
            uniform_binding_points.dedup();

            let max_bindings = unsafe {
                self.gl
                    .get_parameter_i32(glow::MAX_UNIFORM_BUFFER_BINDINGS)
                    .max(0) as u32
            };
            if uniform_binding_points
                .iter()
                .any(|binding| *binding >= max_bindings)
            {
                unsafe {
                    self.gl.delete_vertex_array(gl_vertex_array);
                    self.gl.delete_program(gl_program);
                }
                return Err(RhiError::UnsupportedLimit {
                    limit: "OpenGL uniform buffer binding point".to_string(),
                    requested: uniform_binding_points.iter().copied().max().unwrap_or(0) as u64 + 1,
                    available: max_bindings as u64,
                });
            }
            unsafe {
                for ((block_index, _), binding) in
                    ordinary_blocks.iter().zip(uniform_binding_points.iter())
                {
                    self.gl
                        .uniform_block_binding(gl_program, *block_index, *binding);
                }
            }

            if push_size > 0 {
                if let Some((block_index, block_name)) = push_blocks.first() {
                    let Some(binding) = (0..max_bindings)
                        .rev()
                        .find(|candidate| !uniform_binding_points.contains(candidate))
                    else {
                        unsafe {
                            self.gl.delete_vertex_array(gl_vertex_array);
                            self.gl.delete_program(gl_program);
                        }
                        return Err(RhiError::UnsupportedLimit {
                            limit: "OpenGL push-constant UBO binding".to_string(),
                            requested: 1,
                            available: 0,
                        });
                    };
                    let gl_buffer = match unsafe { self.gl.create_buffer() } {
                        Ok(buffer) => buffer,
                        Err(detail) => {
                            unsafe {
                                self.gl.delete_vertex_array(gl_vertex_array);
                                self.gl.delete_program(gl_program);
                            }
                            return Err(RhiError::Backend { detail });
                        }
                    };
                    unsafe {
                        self.gl.bind_buffer(glow::UNIFORM_BUFFER, Some(gl_buffer));
                        self.gl.buffer_data_size(
                            glow::UNIFORM_BUFFER,
                            push_size as i32,
                            glow::DYNAMIC_DRAW,
                        );
                        self.gl
                            .uniform_block_binding(gl_program, *block_index, binding);
                        self.gl
                            .bind_buffer_base(glow::UNIFORM_BUFFER, binding, Some(gl_buffer));
                        self.gl.bind_buffer(glow::UNIFORM_BUFFER, None);
                    }
                    tracing::debug!(
                        target: "opengl",
                        block = %block_name,
                        binding,
                        size = push_size,
                        "mapped RHI push constants to an OpenGL uniform buffer"
                    );
                    push_constant_buffer = Some(PushConstantBuffer {
                        gl_buffer,
                        binding,
                        size_bytes: push_size,
                    });
                }
            }
        }

        let (idx, gen) = self.pipelines.alloc(PipelineSlot {
            gl_program,
            gl_vertex_array,
            vertex_stride: descriptor.vertex_layout.stride_bytes,
            vertex_attributes,
            topology,
            raster_state,
            depth_state,
            blend_state,
            multisample: sample_count > 1,
            render_pass: descriptor.render_pass,
            pipeline_layout: descriptor.pipeline_layout,
            push_constant_buffer,
            push_uniforms,
            sampler_uniforms,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) {
        let slot = self.pipelines.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                // SAFETY: `self.gl` is a valid `glow::Context` created by this
                // device; all objects were created by the same context and are
                // no longer reachable after this handle is freed.
                unsafe {
                    if let Some(push_constants) = slot.value.push_constant_buffer {
                        self.gl.delete_buffer(push_constants.gl_buffer);
                    }
                    self.gl.delete_vertex_array(slot.value.gl_vertex_array);
                    self.gl.delete_program(slot.value.gl_program);
                };
            }
        }
        self.pipelines.free(handle.index);
    }

    // ██ frame lifecycle █████████████████████████████████████████████████████████████████

    fn begin_frame(
        &mut self,
        _swapchain: SwapchainHandle,
    ) -> Result<(u32, Box<dyn CommandEncoder>), RhiError> {
        Err(opengl_presentation_unsupported())
    }

    fn end_frame(
        &mut self,
        _swapchain: SwapchainHandle,
        _encoder: Box<dyn CommandEncoder>,
        _image_index: u32,
    ) -> Result<RendererStatistics, RhiError> {
        Err(opengl_presentation_unsupported())
    }

    fn recreate_swapchain(
        &mut self,
        _swapchain: SwapchainHandle,
        _width: u32,
        _height: u32,
    ) -> Result<(), RhiError> {
        Err(opengl_presentation_unsupported())
    }

    fn wait_idle(&self) {
        // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
        // the device is alive and not yet destroyed.
        unsafe {
            self.gl.finish();
        }
    }

    // ██ framebuffer readback ███████████████████████████████████████████████████████████████

    fn read_pixels(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, RhiError> {
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }

        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|v| v.checked_mul(4))
            .ok_or(RhiError::Backend {
                detail: "read_pixels: integer overflow in buffer size".to_string(),
            })?;

        let mut pixels = vec![0u8; size];

        // SAFETY: glow's read_pixels writes RGBA data into the pixel buffer.
        // The buffer is sized exactly to hold (width × height × 4) bytes.
        unsafe {
            self.gl.read_pixels(
                x as i32,
                y as i32,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }

        // OpenGL reads rows bottom-to-top; the trait contract specifies
        // top-to-bottom rows. Flip the rows in a second buffer.
        let row_size = (width as usize) * 4;
        let mut flipped = vec![0u8; size];
        for row in 0..height as usize {
            let src_start = (height as usize - 1 - row) * row_size;
            let dst_start = row * row_size;
            flipped[dst_start..dst_start + row_size]
                .copy_from_slice(&pixels[src_start..src_start + row_size]);
        }

        Ok(flipped)
    }
}

// ============================================================================
// Public constructor
// ============================================================================

pub fn backend(gl: glow::Context) -> OpenGlBackend {
    OpenGlBackend::new(gl)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shader_descriptor(source_bytes: Vec<u8>) -> ShaderModuleDescriptor {
        ShaderModuleDescriptor {
            format: ShaderFormat::Glsl,
            stage: ShaderStage::Vertex,
            source_bytes,
            entry_points: vec!["main".to_string()],
            source_hash: [7; 32],
            debug_label: Some("unit-test".to_string()),
        }
    }

    #[test]
    fn glsl_source_decode_preserves_real_source() {
        let descriptor = shader_descriptor(b"#version 450\nvoid main() {}\n".to_vec());
        assert_eq!(
            decode_glsl_source(&descriptor).unwrap(),
            "#version 450\nvoid main() {}\n"
        );
    }

    #[test]
    fn glsl_source_decode_rejects_invalid_input() {
        let mut invalid_utf8 = shader_descriptor(vec![0xff, 0xfe]);
        assert!(matches!(
            decode_glsl_source(&invalid_utf8),
            Err(RhiError::InvalidDescriptor { field, .. }) if field == "shader_module.source_bytes"
        ));

        invalid_utf8.source_bytes = b"void entry() {}".to_vec();
        invalid_utf8.entry_points = vec!["entry".to_string()];
        assert!(matches!(
            decode_glsl_source(&invalid_utf8),
            Err(RhiError::InvalidDescriptor { field, .. }) if field == "shader_module.entry_points"
        ));

        invalid_utf8.entry_points = vec!["main".to_string()];
        invalid_utf8.format = ShaderFormat::SpirV;
        assert!(matches!(
            decode_glsl_source(&invalid_utf8),
            Err(RhiError::InvalidDescriptor { field, .. }) if field == "shader_module.format"
        ));
    }

    #[test]
    fn vertex_attribute_formats_map_to_gl_pointer_kinds() {
        let position = parse_vertex_attribute_format("float32x3").unwrap();
        assert_eq!(position.component_count, 3);
        assert_eq!(position.gl_type, glow::FLOAT);
        assert!(!position.integer);
        assert!(!position.normalized);
        assert_eq!(position.size_bytes, 12);

        let joints = parse_vertex_attribute_format("uint32x4").unwrap();
        assert_eq!(joints.component_count, 4);
        assert_eq!(joints.gl_type, glow::UNSIGNED_INT);
        assert!(joints.integer);
        assert_eq!(joints.size_bytes, 16);

        let color = parse_vertex_attribute_format("rgba8unorm").unwrap();
        assert_eq!(color.gl_type, glow::UNSIGNED_BYTE);
        assert!(color.normalized);
        assert!(!color.integer);
        assert_eq!(color.size_bytes, 4);
        assert!(parse_vertex_attribute_format("mysteryx7").is_none());
    }

    #[test]
    fn vertex_layout_validation_assigns_locations_and_checks_stride() {
        let layout = VertexLayout {
            stride_bytes: 20,
            attributes: vec![
                VertexAttribute {
                    semantic: "position".to_string(),
                    format: "float32x3".to_string(),
                    offset_bytes: 0,
                },
                VertexAttribute {
                    semantic: "uv".to_string(),
                    format: "float32x2".to_string(),
                    offset_bytes: 12,
                },
            ],
        };
        let bindings = parse_vertex_layout(&layout).unwrap();
        assert_eq!(bindings[0].location, 0);
        assert_eq!(bindings[1].location, 1);

        let mut invalid = layout;
        invalid.stride_bytes = 16;
        assert!(matches!(
            parse_vertex_layout(&invalid),
            Err(RhiError::InvalidDescriptor { field, .. })
                if field == "pipeline.vertex_layout.attributes.offset_bytes"
        ));
    }

    #[test]
    fn portable_state_and_binding_names_have_deterministic_mappings() {
        assert_eq!(
            parse_topology(Some("triangle_list")).unwrap(),
            glow::TRIANGLES
        );
        assert_eq!(
            parse_topology(Some("line_strip")).unwrap(),
            glow::LINE_STRIP
        );
        assert!(parse_topology(Some("patches")).is_err());
        assert_eq!(push_uniform_offset("u_pc_64"), Some(64));
        assert_eq!(push_uniform_offset("u_push_constants[0]"), Some(0));
        assert_eq!(push_uniform_offset("u_light_color"), Some(80));
        assert_eq!(gl_binding_point(2, 3), Some(35));
    }

    #[test]
    fn presentation_without_a_platform_swap_callback_fails_closed() {
        assert!(matches!(
            opengl_presentation_unsupported(),
            RhiError::UnsupportedFeature { feature }
                if feature.contains("swap callback")
        ));
    }
}
