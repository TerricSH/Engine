#![forbid(unsafe_code)]

use engine_renderer::{FrameStats, Renderer};
use engine_scene::{extract_renderer_input, Scene};
use engine_serialize::{Diagnostic, DiagnosticSeverity};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    pub application_name: String,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            application_name: "engine".to_string(),
        }
    }
}

pub struct EngineRuntime {
    config: EngineConfig,
    renderer: Renderer,
    scene: Option<Scene>,
}

impl EngineRuntime {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            renderer: Renderer::new(),
            scene: None,
        }
    }
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }
    pub fn load_scene(&mut self, scene: Scene) {
        self.scene = Some(scene);
    }

    pub fn render_frame(&mut self, frame_index: u64) -> Result<FrameStats, Vec<Diagnostic>> {
        let scene = self.scene.as_ref().ok_or_else(|| {
            vec![Diagnostic::new(
                "SC0018",
                DiagnosticSeverity::Error,
                "engine-core",
                "no scene is loaded",
            )]
        })?;
        let input = extract_renderer_input(scene, frame_index)?;
        self.renderer.draw_scene(&input)
    }
}
