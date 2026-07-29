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

#[derive(Clone, Debug)]
enum EditorBuildToolchain {
    Installed(crate::engine_installation::EngineInstallation),
    Development { repository_root: PathBuf },
}

impl EditorBuildService {
    /// Build a service for a running editor.
    ///
    /// Installed distributions are resolved first. Only a process without an
    /// installation manifest may use the compile-time repository fallback.
    pub(crate) fn for_current_editor() -> Result<Self, EditorBuildError> {
        let operation = EditorBuildOperationKind::Validate;
        let sandbox_executable = std::env::current_exe().map_err(|error| {
            EditorBuildError::request(
                operation,
                EditorBuildFailureKind::InvalidRequest,
                format!("could not locate the running sandbox executable: {error}"),
            )
        })?;
        let installation =
            crate::engine_installation::EngineInstallation::discover_from_current_executable()
                .map_err(|message| {
                    EditorBuildError::request(
                        operation,
                        EditorBuildFailureKind::InvalidRequest,
                        message,
                    )
                })?;
        if let Some(installation) = installation {
            return Self::with_installed_powershell(
                installation,
                system_powershell_executable().map_err(|message| {
                    EditorBuildError::request(
                        operation,
                        EditorBuildFailureKind::InvalidRequest,
                        message,
                    )
                })?,
            );
        }

        let repository_root =
            crate::engine_installation::development_source_root().map_err(|message| {
                EditorBuildError::request(
                    operation,
                    EditorBuildFailureKind::InvalidRequest,
                    message,
                )
            })?;
        Self::with_powershell(
            repository_root,
            sandbox_executable,
            system_powershell_executable().map_err(|message| {
                EditorBuildError::request(
                    operation,
                    EditorBuildFailureKind::InvalidRequest,
                    message,
                )
            })?,
        )
    }

    fn with_powershell(
        repository_root: impl Into<PathBuf>,
        sandbox_executable: impl Into<PathBuf>,
        powershell_executable: impl Into<PathBuf>,
    ) -> Result<Self, EditorBuildError> {
        let operation = EditorBuildOperationKind::Validate;
        let repository_root = canonical_directory(repository_root.into(), "repository root")
            .map_err(|message| {
                EditorBuildError::request(
                    operation,
                    EditorBuildFailureKind::InvalidRequest,
                    message,
                )
            })?;
        let sandbox_executable = canonical_file(sandbox_executable.into(), "sandbox executable")
            .map_err(|message| {
                EditorBuildError::request(
                    operation,
                    EditorBuildFailureKind::InvalidRequest,
                    message,
                )
            })?;
        let powershell_executable = powershell_executable.into();
        if powershell_executable.as_os_str().is_empty() {
            return Err(EditorBuildError::request(
                operation,
                EditorBuildFailureKind::InvalidRequest,
                "PowerShell executable must not be empty",
            ));
        }
        Ok(Self {
            toolchain: EditorBuildToolchain::Development { repository_root },
            sandbox_executable,
            powershell_executable,
        })
    }

    fn with_installed_powershell(
        installation: crate::engine_installation::EngineInstallation,
        powershell_executable: impl Into<PathBuf>,
    ) -> Result<Self, EditorBuildError> {
        let operation = EditorBuildOperationKind::Validate;
        let powershell_executable = powershell_executable.into();
        if powershell_executable.as_os_str().is_empty() {
            return Err(EditorBuildError::request(
                operation,
                EditorBuildFailureKind::InvalidRequest,
                "PowerShell executable must not be empty",
            ));
        }
        let sandbox_executable = installation.editor.clone();
        Ok(Self {
            toolchain: EditorBuildToolchain::Installed(installation),
            sandbox_executable,
            powershell_executable,
        })
    }

    /// Launch a cancellable background operation.
    pub(crate) fn start(
        &self,
        project_path: impl AsRef<Path>,
        operation: EditorBuildOperation,
    ) -> Result<EditorBuildTask, EditorBuildError> {
        #[cfg(not(windows))]
        let kind = operation.kind();
        #[cfg(not(windows))]
        if matches!(operation, EditorBuildOperation::PackageWindows(_)) {
            return Err(EditorBuildError::request(
                kind,
                EditorBuildFailureKind::UnsupportedPlatform,
                "the Windows player package script can only run on Windows",
            ));
        }

        let plan = self.plan(project_path.as_ref(), operation)?;
        EditorBuildTask::spawn(plan)
    }

    fn plan(
        &self,
        project_path: &Path,
        operation: EditorBuildOperation,
    ) -> Result<ProcessPlan, EditorBuildError> {
        let kind = operation.kind();
        let project = GameProject::load(project_path).map_err(|error| {
            EditorBuildError::request(
                kind,
                EditorBuildFailureKind::InvalidRequest,
                format!("project cannot be loaded for authoring: {error}"),
            )
        })?;
        let manifest_path = canonical_file(project.manifest_path.clone(), "project manifest")
            .map_err(|message| {
                EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
            })?;

        match operation {
            EditorBuildOperation::Validate => {
                let report_directory = tempfile::Builder::new()
                    .prefix("engine-editor-project-check-")
                    .tempdir()
                    .map_err(|error| {
                        EditorBuildError::request(
                            kind,
                            EditorBuildFailureKind::InvalidRequest,
                            format!("could not create a validation report directory: {error}"),
                        )
                    })?;
                let report_path = report_directory.path().join("project-check.json");
                Ok(ProcessPlan {
                    kind,
                    executable: self.sandbox_executable.clone(),
                    arguments: vec![
                        OsString::from("project"),
                        OsString::from("check"),
                        manifest_path.clone().into_os_string(),
                        OsString::from("--report"),
                        report_path.clone().into_os_string(),
                    ],
                    working_directory: project.root.clone(),
                    completion: CompletionPlan::Validate {
                        report_path,
                        _report_directory: report_directory,
                    },
                })
            }
            EditorBuildOperation::CookAndCompile => Ok(ProcessPlan {
                kind,
                executable: self.sandbox_executable.clone(),
                arguments: vec![
                    OsString::from("project"),
                    OsString::from("build"),
                    manifest_path.clone().into_os_string(),
                ],
                working_directory: project.root.clone(),
                completion: CompletionPlan::CookAndCompile { manifest_path },
            }),
            EditorBuildOperation::PackageWindows(options) => {
                self.plan_windows_package(&project, manifest_path, options)
            }
        }
    }

