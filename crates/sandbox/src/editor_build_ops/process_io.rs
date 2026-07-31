use super::*;

#[derive(Default)]
pub(super) struct CapturedBytes {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CapturedBytes {
    pub(super) fn append(&mut self, incoming: &[u8]) {
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

    pub(super) fn snapshot(&self) -> (String, bool) {
        (
            String::from_utf8_lossy(&self.bytes).into_owned(),
            self.truncated,
        )
    }
}

pub(super) fn spawn_reader<R: Read + Send + 'static>(
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

pub(super) fn output_snapshot(
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

pub(super) fn wait_for_child(child: &Arc<Mutex<Option<Child>>>) -> Result<ExitStatus, String> {
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
pub(super) fn hide_child_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(super) fn hide_child_window(_command: &mut Command) {}

#[cfg(windows)]
pub(super) fn terminate_child_tree(child: &mut Child) -> Result<(), String> {
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
pub(super) fn terminate_child_tree(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|error| format!("could not terminate build process: {error}"))
}

pub(super) fn tail_chars(text: &str, maximum: usize) -> String {
    let count = text.chars().count();
    if count <= maximum {
        text.to_string()
    } else {
        text.chars().skip(count - maximum).collect()
    }
}
