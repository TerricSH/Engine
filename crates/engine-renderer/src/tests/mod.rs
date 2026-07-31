use super::{
    validate_frame_input, AssetId, AxisAlignedBox, BackendRenderer, BlendMode, BonePaletteLayout,
    ClearFlags, Diagnostic, DiagnosticSeverity, EnvironmentCubeMip, EnvironmentMapFormat,
    EnvironmentMapUpload, FrameStats, IndexFormat, LightItem, LightKind, MaterialUpload,
    MeshUpload, MeshVertexFormat, PassGraphOutputMode, RenderFrameInput, RenderView, Renderer,
    RendererFrameStats, ResourceKind, ResourceRemoval, SamplerDescriptor, ShadowMode, SkinnedItem,
    TextureMipLevel, TextureUpload, TextureUploadFormat, ToneMapping, Transparency, UploadReceipt,
    ViewCompose, DIAG_ABORT_UNSUPPORTED, DIAG_BACKEND_MISSING, DIAG_BARRIERS_UNSUPPORTED,
    DIAG_CUSTOM_RENDER_GRAPH_UNSUPPORTED, DIAG_INVALID_ENVIRONMENT_MAP,
    DIAG_INVALID_MATERIAL_VALUES, DIAG_INVALID_MESH_VERTICES, DIAG_INVALID_MORPH_TARGET_SET,
    DIAG_INVALID_TEXTURE_MIPS, DIAG_MATERIAL_UPLOAD_UNSUPPORTED, DIAG_MESH_UPLOAD_UNSUPPORTED,
    DIAG_TEXTURE_UPLOAD_UNSUPPORTED, IDENTITY_MAT4,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[test]
fn shared_render_contract_types_keep_legacy_renderer_paths() {
    assert_eq!(
        std::any::TypeId::of::<IndexFormat>(),
        std::any::TypeId::of::<render_core::IndexFormat>()
    );
    assert_eq!(
        std::any::TypeId::of::<LightKind>(),
        std::any::TypeId::of::<engine_serialize::LightKind>()
    );
    assert_eq!(
        std::any::TypeId::of::<FrameStats>(),
        std::any::TypeId::of::<RendererFrameStats>()
    );
}

#[derive(Default)]
struct NullBackend;

impl BackendRenderer for NullBackend {
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &super::render_graph2::PassNode,
        _barriers: &[super::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn execute_pass(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &super::render_graph2::PassNode,
        _frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn upload_mesh(&mut self, _upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }

    fn upload_texture(&mut self, _upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        _upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }

    fn remove_resource(&mut self, _removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn resize(&mut self, _width: u32, _height: u32) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }
}

include!("draw_scene.rs");
include!("validation.rs");
include!("graph_contract.rs");
include!("upload_contract.rs");
include!("backend_failures.rs");
