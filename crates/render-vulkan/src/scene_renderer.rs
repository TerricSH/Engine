//! Vulkan implementation of [`BackendRenderer`].
//!
//! Consumes [`RenderFrameInput`] and renders each drawable through a
//! forward-shaded pipeline with lighting.
//!
//! Resources are uploaded through the typed renderer contract before a frame
//! references them. The render graph records shadow, HDR forward, tone-map and
//! present passes with explicit abort handling when any pass fails.
//!
//! The portable material contract supports opaque, alpha-masked, alpha-blended,
//! and double-sided PBR base-color materials through explicit pipeline
//! variants.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ash::vk;
use glam::{Mat4, Vec3};

use engine_renderer::backend_shared::{
    apply_extraction_stats, first_missing_ui_texture, order_transparent_back_to_front,
    prepare_normalized_viewport, prepare_tone_map_plan,
    prepare_ui_overlay as prepare_shared_ui_overlay, select_environment_map,
    validate_backend_frame_contract, BackendFrameCapabilities, FrameContractViolation,
    PreparedUiOverlay, ToneMapPlan as ToneMapPushConstants, ToneMapPlanOptions,
};
use engine_renderer::{
    render_graph2, AssetId, BackendRenderer, Diagnostic, DiagnosticSeverity, EnvironmentMapUpload,
    FrameStats, LightKind, MaterialBinding, MaterialUpload, MeshUpload, MeshVertexFormat,
    ParamBlock, PassRegistry, RenderFrameInput, RenderPass, ResourceKind, ResourceRemoval,
    SamplerAddressMode, SamplerFilter, ShadowMode, TextureSlot, TextureUpload, UiBatch,
    UploadReceipt,
};
use render_core::{
    self, BufferDescriptor, BufferHandle, CommandEncoder, Device, FramebufferHandle, IndexFormat,
    MemoryHint, PipelineLayoutDescriptor, PipelineLayoutHandle, PushConstantRange,
    RenderPassDescriptor, RenderPassHandle, ShaderFormat, ShaderModuleDescriptor,
    ShaderModuleHandle, ShaderStage, SwapchainDescriptor, SwapchainHandle, TextureFormat,
};

#[cfg(test)]
use engine_renderer::backend_shared::{
    extraction_stats, PixelRect as UiScissor, ARTISTIC_LIGHTING_REFERENCE_EV100,
    TONE_MAP_MODE_ACES, TONE_MAP_MODE_NONE, TONE_MAP_MODE_REINHARD, UI_VERTEX_STRIDE,
};
#[cfg(test)]
use engine_renderer::ParticleBatch;
#[cfg(test)]
use render_core::PipelineDescriptor;

use crate::device_impl::VulkanDevice;
use crate::instance_data::*;
use crate::shaders_embedded::{
    FORWARD_FRAG_SPV, FORWARD_VERT_SPV, GPU_VFX_BILLBOARD_VERT_SPV, INSTANCED_VERT_SPV,
    SKINNED_VERT_SPV, SKYBOX_FRAG_SPV, SKYBOX_VERT_SPV, VFX_BILLBOARD_FRAG_SPV,
    VFX_BILLBOARD_VERT_SPV,
};
use engine_renderer::{build_clustered_light_frame, normalize_direction};

mod support;

pub use support::GpuMesh;
use support::*;

fn material_texture_ids_for_descriptor(
    material: &MaterialBinding,
    mut texture_available: impl FnMut(&str) -> bool,
) -> Result<[String; 5], Box<Diagnostic>> {
    use crate::device_impl::texture::FALLBACK_MATERIAL_TEXTURE_ID;

    let mut selected = std::array::from_fn(|_| FALLBACK_MATERIAL_TEXTURE_ID.to_string());
    for (index, binding) in MATERIAL_TEXTURE_BINDINGS.into_iter().enumerate() {
        let Some(slot) = material
            .textures
            .iter()
            .find(|slot| slot.binding == binding)
        else {
            continue;
        };
        if !texture_available(&slot.texture.id) {
            return Err(Box::new(Diagnostic::new(
                "RV0260",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "material '{}' references texture '{}' at binding {} before a successful upload",
                    material.material_id.id, slot.texture.id, binding
                ),
            )));
        }
        selected[index] = slot.texture.id.clone();
    }
    Ok(selected)
}

// ============================================================================
// GpuMesh
// ============================================================================

mod backend;
mod drop;
mod forward;
mod frame;
mod lifecycle;
mod post_process;
mod resources;
mod shadow;
mod state;
mod timing;

pub use state::SceneRenderer;

#[cfg(test)]
#[path = "scene_renderer_tests.rs"]
mod tests;
