//! Rendering façade implementation for [`EngineRuntime`](crate::EngineRuntime).
//!
//! The runtime remains the public composition root, while extraction, asset
//! synchronization, submission, and render diagnostics live in this adapter.

use engine_asset::{AssetHandle, AssetRegistry};
use engine_renderer::{
    BackendRenderer, EnvironmentMapUpload, EnvironmentSettings, FrameStats, MaterialUpload,
    MeshUpload, MorphTargetSetUpload, RenderFrameInput, TextureUpload,
};
use engine_scene::{
    extract_renderer_input_from_world, extract_renderer_input_from_world_with_viewport,
    RenderViewportContext,
};
use engine_serialize::{Diagnostic, DiagnosticSeverity};

use crate::{
    missing_registered_render_asset, validate_registered_asset_id, DiagnosticsCollector,
    EngineRuntime, RuntimeDiagnostics,
};

impl EngineRuntime {
    /// Install the backend used by the runtime's sole render pipeline.
    pub fn set_renderer_backend(&mut self, mut backend: Box<dyn BackendRenderer>) {
        backend.set_gpu_timing_enabled(self.config.gpu_timestamps);
        self.renderer.set_backend(backend);
    }

    /// Resize the active backend without exposing unrestricted renderer access.
    pub fn resize_renderer(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        self.renderer.resize(width, height)
    }

    /// Upload an editor-owned, short-lived preview texture.
    pub fn upload_temporary_preview_texture(
        &mut self,
        upload: TextureUpload,
    ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
        let id = upload.texture_id.clone();
        self.asset_registry.insert_typed(id, upload);
        Ok(engine_renderer::UploadReceipt {
            revision: 1,
            warnings: Vec::new(),
        })
    }

    /// Release a temporary preview texture.
    pub fn remove_temporary_preview_texture(
        &mut self,
        texture_id: engine_renderer::AssetId,
    ) -> Result<(), Vec<Diagnostic>> {
        self.asset_registry.unload(&texture_id);
        self.renderer
            .remove_resource(engine_renderer::ResourceRemoval {
                kind: engine_renderer::ResourceKind::Texture,
                resource_id: texture_id,
            })
    }

    /// Runtime asset cache used by renderer-resource synchronization.
    pub fn asset_registry(&self) -> &AssetRegistry {
        &self.asset_registry
    }

    pub fn asset_registry_mut(&mut self) -> &mut AssetRegistry {
        &mut self.asset_registry
    }

    pub fn register_mesh_asset(&mut self, upload: MeshUpload) -> AssetHandle<MeshUpload> {
        let id = upload.mesh_id.clone();
        self.asset_registry.insert_typed(id, upload)
    }

    pub fn register_texture_asset(&mut self, upload: TextureUpload) -> AssetHandle<TextureUpload> {
        let id = upload.texture_id.clone();
        self.asset_registry.insert_typed(id, upload)
    }

    pub fn register_environment_map_asset(
        &mut self,
        upload: EnvironmentMapUpload,
    ) -> AssetHandle<EnvironmentMapUpload> {
        let id = upload.environment_id.clone();
        self.asset_registry.insert_typed(id, upload)
    }

    pub fn register_morph_target_set_asset(
        &mut self,
        upload: MorphTargetSetUpload,
    ) -> AssetHandle<MorphTargetSetUpload> {
        let id = upload.target_set_id.clone();
        self.asset_registry.insert_typed(id, upload)
    }

    pub fn set_environment_settings(&mut self, settings: EnvironmentSettings) {
        self.render_environment = settings;
    }

    pub fn environment_settings(&self) -> &EnvironmentSettings {
        &self.render_environment
    }

    pub fn register_material_asset(
        &mut self,
        upload: MaterialUpload,
    ) -> AssetHandle<MaterialUpload> {
        let id = upload.material_id.clone();
        self.asset_registry.insert_typed(id, upload)
    }

    pub fn diagnostics_collector(&self) -> &DiagnosticsCollector {
        &self.collector
    }

    pub fn diagnostics_collector_mut(&mut self) -> &mut DiagnosticsCollector {
        &mut self.collector
    }

    pub fn runtime_diagnostics(&self) -> RuntimeDiagnostics {
        RuntimeDiagnostics {
            collector: self.collector.clone(),
            reload_queue: None,
            frame_timing: self.frame_timing.summary(),
            runtime_meshes: self.runtime_mesh_memory(),
            #[cfg(feature = "subsystem-scripting-csharp")]
            script_engine_state: format!(
                "{} coroutines={}",
                crate::script_engine_state_summary(&self.script_engine),
                engine_ffi::coroutine::active_managed_coroutine_count(),
            ),
            #[cfg(not(feature = "subsystem-scripting-csharp"))]
            script_engine_state: format!(
                "coroutines={}",
                engine_ffi::coroutine::active_managed_coroutine_count()
            ),
        }
    }

