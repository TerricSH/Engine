use super::*;

impl GameLoop {
    /// Advance the simulation by `dt` seconds.
    ///
    /// Handles physics stepping and ECS ↔ physics sync when
    /// `subsystem-physics` is enabled. Script ticking runs when the
    /// `subsystem-scripting-csharp` feature is active.
    ///
    /// Typical per-frame orchestration:
    /// 1. Resolve input events against `input_map`
    /// 2. Call `update(dt)` for physics + character + scripts
    /// 3. Call `render(frame_idx)` for extraction + draw
    pub fn update(&mut self, dt: f32) {
        // Frame-time attribution (ENG-04): the whole simulation step is the
        // `update` stage; script ticking nests inside it as `script_tick`.
        self.runtime.frame_timing_begin_stage("update");
        self.update_inner(dt);
        self.runtime.frame_timing_end_stage("update");
    }

    pub(super) fn update_inner(&mut self, dt: f32) {
        #[cfg(feature = "subsystem-network")]
        {
            self.network_time_seconds += f64::from(dt.max(0.0));
            if let Err(error) = self.network.tick(self.network_time_seconds) {
                tracing::warn!(%error, "network session tick failed");
            }
            if let Err(error) = self.network.flush_replication(256) {
                tracing::warn!(%error, "network replication flush failed");
            }
        }
        #[cfg(all(feature = "subsystem-network", feature = "subsystem-scripting-csharp"))]
        let script_rpc_events = {
            let (results, unhandled) = self.network.rpc.dispatch_registered(256);
            for (_, result) in results {
                if let Err(error) = result {
                    tracing::warn!(%error, "network RPC handler failed");
                }
            }
            unhandled
        };
        #[cfg(all(
            feature = "subsystem-network",
            not(feature = "subsystem-scripting-csharp")
        ))]
        for (_, result) in self.network.rpc.dispatch(256) {
            if let Err(error) = result {
                tracing::warn!(%error, "network RPC handler failed");
            }
        }
        #[cfg(feature = "subsystem-terrain")]
        self.tick_terrain(None);
        #[cfg(feature = "subsystem-terrain")]
        self.tick_planet_scene_transitions(f64::from(dt));
        #[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
        crate::ragdoll_runtime::reconcile_before_physics(self);
        // Tick physics (ECS → physics → ECS sync).
        #[cfg(feature = "subsystem-physics")]
        {
            self.physics_events.clear();
            if let Some(ref mut physics) = self.physics {
                self.runtime.with_world_mut(|world| {
                    physics.step(dt, world);
                });
                self.physics_events = physics.drain_events();
            }
        }

        #[cfg(feature = "subsystem-gameplay")]
        let (character_direction, character_jump) = self.resolved_character_input();
        #[cfg(not(feature = "subsystem-gameplay"))]
        let (character_direction, character_jump) = (Vec3::ZERO, false);
        #[cfg(feature = "subsystem-navigation")]
        self.queue_runtime_navigation(dt);
        self.update_character(character_direction, character_jump, dt);
        self.update_additional_characters(dt);

        #[cfg(feature = "subsystem-scripting-csharp")]
        let script_ui_events = {
            #[cfg(feature = "subsystem-ui")]
            {
                std::mem::take(&mut self.runtime_ui_events)
                    .into_iter()
                    .map(|event| engine_script::GameplayUiEvent {
                        canvas_id: event.canvas_id,
                        element_id: event.element_id,
                        callback_id: event.callback_id,
                        value: event.value.map(|value| match value {
                            RuntimeUiValue::Bool(value) => {
                                engine_script::GameplayUiValue::Bool(value)
                            }
                            RuntimeUiValue::Float(value) => {
                                engine_script::GameplayUiValue::Float(value)
                            }
                        }),
                    })
                    .collect::<Vec<_>>()
            }
            #[cfg(not(feature = "subsystem-ui"))]
            {
                Vec::<engine_script::GameplayUiEvent>::new()
            }
        };

        #[cfg(feature = "subsystem-scripting-csharp")]
        self.refresh_script_view_context();
        #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-network"))]
        self.refresh_script_network_context(script_rpc_events);
        #[cfg(all(
            feature = "subsystem-scripting-csharp",
            not(feature = "subsystem-network")
        ))]
        self.refresh_script_network_context();
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.refresh_script_xr_context();

        // Build each optional input independently so scripting no longer
        // drags the gameplay, physics, animation, or UI subsystems with it.
        #[cfg(feature = "subsystem-scripting-csharp")]
        {
            self.runtime.frame_timing_begin_stage("script_tick");
            #[cfg(feature = "subsystem-gameplay")]
            let input_actions = self.resolved_script_input_actions();
            #[cfg(not(feature = "subsystem-gameplay"))]
            let input_actions = std::collections::BTreeMap::new();
            #[cfg(feature = "subsystem-gameplay")]
            let input_transitions = self.resolved_script_input_transitions(&input_actions);
            #[cfg(not(feature = "subsystem-gameplay"))]
            let input_transitions = engine_script::GameplayInputTransitions::default();
            #[cfg(feature = "subsystem-physics")]
            let physics_events = self.resolved_script_physics_events();
            #[cfg(not(feature = "subsystem-physics"))]
            let physics_events = std::collections::BTreeMap::new();
            #[cfg(feature = "subsystem-physics")]
            let physics_query_results = std::mem::take(&mut self.script_physics_query_results);
            #[cfg(not(feature = "subsystem-physics"))]
            let physics_query_results = std::collections::BTreeMap::new();

            self.runtime
                .tick_scripts_with_frame_input_ui_and_physics_queries(
                    dt,
                    &input_actions,
                    &input_transitions,
                    &physics_events,
                    &script_ui_events,
                    &physics_query_results,
                );
            self.process_script_network_commands();

            #[cfg(feature = "subsystem-gameplay")]
            {
                self.previous_script_input_actions = input_actions;
            }
            #[cfg(feature = "subsystem-physics")]
            {
                self.execute_script_physics_queries();
                self.queue_script_physics_mutations();
                self.process_script_damage_requests();
            }
            #[cfg(not(feature = "subsystem-physics"))]
            {
                let _ = self.runtime.take_pending_physics_queries();
                let _ = self.runtime.take_pending_physics_mutations();
                let _ = self.runtime.take_pending_damage_requests();
            }
            #[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
            self.process_script_ragdoll_requests();
            #[cfg(not(all(feature = "subsystem-animation", feature = "subsystem-physics")))]
            let _ = self.runtime.take_pending_ragdoll_requests();
            self.runtime.frame_timing_end_stage("script_tick");
        }
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.process_script_save_requests();
        #[cfg(all(feature = "subsystem-scripting-csharp", feature = "subsystem-terrain"))]
        self.process_script_terrain_brushes();
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.refresh_primary_character_from_world();
        #[cfg(feature = "subsystem-scripting-csharp")]
        self.finish_script_pointer_frame();

        #[cfg(all(feature = "subsystem-animation", feature = "subsystem-physics"))]
        {
            crate::ragdoll_runtime::reconcile_after_physics(self, dt);
        }

        #[cfg(feature = "subsystem-animation")]
        self.update_runtime_animation(dt);
        self.runtime
            .with_world_mut(|world| engine_vfx::update_vfx(world, dt));
        #[cfg(feature = "runtime-audio-output")]
        self.update_runtime_audio(dt);
    }

    /// Tick the optional terrain component at the frame boundary. Normal
    /// [`update`](Self::update) calls this with the active camera; editor and
    /// server hosts may supply an absolute/logical focus explicitly.
    #[cfg(feature = "subsystem-terrain")]
    pub fn tick_terrain(&mut self, focus_logical: Option<[f64; 3]>) {
        self.runtime.frame_timing_begin_stage("terrain_stream");
        let physics_changed = self.terrain.tick(&mut self.runtime, focus_logical);
        if physics_changed {
            self.resync_physics_from_world();
        }
        self.runtime.frame_timing_end_stage("terrain_stream");
    }

    /// Begin one predicted XR frame and acquire the native stereo targets.
    /// Rendering hosts use [`engine_xr::XrFrameState`] to replace their two
    /// camera views, render the returned images, then call
    /// [`Self::submit_xr_frame`]. Ordinary desktop frames return `Ok(None)`.
    #[cfg(feature = "subsystem-xr")]
    pub fn begin_xr_frame(
        &mut self,
    ) -> Result<Option<Vec<engine_xr::XrSwapchainImage>>, engine_xr::XrError> {
        self.xr.tick()
    }

    /// Release the acquired eye targets and submit their OpenXR projection
    /// layer. A frame cannot be begun again until this succeeds or reports an
    /// explicit lifecycle error.
    #[cfg(feature = "subsystem-xr")]
    pub fn submit_xr_frame(
        &mut self,
        images: &[engine_xr::XrSwapchainImage],
    ) -> Result<(), engine_xr::XrError> {
        self.xr.submit(images)
    }

    #[cfg(feature = "subsystem-xr")]
    pub fn xr_actions(&self) -> &engine_xr::XrActionSnapshot {
        self.xr.actions()
    }

    /// Snapshot used by the ENG-70 editor panel and headless diagnostics.
    #[cfg(feature = "subsystem-terrain")]
    pub fn terrain_debug_snapshot(&self) -> engine_terrain::TerrainDebugSnapshot {
        self.terrain.debug_snapshot()
    }

    /// Construction footprints resolved from persistent planet-surface
    /// anchors during the most recent terrain tick.
    #[cfg(feature = "subsystem-terrain")]
    pub fn terrain_surface_occupancy(&self) -> &engine_terrain::PlanetSurfaceOccupancy {
        self.terrain.surface_occupancy()
    }

    /// Regenerate resident and in-flight chunks without changing authored
    /// terrain parameters.
    #[cfg(feature = "subsystem-terrain")]
    pub fn terrain_force_regenerate(&mut self) {
        self.terrain.force_regenerate();
    }

    /// Rolling per-pass CPU/GPU frame timing statistics (ENG-04).
    ///
    /// CPU stages: `update` (whole simulation step, with `script_tick`
    /// nested), `extraction`, `sync_render_assets`, `render_submit`. GPU pass
    /// times appear only when the active backend supports timestamps.
    pub fn frame_timing_summary(&self) -> engine_renderer::FrameTimingSummary {
        self.runtime.frame_timing_summary()
    }

    /// Produce a single rendered frame.
    pub fn render(&mut self, frame_index: u64) -> Result<FrameStats, Vec<Diagnostic>> {
        #[cfg(feature = "subsystem-ui")]
        {
            let ui_batches = self.runtime_ui_batches();
            self.runtime.render_frame_with_ui(frame_index, ui_batches)
        }
        #[cfg(not(feature = "subsystem-ui"))]
        {
            self.runtime.render_frame(frame_index)
        }
    }

    /// Render the scene, retained game UI and engine-native overlays inside an
    /// embedded viewport. The desktop editor shell is composed by the OS and
    /// never enters this render path.
    pub fn render_embedded_viewport(
        &mut self,
        frame_index: u64,
        engine_overlay_batches: Vec<engine_renderer::UiBatch>,
        viewport: RenderViewportContext,
    ) -> Result<FrameStats, Vec<Diagnostic>> {
        #[cfg(feature = "subsystem-ui")]
        let ui_batches = {
            let surface_size = viewport.surface_size();
            let output = viewport.output_rect();
            let extent = [
                output.width() * surface_size[0] as f32,
                output.height() * surface_size[1] as f32,
            ];
            self.runtime_ui_viewport = extent;
            let mut scene_ui_batches = self.runtime_ui_batches();
            embed_scene_ui_batches(&mut scene_ui_batches, viewport);
            scene_ui_batches.extend(engine_overlay_batches);
            scene_ui_batches
        };
        #[cfg(not(feature = "subsystem-ui"))]
        let ui_batches = engine_overlay_batches;

        self.runtime
            .render_frame_with_ui_in_viewport(frame_index, ui_batches, viewport)
    }

    /// Validate that the runtime has a loaded scene ready for rendering.
    pub fn validate_ready(&self) -> Result<(), Vec<Diagnostic>> {
        if !self.runtime.has_world() {
            return Err(vec![Diagnostic::new(
                "GL0001",
                DiagnosticSeverity::Error,
                "game_loop",
                "no active World is loaded; call load_scene() or set_world() first",
            )]);
        }
        Ok(())
    }
}
