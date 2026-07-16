//! Release-aware structured logging and panic reports for the sandbox player.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::prelude::*;

struct DiagnosticConfig {
    log_dir: Option<PathBuf>,
    release_id: String,
}

pub fn init() {
    let config = discover_config();
    install_panic_hook(config.log_dir.clone(), config.release_id.clone());

    let console_level = if std::env::args().nth(1).as_deref() == Some("qa-headless") {
        tracing_subscriber::filter::LevelFilter::WARN
    } else {
        tracing_subscriber::filter::LevelFilter::INFO
    };
    let console = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_filter(console_level);
    if let Some(log_file) = config.log_dir.as_deref().and_then(open_log_file) {
        let json = tracing_subscriber::fmt::layer()
            .json()
            .with_ansi(false)
            .with_current_span(true)
            .with_span_list(true)
            .with_writer(Mutex::new(log_file))
            .with_filter(tracing_subscriber::filter::LevelFilter::INFO);
        tracing_subscriber::registry()
            .with(console)
            .with(json)
            .init();
    } else {
        tracing_subscriber::registry().with(console).init();
    }

    tracing::info!(release_id = config.release_id, "diagnostics initialized");
}

fn discover_config() -> DiagnosticConfig {
    let runtime_config = read_runtime_config(Path::new("config/runtime.json"));
    let release_id = std::env::var("ENGINE_RELEASE_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            runtime_config
                .as_ref()
                .and_then(|value| value.get("release_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    let log_dir = match std::env::var("ENGINE_LOG_DIR") {
        Ok(value) if value.eq_ignore_ascii_case("off") || value.trim().is_empty() => None,
        Ok(value) => Some(PathBuf::from(value)),
        Err(_) => runtime_config
            .as_ref()
            .and_then(|value| value.get("log_root"))
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from),
    };

    DiagnosticConfig {
        log_dir,
        release_id,
    }
}

fn read_runtime_config(path: &Path) -> Option<serde_json::Value> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn open_log_file(log_dir: &Path) -> Option<File> {
    if let Err(error) = std::fs::create_dir_all(log_dir) {
        eprintln!("failed to create diagnostic log directory {log_dir:?}: {error}");
        return None;
    }
    let path = log_dir.join(format!("sandbox-{}.jsonl", std::process::id()));
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!("failed to open diagnostic log {path:?}: {error}");
            None
        }
    }
}

fn install_panic_hook(log_dir: Option<PathBuf>, release_id: String) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        if let Some(log_dir) = &log_dir {
            let _ = std::fs::create_dir_all(log_dir);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            let path = log_dir.join(format!("panic-{timestamp}-{}.txt", std::process::id()));
            let location = panic_info
                .location()
                .map(|location| location.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let payload = panic_info
                .payload()
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| {
                    panic_info
                        .payload()
                        .downcast_ref::<String>()
                        .map(String::as_str)
                })
                .unwrap_or("non-string panic payload");
            let report = format!(
                "release_id={release_id}\nprocess_id={}\ntimestamp={timestamp}\nlocation={location}\npayload={payload}\n\nbacktrace:\n{}\n",
                std::process::id(),
                std::backtrace::Backtrace::force_capture()
            );
            let _ = std::fs::write(path, report);
        }
        previous(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_runtime_config_is_ignored() {
        assert!(read_runtime_config(Path::new("missing-runtime-config.json")).is_none());
    }
}
