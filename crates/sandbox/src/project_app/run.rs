pub fn run_project(request: ProjectRunRequest) -> Result<(), String> {
    // Authoring runs rebuild managed scripts before switching to the runtime
    // view of the project. Packaged projects intentionally have no source
    // asset/script requirements and skip this branch.
    if !request.scripts_already_built {
        if let Ok(authoring_project) = GameProject::load(&request.project) {
            if authoring_project.script_project.is_some() {
                crate::project_scripts::build_project_scripts(&authoring_project)?;
            }
        }
    }
    let project = GameProject::load_runtime(&request.project).map_err(|error| error.to_string())?;
    let scene = load_startup_scene(&project.startup_scene)?;
    if request.headless {
        run_headless(
            project,
            scene,
            request.frames.unwrap_or(3),
            request.report.as_deref(),
            request.stream_cells,
        )
    } else {
        run_windowed(project, scene, request.frames, request.stream_cells)
    }
}

pub(super) const MAX_CHAINED_SCENE_TRANSITIONS: usize = 8;

#[cfg(all(feature = "runtime-subsystems", feature = "backend-vulkan"))]
pub(super) fn route_project_player_ui_event(
    game_loop: &mut GameLoop,
    event: &platform::PlatformEvent,
) -> bool {
    match event {
        platform::PlatformEvent::MouseMoved { x, y } => {
            #[cfg(feature = "subsystem-scripting-csharp")]
            game_loop.script_pointer_move(*x as f32, *y as f32);
            game_loop.ui_pointer_move(*x as f32, *y as f32);
            true
        }
        platform::PlatformEvent::MousePressed { button, x, y }
            if *button == platform::MouseButton::Left =>
        {
            #[cfg(feature = "subsystem-scripting-csharp")]
            {
                game_loop.script_pointer_move(*x as f32, *y as f32);
                game_loop.script_pointer_primary(true);
            }
            game_loop.ui_pointer_move(*x as f32, *y as f32);
            game_loop.ui_pointer_left_press();
            true
        }
        platform::PlatformEvent::MouseReleased { button, x, y }
            if *button == platform::MouseButton::Left =>
        {
            #[cfg(feature = "subsystem-scripting-csharp")]
            {
                game_loop.script_pointer_move(*x as f32, *y as f32);
                game_loop.script_pointer_primary(false);
            }
            game_loop.ui_pointer_move(*x as f32, *y as f32);
            game_loop.ui_pointer_left_release();
            true
        }
        platform::PlatformEvent::MousePressed { button, x, y } => {
            #[cfg(not(feature = "subsystem-scripting-csharp"))]
            let _ = (button, x, y);
            #[cfg(feature = "subsystem-scripting-csharp")]
            {
                game_loop.script_pointer_move(*x as f32, *y as f32);
                match button {
                    platform::MouseButton::Right => game_loop.script_pointer_secondary(true),
                    platform::MouseButton::Middle => game_loop.script_pointer_middle(true),
                    _ => {}
                }
            }
            true
        }
        platform::PlatformEvent::MouseReleased { button, x, y } => {
            #[cfg(not(feature = "subsystem-scripting-csharp"))]
            let _ = (button, x, y);
            #[cfg(feature = "subsystem-scripting-csharp")]
            {
                game_loop.script_pointer_move(*x as f32, *y as f32);
                match button {
                    platform::MouseButton::Right => game_loop.script_pointer_secondary(false),
                    platform::MouseButton::Middle => game_loop.script_pointer_middle(false),
                    _ => {}
                }
            }
            true
        }
        platform::PlatformEvent::MouseWheelScrolled { delta } => {
            #[cfg(not(feature = "subsystem-scripting-csharp"))]
            let _ = delta;
            #[cfg(feature = "subsystem-scripting-csharp")]
            game_loop.script_pointer_scroll(delta.0, delta.1);
            true
        }
        platform::PlatformEvent::Resized { width, height } => {
            #[cfg(not(feature = "subsystem-scripting-csharp"))]
            let _ = (width, height);
            #[cfg(feature = "subsystem-scripting-csharp")]
            game_loop.set_script_viewport_size(*width, *height);
            true
        }
        platform::PlatformEvent::Focused(true) | platform::PlatformEvent::Resumed => {
            #[cfg(feature = "subsystem-scripting-csharp")]
            game_loop.script_pointer_focus(true);
            true
        }
        platform::PlatformEvent::Focused(false) | platform::PlatformEvent::Suspended => {
            #[cfg(feature = "subsystem-scripting-csharp")]
            game_loop.script_pointer_focus(false);
            game_loop.cancel_ui_pointer();
            true
        }
        _ => false,
    }
}
use super::*;
