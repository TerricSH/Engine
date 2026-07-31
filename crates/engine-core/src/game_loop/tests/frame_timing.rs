#[cfg(test)]
mod frame_timing_tests {
    use super::*;

    /// Minimal no-op backend so a GameLoop can drive full frames in-process.
    struct NoopBackend;

    impl engine_renderer::BackendRenderer for NoopBackend {
        fn begin_frame(
            &mut self,
            _input: &engine_renderer::RenderFrameInput,
        ) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn apply_pass_barriers(
            &mut self,
            _input: &engine_renderer::RenderFrameInput,
            _pass: &engine_renderer::render_graph2::PassNode,
            _barriers: &[engine_renderer::render_graph2::CompiledBarrier],
        ) -> Result<(), Vec<Diagnostic>> {
            Ok(())
        }

        fn execute_pass(
            &mut self,
            _input: &engine_renderer::RenderFrameInput,
            _pass: &engine_renderer::render_graph2::PassNode,
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

        fn upload_mesh(
            &mut self,
            _upload: engine_renderer::MeshUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            Ok(engine_renderer::UploadReceipt::new(1))
        }

        fn upload_texture(
            &mut self,
            _upload: engine_renderer::TextureUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            Ok(engine_renderer::UploadReceipt::new(1))
        }

        fn upload_material(
            &mut self,
            _upload: engine_renderer::MaterialUpload,
        ) -> Result<engine_renderer::UploadReceipt, Vec<Diagnostic>> {
            Ok(engine_renderer::UploadReceipt::new(1))
        }
    }

    #[test]
    fn game_loop_attributes_update_and_render_stages_to_one_frame() {
        let mut game_loop = GameLoop::new(EngineConfig::default());
        game_loop
            .runtime
            .set_renderer_backend(Box::new(NoopBackend));
        game_loop
            .load_scene(engine_scene::sample_scene())
            .expect("sample scene should load");

        for frame in 0..5 {
            game_loop.update(1.0 / 60.0);
            game_loop.render(frame).expect("frame should render");
        }

        let timings = game_loop
            .runtime
            .last_frame_timings()
            .expect("frame timings after five frames");
        for stage in [
            "update",
            "extraction",
            "sync_render_assets",
            "render_submit",
        ] {
            assert!(
                timings
                    .passes
                    .iter()
                    .any(|pass| pass.name == stage && pass.cpu_ms.is_some()),
                "missing CPU stage '{stage}' in {timings:?}"
            );
        }
        let stage_sum: f32 = timings.passes.iter().filter_map(|pass| pass.cpu_ms).sum();
        assert!(
            (stage_sum - timings.total_cpu_ms).abs() < f32::EPSILON,
            "CPU stage attribution must sum to the frame total"
        );

        let summary = game_loop.frame_timing_summary();
        assert_eq!(summary.window_frames, 5);
        let update = summary
            .passes
            .iter()
            .find(|pass| pass.name == "update")
            .expect("update stats");
        assert_eq!(update.cpu.expect("cpu aggregate").samples, 5);
        assert_eq!(
            summary.gpu_status,
            engine_renderer::GpuTimingStatus::Unavailable,
            "a no-op backend reports GPU timing as unavailable"
        );
    }
}
