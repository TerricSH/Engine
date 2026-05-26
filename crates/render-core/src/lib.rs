#![forbid(unsafe_code)]

use core::fmt;
use core::marker::PhantomData;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackendKind {
    Vulkan,
    OpenGl,
    DirectX12,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub backend: BackendKind,
    pub name: String,
    pub vendor_id: Option<u32>,
    pub device_id: Option<u32>,
    pub driver_version: Option<String>,
    pub capabilities: BackendCapabilities,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_buffer_bytes: u64,
    pub max_texture_array_layers: u32,
    pub max_bind_groups: u8,
    pub max_vertex_attributes: u8,
    pub max_color_attachments: u8,
    pub max_sample_count: u8,
}

pub struct ResourceHandle<KIND> {
    pub index: u32,
    pub generation: u32,
    marker: PhantomData<fn() -> KIND>,
}

impl<KIND> ResourceHandle<KIND> {
    pub const fn new(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            marker: PhantomData,
        }
    }
}

impl<KIND> Clone for ResourceHandle<KIND> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<KIND> Copy for ResourceHandle<KIND> {}

impl<KIND> fmt::Debug for ResourceHandle<KIND> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceHandle")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<KIND> PartialEq for ResourceHandle<KIND> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<KIND> Eq for ResourceHandle<KIND> {}

impl<KIND> core::hash::Hash for ResourceHandle<KIND> {
    fn hash<HASHER: core::hash::Hasher>(&self, state: &mut HASHER) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

pub enum BufferKind {}
pub enum TextureKind {}
pub enum ShaderModuleKind {}
pub enum PipelineKind {}
pub enum BindGroupKind {}
pub enum RenderPassKind {}
pub enum SurfaceKind {}

pub type BufferHandle = ResourceHandle<BufferKind>;
pub type TextureHandle = ResourceHandle<TextureKind>;
pub type ShaderModuleHandle = ResourceHandle<ShaderModuleKind>;
pub type PipelineHandle = ResourceHandle<PipelineKind>;
pub type BindGroupHandle = ResourceHandle<BindGroupKind>;
pub type RenderPassHandle = ResourceHandle<RenderPassKind>;
pub type SurfaceHandle = ResourceHandle<SurfaceKind>;

#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RhiError {
    #[error("unsupported backend")]
    UnsupportedBackend,
    #[error("unsupported feature: {feature}")]
    UnsupportedFeature { feature: String },
    #[error("unsupported limit {limit}: requested {requested}, available {available}")]
    UnsupportedLimit {
        limit: String,
        requested: u64,
        available: u64,
    },
    #[error("invalid descriptor field {field}: {reason}")]
    InvalidDescriptor { field: String, reason: String },
    #[error("invalid resource handle")]
    InvalidHandle,
    #[error("device lost")]
    DeviceLost,
    #[error("surface lost")]
    SurfaceLost,
    #[error("out of memory")]
    OutOfMemory,
    #[error("allocation failed for {bytes} bytes")]
    AllocationFailed { bytes: u64 },
    #[error("validation failed: {detail}")]
    ValidationFailed { detail: String },
    #[error("incompatible bind layout: {reason}")]
    IncompatibleBindLayout { reason: String },
    #[error("backend error: {detail}")]
    Backend { detail: String },
}

impl RhiError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedBackend => "rhi.unsupported_backend",
            Self::UnsupportedFeature { .. } => "rhi.unsupported_feature",
            Self::UnsupportedLimit { .. } => "rhi.unsupported_limit",
            Self::InvalidDescriptor { .. } => "rhi.invalid_descriptor",
            Self::InvalidHandle => "rhi.invalid_handle",
            Self::DeviceLost => "rhi.device_lost",
            Self::SurfaceLost => "rhi.surface_lost",
            Self::OutOfMemory => "rhi.out_of_memory",
            Self::AllocationFailed { .. } => "rhi.allocation_failed",
            Self::ValidationFailed { .. } => "rhi.validation_failed",
            Self::IncompatibleBindLayout { .. } => "rhi.incompatible_bind_layout",
            Self::Backend { .. } => "rhi.backend",
        }
    }

    pub const fn severity(&self) -> &'static str {
        match self {
            Self::UnsupportedBackend
            | Self::UnsupportedFeature { .. }
            | Self::UnsupportedLimit { .. }
            | Self::DeviceLost
            | Self::OutOfMemory => "fatal",
            Self::SurfaceLost => "error-recoverable",
            Self::InvalidDescriptor { .. }
            | Self::InvalidHandle
            | Self::AllocationFailed { .. }
            | Self::ValidationFailed { .. }
            | Self::IncompatibleBindLayout { .. }
            | Self::Backend { .. } => "error",
        }
    }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferUsage(pub u32);

