//! Headless scene QA and CPU performance baseline generation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use engine_renderer::{
    BackendRenderer, Diagnostic, DiagnosticSeverity, FrameStats, MaterialUpload, MeshUpload,
    RenderFrameInput, TextureUpload, UploadReceipt,
};

#[derive(Default)]
pub(crate) struct QaBackend {
    frame_active: bool,
    mesh_triangles: BTreeMap<engine_renderer::AssetId, u64>,
}

impl BackendRenderer for QaBackend {
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        if self.frame_active {
            return Err(vec![qa_error("QA0002", "frame already active")]);
        }
        self.frame_active = true;
        Ok(())
    }

    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        pass: &engine_renderer::render_graph2::PassNode,
        barriers: &[engine_renderer::render_graph2::CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        if let Some(unsupported) = barriers.iter().find(|barrier| {
            let declared_as_attachment = pass
                .inputs
                .iter()
                .chain(&pass.outputs)
                .any(|attachment| attachment.name == barrier.resource_name);
            let declared_as_depth = pass
                .depth_stencil
                .as_ref()
                .is_some_and(|attachment| attachment.name == barrier.resource_name);
            !declared_as_attachment && !declared_as_depth
        }) {
            return Err(vec![qa_error(
                "QA0005",
                format!(
                    "pass '{}' received a barrier for undeclared graph resource '{}'",
                    pass.name, unsupported.resource_name
                ),
            )]);
        }
        // Headless QA validates graph transitions but owns no GPU resources.
        Ok(())
    }

    fn execute_pass(
        &mut self,
        input: &RenderFrameInput,
        pass: &engine_renderer::render_graph2::PassNode,
        stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        if !self.frame_active {
            return Err(vec![qa_error("QA0003", "render pass outside a frame")]);
        }
        if pass.kind == engine_renderer::render_graph2::PassKind::OpaquePbrForward {
            let meshes = input
                .drawables
                .iter()
                .map(|item| &item.mesh)
                .chain(input.skinned_items.iter().map(|item| &item.mesh));
            let mut draw_calls = 0u32;
            for mesh in meshes {
                draw_calls = draw_calls.saturating_add(1);
                stats.triangles = stats
                    .triangles
                    .saturating_add(self.mesh_triangles.get(mesh).copied().unwrap_or(0));
            }
            stats.draw_calls = stats.draw_calls.saturating_add(draw_calls);
            stats.visible_drawables = draw_calls;
        }
        Ok(())
    }

    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        if !self.frame_active {
            return Err(vec![qa_error("QA0004", "ending an inactive frame")]);
        }
        self.frame_active = false;
        Ok(())
    }

    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        self.frame_active = false;
        Ok(())
    }

    fn upload_mesh(&mut self, upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        self.mesh_triangles
            .insert(upload.mesh_id, u64::from(upload.index_count / 3));
        Ok(UploadReceipt::new(1))
    }

    fn upload_texture(&mut self, _upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }

    fn upload_material(
        &mut self,
        _upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Ok(UploadReceipt::new(1))
    }
}

fn qa_error(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, DiagnosticSeverity::Error, "sandbox.qa", message)
}

#[derive(Debug)]
struct QaOptions {
    frames: u64,
    max_average_cpu_ms: f64,
    output: Option<PathBuf>,
}

pub fn run_from_args() {
    let options = match parse_options(std::env::args().skip(2)) {
        Ok(options) => options,
        Err(error) => fail(&error),
    };
    if let Err(error) = run(&options) {
        fail(&error);
    }
}

fn parse_options(args: impl Iterator<Item = String>) -> Result<QaOptions, String> {
    let mut frames = 120u64;
    let mut max_average_cpu_ms = 50.0f64;
    let mut output = None;
    let mut args = args.peekable();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--frames" => frames = parse_u64("--frames", args.next())?,
            "--max-average-cpu-ms" => {
                max_average_cpu_ms = parse_f64("--max-average-cpu-ms", args.next())?;
            }
            "--output" => {
                output = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--output requires a path".to_string())?,
                ));
            }
            _ if argument.starts_with("--frames=") => {
                frames = parse_u64(
                    "--frames",
                    argument.split_once('=').map(|(_, value)| value.into()),
                )?;
            }
            _ if argument.starts_with("--max-average-cpu-ms=") => {
                max_average_cpu_ms = parse_f64(
                    "--max-average-cpu-ms",
                    argument.split_once('=').map(|(_, value)| value.into()),
                )?;
            }
            _ if argument.starts_with("--output=") => {
                output = argument
                    .split_once('=')
                    .map(|(_, value)| PathBuf::from(value));
            }
            _ => return Err(format!("unknown qa-headless argument: {argument}")),
        }
    }
    if frames == 0 || frames > 10_000 {
        return Err("--frames must be in 1..=10000".to_string());
    }
    if !max_average_cpu_ms.is_finite() || max_average_cpu_ms <= 0.0 {
        return Err("--max-average-cpu-ms must be finite and positive".to_string());
    }
    Ok(QaOptions {
        frames,
        max_average_cpu_ms,
        output,
    })
}