    fn plan_windows_package(
        &self,
        project: &GameProject,
        manifest_path: PathBuf,
        options: PackageWindowsOptions,
    ) -> Result<ProcessPlan, EditorBuildError> {
        let kind = EditorBuildOperationKind::PackageWindows;
        validate_release_version(&options.version).map_err(|message| {
            EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
        })?;

        let (script, installation_root) = match &self.toolchain {
            EditorBuildToolchain::Installed(installation) => (
                installation.package_script.clone(),
                Some(installation.root.clone()),
            ),
            EditorBuildToolchain::Development { repository_root } => {
                let script = canonical_file(
                    repository_root.join(".github/scripts/package-windows.ps1"),
                    "development Windows package script",
                )
                .map_err(|message| {
                    EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
                })?;
                if !script.starts_with(repository_root) {
                    return Err(EditorBuildError::request(
                        kind,
                        EditorBuildFailureKind::InvalidRequest,
                        "development Windows package script resolves outside the repository",
                    ));
                }
                (script, None)
            }
        };

        let installed = matches!(&self.toolchain, EditorBuildToolchain::Installed(_));
        let output_root = resolve_output_directory(
            &project.root,
            &options.output_root,
            "package output root",
            installed,
        )
        .map_err(|message| {
            EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
        })?;
        if installed {
            validate_installed_package_output(project, &output_root, &options.version).map_err(
                |message| {
                    EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
                },
            )?;
        }
        let cargo_target_dir = options
            .cargo_target_dir
            .as_deref()
            .map(|path| {
                resolve_output_directory(&project.root, path, "Cargo target directory", false)
            })
            .transpose()
            .map_err(|message| {
                EditorBuildError::request(kind, EditorBuildFailureKind::InvalidRequest, message)
            })?;
        if installation_root.is_some() && cargo_target_dir.is_some() {
            return Err(EditorBuildError::request(
                kind,
                EditorBuildFailureKind::InvalidRequest,
                "an installed engine uses prebuilt tools and does not accept a Cargo target directory",
            ));
        }

        let mut arguments = vec![
            OsString::from("-NoLogo"),
            OsString::from("-NoProfile"),
            OsString::from("-NonInteractive"),
            OsString::from("-ExecutionPolicy"),
            OsString::from("Bypass"),
            OsString::from("-File"),
            script.into_os_string(),
            OsString::from("-ProjectPath"),
            manifest_path.into_os_string(),
            OsString::from("-Version"),
            OsString::from(&options.version),
            OsString::from("-OutputRoot"),
            output_root.clone().into_os_string(),
            OsString::from("-Backend"),
            OsString::from("vulkan"),
        ];
        if let Some(root) = installation_root.as_ref() {
            arguments.push(OsString::from("-EngineInstallRoot"));
            arguments.push(root.clone().into_os_string());
        }
        if let Some(target_dir) = cargo_target_dir {
            arguments.push(OsString::from("-CargoTargetDir"));
            arguments.push(target_dir.into_os_string());
        }
        if options.skip_build {
            arguments.push(OsString::from("-SkipBuild"));
        }
        if options.skip_smoke {
            arguments.push(OsString::from("-SkipSmoke"));
        }
        if options.allow_dirty {
            arguments.push(OsString::from("-AllowDirty"));
        }

        let release_root = output_root.join(&options.version);
        Ok(ProcessPlan {
            kind,
            executable: self.powershell_executable.clone(),
            arguments,
            working_directory: project.root.clone(),
            completion: CompletionPlan::PackageWindows {
                version: options.version,
                allow_dirty: options.allow_dirty,
                release_root,
            },
        })
    }
}

/// A running operation. Poll `try_complete`, read `output_snapshot`, or call
/// `cancel` from the editor without blocking its frame loop.
pub(crate) struct EditorBuildTask {
    operation: EditorBuildOperationKind,
    child: Arc<Mutex<Option<Child>>>,
    cancel_requested: Arc<AtomicBool>,
    stdout: Arc<Mutex<CapturedBytes>>,
    stderr: Arc<Mutex<CapturedBytes>>,
    receiver: mpsc::Receiver<Result<EditorBuildResult, EditorBuildError>>,
}