    pub fn render_frame(&mut self, frame_index: u64) -> Result<FrameStats, Vec<Diagnostic>> {
        self.render_frame_with_ui(frame_index, Vec::new())
    }

    pub fn render_frame_with_ui(
        &mut self,
        frame_index: u64,
        ui_batches: Vec<engine_renderer::UiBatch>,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        self.render_frame_submission(frame_index, ui_batches, None)
    }

    pub fn render_frame_with_ui_in_viewport(
        &mut self,
        frame_index: u64,
        ui_batches: Vec<engine_renderer::UiBatch>,
        viewport: RenderViewportContext,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        self.render_frame_submission(frame_index, ui_batches, Some(viewport))
    }

    fn render_frame_submission(
        &mut self,
        frame_index: u64,
        ui_batches: Vec<engine_renderer::UiBatch>,
        viewport: Option<RenderViewportContext>,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        self.drain_deferred_runtime_mesh_removals();
        self.frame_timing.begin_stage("extraction");
        let extraction = self.world_slot.with_world(|world| {
            let mut result = match viewport {
                Some(viewport) => {
                    extract_renderer_input_from_world_with_viewport(world, frame_index, viewport)
                }
                None => extract_renderer_input_from_world(world, frame_index),
            };
            if let Ok(input) = &mut result {
                engine_vfx::extract_vfx(world, input);
            }
            result
        });
        let mut input = if let Some(result) = extraction {
            match result {
                Ok(input) => input,
                Err(diagnostics) => {
                    self.frame_timing.end_stage("extraction");
                    self.frame_timing.discard_frame();
                    return Err(diagnostics);
                }
            }
        } else if self.scene.is_some() {
            self.frame_timing.end_stage("extraction");
            self.frame_timing.discard_frame();
            return Err(vec![Diagnostic::new(
                "SC0019",
                DiagnosticSeverity::Error,
                "engine-core",
                "a scene snapshot exists without an active World; reload it through load_scene",
            )]);
        } else {
            self.frame_timing.end_stage("extraction");
            self.frame_timing.discard_frame();
            return Err(vec![Diagnostic::new(
                "SC0018",
                DiagnosticSeverity::Error,
                "engine-core",
                "no scene is loaded",
            )]);
        };
        self.render_extension_registry
            .produce_all(&mut input, frame_index);
        if input.render_options.environment == EnvironmentSettings::default() {
            input.render_options.environment = self.render_environment.clone();
        }
        input.ui_batches.extend(ui_batches);
        self.refresh_generated_ui_assets();
        self.frame_timing.end_stage("extraction");

        self.frame_timing.begin_stage("sync_render_assets");
        let sync_result = self.sync_render_assets(&input);
        self.frame_timing.end_stage("sync_render_assets");
        if let Err(diagnostics) = sync_result {
            self.collector.push_asset_diags(diagnostics.clone());
            self.frame_timing.discard_frame();
            return Err(diagnostics);
        }

        self.frame_timing.begin_stage("render_submit");
        let result = self.renderer.draw_scene(&input);
        self.frame_timing.end_stage("render_submit");
        match result {
            Ok(stats) => {
                self.frame_timing.finish_frame(
                    frame_index,
                    stats.gpu_timing,
                    stats.gpu_pass_frame_index,
                    stats.gpu_pass_times.clone(),
                );
                self.collector.record_frame(frame_index, &stats);
                Ok(stats)
            }
            Err(diagnostics) => {
                self.frame_timing.discard_frame();
                Err(diagnostics)
            }
        }
    }

    pub fn frame_timing_summary(&self) -> engine_renderer::FrameTimingSummary {
        self.frame_timing.summary()
    }

    pub fn last_frame_timings(&self) -> Option<&engine_renderer::FrameTimings> {
        self.frame_timing.last_frame()
    }

    pub(crate) fn frame_timing_begin_stage(&mut self, name: &str) {
        self.frame_timing.begin_stage(name);
    }

    pub(crate) fn frame_timing_end_stage(&mut self, name: &str) {
        self.frame_timing.end_stage(name);
    }

