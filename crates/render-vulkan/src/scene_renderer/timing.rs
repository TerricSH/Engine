use super::*;

impl SceneRenderer {
    /// Evaluate device support once and configure the profiler.
    pub(super) fn configure_gpu_timestamps(&mut self) {
        if self.gpu_timing_configured {
            return;
        }
        self.gpu_timing_configured = true;
        if !self.gpu_timing_enabled {
            self.gpu_timestamps
                .configure(crate::timestamps::TimestampSupport::Disabled, 0);
            return;
        }
        let limits = &self.device.adapter.properties.limits;
        let support = crate::timestamps::evaluate_support(
            true,
            limits.timestamp_compute_and_graphics == vk::TRUE,
            limits.timestamp_period,
        );
        let slots = self.device.frame_sync.len().max(1);
        self.gpu_timestamps.configure(support, slots);
    }

    /// Read back the slot's previous frame (its fence was just waited inside
    /// the device `begin_frame`), then reset the pool and start recording.
    pub(super) fn gpu_timestamps_begin_frame(&mut self, frame_index: u64) {
        self.configure_gpu_timestamps();
        let fi = self.device.current_frame;
        let device = self.device.logical_device.device.clone();
        if let Some(query_count) = self.gpu_timestamps.readback_len(fi) {
            let ticks = self.timestamp_pools.read(&device, fi, query_count);
            self.gpu_timestamps.deliver_readback(fi, ticks.as_deref());
        }
        let Some(_) = self.gpu_timestamps.begin_recording(fi, frame_index) else {
            return;
        };
        if self.timestamp_pools.ensure_created(&device).is_err() {
            self.gpu_timestamps
                .degrade("timestamp query pool creation failed");
            return;
        }
        let cmd = self.device.frame_sync[fi].command_buffer;
        self.timestamp_pools.cmd_reset(&device, cmd, fi);
    }

    /// Record the start timestamp for `pass_name`; returns the query to write.
    pub(super) fn gpu_timestamp_pass_start(&mut self, pass_name: &str) -> Option<(u32, usize)> {
        let fi = self.device.current_frame;
        self.gpu_timestamps
            .stamp_start(pass_name)
            .map(|query| (query, fi))
    }

    /// Record the end timestamp paired with the most recent start.
    pub(super) fn gpu_timestamp_pass_end(&mut self) -> Option<(u32, usize)> {
        let fi = self.device.current_frame;
        self.gpu_timestamps.stamp_end().map(|query| (query, fi))
    }

    /// Close the frame's recording after submission and publish backend GPU
    /// timing into the frame statistics.
    pub(super) fn gpu_timestamps_end_frame(&mut self, stats: &mut FrameStats) {
        self.gpu_timestamps.finish_recording();
        stats.gpu_timing = self.gpu_timestamps.status();
        if let Some(batch) = self.gpu_timestamps.take_latest() {
            stats.gpu_pass_frame_index = Some(batch.frame_index);
            stats.gpu_pass_times = batch.passes;
        }
    }

    pub(super) fn recover_failed_device_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        self.device
            .abort_current_frame_recording()
            .map_err(|error| {
                vec![Diagnostic::new(
                    "RV0244",
                    DiagnosticSeverity::Error,
                    "scene_renderer",
                    format!("failed to recover Vulkan frame state: {error:?}"),
                )]
            })?;
        self.device.destroy_scene_framebuffers(&self.framebuffers);
        self.framebuffers.clear();
        self.device.resize(self.width, self.height);
        Ok(())
    }
}

// ============================================================================
// BackendRenderer implementation
// ============================================================================