impl EditorBuildTask {
    fn spawn(plan: ProcessPlan) -> Result<Self, EditorBuildError> {
        let mut command = Command::new(&plan.executable);
        command
            .args(&plan.arguments)
            .current_dir(&plan.working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_child_window(&mut command);

        let mut child = command.spawn().map_err(|error| {
            EditorBuildError::request(
                plan.kind,
                EditorBuildFailureKind::SpawnFailed,
                format!(
                    "could not start {} using {}: {error}",
                    plan.kind.display_name(),
                    plan.executable.display()
                ),
            )
        })?;
        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let child = Arc::new(Mutex::new(Some(child)));
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let stdout = Arc::new(Mutex::new(CapturedBytes::default()));
        let stderr = Arc::new(Mutex::new(CapturedBytes::default()));
        let stdout_reader = spawn_reader(stdout_pipe, Arc::clone(&stdout));
        let stderr_reader = spawn_reader(stderr_pipe, Arc::clone(&stderr));
        let (sender, receiver) = mpsc::channel();

        let worker_child = Arc::clone(&child);
        let worker_cancel = Arc::clone(&cancel_requested);
        let worker_stdout = Arc::clone(&stdout);
        let worker_stderr = Arc::clone(&stderr);
        let operation = plan.kind;
        thread::spawn(move || {
            let started = Instant::now();
            let status = wait_for_child(&worker_child);
            if let Some(reader) = stdout_reader {
                let _ = reader.join();
            }
            if let Some(reader) = stderr_reader {
                let _ = reader.join();
            }
            let output = output_snapshot(&worker_stdout, &worker_stderr);
            let elapsed = started.elapsed();
            let result = match status {
                Err(message) => Err(EditorBuildError::process(
                    operation,
                    EditorBuildFailureKind::WorkerFailed,
                    message,
                    None,
                    output,
                )),
                Ok(status) if worker_cancel.load(Ordering::Acquire) => {
                    Err(EditorBuildError::process(
                        operation,
                        EditorBuildFailureKind::Cancelled,
                        "operation was cancelled",
                        status.code(),
                        output,
                    ))
                }
                Ok(status) if !status.success() => Err(EditorBuildError::process(
                    operation,
                    EditorBuildFailureKind::ProcessFailed,
                    "authoritative build process failed",
                    status.code(),
                    output,
                )),
                Ok(_) => plan.completion.finish(elapsed, output),
            };
            let _ = sender.send(result);
        });

        Ok(Self {
            operation,
            child,
            cancel_requested,
            stdout,
            stderr,
            receiver,
        })
    }

    pub(crate) fn operation(&self) -> EditorBuildOperationKind {
        self.operation
    }

    pub(crate) fn output_snapshot(&self) -> EditorBuildOutput {
        output_snapshot(&self.stdout, &self.stderr)
    }

    pub(crate) fn try_complete(&mut self) -> Option<Result<EditorBuildResult, EditorBuildError>> {
        match self.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(EditorBuildError::process(
                self.operation,
                EditorBuildFailureKind::WorkerFailed,
                "build worker exited without returning a result",
                None,
                self.output_snapshot(),
            ))),
        }
    }

    /// Cancel the process and, on Windows, its complete descendant process tree.
    /// Returns false if the operation had already exited.
    pub(crate) fn cancel(&self) -> Result<bool, EditorBuildError> {
        self.cancel_requested.store(true, Ordering::Release);
        let mut child = self.child.lock().map_err(|_| {
            EditorBuildError::process(
                self.operation,
                EditorBuildFailureKind::WorkerFailed,
                "build process state is poisoned",
                None,
                self.output_snapshot(),
            )
        })?;
        let Some(child) = child.as_mut() else {
            return Ok(false);
        };
        if child
            .try_wait()
            .map_err(|error| {
                EditorBuildError::process(
                    self.operation,
                    EditorBuildFailureKind::WorkerFailed,
                    format!("could not query build process before cancellation: {error}"),
                    None,
                    self.output_snapshot(),
                )
            })?
            .is_some()
        {
            return Ok(false);
        }
        terminate_child_tree(child).map_err(|message| {
            EditorBuildError::process(
                self.operation,
                EditorBuildFailureKind::WorkerFailed,
                message,
                None,
                self.output_snapshot(),
            )
        })?;
        Ok(true)
    }
}

#[derive(Debug)]
struct ProcessPlan {
    kind: EditorBuildOperationKind,
    executable: PathBuf,
    arguments: Vec<OsString>,
    working_directory: PathBuf,
    completion: CompletionPlan,
}

#[derive(Debug)]
enum CompletionPlan {
    Validate {
        report_path: PathBuf,
        _report_directory: tempfile::TempDir,
    },
    CookAndCompile {
        manifest_path: PathBuf,
    },
    PackageWindows {
        version: String,
        allow_dirty: bool,
        release_root: PathBuf,
    },
}

impl CompletionPlan {
    fn finish(
        self,
        elapsed: Duration,
        output: EditorBuildOutput,
    ) -> Result<EditorBuildResult, EditorBuildError> {
        match self {
            Self::Validate { report_path, .. } => finish_validation(&report_path, elapsed, output),
            Self::CookAndCompile { manifest_path } => {
                finish_cook_and_compile(&manifest_path, elapsed, output)
            }
            Self::PackageWindows {
                version,
                allow_dirty,
                release_root,
            } => finish_windows_package(version, allow_dirty, release_root, elapsed, output),
        }
    }
}

fn finish_validation(
    report_path: &Path,
    elapsed: Duration,
    output: EditorBuildOutput,
) -> Result<EditorBuildResult, EditorBuildError> {
    let operation = EditorBuildOperationKind::Validate;
    let report = read_json(report_path).map_err(|message| {
        EditorBuildError::process(
            operation,
            EditorBuildFailureKind::InvalidResult,
            message,
            None,
            output.clone(),
        )
    })?;
    require_json_string(&report, "schema", PROJECT_CHECK_SCHEMA)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    if report.get("passed").and_then(Value::as_bool) != Some(true) {
        return Err(invalid_result(
            operation,
            "project check report does not say passed=true",
            output,
        ));
    }
    let project = required_string(&report, "project")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let startup_scene_id = required_string(&report, "startup_scene_id")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let scenes = required_u64(&report, "scenes")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let entities = required_u64(&report, "entities")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let declared_assets = required_u64(&report, "declared_assets")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let cooked_assets = required_u64(&report, "cooked_assets")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;

    Ok(EditorBuildResult::Validated(ProjectValidationResult {
        project,
        startup_scene_id,
        scenes,
        entities,
        declared_assets,
        cooked_assets,
        report,
        elapsed,
        output,
    }))
}

fn finish_cook_and_compile(
    manifest_path: &Path,
    elapsed: Duration,
    output: EditorBuildOutput,
) -> Result<EditorBuildResult, EditorBuildError> {
    let operation = EditorBuildOperationKind::CookAndCompile;
    let project = GameProject::load(manifest_path).map_err(|error| {
        invalid_result(
            operation,
            format!("built project can no longer be loaded: {error}"),
            output.clone(),
        )
    })?;
    if !project.cooked_assets.is_dir() {
        return Err(invalid_result(
            operation,
            format!(
                "cook command succeeded without producing {}",
                project.cooked_assets.display()
            ),
            output,
        ));
    }
    Ok(EditorBuildResult::CookedAndCompiled(CookAndCompileResult {
        project: project.manifest.name,
        manifest_path: project.manifest_path,
        cooked_assets: project.cooked_assets,
        scripts_configured: project.script_project.is_some(),
        elapsed,
        output,
    }))
}

