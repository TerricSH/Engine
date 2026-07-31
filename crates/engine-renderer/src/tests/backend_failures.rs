#[test]
fn masked_blended_and_double_sided_materials_enter_the_backend() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut renderer = Renderer::new_with_backend(Box::new(CountingBackend {
        upload_calls: Arc::clone(&calls),
    }));

    let mut masked = valid_material_upload();
    masked.material_id = AssetId::new("material.masked");
    masked.transparency = Transparency::Masked { cutoff: 0.42 };
    masked.double_sided = true;
    renderer.upload_material(masked).unwrap();

    let mut blended = valid_material_upload();
    blended.material_id = AssetId::new("material.blended");
    blended.transparency = Transparency::Blend;
    renderer.upload_material(blended).unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[derive(Clone, Copy)]
enum FailureStage {
    Barrier,
    Pass,
    End,
}

struct FailingFrameBackend {
    stage: FailureStage,
    abort_calls: Arc<AtomicUsize>,
    abort_fails: bool,
}

impl FailingFrameBackend {
    fn stage_error(&self) -> Vec<Diagnostic> {
        vec![Diagnostic::new(
            "TEST_FRAME_FAILURE",
            DiagnosticSeverity::Error,
            "renderer.test",
            "injected frame failure",
        )]
    }
}

impl BackendRenderer for FailingFrameBackend {
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &super::render_graph2::PassNode,
        _barriers: &[super::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        if matches!(self.stage, FailureStage::Barrier) {
            Err(self.stage_error())
        } else {
            Ok(())
        }
    }

    fn execute_pass(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &super::render_graph2::PassNode,
        _frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if matches!(self.stage, FailureStage::Pass) {
            Err(self.stage_error())
        } else {
            Ok(())
        }
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        if matches!(self.stage, FailureStage::End) {
            Err(self.stage_error())
        } else {
            Ok(())
        }
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        self.abort_calls.fetch_add(1, Ordering::SeqCst);
        if self.abort_fails {
            Err(vec![Diagnostic::new(
                "TEST_ABORT_FAILURE",
                DiagnosticSeverity::Error,
                "renderer.test",
                "injected abort failure",
            )])
        } else {
            Ok(())
        }
    }
}

#[test]
fn barrier_pass_and_end_failures_abort_the_frame_once() {
    for stage in [FailureStage::Barrier, FailureStage::Pass, FailureStage::End] {
        let abort_calls = Arc::new(AtomicUsize::new(0));
        let mut renderer = Renderer::new_with_backend(Box::new(FailingFrameBackend {
            stage,
            abort_calls: Arc::clone(&abort_calls),
            abort_fails: false,
        }));
        let diagnostics = renderer.draw_scene(&valid_frame()).unwrap_err();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "TEST_FRAME_FAILURE"));
        assert_eq!(abort_calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn abort_error_is_appended_without_losing_the_original_failure() {
    let abort_calls = Arc::new(AtomicUsize::new(0));
    let mut renderer = Renderer::new_with_backend(Box::new(FailingFrameBackend {
        stage: FailureStage::Pass,
        abort_calls: Arc::clone(&abort_calls),
        abort_fails: true,
    }));
    let diagnostics = renderer.draw_scene(&valid_frame()).unwrap_err();
    assert_eq!(diagnostics[0].code, "TEST_FRAME_FAILURE");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "TEST_ABORT_FAILURE"));
    assert_eq!(abort_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn default_abort_reports_that_the_backend_cannot_reset_recording_state() {
    let mut backend = UnsupportedBackend;
    assert!(backend
        .abort_frame()
        .unwrap_err()
        .iter()
        .any(|diagnostic| diagnostic.code == DIAG_ABORT_UNSUPPORTED));
}
