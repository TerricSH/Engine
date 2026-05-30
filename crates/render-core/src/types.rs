use serde::{Deserialize, Serialize};

use crate::handles::{
    PipelineLayoutHandle, RenderPassHandle, ShaderModuleHandle, SurfaceHandle, TextureHandle,
};

// ============================================================================
// Enums
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackendKind {
    Vulkan,
    OpenGl,
    DirectX12,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ShaderFormat {
    SpirV,
    Glsl,
    Dxil,
    Wgsl,
    Hlsl,
    MslSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndexFormat {
    U16,
    U32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TextureFormat {
    Rgba8Unorm,
    Bgra8Unorm,
    Rgba16Float,
    Depth32Float,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationMode {
    Disabled,
    Standard,
    Strict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresentMode {
    Fifo,
    Mailbox,
    Immediate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryHint {
    GpuOnly,
    CpuToGpu,
    GpuToCpu,
    CpuOnly,
}

// ============================================================================
// Adapter / Capabilities / Limits
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub backend: BackendKind,
    pub name: String,
    pub vendor_id: Option<u32>,
    pub device_id: Option<u32>,
    pub driver_version: Option<String>,
    pub capabilities: BackendCapabilities,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BackendCapabilities {
    pub max_texture_dimension_2d: u32,
    pub max_color_attachments: u8,
    pub supports_swapchain: bool,
    pub supports_timestamps: bool,
    pub supports_debug_markers: bool,
    pub supported_shader_formats: Vec<ShaderFormat>,
    pub supported_surface_formats: Vec<TextureFormat>,
    pub limits: ResourceLimits,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_buffer_bytes: u64,
    pub max_texture_array_layers: u32,
    pub max_bind_groups: u8,
    pub max_vertex_attributes: u8,
    pub max_color_attachments: u8,
    pub max_sample_count: u8,
}

// ============================================================================
// Usage flags
// ============================================================================
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferUsage(pub u32);

impl BufferUsage {
    pub const VERTEX: Self = Self(1 << 0);
    pub const INDEX: Self = Self(1 << 1);
    pub const UNIFORM: Self = Self(1 << 2);
    pub const STORAGE: Self = Self(1 << 3);
    pub const COPY_SRC: Self = Self(1 << 4);
    pub const COPY_DST: Self = Self(1 << 5);
    pub const INDIRECT: Self = Self(1 << 6);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureUsage(pub u32);

impl TextureUsage {
    pub const SAMPLED: Self = Self(1 << 0);
    pub const COLOR_ATTACHMENT: Self = Self(1 << 1);
    pub const DEPTH_ATTACHMENT: Self = Self(1 << 2);
    pub const COPY_SRC: Self = Self(1 << 3);
    pub const COPY_DST: Self = Self(1 << 4);
}

// ============================================================================
// Descriptors
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDescriptor {
    pub adapter: AdapterInfo,
    pub required_features: Vec<String>,
    pub required_limits: ResourceLimits,
    pub debug_label: Option<String>,
    pub validation_mode: ValidationMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceDescriptor {
    pub window_handle: SurfaceTarget,
    pub width: u32,
    pub height: u32,
    pub preferred_format: TextureFormat,
    pub present_mode: PresentMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapchainDescriptor {
    pub surface: SurfaceHandle,
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
    pub debug_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurfaceTarget {
    RawWindowHandleToken(u64),
    Headless,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferDescriptor {
    pub size_bytes: u64,
    pub usage_flags: BufferUsage,
    pub memory_hint: MemoryHint,
    pub debug_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureDescriptor {
    pub width: u32,
    pub height: u32,
    pub depth_or_layers: u32,
    pub mip_levels: u32,
    pub format: TextureFormat,
    pub usage_flags: TextureUsage,
    pub sample_count: u8,
    pub debug_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderModuleDescriptor {
    pub format: ShaderFormat,
    pub entry_points: Vec<String>,
    pub source_hash: [u8; 32],
    pub debug_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderPassDescriptor {
    pub color_attachments: Vec<TextureFormat>,
    pub depth_stencil_format: Option<TextureFormat>,
    pub sample_count: u8,
    pub debug_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FramebufferDescriptor {
    pub render_pass: RenderPassHandle,
    pub color_attachments: Vec<TextureHandle>,
    pub depth_stencil_attachment: Option<TextureHandle>,
    pub width: u32,
    pub height: u32,
    pub debug_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineLayoutDescriptor {
    pub bind_group_layouts: Vec<BindGroupLayoutDescriptor>,
    pub push_constant_ranges: Vec<PushConstantRange>,
    pub debug_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushConstantRange {
    pub stage_flags: u32,
    pub offset: u32,
    pub size: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineDescriptor {
    pub shader_modules: Vec<ShaderModuleHandle>,
    pub vertex_layout: VertexLayout,
    pub bind_layouts: Vec<BindGroupLayoutDescriptor>,
    pub pipeline_layout: Option<PipelineLayoutHandle>,
    pub raster_state: RasterState,
    pub depth_state: DepthState,
    pub blend_state: BlendState,
    pub render_targets: Vec<TextureFormat>,
    pub debug_label: Option<String>,
    // P1.2+: Pipeline topology, polygon mode, sample count, render pass
    pub topology: Option<String>,
    pub polygon_mode: Option<String>,
    pub sample_count: Option<u8>,
    pub render_pass: Option<RenderPassHandle>,
}

impl Default for PipelineDescriptor {
    fn default() -> Self {
        Self {
            shader_modules: Vec::new(),
            vertex_layout: VertexLayout::default(),
            bind_layouts: Vec::new(),
            pipeline_layout: None,
            raster_state: RasterState::default(),
            depth_state: DepthState::default(),
            blend_state: BlendState::default(),
            render_targets: vec![],
            debug_label: None,
            topology: Some("triangle_list".into()),
            polygon_mode: Some("fill".into()),
            sample_count: Some(1),
            render_pass: None,
        }
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Per-frame statistics populated by the renderer after end_frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RendererStatistics {
    pub draw_calls: u32,
    pub triangles: u64,
    pub gpu_frame_ms: f32,
}

// ============================================================================
// Pipeline variant key  (FD-040)
// ============================================================================

/// Bit-packed variant key for shader pipeline permutations (per FD-040).
///
/// Engine-reserved bits:
///   bit 0: SKINNED
///   bit 1: INSTANCED
///   bit 2: SHADOW_PASS
///   bits 3-7: MAX_LIGHTS_<N> (3 bits = 0-7)
///   bits 8+: material-defined
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct PipelineVariantKey(pub u64);

impl PipelineVariantKey {
    pub const NONE: Self = Self(0);
    pub const SKINNED: Self = Self(1 << 0);
    pub const INSTANCED: Self = Self(1 << 1);
    pub const SHADOW_PASS: Self = Self(1 << 2);

    pub const fn new(key: u64) -> Self {
        Self(key)
    }
    pub fn with_bit(mut self, bit: u64) -> Self {
        self.0 |= bit;
        self
    }

    /// Combine this key with another by OR-ing their bitmasks.
    pub fn with(mut self, flag: Self) -> Self {
        self.0 |= flag.0;
        self
    }

    /// Returns `true` if all bits in `other` are set in `self`.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns the raw bitmask value.
    pub fn bits(&self) -> u64 {
        self.0
    }
}

// ============================================================================
// Pipeline sub-types
// ============================================================================

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VertexLayout {
    pub stride_bytes: u32,
    pub attributes: Vec<VertexAttribute>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VertexAttribute {
    pub semantic: String,
    pub format: String,
    pub offset_bytes: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BindGroupLayoutDescriptor {
    pub set_index: u8,
    pub bindings: Vec<BindGroupLayoutBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BindGroupLayoutBinding {
    pub binding: u32,
    pub resource_kind: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RasterState {
    pub cull_mode: Option<String>,
    pub front_face: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DepthState {
    pub format: Option<TextureFormat>,
    pub write_enabled: bool,
    pub compare: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlendState {
    pub mode: Option<String>,
}

// ── Specialization constants for pipeline variants (Phase C) ──────────────

/// Value of a single specialization constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpecValue {
    Bool(bool),
    U32(u32),
    F32(f32),
}

/// A single specialization constant entry.
#[derive(Clone, Debug, PartialEq)]
pub struct SpecConstant {
    pub id: u32,
    pub value: SpecValue,
}