fn finish_windows_package(
    version: String,
    allow_dirty: bool,
    release_root: PathBuf,
    elapsed: Duration,
    output: EditorBuildOutput,
) -> Result<EditorBuildResult, EditorBuildError> {
    let operation = EditorBuildOperationKind::PackageWindows;
    let stage_root = release_root.join(WINDOWS_PLATFORM);
    let release_manifest_path = stage_root.join("manifests/release.json");
    let release_manifest = read_json(&release_manifest_path)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    require_json_string(&release_manifest, "schema", RELEASE_METADATA_SCHEMA)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    require_json_string(&release_manifest, "release_id", &version)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    require_json_string(&release_manifest, "platform", WINDOWS_PLATFORM)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    require_json_string(&release_manifest, "backend", "vulkan")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let dirty = release_manifest
        .get("dirty")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            invalid_result(
                operation,
                "release metadata field 'dirty' is missing or is not a boolean",
                output.clone(),
            )
        })?;
    if dirty && !allow_dirty {
        return Err(invalid_result(
            operation,
            "release metadata is dirty although dirty packaging was not authorized",
            output,
        ));
    }

    let archive_path = release_root.join(format!("{WINDOWS_PLATFORM}.zip"));
    let symbols_archive_path = release_root.join(format!("{WINDOWS_PLATFORM}-symbols.zip"));
    let archive_sha256 = verify_checksum_sidecar(&archive_path)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let symbols_sha256 = verify_checksum_sidecar(&symbols_archive_path)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;

    Ok(EditorBuildResult::PackagedWindows(PackageWindowsResult {
        version,
        release_root,
        archive_path,
        archive_sha256,
        symbols_archive_path,
        symbols_sha256,
        release_manifest_path,
        dirty,
        elapsed,
        output,
    }))
}

fn invalid_result(
    operation: EditorBuildOperationKind,
    message: impl Into<String>,
    output: EditorBuildOutput,
) -> EditorBuildError {
    EditorBuildError::process(
        operation,
        EditorBuildFailureKind::InvalidResult,
        message,
        None,
        output,
    )
}

fn validate_release_version(version: &str) -> Result<(), String> {
    let bytes = version.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err("release version must contain 1..=64 ASCII characters".into());
    }
    if !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!(
            "invalid release version '{version}'; use an ASCII letter or digit first, followed by letters, digits, '.', '_' or '-'"
        ));
    }
    Ok(())
}

fn resolve_output_directory(
    project_root: &Path,
    requested: &Path,
    label: &str,
    require_project_local: bool,
) -> Result<PathBuf, String> {
    if requested.as_os_str().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("{label} must not contain '..' traversal"));
    }
    let requested_path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        project_root.join(requested)
    };
    let comparable =
        portable_windows_path(resolve_through_existing_ancestor(&requested_path, label)?);
    let project_root =
        portable_windows_path(std::fs::canonicalize(project_root).map_err(|error| {
            format!(
                "could not resolve project root {}: {error}",
                project_root.display()
            )
        })?);
    if comparable.parent().is_none() || comparable == project_root {
        return Err(format!(
            "{label} must be a dedicated directory, not a filesystem or project root"
        ));
    }
    if require_project_local && !comparable.starts_with(&project_root) {
        return Err(format!(
            "{label} must remain inside the project workspace for an installed engine: {}",
            requested.display()
        ));
    }
    if requested_path.is_file() {
        return Err(format!(
            "{label} points to a regular file: {}",
            requested_path.display()
        ));
    }
    Ok(comparable)
}

fn validate_installed_package_output(
    project: &GameProject,
    output_root: &Path,
    version: &str,
) -> Result<(), String> {
    let output_root = portable_windows_path(resolve_through_existing_ancestor(
        output_root,
        "package output root",
    )?);
    let release_root = portable_windows_path(resolve_through_existing_ancestor(
        &output_root.join(version),
        "package release directory",
    )?);
    let mut protected_directories = vec![
        ("cooked_assets", project.cooked_assets.clone()),
        ("managed script SDK", project.root.join("build/script-sdk")),
        (
            "managed script host",
            project.root.join("build/script-host"),
        ),
    ];
    if let Some(script_output) = project.script_assembly.as_deref().and_then(Path::parent) {
        protected_directories.push(("script_assembly output", script_output.to_path_buf()));
    }

    for (protected_label, protected_path) in protected_directories {
        let protected_path = portable_windows_path(resolve_through_existing_ancestor(
            &protected_path,
            protected_label,
        )?);
        for (candidate_label, candidate) in [
            ("package output root", output_root.as_path()),
            ("package release directory", release_root.as_path()),
        ] {
            if paths_overlap(candidate, &protected_path) {
                return Err(format!(
                    "{candidate_label} {} overlaps the project-owned {protected_label} directory {}; choose a dedicated package output directory",
                    candidate.display(),
                    protected_path.display()
                ));
            }
        }
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn resolve_through_existing_ancestor(path: &Path, label: &str) -> Result<PathBuf, String> {
    let mut existing = path.to_path_buf();
    let mut missing = Vec::<OsString>::new();
    loop {
        match std::fs::symlink_metadata(&existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name().ok_or_else(|| {
                    format!(
                        "{label} has no existing filesystem ancestor: {}",
                        path.display()
                    )
                })?;
                missing.push(name.to_os_string());
                existing = existing
                    .parent()
                    .ok_or_else(|| {
                        format!(
                            "{label} has no existing filesystem ancestor: {}",
                            path.display()
                        )
                    })?
                    .to_path_buf();
            }
            Err(error) => {
                return Err(format!(
                    "could not inspect {label} ancestor {}: {error}",
                    existing.display()
                ))
            }
        }
    }
    let mut resolved = std::fs::canonicalize(&existing).map_err(|error| {
        format!(
            "could not resolve {label} ancestor {}: {error}",
            existing.display()
        )
    })?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

#[cfg(windows)]
fn system_windows_executable(relative: &str) -> Result<PathBuf, String> {
    let system_root = std::env::var_os("SystemRoot")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "SystemRoot is not configured".to_string())?;
    canonical_file(
        PathBuf::from(system_root).join("System32").join(relative),
        relative,
    )
}

#[cfg(windows)]
fn system_powershell_executable() -> Result<PathBuf, String> {
    system_windows_executable(r"WindowsPowerShell\v1.0\powershell.exe")
}

#[cfg(not(windows))]
fn system_powershell_executable() -> Result<PathBuf, String> {
    Ok(PathBuf::from("powershell.exe"))
}

fn canonical_directory(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| format!("could not resolve {label} {}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "{label} is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(portable_windows_path(canonical))
}

fn canonical_file(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| format!("could not resolve {label} {}: {error}", path.display()))?;
    if !canonical.is_file() {
        return Err(format!(
            "{label} is not a regular file: {}",
            canonical.display()
        ));
    }
    Ok(portable_windows_path(canonical))
}

#[cfg(windows)]
fn portable_windows_path(path: PathBuf) -> PathBuf {
    let display = path.to_string_lossy();
    if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(ordinary) = display.strip_prefix(r"\\?\") {
        PathBuf::from(ordinary)
    } else {
        path
    }
}

#[cfg(not(windows))]
fn portable_windows_path(path: PathBuf) -> PathBuf {
    path
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read result {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("result is not valid JSON {}: {error}", path.display()))
}

fn require_json_string(value: &Value, field: &str, expected: &str) -> Result<(), String> {
    let actual = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("result field '{field}' is missing or is not a string"))?;
    if actual != expected {
        return Err(format!(
            "result field '{field}' is '{actual}', expected '{expected}'"
        ));
    }
    Ok(())
}

fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("result field '{field}' is missing or is not a string"))
}

fn required_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("result field '{field}' is missing or is not an unsigned integer"))
}

fn verify_checksum_sidecar(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err(format!("package artifact is missing: {}", path.display()));
    }
    let mut sidecar = path.as_os_str().to_os_string();
    sidecar.push(".sha256");
    let sidecar = PathBuf::from(sidecar);
    let contents = std::fs::read_to_string(&sidecar)
        .map_err(|error| format!("could not read checksum {}: {error}", sidecar.display()))?;
    let mut fields = contents.split_whitespace();
    let expected_hash = fields
        .next()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| format!("checksum file is invalid: {}", sidecar.display()))?
        .to_ascii_lowercase();
    let expected_name = path.file_name().and_then(OsStr::to_str).ok_or_else(|| {
        format!(
            "package artifact has no portable file name: {}",
            path.display()
        )
    })?;
    let sidecar_name = fields
        .next()
        .ok_or_else(|| format!("checksum file has no artifact name: {}", sidecar.display()))?;
    if sidecar_name != expected_name || fields.next().is_some() {
        return Err(format!(
            "checksum file does not name exactly '{expected_name}': {}",
            sidecar.display()
        ));
    }

    let mut file = File::open(path)
        .map_err(|error| format!("could not read package {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("could not hash package {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let actual_hash = format!("{:x}", hasher.finalize());
    if actual_hash != expected_hash {
        return Err(format!(
            "package checksum mismatch for {}: expected {expected_hash}, got {actual_hash}",
            path.display()
        ));
    }
    Ok(actual_hash)
}

#[derive(Default)]
struct CapturedBytes {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CapturedBytes {
    fn append(&mut self, incoming: &[u8]) {
        if incoming.len() >= CAPTURE_LIMIT_BYTES {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&incoming[incoming.len() - CAPTURE_LIMIT_BYTES..]);
            self.truncated = true;
            return;
        }
        let overflow = self
            .bytes
            .len()
            .saturating_add(incoming.len())
            .saturating_sub(CAPTURE_LIMIT_BYTES);
        if overflow > 0 {
            self.bytes.drain(..overflow);
            self.truncated = true;
        }
        self.bytes.extend_from_slice(incoming);
    }

    fn snapshot(&self) -> (String, bool) {
        (
            String::from_utf8_lossy(&self.bytes).into_owned(),
            self.truncated,
        )
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    reader: Option<R>,
    destination: Arc<Mutex<CapturedBytes>>,
) -> Option<thread::JoinHandle<()>> {
    let mut reader = reader?;
    Some(thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            let Ok(count) = reader.read(&mut buffer) else {
                break;
            };
            if count == 0 {
                break;
            }
            if let Ok(mut destination) = destination.lock() {
                destination.append(&buffer[..count]);
            } else {
                break;
            }
        }
    }))
}

fn output_snapshot(
    stdout: &Arc<Mutex<CapturedBytes>>,
    stderr: &Arc<Mutex<CapturedBytes>>,
) -> EditorBuildOutput {
    let (stdout, stdout_truncated) = stdout
        .lock()
        .map(|stream| stream.snapshot())
        .unwrap_or_else(|_| ("<stdout capture unavailable>".into(), true));
    let (stderr, stderr_truncated) = stderr
        .lock()
        .map(|stream| stream.snapshot())
        .unwrap_or_else(|_| ("<stderr capture unavailable>".into(), true));
    EditorBuildOutput {
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    }
}

