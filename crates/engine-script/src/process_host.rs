//! Process-based CoreCLR script host (stub).
//!
//! [`ProcessHost`] communicates with a C# script runtime running as a child
//! process via a JSON-line protocol over stdin/stdout. This is a **stub**
//! implementation that demonstrates the architecture but does not drive an
//! actual child process — real in-process CoreCLR hosting would require native
//! `hostfxr` / `nethost` FFI which is out of scope for the initial milestone.
//!
//! The wire format is defined in [`protocol`]; the child process (e.g. the
//! sample in `scripts/csharp/`) implements the same protocol on the other end.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use crate::host::{ScriptError, ScriptHandle, ScriptHost, ScriptInstance};
use crate::protocol::ScriptMessage;
use crate::value::ScriptValue;

// ---------------------------------------------------------------------------
// Process-based script instance
// ---------------------------------------------------------------------------

/// A script instance living in a child process.
///
/// Each method call or field access sends a JSON message to the child and
/// waits for a response.
pub struct ProcessScriptInstance {
    instance_id: String,
    sender: Box<dyn FnMut(&ScriptMessage) -> Result<ScriptMessage, ScriptError> + Send>,
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
        let response = (self.sender)(&ScriptMessage::CallMethod {
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
        let response = (self.sender)(&ScriptMessage::SetField {
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

    fn get_field(&self, _name: &str) -> Option<ScriptValue> {
        // Requires mutable sender; real impl would use shared state.
        None
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
    /// Pipe to the child's stdin.
    stdin: Option<ChildStdin>,
    /// Buffered reader for the child's stdout.
    stdout_reader: Option<BufReader<ChildStdout>>,
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
            stdin: None,
            stdout_reader: None,
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

        let stdin = child.stdin.take();
        let stdout_reader = child.stdout.take().map(BufReader::new);
        self.child = Some(child);
        self.stdin = stdin;
        self.stdout_reader = stdout_reader;
        Ok(())
    }

    /// Send a JSON message and read a JSON response via the child's pipes.
    fn send(&mut self, msg: &ScriptMessage) -> Result<ScriptMessage, ScriptError> {
        let _ = self.child.as_ref().ok_or_else(|| {
            ScriptError::HostError("Process not launched — call launch() first".to_string())
        })?;

        let stdin = self.stdin.as_mut().ok_or_else(|| {
            ScriptError::HostError("Child stdin not available".to_string())
        })?;

        let stdout = self.stdout_reader.as_mut().ok_or_else(|| {
            ScriptError::HostError("Child stdout not available".to_string())
        })?;

        // Serialize to JSON and write as a single line
        let json = serde_json::to_string(msg).map_err(|e| {
            ScriptError::HostError(format!("Failed to serialize message: {e}"))
        })?;

        writeln!(stdin, "{json}").map_err(|e| {
            ScriptError::HostError(format!("Failed to write to child stdin: {e}"))
        })?;
        stdin.flush().map_err(|e| {
            ScriptError::HostError(format!("Failed to flush child stdin: {e}"))
        })?;

        // Read one response line
        let mut line = String::new();
        stdout.read_line(&mut line).map_err(|e| {
            ScriptError::HostError(format!("Failed to read from child stdout: {e}"))
        })?;

        if line.is_empty() {
            return Err(ScriptError::HostError(
                "Child process closed stdout unexpectedly".to_string(),
            ));
        }

        serde_json::from_str(line.trim()).map_err(|e| {
            ScriptError::HostError(format!("Failed to deserialize response: {e}"))
        })
    }

    /// Shut down the child process gracefully and wait for it to exit.
    pub fn shutdown(&mut self) -> Result<(), ScriptError> {
        if self.child.is_some() {
            let _ = self.send(&ScriptMessage::Shutdown);
        }
        // Drop pipes first so the child sees EOF and can exit.
        drop(self.stdin.take());
        drop(self.stdout_reader.take());
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
        Ok(())
    }

    /// Build a sender closure that the stub instances use.
    ///
    /// In a real implementation this would share the process pipes behind an
    /// `Arc<Mutex<...>>`. For the stub we return a closure that immediately
    /// errors, indicating that the instance cannot communicate until we wire
    /// up shared IO.
    fn make_sender(
        &self,
    ) -> Box<dyn FnMut(&ScriptMessage) -> Result<ScriptMessage, ScriptError> + Send> {
        Box::new(|_msg| {
            Err(ScriptError::UnsupportedFeature(
                "ProcessScriptInstance communication requires shared IO (Arc<Mutex<...>>)"
                    .into(),
            ))
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
        // Encode assembly bytes as hex (stub — real impl would use BASE64)
        let data_encoded = hex_encode(assembly_data);

        let response = self.send(&ScriptMessage::LoadAssembly {
            id: id.to_string(),
            data_base64: data_encoded,
        })?;

        match response {
            ScriptMessage::AssemblyLoaded { id: resp_id, .. } => {
                let handle = ScriptHandle::new(&resp_id);
                self.assemblies.push((
                    handle.clone(),
                    ScriptHostState::Loaded {
                        id: id.to_string(),
                    },
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
        _handle: &ScriptHandle,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        let instance_id = format!("inst-{:04x}", self.next_instance_id);
        self.next_instance_id += 1;

        let _ = &instance_id;

        let sender = self.make_sender();
        Ok(Box::new(ProcessScriptInstance {
            instance_id,
            sender,
        }))
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

/// Hex-encode bytes as a stand-in for BASE64 (real impl should use BASE64).
fn hex_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut hex = String::with_capacity(data.len() * 2);
    for byte in data {
        write!(hex, "{byte:02x}").unwrap();
    }
    hex
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
    fn process_host_hex_encode() {
        assert_eq!(hex_encode(b"hello"), "68656c6c6f");
        assert_eq!(hex_encode(b"\x00\xff"), "00ff");
        assert_eq!(hex_encode(b""), "");
    }

    #[test]
    fn process_script_instance_debug() {
        let sender = Box::new(|_: &ScriptMessage| {
            Ok(ScriptMessage::MethodResult {
                instance_id: "test".into(),
                result: ScriptValue::Null,
            })
        });
        let inst = ProcessScriptInstance {
            instance_id: "inst-001".into(),
            sender,
        };
        let debug = format!("{:?}", inst);
        assert!(debug.contains("ProcessScriptInstance"));
    }

    #[test]
    fn process_script_instance_call_with_mock_sender() {
        let sender = Box::new(|msg: &ScriptMessage| match msg {
            ScriptMessage::CallMethod { .. } => Ok(ScriptMessage::MethodResult {
                instance_id: "test".into(),
                result: ScriptValue::Int(42),
            }),
            _ => Err(ScriptError::ExecutionError("unexpected".into())),
        });
        let mut inst = ProcessScriptInstance {
            instance_id: "test".into(),
            sender,
        };
        let result = inst.call("Foo", &[]).unwrap();
        assert_eq!(result, ScriptValue::Int(42));
    }

    #[test]
    fn process_script_instance_call_error() {
        let sender = Box::new(|_: &ScriptMessage| {
            Ok(ScriptMessage::Error {
                message: "oops".into(),
            })
        });
        let mut inst = ProcessScriptInstance {
            instance_id: "test".into(),
            sender,
        };
        let result = inst.call("Foo", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn process_script_instance_set_field() {
        let sender = Box::new(|msg: &ScriptMessage| match msg {
            ScriptMessage::SetField { .. } => Ok(ScriptMessage::FieldValue {
                instance_id: "test".into(),
                name: "speed".into(),
                value: None,
            }),
            _ => Err(ScriptError::ExecutionError("unexpected".into())),
        });
        let mut inst = ProcessScriptInstance {
            instance_id: "test".into(),
            sender,
        };
        assert!(inst.set_field("speed", ScriptValue::Float(1.0)).is_ok());
    }

    #[test]
    fn process_script_instance_get_field_returns_none() {
        let sender = Box::new(|_: &ScriptMessage| {
            Ok(ScriptMessage::FieldValue {
                instance_id: "test".into(),
                name: "x".into(),
                value: None,
            })
        });
        let inst = ProcessScriptInstance {
            instance_id: "test".into(),
            sender,
        };
        assert_eq!(inst.get_field("x"), None);
    }
}
