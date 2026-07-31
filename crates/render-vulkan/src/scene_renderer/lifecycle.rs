use super::*;

impl SceneRenderer {
    /// Create a new scene renderer backed by the given [`VulkanDevice`].
    ///
    /// `width` and `height` represent the initial swapchain extent in
    /// logical pixels.
    pub fn new(device: VulkanDevice, width: u32, height: u32) -> Self {
        Self {
            device,
            initialized: false,
            meshes: BTreeMap::new(),
            texture_uploads: HashMap::new(),
            environment_uploads: HashMap::new(),
            environment_revisions: HashMap::new(),
            active_environment_id: None,
            morph_target_sets: HashMap::new(),
            fallback_morph_buffer: None,
            uploaded_materials: HashMap::new(),
            material_cache: HashMap::new(),
            material_cache_order: Vec::new(),
            bone_palette_buffers: HashMap::new(),
            bone_palette_buffers_order: Vec::new(),
            skinned_desc_cache: HashMap::new(),
            skinned_desc_cache_order: Vec::new(),
            rp: None,
            pll: None,
            forward_shader_modules: Vec::new(),
            skinned_shader_modules: Vec::new(),
            ui_vbs: [None; 2],
            ui_vb_capacities: [0; 2],
            particle_instance_vbs: [None; 2],
            particle_instance_capacities: [0; 2],
            static_instance_vbs: [None; 2],
            static_instance_capacities: [0; 2],
            framebuffers: Vec::new(),
            cur_fb_index: 0,
            cur_sc: None,
            cur_ii: None,
            cur_enc: None,
            width: width.max(1),
            height: height.max(1),
            pass_registry: PassRegistry::new(),
            gpu_timestamps: crate::timestamps::GpuTimestampProfiler::new(),
            timestamp_pools: crate::timestamps::TimestampQueryPools::new(),
            gpu_timing_enabled: true,
            gpu_timing_configured: false,
        }
    }

    /// Register and prepare a custom render pass.
    ///
    /// Registration is allowed whenever no frame is active. Preparation is
    /// performed exactly once before the pass is inserted into the registry,
    /// so a failing pass cannot become visible to graph execution. Built-in
    /// pass names are reserved because Vulkan dispatches those passes directly.
    pub fn register_pass(&mut self, pass: Box<dyn RenderPass>) -> Result<(), Vec<Diagnostic>> {
        if self.cur_enc.is_some() || self.cur_sc.is_some() || self.cur_ii.is_some() {
            return Err(vec![Diagnostic::new(
                "RV0297",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "cannot register a custom render pass while a frame is active",
            )]);
        }
        prepare_and_register_custom_pass(&mut self.pass_registry, &mut self.device, pass)
    }

    /// Forward a resize notification to the underlying device.
    ///
    /// The swapchain will be re-created on the next frame.
    pub fn resize(&mut self, w: u32, h: u32) {
        self.width = w.max(1);
        self.height = h.max(1);
        self.device.resize(w, h);
    }

    /// Block until the GPU is idle.
    pub fn wait_idle(&self) {
        self.device.wait_idle();
    }

    // ------------------------------------------------------------------
    // Pipeline initialisation (lazy; called on the first frame).
    // ------------------------------------------------------------------

    pub(super) fn configure_scene_shaders(&mut self) {
        self.device
            .set_forward_shaders(FORWARD_VERT_SPV, FORWARD_FRAG_SPV);
        self.device
            .set_skybox_shaders(SKYBOX_VERT_SPV, SKYBOX_FRAG_SPV);
        if !SKINNED_VERT_SPV.is_empty() {
            self.device.set_skinned_vertex_shader(SKINNED_VERT_SPV);
        }
        if !VFX_BILLBOARD_VERT_SPV.is_empty()
            && !GPU_VFX_BILLBOARD_VERT_SPV.is_empty()
            && !VFX_BILLBOARD_FRAG_SPV.is_empty()
        {
            self.device.set_vfx_billboard_shaders(
                VFX_BILLBOARD_VERT_SPV,
                GPU_VFX_BILLBOARD_VERT_SPV,
                VFX_BILLBOARD_FRAG_SPV,
            );
        }
        if !INSTANCED_VERT_SPV.is_empty() {
            self.device.set_instanced_vertex_shader(INSTANCED_VERT_SPV);
        }
    }

