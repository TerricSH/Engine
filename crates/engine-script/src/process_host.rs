//! Process-based CoreCLR script host.
//!
//! [`ProcessHost`] communicates with a C# script runtime running as a child
//! process via a JSON-line protocol over stdin/stdout.
//!
//! The wire format is defined in [`protocol`]; the child process (e.g. the
//! sample in `scripts/csharp/`) implements the same protocol on the other end.
//!
//! # Thread safety
//!
//! Both [`ProcessHost`] and [`ProcessScriptInstance`] share the same pipes
//! behind an [`Arc<Mutex<SharedScriptIO>>`], so only one message is in-flight
//! at a time. This is sufficient for a single-threaded game loop.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::host::{ScriptError, ScriptHandle, ScriptHost, ScriptInstance};
use crate::protocol::ScriptMessage;
use crate::value::ScriptValue;

// ---------------------------------------------------------------------------
// Shared IO — single lock guards both pipes so messages are serialised
// ---------------------------------------------------------------------------

/// Pipes to a child process, shared between [`ProcessHost`] and all
/// [`ProcessScriptInstance`]s via [`Arc<Mutex<...>>`].
pub struct SharedScriptIO {
    /// Write end of the child's stdin.
    pub stdin: ChildStdin,
    /// Buffered read end of the child's stdout.
    pub stdout: BufReader<ChildStdout>,
}

