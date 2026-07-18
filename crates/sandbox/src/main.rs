#![forbid(unsafe_code)]

use engine_asset::ReloadCoordinator;
use engine_core::{EngineConfig, EngineRuntime};

mod diagnostics;
#[cfg(any(all(feature = "tooling-editor", feature = "backend-vulkan"), test))]
mod editor_asset_ops;
#[cfg(any(all(feature = "tooling-editor", feature = "backend-vulkan"), test))]
mod editor_build_ops;
mod project_app;
mod project_cli;
mod project_input;
mod project_scripts;
mod qa;
mod release_diagnostics;

#[cfg(feature = "backend-vulkan")]
fn log_renderer_diagnostics(operation: &str, diagnostics: &[engine_renderer::Diagnostic]) {
    for diagnostic in diagnostics {
        tracing::error!(
            operation,
            code = diagnostic.code,
            system = diagnostic.system,
            message = diagnostic.message,
            "renderer operation failed"
        );
    }
}

fn main() {
    release_diagnostics::init();
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "help".to_string());
    match command.as_str() {
        "help" | "--help" | "-h" => project_cli::print_global_help(),
        "project" => run_project_command(),
        "game" => run_game_project(),
        "qa-headless" => qa::run_from_args(),
        "gate04-scene" => run_gate04_scene(),
        "editor" => run_editor_command(),
        other => {
            tracing::error!(command = other, "unknown sandbox command");
            std::process::exit(2);
        }
    }
}

fn run_project_command() {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    match project_cli::dispatch(&arguments) {
        Ok(project_cli::ProjectAction::Complete) => {}
        Ok(project_cli::ProjectAction::Run(request)) => run_game_request(request),
        Ok(project_cli::ProjectAction::Edit(project)) => run_editor(project),
        Err(error) => command_failed(error),
    }
}

fn run_game_project() {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    match project_cli::parse_run_request(&arguments) {
        Ok(request) => run_game_request(request),
        Err(error) => command_failed(error),
    }
}

fn run_game_request(request: project_cli::ProjectRunRequest) {
    if let Err(error) = project_app::run_project(request) {
        command_failed(error);
    }
}

fn run_editor_command() {
    let arguments = std::env::args().skip(2).collect::<Vec<_>>();
    match arguments.as_slice() {
        [project] if !project.starts_with('-') => run_editor(project.into()),
        _ => command_failed("usage: sandbox editor <project-directory-or-manifest>"),
    }
}

fn command_failed(error: impl std::fmt::Display) -> ! {
    tracing::error!(%error, "command failed");
    eprintln!("error: {error}");
    std::process::exit(2);
}

#[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
mod editor_app;
#[cfg(all(feature = "tooling-editor", feature = "backend-vulkan"))]
use editor_app::run_editor;

#[cfg(not(all(feature = "tooling-editor", feature = "backend-vulkan")))]
fn run_editor(_project: std::path::PathBuf) {
    tracing::error!("editor requires `tooling-editor` and `backend-vulkan` features");
    std::process::exit(2);
}

fn run_gate04_scene() {
    let mut runtime = EngineRuntime::new(EngineConfig::default());
    if let Err(diagnostics) = runtime.load_scene(engine_scene::sample_scene()) {
        for diagnostic in diagnostics {
            tracing::error!(
                code = diagnostic.code,
                message = diagnostic.message,
                "scene load failed"
            );
        }
        std::process::exit(2);
    }

    let dir = std::env::temp_dir().join("sandbox_reload");
    let _ = std::fs::create_dir_all(&dir);
    let reload_coordinator = ReloadCoordinator::new_with_registry(
        &dir,
        &dir,
        &dir,
        runtime.asset_type_registry().clone(),
    )
    .expect("reload coordinator creation should succeed");
    let mut sandbox_diags = diagnostics::SandboxDiagnostics::new();

    match runtime.render_frame(0) {
        Ok(stats) => {
            tracing::info!(
                draw_calls = stats.draw_calls,
                "gate04 scene rendered through contract path"
            );

            // The runtime's DiagnosticsCollector already recorded frame stats
            // inside render_frame(). Build a RuntimeDiagnostics snapshot and
            // feed it to the sandbox aggregator along with reload coordinator state.
            let runtime_diags = runtime.runtime_diagnostics();
            sandbox_diags.update(&runtime_diags, &reload_coordinator);

            let all = sandbox_diags.all_diagnostics();
            tracing::info!(count = all.len(), "sandbox diagnostics collected");
            for diagnostic in &all {
                tracing::debug!(
                    code = diagnostic.code,
                    severity = ?diagnostic.severity,
                    message = diagnostic.message,
                    "aggregated diagnostic"
                );
            }

            tracing::info!(
                draw_calls = stats.draw_calls,
                triangles = stats.triangles,
                gpu_ms = stats.gpu_frame_ms,
                visible = stats.visible_drawables,
                culled = stats.culled_drawables,
                "gate04 frame stats"
            );
        }
        Err(diagnostics) => {
            for diagnostic in diagnostics {
                tracing::error!(code = diagnostic.code, message = diagnostic.message);
            }
            std::process::exit(1);
        }
    }
}
