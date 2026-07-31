use super::*;

impl SceneRenderer {
    /// Common initialisation + swapchain creation + device begin-frame.
    ///
    /// Called by both [`render_frame`] and [`begin_frame`].
    ///
    /// `msaa_samples` is the MSAA sample count from `RenderOptions`, capped
    /// to the device's maximum. It is set on the device before swapchain/HDR
    /// resource creation.
    pub(super) fn begin_frame_impl(
        &mut self,
        input: &RenderFrameInput,
        msaa_samples: vk::SampleCountFlags,
    ) -> Result<(SwapchainHandle, u32, Box<dyn CommandEncoder>), Vec<Diagnostic>> {
        let (view, projection) = input
            .views
            .first()
            .map(|view| {
                (
                    Mat4::from_cols_array(&view.view_matrix),
                    Mat4::from_cols_array(&view.projection_matrix),
                )
            })
            .unwrap_or((Mat4::IDENTITY, Mat4::IDENTITY));
        let matrices_are_finite = view
            .to_cols_array()
            .into_iter()
            .chain(projection.to_cols_array())
            .all(f32::is_finite);
        if !matrices_are_finite || view.determinant().abs() <= f32::EPSILON {
            return Err(vec![Diagnostic::new(
                "RV0210",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "view and projection matrices must be finite and the view matrix invertible",
            )]);
        }
        self.validate_uploaded_meshes(input)?;
        self.prepare_frame_cache_capacity(input)?;
        if let Some(texture_id) = first_missing_ui_texture(&input.ui_batches, |id| {
            self.device.textures.contains_key(id)
        }) {
            return Err(vec![Diagnostic::new(
                "RV0308",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("UI batch references texture '{texture_id}' before a successful upload"),
            )]);
        }
        prepare_ui_overlay(&input.ui_batches, self.width, self.height).map_err(|message| {
            vec![Diagnostic::new(
                "RV0298",
                DiagnosticSeverity::Error,
                "scene_renderer",
                message,
            )]
        })?;

        // Apply the requested MSAA sample count to the device before any
        // resource creation takes place (ensure_sc, ensure_hdr_resources).
        let max_samples = self
            .device
            .cached_adapter_info
            .capabilities
            .limits
            .max_sample_count;
        let requested_samples = input.render_options.msaa_samples;
        if requested_samples > max_samples {
            return Err(vec![Diagnostic::new(
                "RV0317",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!(
                    "requested {requested_samples}x MSAA exceeds the Vulkan adapter limit of {max_samples}x"
                ),
            )]);
        }
        if self.device.hdr_msaa_samples != msaa_samples {
            if self.device.swapchain.is_some() {
                self.device.wait_idle_checked().map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0317",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("wait before rebuilding Vulkan MSAA resources: {error}"),
                    )]
                })?;
                self.device.destroy_hdr_resources();
            }
            self.device.hdr_msaa_samples = msaa_samples;
        }
        if !self.initialized {
            // Swapchain setup creates the HDR forward pipeline, so the scene
            // shaders must be registered before `create_swapchain`.
            self.configure_scene_shaders();
        }

        if self.device.swapchain_recreate_pending {
            self.device.wait_idle_checked().map_err(|error| {
                vec![Diagnostic::new(
                    "RV0314",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("wait for suboptimal swapchain replacement: {error}"),
                )]
            })?;
            self.device.destroy_scene_framebuffers(&self.framebuffers);
            self.framebuffers.clear();
            self.device.destroy_swapchain_resources();
        }

        let sc_desc = SwapchainDescriptor {
            surface: render_core::SurfaceHandle::new(0, 1),
            width: self.width,
            height: self.height,
            vsync: false,
            debug_label: None,
        };
        let sc_h = self.device.create_swapchain(&sc_desc).map_err(|e| {
            vec![Diagnostic::new(
                "RV0207",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create_swapchain: {e:?}"),
            )]
        })?;
        // First creation initializes HDR through the swapchain path. If only
        // the sample count changed, the swapchain stayed live and the HDR
        // resources were torn down above, so recreate them explicitly.
        self.device.ensure_hdr_resources().map_err(|error| {
            vec![Diagnostic::new(
                "RV0317",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("create Vulkan MSAA resolve resources: {error}"),
            )]
        })?;

        if !input.ui_batches.is_empty() {
            self.device.ensure_ui_overlay_resources().map_err(|error| {
                vec![Diagnostic::new(
                    "RV0309",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("initialize UI overlay resources: {error}"),
                )]
            })?;
        }

        // Swapchain creation also establishes the per-frame, shadow, and
        // material descriptor layouts. Scene pipelines and framebuffers must
        // not be created until those Vulkan objects are valid.
        self.init_once()?;
        self.ensure_scene_framebuffers()?;

        let camera_world = view.inverse().w_axis;
        let camera_position_vec = camera_world.truncate();
        let requested_environment =
            select_environment_map(&input.render_options.environment, camera_position_vec)
                .map(|asset| asset.id.clone());
        if requested_environment != self.active_environment_id {
            self.device.wait_idle_checked().map_err(|error| {
                vec![Diagnostic::new(
                    "RV0321",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("wait before environment-map switch: {error}"),
                )]
            })?;
            if let Some(environment_id) = &requested_environment {
                let upload = self
                    .environment_uploads
                    .get(environment_id)
                    .cloned()
                    .ok_or_else(|| {
                        vec![Diagnostic::new(
                            "RV0322",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!(
                                "frame references environment map '{environment_id}' before upload"
                            ),
                        )]
                    })?;
                self.device
                    .upload_environment_map(&upload)
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0323",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("activate environment map '{environment_id}': {error}"),
                        )]
                    })?;
            } else {
                self.device
                    .restore_procedural_environment()
                    .map_err(|error| {
                        vec![Diagnostic::new(
                            "RV0323",
                            DiagnosticSeverity::Error,
                            "scene_renderer",
                            format!("restore procedural environment: {error}"),
                        )]
                    })?;
            }
            self.active_environment_id = requested_environment;
        }

        let (ii, encoder) = self.device.begin_frame(sc_h).map_err(|e| {
            vec![Diagnostic::new(
                "RV0208",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("begin_frame: {e:?}"),
            )]
        })?;

        self.cur_fb_index = ii;

        // `begin_frame` waits for the current frame fence. Only now is it safe
        // to update the persistently mapped UBO owned by that frame slot.
        self.device.write_default_ubo();
        let view_projection = (projection * view).to_cols_array();
        let mut view_projection_bytes = Vec::with_capacity(64);
        for value in view_projection {
            view_projection_bytes.extend_from_slice(&value.to_ne_bytes());
        }
        self.device.write_ubo_current(&view_projection_bytes, 64);

        let camera_position = [camera_world.x, camera_world.y, camera_world.z, 1.0f32];
        let mut camera_bytes = Vec::with_capacity(16);
        for value in camera_position {
            camera_bytes.extend_from_slice(&value.to_ne_bytes());
        }
        self.device.write_ubo_current(&camera_bytes, 160);
        let environment = &input.render_options.environment;
        let environment_parameters = [
            environment.intensity,
            environment.rotation_radians.sin(),
            environment.rotation_radians.cos(),
            0.0f32,
        ];
        let mut environment_bytes = Vec::with_capacity(16);
        for value in environment_parameters {
            environment_bytes.extend_from_slice(&value.to_ne_bytes());
        }
        self.device.write_ubo_current(&environment_bytes, 384);

        Ok((sc_h, ii, encoder))
    }

    // ------------------------------------------------------------------
    // Extracted pass-execution helpers (called by registered passes)
    // ------------------------------------------------------------------

    pub(super) fn upload_particle_instance_stream(
        &mut self,
        prepared: &PreparedParticleInstances,
    ) -> Result<Option<vk::Buffer>, Vec<Diagnostic>> {
        if prepared.instance_bytes.is_empty() {
            return Ok(None);
        }
        let frame_index = self.device.current_frame;
        let required_bytes = u64::try_from(prepared.instance_bytes.len()).map_err(|_| {
            vec![Diagnostic::new(
                "RV0332",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "particle instance data exceeds the Vulkan buffer size contract",
            )]
        })?;
        if self.particle_instance_capacities[frame_index] < required_bytes {
            if let Some(old) = self.particle_instance_vbs[frame_index].take() {
                self.device.destroy_buffer(old);
            }
            let capacity = required_bytes.next_power_of_two();
            let buffer = self
                .device
                .create_buffer(&BufferDescriptor {
                    size_bytes: capacity,
                    usage_flags: render_core::BufferUsage::VERTEX,
                    memory_hint: MemoryHint::CpuToGpu,
                    debug_label: Some(format!("vfx-instances-{frame_index}")),
                })
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0333",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("create VFX instance buffer: {error:?}"),
                    )]
                })?;
            self.particle_instance_vbs[frame_index] = Some(buffer);
            self.particle_instance_capacities[frame_index] = capacity;
        }
        let handle = self.particle_instance_vbs[frame_index].ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0334",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "VFX instance buffer was not retained after creation",
            )]
        })?;
        self.device
            .write_buffer(handle, &prepared.instance_bytes, 0)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0335",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("write VFX instance buffer: {error:?}"),
                )]
            })?;
        self.device
            .buffers
            .get(handle.index, handle.generation)
            .map(|entry| Some(entry.buffer))
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0336",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    "VFX instance buffer handle became invalid before recording",
                )]
            })
    }

    pub(super) fn upload_static_instance_stream(
        &mut self,
        prepared: &PreparedStaticInstances,
    ) -> Result<Option<vk::Buffer>, Vec<Diagnostic>> {
        if prepared.instance_bytes.is_empty() {
            return Ok(None);
        }
        let frame_index = self.device.current_frame;
        let required_bytes = u64::try_from(prepared.instance_bytes.len()).map_err(|_| {
            vec![Diagnostic::new(
                "RV0343",
                DiagnosticSeverity::Error,
                "scene_renderer",
                "static instance data exceeds the Vulkan buffer size contract",
            )]
        })?;
        if self.static_instance_capacities[frame_index] < required_bytes {
            if let Some(old) = self.static_instance_vbs[frame_index].take() {
                self.device.destroy_buffer(old);
            }
            let capacity = required_bytes.next_power_of_two();
            let buffer = self
                .device
                .create_buffer(&BufferDescriptor {
                    size_bytes: capacity,
                    usage_flags: render_core::BufferUsage::VERTEX,
                    memory_hint: MemoryHint::CpuToGpu,
                    debug_label: Some(format!("static-instances-{frame_index}")),
                })
                .map_err(|error| {
                    vec![Diagnostic::new(
                        "RV0344",
                        DiagnosticSeverity::Error,
                        "scene_renderer",
                        format!("create static instance buffer: {error:?}"),
                    )]
                })?;
            self.static_instance_vbs[frame_index] = Some(buffer);
            self.static_instance_capacities[frame_index] = capacity;
        }
        let handle = self.static_instance_vbs[frame_index].ok_or_else(|| {
            vec![Diagnostic::new(
                "RV0345",
                DiagnosticSeverity::Fatal,
                "scene_renderer",
                "static instance buffer was not retained after creation",
            )]
        })?;
        self.device
            .write_buffer(handle, &prepared.instance_bytes, 0)
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0346",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("write static instance buffer: {error:?}"),
                )]
            })?;
        self.device
            .buffers
            .get(handle.index, handle.generation)
            .map(|entry| Some(entry.buffer))
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "RV0347",
                    DiagnosticSeverity::Fatal,
                    "scene_renderer",
                    "static instance buffer handle became invalid before recording",
                )]
            })
    }
}
