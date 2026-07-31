//! DirectX 12 implementation of [`engine_renderer::BackendRenderer`].
//!
//! The portable upload path supports owned static/skinned PBR meshes, textures,
//! and opaque, alpha-masked, alpha-blended, or double-sided material variants.

// ============================================================================
// Windows + backend-dx12: full implementation
// ============================================================================

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use std::collections::HashMap;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use engine_renderer::backend_shared::{
    extraction_stats, order_transparent_back_to_front, prepare_normalized_viewport,
    prepare_ui_overlay, select_environment_map, validate_backend_frame_contract,
    BackendFrameCapabilities, FrameContractViolation, PreparedUiOverlay,
};
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use engine_renderer::{
    render_graph2, BackendRenderer, Diagnostic, DiagnosticSeverity, EnvironmentMapUpload,
    FrameStats, MaterialUpload, MeshUpload, MeshVertexFormat as RendererMeshVertexFormat,
    MorphTargetSetUpload, RenderFrameInput, ResourceKind, ResourceRemoval, ShadowMode,
    TextureUpload, Transparency, UploadReceipt,
};
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use glam::Mat4;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use render_core::{
    BufferDescriptor, BufferHandle, CommandEncoder, Device, FramebufferHandle,
    IndexFormat as RhiIndexFormat, MemoryHint, PipelineDescriptor, PipelineHandle,
    PipelineLayoutHandle, RenderPassHandle, SwapchainHandle, TextureHandle,
};

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::device::Dx12Device;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use crate::scene_data::*;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod backend;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod forward;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod lifecycle;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod pipelines;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod post_process;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod resources;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod shadow;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod support;

#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub(crate) use support::Dx12ShadowFrameData;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
use support::*;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
pub use support::{Dx12GpuMesh, Dx12SceneRenderer};

#[cfg(all(test, target_os = "windows", feature = "backend-dx12"))]
#[path = "scene_renderer_material_tests.rs"]
mod material_tests;
#[cfg(all(test, not(all(target_os = "windows", feature = "backend-dx12"))))]
#[path = "scene_renderer_stub_tests.rs"]
mod stub_tests;

#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
mod fallback;
#[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
pub use fallback::Dx12SceneRenderer;
