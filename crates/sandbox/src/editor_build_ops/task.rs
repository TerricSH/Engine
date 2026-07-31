use super::*;

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
    pub(super) fn spawn(plan: ProcessPlan) -> Result<Self, EditorBuildError> {
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
pub(super) struct ProcessPlan {
    pub(super) kind: EditorBuildOperationKind,
    pub(super) executable: PathBuf,
    pub(super) arguments: Vec<OsString>,
    pub(super) working_directory: PathBuf,
    pub(super) completion: CompletionPlan,
}

#[derive(Debug)]
pub(super) enum CompletionPlan {
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
