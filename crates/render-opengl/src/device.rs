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

mod frame;
mod framebuffers;
mod pipelines;
mod resources;

use frame::impl_device_frame;
use framebuffers::impl_device_framebuffers;
use pipelines::impl_device_pipelines;
use resources::impl_device_resources;

impl Device for OpenGlDevice {
    impl_device_resources!();
    impl_device_framebuffers!();
    impl_device_pipelines!();
    impl_device_frame!();
}
pub fn backend(gl: glow::Context) -> OpenGlBackend {
    OpenGlBackend::new(gl)
}

#[cfg(test)]
#[path = "device/tests.rs"]
mod tests;