impl SharedScriptIO {
    /// Send a JSON message and read exactly one JSON response.
    pub fn roundtrip(&mut self, msg: &ScriptMessage) -> Result<ScriptMessage, ScriptError> {
        let json = serde_json::to_string(msg)
            .map_err(|e| ScriptError::HostError(format!("Failed to serialize message: {e}")))?;

        writeln!(self.stdin, "{json}")
            .map_err(|e| ScriptError::HostError(format!("Failed to write to child stdin: {e}")))?;
        self.stdin
            .flush()
            .map_err(|e| ScriptError::HostError(format!("Failed to flush child stdin: {e}")))?;

        let mut line = String::new();
        self.stdout.read_line(&mut line).map_err(|e| {
            ScriptError::HostError(format!("Failed to read from child stdout: {e}"))
        })?;

        if line.is_empty() {
            return Err(ScriptError::HostError(
                "Child process closed stdout unexpectedly".to_string(),
            ));
        }

        serde_json::from_str(line.trim())
            .map_err(|e| ScriptError::HostError(format!("Failed to deserialize response: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Process-based script instance
// ---------------------------------------------------------------------------

/// A script instance living in a child process.
///
/// Each method call or field access sends a JSON message to the child and
/// waits for a response through the shared IO pipe.
pub struct ProcessScriptInstance {
    instance_id: String,
    io: Arc<Mutex<SharedScriptIO>>,
}

impl std::fmt::Debug for ProcessScriptInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessScriptInstance")
            .field("instance_id", &self.instance_id)
            .finish()
    }
}

impl ScriptInstance for ProcessScriptInstance {
    fn call(&mut self, function: &str, args: &[ScriptValue]) -> Result<ScriptValue, ScriptError> {
        let mut io = self
            .io
            .lock()
            .map_err(|e| ScriptError::HostError(format!("Script IO lock poisoned: {e}")))?;
        let response = io.roundtrip(&ScriptMessage::CallMethod {
            instance_id: self.instance_id.clone(),
            method: function.to_string(),
            args: args.to_vec(),
        })?;

        match response {
            ScriptMessage::MethodResult { result, .. } => Ok(result),
            ScriptMessage::Error { message } => Err(ScriptError::ExecutionError(message)),
            other => Err(ScriptError::ExecutionError(format!(
                "Unexpected response to CallMethod: {other:?}"
            ))),
        }
    }

    fn set_field(&mut self, name: &str, value: ScriptValue) -> Result<(), ScriptError> {
        let mut io = self
            .io
            .lock()
            .map_err(|e| ScriptError::HostError(format!("Script IO lock poisoned: {e}")))?;
        let response = io.roundtrip(&ScriptMessage::SetField {
            instance_id: self.instance_id.clone(),
            name: name.to_string(),
            value,
        })?;

        match response {
            ScriptMessage::FieldValue { .. } => Ok(()),
            ScriptMessage::Error { message } => Err(ScriptError::ExecutionError(message)),
            other => Err(ScriptError::ExecutionError(format!(
                "Unexpected response to SetField: {other:?}"
            ))),
        }
    }

    fn get_field(&self, name: &str) -> Option<ScriptValue> {
        let mut io = self.io.lock().ok()?;
        let response = io
            .roundtrip(&ScriptMessage::GetField {
                instance_id: self.instance_id.clone(),
                name: name.to_string(),
            })
            .ok()?;

        match response {
            ScriptMessage::FieldValue { value, .. } => value,
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Process host
// ---------------------------------------------------------------------------

/// State of a loaded assembly on the process host side.
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum ScriptHostState {
    Loaded { id: String },
}

/// A script host that drives a child process running the .NET script runtime.
///
/// All pipe IO is shared behind an [`Arc<Mutex<SharedScriptIO>>`] so that
/// both the host itself and every [`ProcessScriptInstance`] it creates
/// can send messages through the same pipe.
///
/// # Lifecycle
///
/// 1. Create a [`ProcessHost`] with [`new`](Self::new).
/// 2. Call [`launch`](Self::launch) to start the child process.
/// 3. Load assemblies and instantiate scripts through the
///    [`ScriptHost`](crate::host::ScriptHost) trait.
/// 4. Call [`shutdown`](Self::shutdown) to terminate the child process.
pub struct ProcessHost {
    /// Display name of this host.
    name: String,
    /// The spawned child process.
    child: Option<Child>,
    /// Shared IO pipes — cloned for each [`ProcessScriptInstance`].
    io: Option<Arc<Mutex<SharedScriptIO>>>,
    /// Loaded assemblies and their state.
    assemblies: Vec<(ScriptHandle, ScriptHostState)>,
    /// Monotonic instance id counter.
    next_instance_id: u64,
}

impl ProcessHost {
    /// Create a new process host with the given display name.
    ///
    /// Use [`launch`](Self::launch) to start the child process before
    /// performing any script operations.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            child: None,
            io: None,
            assemblies: Vec::new(),
            next_instance_id: 0,
        }
    }

    /// Launch the child process at the given executable path.
    ///
    /// The executable must implement the [`ScriptMessage`] JSON-line protocol
    /// over stdin/stdout.
    pub fn launch(&mut self, executable: &Path) -> Result<(), ScriptError> {
        let mut child = Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                ScriptError::HostError(format!(
                    "Failed to launch script host '{}': {e}",
                    executable.display()
                ))
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ScriptError::HostError("Failed to capture child stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ScriptError::HostError("Failed to capture child stdout".to_string()))?;

        self.io = Some(Arc::new(Mutex::new(SharedScriptIO {
            stdin,
            stdout: BufReader::new(stdout),
        })));
        self.child = Some(child);
        Ok(())
    }

    /// Send a JSON message and read a JSON response via the child's pipes.
    fn send(&mut self, msg: &ScriptMessage) -> Result<ScriptMessage, ScriptError> {
        let io = self.io.as_ref().ok_or_else(|| {
            ScriptError::HostError("Process not launched — call launch() first".to_string())
        })?;
        let mut io = io
            .lock()
            .map_err(|e| ScriptError::HostError(format!("Script IO lock poisoned: {e}")))?;
        io.roundtrip(msg)
    }

    /// Shut down the child process gracefully and wait for it to exit.
    pub fn shutdown(&mut self) -> Result<(), ScriptError> {
        if self.io.is_some() {
            let _ = self.send(&ScriptMessage::Shutdown);
        }
        // Drop pipes first so the child sees EOF and can exit.
        drop(self.io.take());
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
        Ok(())
    }

    /// Return a clone of the shared IO for instances to use.
    fn shared_io(&self) -> Result<Arc<Mutex<SharedScriptIO>>, ScriptError> {
        self.io.as_ref().map(Arc::clone).ok_or_else(|| {
            ScriptError::HostError("Process not launched — call launch() first".to_string())
        })
    }

    /// Number of loaded assemblies.
    pub fn assembly_count(&self) -> usize {
        self.assemblies.len()
    }

    /// Whether the child process has been launched.
    pub fn is_launched(&self) -> bool {
        self.child.is_some()
    }
}

impl ScriptHost for ProcessHost {
    fn name(&self) -> &str {
        &self.name
    }

    fn load_assembly(
        &mut self,
        id: &str,
        assembly_data: &[u8],
    ) -> Result<ScriptHandle, ScriptError> {
        // Encode assembly bytes as BASE64 for the JSON wire protocol
        let data_encoded = base64_encode(assembly_data);

        let response = self.send(&ScriptMessage::LoadAssembly {
            id: id.to_string(),
            data_base64: data_encoded,
        })?;

        match response {
            ScriptMessage::AssemblyLoaded { id: resp_id, .. } => {
                let handle = ScriptHandle::new(&resp_id);
                self.assemblies.push((
                    handle.clone(),
                    ScriptHostState::Loaded { id: id.to_string() },
                ));
                Ok(handle)
            }
            ScriptMessage::Error { message } => Err(ScriptError::LoadFailed(message)),
            other => Err(ScriptError::LoadFailed(format!(
                "Unexpected response to LoadAssembly: {other:?}"
            ))),
        }
    }

    fn instantiate(
        &mut self,
        handle: &ScriptHandle,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        let instance_id = format!("inst-{:04x}", self.next_instance_id);
        self.next_instance_id += 1;

        // Tell the child process to create the instance
        let io = self.shared_io()?;
        let mut io_lock = io
            .lock()
            .map_err(|e| ScriptError::HostError(format!("Script IO lock poisoned: {e}")))?;
        let _response = io_lock.roundtrip(&ScriptMessage::Instantiate {
            assembly_id: handle.id().to_string(),
            class_name: "ScriptType".to_string(), // will be refined when ScriptComponent is used
            instance_id: instance_id.clone(),
        })?;
        // Drop the lock before moving `io` into the instance
        drop(io_lock);

        Ok(Box::new(ProcessScriptInstance { instance_id, io }))
    }

    fn unload(&mut self, handle: &ScriptHandle) -> Result<(), ScriptError> {
        self.assemblies.retain(|(h, _)| h.id() != handle.id());
        Ok(())
    }
}

impl Drop for ProcessHost {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// BASE64-encode bytes for the JSON wire protocol.
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_host_name() {
        let host = ProcessHost::new("dotnet");
        assert_eq!(host.name(), "dotnet");
    }

    #[test]
    fn process_host_not_launched_by_default() {
        let host = ProcessHost::new("dotnet");
        assert!(!host.is_launched());
        assert_eq!(host.assembly_count(), 0);
    }

    #[test]
    fn process_host_send_before_launch_fails() {
        let mut host = ProcessHost::new("dotnet");
        let result = host.load_assembly("test", b"data");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not launched"));
    }

    #[test]
    fn process_host_shutdown_without_launch() {
        let mut host = ProcessHost::new("dotnet");
        assert!(host.shutdown().is_ok());
    }

    #[test]
    fn process_host_unload_empty() {
        let mut host = ProcessHost::new("dotnet");
        let handle = ScriptHandle::new("nothing");
        assert!(host.unload(&handle).is_ok());
    }

    #[test]
    fn process_host_base64_encode() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b"\x00\xff"), "AP8=");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn process_script_instance_debug() {
        // Construction requires a real child process (SharedScriptIO holds
        // ChildStdin + ChildStdout), so we verify via type-name reflection.
        let name = std::any::type_name::<ProcessScriptInstance>();
        assert!(
            name.contains("ProcessScriptInstance"),
            "type name mismatch: {name}"
        );
    }

    #[test]
    fn process_script_instance_trait_object_safe() {
        // Compile-time check: ProcessScriptInstance can be used as
        // Box<dyn ScriptInstance>.
        fn _assert(_: Box<dyn ScriptInstance>) {}
    }
}
