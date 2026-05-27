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

#[cfg(test)]
mod tests {
    use super::*;

    // ── EngineConfig tests ───────────────────────────────────────────────

    #[test]
    fn engine_config_defaults() {
        let config = EngineConfig::default();
        assert_eq!(config.application_name, "engine");
    }

    #[test]
    fn engine_config_debug() {
        let config = EngineConfig::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("EngineConfig"));
    }

    #[test]
    fn engine_config_partial_eq() {
        let a = EngineConfig::default();
        let b = EngineConfig::default();
        let c = EngineConfig {
            application_name: "custom".to_string(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn engine_config_clone() {
        let config = EngineConfig::default();
        let cloned = config.clone();
        assert_eq!(config, cloned);
    }

    // ── EngineRuntime tests ──────────────────────────────────────────────

    #[test]
    fn engine_runtime_creation() {
        let config = EngineConfig::default();
        let runtime = EngineRuntime::new(config.clone());
        assert_eq!(*runtime.config(), config);
    }

    #[test]
    fn engine_runtime_config_accessor() {
        let config = EngineConfig::default();
        let runtime = EngineRuntime::new(config);
        let retrieved = runtime.config();
        assert_eq!(retrieved.application_name, "engine");
    }

    #[test]
    fn engine_runtime_render_frame_without_scene_fails() {
        let config = EngineConfig::default();
        let mut runtime = EngineRuntime::new(config);
        let result = runtime.render_frame(0);
        assert!(result.is_err());
    }
}
