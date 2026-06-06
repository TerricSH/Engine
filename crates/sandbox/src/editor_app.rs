use std::sync::Arc;

use engine_core::{create_vulkan_backend_renderer, EngineConfig, EngineRuntime};
use engine_editor::{
    EditorScene, EditorUi, HierarchyPanel, InspectorPanel, SceneViewPanel,
};
use engine_scene::sample_scene;
use platform::winit::window::Window;
use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

pub struct EditorApp {
    runtime: Option<EngineRuntime>,
    editor_scene: Option<EditorScene>,
    hierarchy: HierarchyPanel,
    scene_view: SceneViewPanel,
    inspector: InspectorPanel,
    ui: EditorUi,
    frame: u64,
    mouse_x: f64,
    mouse_y: f64,
    window_w: f32,
    window_h: f32,
}

impl EditorApp {
    pub fn new() -> Self {
        Self {
            runtime: None,
            editor_scene: None,
            hierarchy: HierarchyPanel::new("Hierarchy"),
            scene_view: SceneViewPanel::new("Scene View"),
            inspector: InspectorPanel::new("Inspector"),
            ui: EditorUi::new(),
            frame: 0,
            mouse_x: 0.0,
            mouse_y: 0.0,
            window_w: 1600.0,
            window_h: 900.0,
        }
    }

    fn init_scene(&mut self) {
        let scene = sample_scene();
        if let Some(ref mut runtime) = self.runtime {
            runtime.load_scene_to_world(scene.clone());
        }
        self.editor_scene = Some(EditorScene::new(scene));
        tracing::info!("editor: sample scene loaded");
    }

    fn render_editor_frame(&mut self) {
        let Some(ref mut editor_scene) = self.editor_scene else { return };
        let Some(ref mut runtime) = self.runtime else { return };

        // ── 1. Begin UI frame ──────────────────────────────────────
        self.ui.set_pointer(self.mouse_x as f32, self.mouse_y as f32);
        self.ui.begin_frame();

        // ── 2. Layout and render panels ────────────────────────────
        let gap = 4.0;
        let left_w = 220.0;
        let right_w = 280.0;
        let center_w = (self.window_w - left_w - right_w - gap * 4.0).max(100.0);

        // ── Hierarchy (left) ───────────────────────────────────────
        self.ui.set_panel_rect(gap, 4.0, left_w);
        self.ui.separator();
        let hierarchy_commands = self.hierarchy.ui(&mut self.ui, &editor_scene.scene);
        for cmd in hierarchy_commands {
            let _ = editor_scene.execute(cmd);
        }
        if let Some(sel) = self.hierarchy.selected().cloned() {
            editor_scene.selected_entity = Some(sel);
        }

        // ── Scene View (center) ────────────────────────────────────
        self.ui.set_panel_rect(left_w + gap * 2.0, 4.0, center_w);
        self.scene_view.ui_with_scene(&mut self.ui, &editor_scene.scene);

        // ── Inspector (right) ──────────────────────────────────────
        let inspector_left = left_w + center_w + gap * 3.0;
        self.ui.set_panel_rect(inspector_left + 4.0, 4.0, right_w);
        let inspector_commands = self.inspector.ui(
            &mut self.ui,
            &editor_scene.scene,
            editor_scene.selected_entity.as_ref(),
        );
        for cmd in inspector_commands {
            let _ = editor_scene.execute(cmd);
        }

        // ── 3. End UI frame ────────────────────────────────────────
        let _canvas = self.ui.end_frame();

        // ── 4. Render 3D scene ─────────────────────────────────────
        match runtime.render_frame(self.frame) {
            Ok(stats) => tracing::debug!(
                frame = self.frame,
                draw_calls = stats.draw_calls,
                "editor frame"
            ),
            Err(diags) => {
                for d in &diags {
                    tracing::warn!(code = d.code, msg = d.message, "editor render");
                }
            }
        }

        self.frame += 1;
    }
}

impl WindowApp for EditorApp {
    fn on_create(&mut self, window: Arc<Window>) {
        let size = window.inner_size();
        self.window_w = size.width as f32;
        self.window_h = size.height as f32;

        let display_handle = match window.display_handle() {
            Ok(h) => h.as_raw(),
            Err(e) => {
                tracing::error!("display handle: {e}");
                return;
            }
        };
        let window_handle = match window.window_handle() {
            Ok(h) => h.as_raw(),
            Err(e) => {
                tracing::error!("window handle: {e}");
                return;
            }
        };

        match create_vulkan_backend_renderer(
            display_handle,
            window_handle,
            size.width.max(1),
            size.height.max(1),
            std::env::var("ENGINE_VK_VALIDATION").is_ok(),
            None,
        ) {
            Ok(backend) => {
                let mut runtime = EngineRuntime::new(EngineConfig {
                    application_name: "editor".to_string(),
                });
                runtime.renderer_mut().set_backend(backend);
                self.runtime = Some(runtime);
                tracing::info!("editor: Vulkan backend initialized");
            }
            Err(e) => {
                tracing::error!("Vulkan backend creation failed: {e}");
                return;
            }
        }

        self.init_scene();
        tracing::info!("editor: fully initialized");
    }

    fn on_event(&mut self, window: &Window, event: PlatformEvent) -> EventFlow {
        match event {
            PlatformEvent::MouseMoved { x, y } => {
                self.mouse_x = x;
                self.mouse_y = y;
            }
            PlatformEvent::MousePressed { button, .. } => {
                if button == platform::MouseButton::Left {
                    self.ui.set_mouse_pressed();
                }
            }
            PlatformEvent::MouseReleased { button, .. } => {
                if button == platform::MouseButton::Left {
                    self.ui.set_mouse_released();
                }
            }
            PlatformEvent::Resized { width, height } => {
                self.window_w = width as f32;
                self.window_h = height as f32;
                if let Some(ref mut runtime) = self.runtime {
                    let _ = runtime.renderer_mut().resize(width, height);
                }
            }
            PlatformEvent::Redraw => {
                self.render_editor_frame();
                window.request_redraw();
            }
            PlatformEvent::CloseRequested => return EventFlow::Exit,
            _ => {}
        }
        EventFlow::Continue
    }
}

pub fn run_editor() {
    let app = EditorApp::new();
    if let Err(e) = platform::run(
        WindowDescriptor {
            title: "Engine Editor".to_string(),
            width: 1600,
            height: 900,
        },
        app,
    ) {
        tracing::error!("editor: {e}");
    }
}