fn parse_u64(label: &str, value: Option<String>) -> Result<u64, String> {
    value
        .ok_or_else(|| format!("{label} requires a value"))?
        .parse()
        .map_err(|_| format!("{label} requires an unsigned integer"))
}

fn parse_f64(label: &str, value: Option<String>) -> Result<f64, String> {
    value
        .ok_or_else(|| format!("{label} requires a value"))?
        .parse()
        .map_err(|_| format!("{label} requires a number"))
}

fn run(options: &QaOptions) -> Result<(), String> {
    let mut runtime = engine_core::EngineRuntime::new(engine_core::EngineConfig::default());
    runtime.set_renderer_backend(Box::<QaBackend>::default());

    let load_started = Instant::now();
    runtime
        .load_scene(engine_scene::sample_scene())
        .map_err(|diagnostics| format_diagnostics("scene load", &diagnostics))?;
    let scene_load_ms = load_started.elapsed().as_secs_f64() * 1_000.0;

    let mut total_cpu_ms = 0.0f64;
    let mut max_cpu_ms = 0.0f64;
    let mut last_stats = FrameStats::default();
    for frame in 0..options.frames {
        let frame_started = Instant::now();
        last_stats = runtime
            .render_frame(frame)
            .map_err(|diagnostics| format_diagnostics("render frame", &diagnostics))?;
        let cpu_ms = frame_started.elapsed().as_secs_f64() * 1_000.0;
        total_cpu_ms += cpu_ms;
        max_cpu_ms = max_cpu_ms.max(cpu_ms);
    }
    let average_cpu_ms = total_cpu_ms / options.frames as f64;
    let passed = average_cpu_ms <= options.max_average_cpu_ms
        && last_stats.draw_calls > 0
        && last_stats.visible_drawables > 0
        && last_stats.triangles > 0;

    let report = serde_json::json!({
        "schema": "QaReport-v0",
        "passed": passed,
        "scene": "engine_scene::sample_scene",
        "frames": options.frames,
        "thresholds": {
            "max_average_cpu_ms": options.max_average_cpu_ms,
            "minimum_draw_calls": 1,
            "minimum_visible_drawables": 1,
            "minimum_triangles": 1
        },
        "metrics": {
            "scene_load_ms": scene_load_ms,
            "average_cpu_frame_ms": average_cpu_ms,
            "max_cpu_frame_ms": max_cpu_ms,
            "draw_calls": last_stats.draw_calls,
            "visible_drawables": last_stats.visible_drawables,
            "triangles": last_stats.triangles,
            "gpu_frame_ms": null,
            "process_memory_bytes": null
        },
        "notes": [
            "Headless contract backend; GPU and process-memory baselines require a controlled hardware runner."
        ]
    });
    let json = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("serialize QA report: {error}"))?;
    if let Some(output) = &options.output {
        write_report(output, &json)?;
    }
    println!("{json}");

    if passed {
        Ok(())
    } else {
        Err(format!(
            "QA thresholds failed: average_cpu_ms={average_cpu_ms:.3}, draw_calls={}, visible={}, triangles={}",
            last_stats.draw_calls, last_stats.visible_drawables, last_stats.triangles
        ))
    }
}

fn write_report(path: &Path, json: &str) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create QA report directory: {error}"))?;
    }
    std::fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("write QA report {}: {error}", path.display()))
}

fn format_diagnostics(operation: &str, diagnostics: &[Diagnostic]) -> String {
    let messages = diagnostics
        .iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("; ");
    format!("{operation} failed: {messages}")
}

fn fail(error: &str) -> ! {
    tracing::error!(error, "headless QA failed");
    eprintln!("headless QA failed: {error}");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_reject_zero_frames() {
        let error = parse_options(["--frames=0".to_string()].into_iter()).unwrap_err();
        assert!(error.contains("1..=10000"));
    }

    #[test]
    fn headless_scene_produces_draw_calls() {
        let options = QaOptions {
            frames: 2,
            max_average_cpu_ms: 1_000.0,
            output: None,
        };
        run(&options).unwrap();
    }
}