    pub(super) fn create_scene_shader_modules(&mut self) -> Result<(), Vec<Diagnostic>> {
        if !self.forward_shader_modules.is_empty() && !self.skinned_shader_modules.is_empty() {
            return Ok(());
        }
        if FORWARD_VERT_SPV.is_empty()
            || FORWARD_FRAG_SPV.is_empty()
            || SKINNED_VERT_SPV.is_empty()
            || VFX_BILLBOARD_VERT_SPV.is_empty()
            || VFX_BILLBOARD_FRAG_SPV.is_empty()
            || INSTANCED_VERT_SPV.is_empty()
        {
            return Err(vec![Diagnostic::new(
                "RV0293",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "embedded forward, skinned, instanced, or VFX SPIR-V is unavailable",
            )]);
        }

        let forward_vertex = self
            .device
            .create_shader_module(&ShaderModuleDescriptor {
                format: ShaderFormat::SpirV,
                stage: ShaderStage::Vertex,
                source_bytes: FORWARD_VERT_SPV.to_vec(),
                entry_points: vec!["main".into()],
                source_hash: [0x61; 32],
                debug_label: Some("scene-forward-vs".into()),
            })
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0294",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create forward vertex shader: {error:?}"),
                )]
            })?;

        let fragment = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::SpirV,
            stage: ShaderStage::Fragment,
            source_bytes: FORWARD_FRAG_SPV.to_vec(),
            entry_points: vec!["main".into()],
            source_hash: [0x62; 32],
            debug_label: Some("scene-forward-fs".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                self.device.destroy_shader_module(forward_vertex);
                return Err(vec![Diagnostic::new(
                    "RV0295",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create forward fragment shader: {error:?}"),
                )]);
            }
        };

        let skinned_vertex = match self.device.create_shader_module(&ShaderModuleDescriptor {
            format: ShaderFormat::SpirV,
            stage: ShaderStage::Vertex,
            source_bytes: SKINNED_VERT_SPV.to_vec(),
            entry_points: vec!["main".into()],
            source_hash: [0x63; 32],
            debug_label: Some("scene-skinned-vs".into()),
        }) {
            Ok(shader) => shader,
            Err(error) => {
                self.device.destroy_shader_module(fragment);
                self.device.destroy_shader_module(forward_vertex);
                return Err(vec![Diagnostic::new(
                    "RV0296",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create skinned vertex shader: {error:?}"),
                )]);
            }
        };

        self.forward_shader_modules = vec![forward_vertex, fragment];
        self.skinned_shader_modules = vec![skinned_vertex, fragment];
        Ok(())
    }

    /// Create the render pass and pipeline layout used by scene-forward draws.
    ///
    /// This is called once from [`begin_frame_impl`] when
    /// `self.initialized` is `false`.
    pub(super) fn init_once(&mut self) -> Result<(), Vec<Diagnostic>> {
        if self.initialized {
            return Ok(());
        }

        // Ensure material descriptor infrastructure (set=2) exists before
        // creating the pipeline layout so the fallback picks it up.
        self.device
            .create_material_descriptor_infra()
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0213",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_material_descriptor_infra: {e:?}"),
                )]
            })?;

        // --- Render pass  (colour + depth) ---
        // NOTE: the scene-forward render pass renders directly to the
        // swapchain (BGRA8, always single-sampled).  MSAA is handled by
        // the HDR offscreen forward pass instead.
        let rp_desc = RenderPassDescriptor {
            color_attachments: vec![TextureFormat::Bgra8Unorm],
            depth_stencil_format: Some(TextureFormat::Depth32Float),
            sample_count: 1,
            present_after: true,
            debug_label: Some("scene-rp".into()),
        };
        let rp = self.device.create_render_pass(&rp_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0200",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_render_pass: {e:?}"),
            )]
        })?;

        // --- Pipeline layout  (push constants for MVP) ---
        let pll_desc = PipelineLayoutDescriptor {
            bind_group_layouts: vec![],
            push_constant_ranges: vec![PushConstantRange {
                // VK_SHADER_STAGE_VERTEX_BIT = 0x01
                stage_flags: 0x01,
                offset: 0,
                size: 128, // 4x4 f32 matrix (64 B) + spare uniform data
            }],
            debug_label: Some("scene-pll".into()),
        };
        let pll = self.device.create_pipeline_layout(&pll_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0201",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_pipeline_layout: {e:?}"),
            )]
        })?;

        self.create_scene_shader_modules()?;

        // Material descriptor infrastructure (set=2: UBO + texture).
        self.device
            .create_material_descriptor_infra()
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0210",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_material_descriptor_infra: {e:?}"),
                )]
            })?;

        // Shadow-mapping resources.
        // Ensure the device has created shadow resources (idempotent).
        self.device.ensure_shadow().map_err(|e| {
            vec![Diagnostic::new(
                "RV0211",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("ensure_shadow: {e:?}"),
            )]
        })?;

        // Environment cubemap (IBL, set=1 binding=1).
        self.device.create_env_cubemap().map_err(|e| {
            vec![Diagnostic::new(
                "RV0212",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_env_cubemap: {e:?}"),
            )]
        })?;

        // Light SSBO (set=1 binding=2).
        self.device
            .create_clustered_lighting_buffers()
            .map_err(|e| {
                vec![Diagnostic::new(
                    "RV0222",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("create_clustered_lighting_buffers: {e:?}"),
                )]
            })?;

        self.rp = Some(rp);
        self.pll = Some(pll);

        // Framebuffers (per swapchain image, color + depth).
        self.framebuffers = self.device.create_scene_framebuffers(rp).map_err(|e| {
            vec![Diagnostic::new(
                "RV0213",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_scene_framebuffers: {e:?}"),
            )]
        })?;

        // UI overlay pipeline.
        self.initialized = true;
        Ok(())
    }

    pub(super) fn ensure_scene_framebuffers(&mut self) -> Result<(), Vec<Diagnostic>> {
        if !self.framebuffers.is_empty() {
            return Ok(());
        }
        let render_pass = self.rp.ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0232",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "scene render pass is unavailable while rebuilding framebuffers",
            )]
        })?;
        self.framebuffers =
            self.device
                .create_scene_framebuffers(render_pass)
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0234",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("create_scene_framebuffers: {error:?}"),
                    )]
                })?;
        Ok(())
    }

    pub(super) fn material_binding_for_drawable(
        &self,
        input: &RenderFrameInput,
        material_id: &AssetId,
    ) -> Result<MaterialBinding, Vec<Diagnostic>> {
        input
            .materials
            .iter()
            .find(|material| material.material_id == *material_id)
            .cloned()
            .or_else(|| {
                self.uploaded_materials
                    .get(&material_id.id)
                    .map(|state| state.binding.clone())
            })
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0232",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("material '{}' was not uploaded", material_id.id),
                )]
            })
    }

    pub(super) fn prepare_frame_cache_capacity(
        &mut self,
        input: &RenderFrameInput,
    ) -> Result<(), Vec<Diagnostic>> {
        let required_materials: BTreeSet<String> = input
            .drawables
            .iter()
            .map(|item| item.material.id.clone())
            .chain(
                input
                    .skinned_items
                    .iter()
                    .map(|item| item.material.id.clone()),
            )
            .collect();
        if required_materials.len() > MAX_MATERIALS {
            return Err(vec![Diagnostic::new(
                "RV0271",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "frame needs {} materials, backend capacity is {MAX_MATERIALS}",
                    required_materials.len()
                ),
            )]);
        }
        let missing_materials = required_materials
            .iter()
            .filter(|id| !self.material_cache.contains_key(*id))
            .count();
        while self.material_cache.len() + missing_materials > MAX_MATERIALS {
            let candidate = self
                .material_cache_order
                .iter()
                .find(|id| !required_materials.contains(*id))
                .cloned()
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0272",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "material cache cannot reserve the current frame's working set",
                    )]
                })?;
            self.evict_material_by_id(&candidate)?;
        }

        let required_skeletons: BTreeSet<String> = input
            .skinned_items
            .iter()
            .map(|item| item.skeleton.id.clone())
            .collect();
        let required_skinned_sets: BTreeSet<String> = input
            .skinned_items
            .iter()
            .map(|item| format!("{}:{}", item.material.id, item.skeleton.id))
            .collect();
        if required_skeletons.len() > MAX_BONE_PALETTES
            || required_skinned_sets.len() > MAX_BONE_PALETTES
        {
            return Err(vec![Diagnostic::new(
                "RV0273",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "frame exceeds skinned capacity: {} skeletons, {} material/skeleton pairs, limit {MAX_BONE_PALETTES}",
                    required_skeletons.len(),
                    required_skinned_sets.len()
                ),
            )]);
        }
        let missing_skeletons = required_skeletons
            .iter()
            .filter(|id| !self.bone_palette_buffers.contains_key(*id))
            .count();
        while self.bone_palette_buffers.len() + missing_skeletons > MAX_BONE_PALETTES {
            let candidate = self
                .bone_palette_buffers_order
                .iter()
                .find(|id| !required_skeletons.contains(*id))
                .cloned()
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0274",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "bone cache cannot reserve the current frame's working set",
                    )]
                })?;
            self.evict_skeleton_by_id(&candidate)?;
        }
        let missing_skinned_sets = required_skinned_sets
            .iter()
            .filter(|key| !self.skinned_desc_cache.contains_key(*key))
            .count();
        while self.skinned_desc_cache.len() + missing_skinned_sets > MAX_BONE_PALETTES {
            let candidate = self
                .skinned_desc_cache_order
                .iter()
                .find(|key| !required_skinned_sets.contains(*key))
                .cloned()
                .ok_or_else(|| {
                    vec![Diagnostic::new(
                        "RV0275",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        "skinned descriptor cache cannot reserve the current frame's working set",
                    )]
                })?;
            self.evict_skinned_descriptor_by_key(&candidate)?;
        }
        Ok(())
    }
}