impl BufferUsage {
    pub const VERTEX: Self = Self(1 << 0);
    pub const INDEX: Self = Self(1 << 1);
    pub const UNIFORM: Self = Self(1 << 2);
    pub const STORAGE: Self = Self(1 << 3);
    pub const COPY_SRC: Self = Self(1 << 4);
    pub const COPY_DST: Self = Self(1 << 5);
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDescriptor {
    pub adapter: AdapterInfo,
    pub required_features: Vec<String>,
    pub required_limits: ResourceLimits,
    pub debug_label: Option<String>,
    pub validation_mode: ValidationMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceDescriptor {
    pub window_handle: SurfaceTarget,
    pub width: u32,
    pub height: u32,
    pub preferred_format: TextureFormat,
    pub present_mode: PresentMode,
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindGroupLayoutDescriptor {
    pub set_index: u8,
    pub bindings: Vec<BindGroupLayoutBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindGroupLayoutBinding {
    pub binding: u32,
    pub resource_kind: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RasterState {
    pub cull_mode: Option<String>,
    pub front_face: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthState {
    pub format: Option<TextureFormat>,
    pub write_enabled: bool,
    pub compare: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlendState {
    pub mode: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineDescriptor {
    pub shader_modules: Vec<ShaderModuleHandle>,
    pub vertex_layout: VertexLayout,
    pub bind_layouts: Vec<BindGroupLayoutDescriptor>,
    pub raster_state: RasterState,
    pub depth_state: DepthState,
    pub blend_state: BlendState,
    pub render_targets: Vec<TextureFormat>,
    pub debug_label: Option<String>,
}

pub trait Backend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError>;
    fn create_device(&self, descriptor: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError>;
}

pub trait Device: Send + Sync {
    fn adapter_info(&self) -> &AdapterInfo;

    fn create_surface(&self, _descriptor: &SurfaceDescriptor) -> Result<SurfaceHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "surface creation is not implemented by this device".to_string(),
        })
    }

    fn create_buffer(&self, _descriptor: &BufferDescriptor) -> Result<BufferHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "buffer creation is not implemented by this device".to_string(),
        })
    }

    fn create_texture(&self, _descriptor: &TextureDescriptor) -> Result<TextureHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "texture creation is not implemented by this device".to_string(),
        })
    }

    fn create_shader_module(
        &self,
        _descriptor: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "shader module creation is not implemented by this device".to_string(),
        })
    }

    fn create_pipeline(
        &self,
        _descriptor: &PipelineDescriptor,
    ) -> Result<PipelineHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "pipeline creation is not implemented by this device".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::RhiError;

    #[test]
    fn rhi_error_codes_match_registry() {
        let cases = [
            (RhiError::UnsupportedBackend, "rhi.unsupported_backend"),
            (
                RhiError::UnsupportedFeature {
                    feature: "timeline-semaphore".to_string(),
                },
                "rhi.unsupported_feature",
            ),
            (
                RhiError::UnsupportedLimit {
                    limit: "max_bind_groups".to_string(),
                    requested: 4,
                    available: 2,
                },
                "rhi.unsupported_limit",
            ),
            (RhiError::InvalidHandle, "rhi.invalid_handle"),
            (RhiError::DeviceLost, "rhi.device_lost"),
            (RhiError::SurfaceLost, "rhi.surface_lost"),
            (RhiError::OutOfMemory, "rhi.out_of_memory"),
            (
                RhiError::AllocationFailed { bytes: 64 },
                "rhi.allocation_failed",
            ),
            (
                RhiError::ValidationFailed {
                    detail: "bad layout".to_string(),
                },
                "rhi.validation_failed",
            ),
            (
                RhiError::IncompatibleBindLayout {
                    reason: "set count".to_string(),
                },
                "rhi.incompatible_bind_layout",
            ),
            (
                RhiError::Backend {
                    detail: "driver".to_string(),
                },
                "rhi.backend",
            ),
        ];

        for (error, code) in cases {
            assert_eq!(error.code(), code);
        }
    }
}