    fn sync_render_assets(&mut self, input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        let mut material_ids = std::collections::BTreeMap::new();
        let mut mesh_ids = std::collections::BTreeMap::new();
        for drawable in &input.drawables {
            material_ids.insert(drawable.material.id.clone(), drawable.material.clone());
            mesh_ids.insert(drawable.mesh.id.clone(), drawable.mesh.clone());
        }
        for item in &input.skinned_items {
            material_ids.insert(item.material.id.clone(), item.material.clone());
            mesh_ids.insert(item.mesh.id.clone(), item.mesh.clone());
        }

        let mut materials = Vec::new();
        let mut texture_ids = std::collections::BTreeMap::new();
        for id in material_ids.values() {
            let handle = self
                .asset_registry
                .get::<MaterialUpload>(id)
                .ok_or_else(|| missing_registered_render_asset("material", id))?;
            let upload = handle.get().clone();
            validate_registered_asset_id("material", id, &upload.material_id)?;
            for texture in upload.texture_references().into_iter().flatten() {
                texture_ids.insert(texture.id.clone(), texture.clone());
            }
            materials.push(upload);
        }
        for batch in &input.ui_batches {
            if let Some(texture) = &batch.texture {
                texture_ids.insert(texture.id.clone(), texture.clone());
            }
        }

        let mut textures = Vec::new();
        for id in texture_ids.values() {
            let handle = self
                .asset_registry
                .get::<TextureUpload>(id)
                .ok_or_else(|| missing_registered_render_asset("texture", id))?;
            let upload = handle.get().clone();
            validate_registered_asset_id("texture", id, &upload.texture_id)?;
            textures.push(upload);
        }

        let mut meshes = Vec::new();
        for id in mesh_ids.values() {
            let handle = self
                .asset_registry
                .get::<MeshUpload>(id)
                .ok_or_else(|| missing_registered_render_asset("mesh", id))?;
            let upload = handle.get().clone();
            validate_registered_asset_id("mesh", id, &upload.mesh_id)?;
            meshes.push(upload);
        }

        let mut morph_target_ids = std::collections::BTreeMap::new();
        for item in &input.skinned_items {
            if let Some(id) = &item.morph_target_set {
                morph_target_ids.insert(id.id.clone(), id.clone());
            }
        }
        let mut morph_target_sets = Vec::new();
        for id in morph_target_ids.values() {
            let handle = self
                .asset_registry
                .get::<MorphTargetSetUpload>(id)
                .ok_or_else(|| missing_registered_render_asset("morph target set", id))?;
            let upload = handle.get().clone();
            validate_registered_asset_id("morph target set", id, &upload.target_set_id)?;
            morph_target_sets.push(upload);
        }

        let mut environment_ids = std::collections::BTreeMap::new();
        if let Some(environment) = &input.render_options.environment.environment_map {
            environment_ids.insert(environment.id.clone(), environment.clone());
        }
        for probe in &input.render_options.environment.reflection_probes {
            environment_ids.insert(
                probe.environment_map.id.clone(),
                probe.environment_map.clone(),
            );
        }
        let mut environment_maps = Vec::new();
        for id in environment_ids.values() {
            let handle = self
                .asset_registry
                .get::<EnvironmentMapUpload>(id)
                .ok_or_else(|| missing_registered_render_asset("environment map", id))?;
            let upload = handle.get().clone();
            validate_registered_asset_id("environment map", id, &upload.environment_id)?;
            environment_maps.push(upload);
        }

        for upload in textures {
            let receipt = self.renderer.upload_texture(upload)?;
            self.collector.push_asset_diags(receipt.warnings);
        }
        for upload in materials {
            let receipt = self.renderer.upload_material(upload)?;
            self.collector.push_asset_diags(receipt.warnings);
        }
        for upload in meshes {
            let receipt = self.renderer.upload_mesh(upload)?;
            self.collector.push_asset_diags(receipt.warnings);
        }
        for upload in morph_target_sets {
            let receipt = self.renderer.upload_morph_target_set(upload)?;
            self.collector.push_asset_diags(receipt.warnings);
        }
        for upload in environment_maps {
            let receipt = self.renderer.upload_environment_map(upload)?;
            self.collector.push_asset_diags(receipt.warnings);
        }
        Ok(())
    }

    fn refresh_generated_ui_assets(&mut self) {
        #[cfg(feature = "subsystem-ui")]
        if let Some(upload) = engine_ui::font_atlas_texture_upload() {
            let changed = self
                .asset_registry
                .get::<TextureUpload>(&upload.texture_id)
                .is_none_or(|current| current.get().content_hash != upload.content_hash);
            if changed {
                self.asset_registry
                    .insert_typed(upload.texture_id.clone(), upload);
            }
        }
    }
}