fn wait_for_child(child: &Arc<Mutex<Option<Child>>>) -> Result<ExitStatus, String> {
    loop {
        let status = {
            let mut child = child
                .lock()
                .map_err(|_| "build process state is poisoned".to_string())?;
            let Some(process) = child.as_mut() else {
                return Err("build process handle disappeared before completion".into());
            };
            process
                .try_wait()
                .map_err(|error| format!("could not wait for build process: {error}"))?
        };
        if let Some(status) = status {
            let mut child = child
                .lock()
                .map_err(|_| "build process state is poisoned".to_string())?;
            child.take();
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn hide_child_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_child_window(_command: &mut Command) {}

#[cfg(windows)]
fn terminate_child_tree(child: &mut Child) -> Result<(), String> {
    let taskkill = system_windows_executable("taskkill.exe")?;
    let mut command = Command::new(taskkill);
    command
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_child_window(&mut command);
    let status = command
        .status()
        .map_err(|error| format!("could not start taskkill for build cancellation: {error}"))?;
    let terminated = status.success()
        || child
            .try_wait()
            .map_err(|error| format!("could not query build process after taskkill: {error}"))?
            .is_some();
    if terminated {
        Ok(())
    } else {
        Err(format!(
            "taskkill could not terminate build process tree {} (exit code {:?})",
            child.id(),
            status.code()
        ))
    }
}

#[cfg(not(windows))]
fn terminate_child_tree(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|error| format!("could not terminate build process: {error}"))
}

fn tail_chars(text: &str, maximum: usize) -> String {
    let count = text.chars().count();
    if count <= maximum {
        text.to_string()
    } else {
        text.chars().skip(count - maximum).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_asset::project::ProjectManifest;
    use std::io::Write;

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf()
    }

    fn project_fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir_all(root.join("assets/source")).unwrap();
        std::fs::create_dir_all(root.join("assets/scenes")).unwrap();
        std::fs::create_dir_all(root.join("build/cooked")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join("assets/scenes/main.scene.ron"),
            "(scene_id:\"main\",name:\"Main\",entities:[],dependencies:[])\n",
        )
        .unwrap();
        std::fs::write(
            root.join("config/input.actions.json"),
            crate::project_input::starter_input_json(),
        )
        .unwrap();
        let manifest = ProjectManifest::new("Build Service Test");
        let manifest_path = manifest.write_to_root(&root).unwrap();
        (directory, manifest_path)
    }

    fn scripted_project_fixture() -> (tempfile::TempDir, PathBuf) {
        let (directory, manifest_path) = project_fixture();
        let root = manifest_path.parent().unwrap();
        let script_project = root.join("scripts/GameScripts/GameScripts.csproj");
        std::fs::create_dir_all(script_project.parent().unwrap()).unwrap();
        std::fs::write(&script_project, "<Project />\n").unwrap();
        let mut manifest = ProjectManifest::new("Scripted Build Service Test");
        manifest.script_project = Some(PathBuf::from("scripts/GameScripts/GameScripts.csproj"));
        manifest.script_assembly = Some(PathBuf::from("build/scripts/GameScripts.dll"));
        let manifest_path = manifest.write_to_root(root).unwrap();
        (directory, manifest_path)
    }

    fn service() -> EditorBuildService {
        EditorBuildService::with_powershell(
            workspace_root(),
            std::env::current_exe().unwrap(),
            PathBuf::from("powershell.exe"),
        )
        .unwrap()
    }

    fn installed_service() -> (tempfile::TempDir, EditorBuildService) {
        use crate::engine_installation::{
            EngineInstallation, EngineInstallationManifest, ENGINE_INSTALLATION_FILE_NAME,
            ENGINE_INSTALLATION_SCHEMA,
        };
        use std::collections::BTreeMap;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("installed-engine");
        for folder in [
            "bin",
            "runtime/windows-x86_64",
            "tools",
            "sdk",
            "sdk/script-host",
        ] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        let files = [
            "bin/EngineEditor.exe",
            "runtime/windows-x86_64/GameRuntime.exe",
            "runtime/windows-x86_64/GameRuntime.pdb",
            "tools/asset-cook.exe",
            "tools/package-windows.ps1",
            "sdk/EngineGameplay.dll",
            "sdk/script-host/EngineScriptHost.exe",
            "THIRD_PARTY_NOTICES.txt",
        ];
        for path in files {
            std::fs::write(root.join(path), path.as_bytes()).unwrap();
        }
        let hashes = files
            .into_iter()
            .map(|path| {
                (
                    path.to_string(),
                    format!(
                        "{:x}",
                        Sha256::digest(std::fs::read(root.join(path)).unwrap())
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let manifest = EngineInstallationManifest {
            schema: ENGINE_INSTALLATION_SCHEMA.into(),
            engine_version: "v-test".into(),
            editor: "bin/EngineEditor.exe".into(),
            windows_runtime: "runtime/windows-x86_64/GameRuntime.exe".into(),
            windows_symbols: "runtime/windows-x86_64/GameRuntime.pdb".into(),
            asset_cooker: "tools/asset-cook.exe".into(),
            package_script: "tools/package-windows.ps1".into(),
            managed_sdk: "sdk/EngineGameplay.dll".into(),
            script_host: "sdk/script-host".into(),
            notices: "THIRD_PARTY_NOTICES.txt".into(),
            script_api: engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA.into(),
            script_api_version: engine_script_api::GAMEPLAY_SCRIPT_API_VERSION.into(),
            script_api_sha256: "a".repeat(64),
            source_commit: "fixture".into(),
            source_date_epoch: 1_700_000_000,
            rustc: "rustc fixture".into(),
            files: hashes,
        };
        std::fs::write(
            root.join(ENGINE_INSTALLATION_FILE_NAME),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let installation = EngineInstallation::load(root).unwrap();
        let service =
            EditorBuildService::with_installed_powershell(installation, "powershell.exe").unwrap();
        (directory, service)
    }

    fn argument_strings(plan: &ProcessPlan) -> Vec<String> {
        plan.arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn release_versions_match_the_authoritative_script_contract() {
        let maximum_version = "a".repeat(64);
        for valid in ["1", "v1.2.3", "nightly_2026-07-18", &maximum_version] {
            assert!(validate_release_version(valid).is_ok(), "{valid}");
        }
        let oversized_version = "a".repeat(65);
        for invalid in [
            "",
            ".hidden",
            "two words",
            "a/b",
            "版本",
            &oversized_version,
        ] {
            assert!(validate_release_version(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn package_plan_calls_the_official_script_without_allow_dirty_by_default() {
        let (directory, manifest_path) = project_fixture();
        let output = directory.path().join("release-output");
        let plan = service()
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(PackageWindowsOptions::new("v1.2.3", &output)),
            )
            .unwrap();
        let arguments = argument_strings(&plan);

        assert_eq!(plan.kind, EditorBuildOperationKind::PackageWindows);
        assert!(arguments.iter().any(|argument| {
            argument
                .replace('\\', "/")
                .ends_with("/.github/scripts/package-windows.ps1")
        }));
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-ProjectPath"
                && Path::new(&arguments[1]).file_name() == Some(OsStr::new("game.project.json"))
        }));
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-OutputRoot" && Path::new(&arguments[1]) == output
        }));
        assert!(!arguments.iter().any(|argument| argument == "-AllowDirty"));
        assert!(!arguments.iter().any(|argument| argument == "-SkipSmoke"));
    }

    #[test]
    fn installed_package_plan_uses_project_local_output_and_prebuilt_toolchain() {
        let (_installation, service) = installed_service();
        let (_project, manifest_path) = project_fixture();
        let project_root = manifest_path.parent().unwrap();
        let plan = service
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                    "v-installed",
                    "Dist",
                )),
            )
            .unwrap();
        let arguments = argument_strings(&plan);
        assert_eq!(plan.working_directory, project_root);
        assert!(arguments.iter().any(|argument| {
            argument
                .replace('\\', "/")
                .ends_with("/tools/package-windows.ps1")
        }));
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-EngineInstallRoot"
                && Path::new(&arguments[1])
                    .join("engine.installation.json")
                    .is_file()
        }));
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-OutputRoot" && Path::new(&arguments[1]) == project_root.join("Dist")
        }));
        assert!(!arguments.iter().any(|argument| {
            matches!(
                argument.as_str(),
                "-CargoTargetDir" | "-SkipBuild" | "-AllowDirty"
            )
        }));

        let outside_output = project_root.parent().unwrap().join("outside-dist");
        let error = service
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                    "v-outside",
                    outside_output,
                )),
            )
            .unwrap_err();
        assert_eq!(error.kind, EditorBuildFailureKind::InvalidRequest);
        assert!(error.message.contains("inside the project workspace"));
    }

    #[test]
    fn installed_package_plan_rejects_project_owned_build_directories() {
        let (_installation, service) = installed_service();
        let (_project, manifest_path) = scripted_project_fixture();
        let project_root = manifest_path.parent().unwrap();

        for (label, output) in [
            ("cooked_assets", "build/cooked/package-output"),
            ("script_assembly output", "build/scripts/package-output"),
            ("managed script SDK", "build/script-sdk/package-output"),
            ("managed script host", "build/script-host/package-output"),
        ] {
            let error = service
                .plan(
                    &manifest_path,
                    EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                        "v-conflict",
                        output,
                    )),
                )
                .unwrap_err();
            assert_eq!(error.kind, EditorBuildFailureKind::InvalidRequest);
            assert!(
                error.message.contains(label),
                "expected {label:?} in error for {output:?}: {}",
                error.message
            );
        }

        let safe = service
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                    "v-safe",
                    "build/releases",
                )),
            )
            .unwrap();
        let arguments = argument_strings(&safe);
        assert!(arguments.windows(2).any(|arguments| {
            arguments[0] == "-OutputRoot"
                && Path::new(&arguments[1]) == project_root.join("build/releases")
        }));
    }

    #[test]
    fn dirty_and_skip_switches_require_explicit_options() {
        let (directory, manifest_path) = project_fixture();
        let mut options =
            PackageWindowsOptions::new("local-dry-run", directory.path().join("release-output"));
        options.allow_dirty = true;
        options.skip_build = true;
        options.skip_smoke = true;
        let plan = service()
            .plan(
                &manifest_path,
                EditorBuildOperation::PackageWindows(options),
            )
            .unwrap();
        let arguments = argument_strings(&plan);

        assert!(arguments.iter().any(|argument| argument == "-AllowDirty"));
        assert!(arguments.iter().any(|argument| argument == "-SkipBuild"));
        assert!(arguments.iter().any(|argument| argument == "-SkipSmoke"));
    }

    #[test]
    fn validate_and_cook_plans_use_distinct_formal_cli_commands() {
        let (_directory, manifest_path) = project_fixture();
        let project_root = manifest_path.parent().unwrap();
        let validate = service()
            .plan(&manifest_path, EditorBuildOperation::Validate)
            .unwrap();
        let cook = service()
            .plan(&manifest_path, EditorBuildOperation::CookAndCompile)
            .unwrap();
        let validate_arguments = argument_strings(&validate);
        let cook_arguments = argument_strings(&cook);

        assert_eq!(&validate_arguments[..2], &["project", "check"]);
        assert!(validate_arguments
            .iter()
            .any(|argument| argument == "--report"));
        assert_eq!(&cook_arguments[..2], &["project", "build"]);
        assert!(!cook_arguments.iter().any(|argument| argument == "--report"));
        assert_eq!(validate.working_directory, project_root);
        assert_eq!(cook.working_directory, project_root);
    }

    #[test]
    fn unsafe_package_roots_and_versions_are_rejected_before_spawn() {
        let (_directory, manifest_path) = project_fixture();
        let service = service();
        let bad_version = service.plan(
            &manifest_path,
            EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                "../escape",
                "artifacts/release",
            )),
        );
        assert_eq!(
            bad_version.unwrap_err().kind,
            EditorBuildFailureKind::InvalidRequest
        );

        let project_root = manifest_path.parent().unwrap();
        let root_output = service.plan(
            &manifest_path,
            EditorBuildOperation::PackageWindows(PackageWindowsOptions::new(
                "safe-version",
                project_root,
            )),
        );
        assert_eq!(
            root_output.unwrap_err().kind,
            EditorBuildFailureKind::InvalidRequest
        );
    }

    #[test]
    fn validation_completion_requires_the_expected_report_contract() {
        let directory = tempfile::tempdir().unwrap();
        let report = directory.path().join("report.json");
        std::fs::write(
            &report,
            serde_json::to_vec(&serde_json::json!({
                "schema": PROJECT_CHECK_SCHEMA,
                "passed": true,
                "project": "Fixture",
                "startup_scene_id": "main",
                "scenes": 1,
                "entities": 2,
                "declared_assets": 3,
                "cooked_assets": 3
            }))
            .unwrap(),
        )
        .unwrap();

        let result = finish_validation(
            &report,
            Duration::from_millis(10),
            EditorBuildOutput::default(),
        )
        .unwrap();
        let EditorBuildResult::Validated(result) = result else {
            panic!("wrong result kind");
        };
        assert_eq!(result.project, "Fixture");
        assert_eq!(result.declared_assets, 3);

        std::fs::write(&report, br#"{"schema":"wrong","passed":true}"#).unwrap();
        let error =
            finish_validation(&report, Duration::ZERO, EditorBuildOutput::default()).unwrap_err();
        assert_eq!(error.kind, EditorBuildFailureKind::InvalidResult);
    }

    #[test]
    fn package_completion_verifies_metadata_and_both_checksums() {
        let directory = tempfile::tempdir().unwrap();
        let release_root = directory.path().join("v9");
        let manifest_directory = release_root.join(WINDOWS_PLATFORM).join("manifests");
        std::fs::create_dir_all(&manifest_directory).unwrap();
        std::fs::write(
            manifest_directory.join("release.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": RELEASE_METADATA_SCHEMA,
                "release_id": "v9",
                "platform": WINDOWS_PLATFORM,
                "backend": "vulkan",
                "dirty": false
            }))
            .unwrap(),
        )
        .unwrap();
        for name in [
            format!("{WINDOWS_PLATFORM}.zip"),
            format!("{WINDOWS_PLATFORM}-symbols.zip"),
        ] {
            let path = release_root.join(&name);
            std::fs::write(&path, name.as_bytes()).unwrap();
            let hash = format!("{:x}", Sha256::digest(name.as_bytes()));
            let mut sidecar = File::create(format!("{}.sha256", path.display())).unwrap();
            writeln!(sidecar, "{hash}  {name}").unwrap();
        }

        let result = finish_windows_package(
            "v9".into(),
            false,
            release_root,
            Duration::from_secs(1),
            EditorBuildOutput::default(),
        )
        .unwrap();
        let EditorBuildResult::PackagedWindows(result) = result else {
            panic!("wrong result kind");
        };
        assert_eq!(result.version, "v9");
        assert!(!result.dirty);
        assert_eq!(result.archive_sha256.len(), 64);
        assert_eq!(result.symbols_sha256.len(), 64);
    }

    #[test]
    fn bounded_capture_keeps_the_diagnostic_tail() {
        let mut capture = CapturedBytes::default();
        capture.append(&vec![b'a'; CAPTURE_LIMIT_BYTES]);
        capture.append(b"final-error");
        let (text, truncated) = capture.snapshot();
        assert!(truncated);
        assert_eq!(text.len(), CAPTURE_LIMIT_BYTES);
        assert!(text.ends_with("final-error"));
    }

    #[test]
    fn displayed_process_error_contains_stderr_tail() {
        let error = EditorBuildError::process(
            EditorBuildOperationKind::PackageWindows,
            EditorBuildFailureKind::ProcessFailed,
            "authoritative build process failed",
            Some(7),
            EditorBuildOutput {
                stderr: "release packaging requires a clean worktree".into(),
                ..EditorBuildOutput::default()
            },
        );
        let display = error.to_string();
        assert!(display.contains("exit code 7"));
        assert!(display.contains("clean worktree"));
    }

    /// Marker line the child-process helper prints so the parent test can
    /// confirm stdout capture flows through the background-task machinery.
    const CHILD_HELPER_STDOUT_MARKER: &str = "editor-build-task-test";

    /// Child-process entry point spawned by
    /// `background_task_exposes_output_completion_and_finished_cancellation`.
    ///
    /// Ignored during normal test runs: the parent test re-invokes this test
    /// binary with `--exact`/`--ignored` so the child prints a marker line
    /// and exits successfully. Spawning the current test executable keeps the
    /// background-task test hermetic — it no longer depends on PowerShell,
    /// `/bin/sh`, or any other system tool being installed (ENG-71).
    #[test]
    #[ignore = "child-process helper for the background-task test"]
    fn child_process_helper_prints_marker_and_exits() {
        println!("{CHILD_HELPER_STDOUT_MARKER}");
    }

    #[test]
    fn background_task_exposes_output_completion_and_finished_cancellation() {
        let (_directory, manifest_path) = project_fixture();
        // Spawn the current test executable in its helper mode: the binary
        // always exists on every platform and needs no system shell, so the
        // test exercises the spawn/output/cancellation machinery without
        // conflating code correctness with environment contents.
        let executable = std::env::current_exe().expect("current test executable");
        let arguments = vec![
            OsString::from("--exact"),
            OsString::from("editor_build_ops::tests::child_process_helper_prints_marker_and_exits"),
            OsString::from("--ignored"),
            OsString::from("--nocapture"),
        ];
        let plan = ProcessPlan {
            kind: EditorBuildOperationKind::CookAndCompile,
            executable,
            arguments,
            working_directory: workspace_root(),
            completion: CompletionPlan::CookAndCompile { manifest_path },
        };
        let mut task = EditorBuildTask::spawn(plan).unwrap();
        assert_eq!(task.operation(), EditorBuildOperationKind::CookAndCompile);
        let deadline = Instant::now() + Duration::from_secs(30);
        let result = loop {
            if let Some(result) = task.try_complete() {
                break result.unwrap();
            }
            assert!(Instant::now() < deadline, "background task timed out");
            thread::sleep(Duration::from_millis(10));
        };
        let EditorBuildResult::CookedAndCompiled(result) = result else {
            panic!("wrong result kind");
        };
        assert_eq!(result.project, "Build Service Test");
        assert!(task
            .output_snapshot()
            .stdout
            .contains(CHILD_HELPER_STDOUT_MARKER));
        assert!(!task.cancel().unwrap());
    }

    #[test]
    fn current_editor_service_rejects_a_missing_project_before_spawn() {
        let service = EditorBuildService::for_current_editor().unwrap();
        let missing = tempfile::tempdir().unwrap().path().join("missing-project");
        let error = match service.start(&missing, EditorBuildOperation::Validate) {
            Ok(_) => panic!("missing project unexpectedly started"),
            Err(error) => error,
        };
        assert_eq!(error.kind, EditorBuildFailureKind::InvalidRequest);
    }
}
