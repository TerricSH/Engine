#![forbid(unsafe_code)]

use engine_core::{EngineConfig, EngineRuntime};

fn main() {
    tracing_subscriber::fmt::init();
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "workspace".to_string());
    match command.as_str() {
        "workspace" => tracing::info!("engine workspace initialized"),
        "gate04-scene" => run_gate04_scene(),
        "triangle" => run_triangle(),
        "textured-object" => run_textured_object(),
        "resize-smoke" => run_resize_smoke(),
        other => {
            tracing::error!(command = other, "unknown sandbox command");
            std::process::exit(2);
        }
    }
}

fn run_gate04_scene() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    runtime.load_scene(engine_scene::sample_scene());
    match runtime.render_frame(0) {
        Ok(stats) => tracing::info!(
            draw_calls = stats.draw_calls,
            "gate04 scene rendered through contract path"
        ),
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                tracing::error!(code = diagnostic.code, message = diagnostic.message);
            }
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "backend-vulkan")]
fn run_triangle() {
    run_vulkan_scene(
        "Engine Sandbox - Triangle",
        render_vulkan::VulkanSceneKind::Triangle,
        false,
    );
}

#[cfg(feature = "backend-vulkan")]
fn run_textured_object() {
    run_vulkan_scene(
        "Engine Sandbox - Textured Object",
        render_vulkan::VulkanSceneKind::TexturedQuad,
        false,
    );
}

#[cfg(feature = "backend-vulkan")]
fn run_resize_smoke() {
    run_vulkan_scene(
        "Engine Sandbox - Resize Smoke",
        render_vulkan::VulkanSceneKind::TexturedQuad,
        true,
    );
}

#[cfg(feature = "backend-vulkan")]
fn run_vulkan_scene(title: &str, scene: render_vulkan::VulkanSceneKind, auto_resize: bool) {
    use std::sync::Arc;

    use platform::winit::window::Window;
    use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
    use render_vulkan::{VulkanRenderer, VulkanRendererDescriptor, VulkanSceneKind};

    struct VulkanSampleApp {
        renderer: Option<VulkanRenderer>,
        frames: u64,
        max_frames: Option<u64>,
        scene: VulkanSceneKind,
        auto_resize: bool,
    }

    impl WindowApp for VulkanSampleApp {
        fn on_create(&mut self, window: Arc<Window>) {
            let size = window.inner_size();
            let display_handle = match window.display_handle() {
                Ok(h) => h.as_raw(),
                Err(err) => {
                    tracing::error!(error = %err, "failed to acquire raw display handle");
                    return;
                }
            };
            let window_handle = match window.window_handle() {
                Ok(h) => h.as_raw(),
                Err(err) => {
                    tracing::error!(error = %err, "failed to acquire raw window handle");
                    return;
                }
            };
            let enable_validation = std::env::var("ENGINE_VK_VALIDATION").is_ok();
            match VulkanRenderer::new(VulkanRendererDescriptor {
                display_handle,
                window_handle,
                width: size.width.max(1),
                height: size.height.max(1),
                enable_validation,
                scene: self.scene,
            }) {
                Ok(renderer) => {
                    tracing::info!("vulkan renderer initialized");
                    self.renderer = Some(renderer);
                }
                Err(err) => {
                    tracing::error!(error = %err, "vulkan renderer initialization failed");
                    std::process::exit(1);
                }
            }
        }

        fn on_event(&mut self, window: &Window, event: PlatformEvent) -> EventFlow {
            match event {
                PlatformEvent::Resized { width, height } => {
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.resize(width, height);
                    }
                    EventFlow::Continue
                }
                PlatformEvent::Redraw => {
                    if let Some(renderer) = self.renderer.as_mut() {
                        if self.auto_resize {
                            request_resize_step(window, self.frames);
                        }
                        if let Err(err) = renderer.render() {
                            tracing::error!(error = %err, "frame render failed");
                            return EventFlow::Exit;
                        }
                        self.frames += 1;
                        if let Some(limit) = self.max_frames {
                            if self.frames >= limit {
                                tracing::info!(
                                    frames = self.frames,
                                    "frame limit reached; exiting"
                                );
                                renderer.wait_idle();
                                return EventFlow::Exit;
                            }
                        }
                    }
                    EventFlow::Continue
                }
                PlatformEvent::CloseRequested => {
                    if let Some(renderer) = self.renderer.as_ref() {
                        renderer.wait_idle();
                    }
                    EventFlow::Exit
                }
                PlatformEvent::Resumed | PlatformEvent::Suspended => EventFlow::Continue,
            }
        }
    }

    let max_frames = parse_frame_limit();

    let app = VulkanSampleApp {
        renderer: None,
        frames: 0,
        max_frames,
        scene,
        auto_resize,
    };
    if let Err(err) = platform::run(
        WindowDescriptor {
            title: title.to_string(),
            width: 1280,
            height: 720,
        },
        app,
    ) {
        tracing::error!(error = %err, "platform run failed");
        std::process::exit(1);
    }
}

#[cfg(feature = "backend-vulkan")]
fn parse_frame_limit() -> Option<u64> {
    let mut args = std::env::args().skip(2);
    while let Some(arg) = args.next() {
        if arg == "--frames" {
            return args.next().and_then(|value| value.parse::<u64>().ok());
        }
        if let Some(value) = arg.strip_prefix("--frames=") {
            return value.parse::<u64>().ok();
        }
    }
    None
}

#[cfg(feature = "backend-vulkan")]
fn request_resize_step(window: &platform::winit::window::Window, frame: u64) {
    let size = match frame {
        30 => Some((960, 540)),
        60 => Some((320, 240)),
        90 => Some((1280, 720)),
        _ => None,
    };
    if let Some((width, height)) = size {
        let _ = window.request_inner_size(platform::winit::dpi::PhysicalSize::new(width, height));
        tracing::info!(width, height, "resize-smoke requested window size");
    }
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_triangle() {
    tracing::error!(
        "the `triangle` command requires the `backend-vulkan` feature \
         (cargo run -p sandbox --features backend-vulkan -- triangle)"
    );
    std::process::exit(2);
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_textured_object() {
    tracing::error!(
        "the `textured-object` command requires the `backend-vulkan` feature \
         (cargo run -p sandbox --features backend-vulkan -- textured-object)"
    );
    std::process::exit(2);
}

#[cfg(not(feature = "backend-vulkan"))]
fn run_resize_smoke() {
    tracing::error!(
        "the `resize-smoke` command requires the `backend-vulkan` feature \
         (cargo run -p sandbox --features backend-vulkan -- resize-smoke)"
    );
    std::process::exit(2);
}
