//! Headless build operations used by the editor.
//!
//! This module deliberately delegates authoring builds to the public sandbox
//! project CLI and Windows releases to the engine distribution's packaging
//! tool. Source-tree development uses the same script from `.github/scripts`;
//! an installed editor resolves every tool from `engine.installation.json`.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use engine_asset::project::GameProject;
use serde_json::Value;
use sha2::{Digest, Sha256};

const CAPTURE_LIMIT_BYTES: usize = 2 * 1024 * 1024;
const DIAGNOSTIC_TAIL_CHARS: usize = 16 * 1024;
const WINDOWS_PLATFORM: &str = "windows-x86_64";
const RELEASE_METADATA_SCHEMA: &str = "ReleaseMetadata-v0";
const PROJECT_CHECK_SCHEMA: &str = "ProjectCheckReport-v0";

/// Stable operation names exposed to editor menus, progress UI, and logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorBuildOperationKind {
    Validate,
    CookAndCompile,
    PackageWindows,
}

impl EditorBuildOperationKind {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Validate => "Validate Project",
            Self::CookAndCompile => "Cook & Compile Project",
            Self::PackageWindows => "Package Windows Player",
        }
    }
}

/// A build operation that can run outside the editor frame loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EditorBuildOperation {
    Validate,
    CookAndCompile,
    PackageWindows(PackageWindowsOptions),
}

impl EditorBuildOperation {
    pub(crate) fn kind(&self) -> EditorBuildOperationKind {
        match self {
            Self::Validate => EditorBuildOperationKind::Validate,
            Self::CookAndCompile => EditorBuildOperationKind::CookAndCompile,
            Self::PackageWindows(_) => EditorBuildOperationKind::PackageWindows,
        }
    }
}

/// Explicit Windows package settings.
///
/// `allow_dirty` is intentionally false by default. Enabling it is suitable
/// only for an explicitly requested local dry run because release metadata will
/// record that the package was produced from a dirty worktree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackageWindowsOptions {
    pub(crate) version: String,
    pub(crate) output_root: PathBuf,
    pub(crate) cargo_target_dir: Option<PathBuf>,
    pub(crate) skip_build: bool,
    pub(crate) skip_smoke: bool,
    pub(crate) allow_dirty: bool,
}

impl PackageWindowsOptions {
    pub(crate) fn new(version: impl Into<String>, output_root: impl Into<PathBuf>) -> Self {
        Self {
            version: version.into(),
            output_root: output_root.into(),
            cargo_target_dir: None,
            skip_build: false,
            skip_smoke: false,
            allow_dirty: false,
        }
    }
}

/// Why a build request could not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorBuildFailureKind {
    InvalidRequest,
    #[cfg(not(windows))]
    UnsupportedPlatform,
    SpawnFailed,
    Cancelled,
    ProcessFailed,
    InvalidResult,
    WorkerFailed,
}

/// Bounded stdout/stderr captured from a build process.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditorBuildOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
}

impl EditorBuildOutput {
    fn diagnostic_tail(&self) -> Option<String> {
        let text = if self.stderr.trim().is_empty() {
            self.stdout.trim()
        } else {
            self.stderr.trim()
        };
        if text.is_empty() {
            None
        } else {
            Some(tail_chars(text, DIAGNOSTIC_TAIL_CHARS))
        }
    }
}

/// Structured error suitable for a build status panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditorBuildError {
    pub(crate) operation: EditorBuildOperationKind,
    pub(crate) kind: EditorBuildFailureKind,
    pub(crate) message: String,
    pub(crate) exit_code: Option<i32>,
    pub(crate) output: EditorBuildOutput,
}

impl EditorBuildError {
    fn request(
        operation: EditorBuildOperationKind,
        kind: EditorBuildFailureKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            kind,
            message: message.into(),
            exit_code: None,
            output: EditorBuildOutput::default(),
        }
    }

    fn process(
        operation: EditorBuildOperationKind,
        kind: EditorBuildFailureKind,
        message: impl Into<String>,
        exit_code: Option<i32>,
        output: EditorBuildOutput,
    ) -> Self {
        Self {
            operation,
            kind,
            message: message.into(),
            exit_code,
            output,
        }
    }
}

impl fmt::Display for EditorBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}",
            self.operation.display_name(),
            self.message
        )?;
        if let Some(exit_code) = self.exit_code {
            write!(formatter, " (exit code {exit_code})")?;
        }
        if let Some(diagnostic) = self.output.diagnostic_tail() {
            write!(formatter, "\n{diagnostic}")?;
        }
        Ok(())
    }
}

impl std::error::Error for EditorBuildError {}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProjectValidationResult {
    pub(crate) project: String,
    pub(crate) startup_scene_id: String,
    pub(crate) scenes: u64,
    pub(crate) entities: u64,
    pub(crate) declared_assets: u64,
    pub(crate) cooked_assets: u64,
    pub(crate) report: Value,
    pub(crate) elapsed: Duration,
    pub(crate) output: EditorBuildOutput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CookAndCompileResult {
    pub(crate) project: String,
    pub(crate) manifest_path: PathBuf,
    pub(crate) cooked_assets: PathBuf,
    pub(crate) scripts_configured: bool,
    pub(crate) elapsed: Duration,
    pub(crate) output: EditorBuildOutput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackageWindowsResult {
    pub(crate) version: String,
    pub(crate) release_root: PathBuf,
    pub(crate) archive_path: PathBuf,
    pub(crate) archive_sha256: String,
    pub(crate) symbols_archive_path: PathBuf,
    pub(crate) symbols_sha256: String,
    pub(crate) release_manifest_path: PathBuf,
    pub(crate) dirty: bool,
    pub(crate) elapsed: Duration,
    pub(crate) output: EditorBuildOutput,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EditorBuildResult {
    Validated(ProjectValidationResult),
    CookedAndCompiled(CookAndCompileResult),
    PackagedWindows(PackageWindowsResult),
}

/// Configures and launches the engine's authoritative build entry points.
#[derive(Clone, Debug)]
pub(crate) struct EditorBuildService {
    toolchain: EditorBuildToolchain,
    sandbox_executable: PathBuf,
    powershell_executable: PathBuf,
}

mod completion;
mod process_io;
mod service;
mod task;
mod validation;

#[cfg(test)]
use completion::{finish_validation, finish_windows_package};
use process_io::{
    hide_child_window, output_snapshot, spawn_reader, tail_chars, terminate_child_tree,
    wait_for_child, CapturedBytes,
};
use service::EditorBuildToolchain;
pub(crate) use task::EditorBuildTask;
use task::{CompletionPlan, ProcessPlan};
#[cfg(windows)]
use validation::system_windows_executable;
use validation::{
    canonical_directory, canonical_file, read_json, require_json_string, required_string,
    required_u64, resolve_output_directory, system_powershell_executable,
    validate_installed_package_output, validate_release_version, verify_checksum_sidecar,
};

#[cfg(test)]
mod tests {
    include!("editor_build_ops/tests/common.rs");
    include!("editor_build_ops/tests/plans.rs");
    include!("editor_build_ops/tests/results.rs");
}
