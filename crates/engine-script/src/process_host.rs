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

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use crate::host::{ScriptError, ScriptHandle, ScriptHost, ScriptInstance};
use crate::protocol::ScriptMessage;
use crate::value::ScriptValue;
use crate::{GameplayCommand, GameplayContext};

fn encode_gameplay_context(
    instance_id: &str,
    context: &GameplayContext,
) -> Result<String, ScriptError> {
    serde_json::to_string(context).map_err(|error| {
        ScriptError::HostError(format!(
            "Failed to serialize gameplay context for instance '{instance_id}': {error}"
        ))
    })
}

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
            ScriptMessage::Error {
                code,
                operation,
                message,
                assembly_id,
            } => Err(ScriptError::ExecutionError(protocol_error_message(
                &code,
                &operation,
                &message,
                assembly_id.as_deref(),
            ))),
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
            ScriptMessage::Error {
                code,
                operation,
                message,
                assembly_id,
            } => Err(ScriptError::ExecutionError(protocol_error_message(
                &code,
                &operation,
                &message,
                assembly_id.as_deref(),
            ))),
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

    fn set_gameplay_context(&mut self, context: &GameplayContext) -> Result<(), ScriptError> {
        let context_json = encode_gameplay_context(&self.instance_id, context)?;
        let mut io = self
            .io
            .lock()
            .map_err(|e| ScriptError::HostError(format!("Script IO lock poisoned: {e}")))?;
        let response = io.roundtrip(&ScriptMessage::SetGameplayContext {
            instance_id: self.instance_id.clone(),
            context_json,
        })?;

        match response {
            ScriptMessage::GameplayContextSet { instance_id }
                if instance_id == self.instance_id =>
            {
                Ok(())
            }
            ScriptMessage::Error {
                code,
                operation,
                message,
                assembly_id,
            } => Err(ScriptError::ExecutionError(format!(
                "SetGameplayContext failed for '{}': {}",
                self.instance_id,
                protocol_error_message(&code, &operation, &message, assembly_id.as_deref())
            ))),
            other => Err(ScriptError::ExecutionError(format!(
                "Unexpected response to SetGameplayContext for '{}': {other:?}",
                self.instance_id
            ))),
        }
    }

    fn drain_gameplay_commands(&mut self) -> Result<Vec<GameplayCommand>, ScriptError> {
        let mut io = self
            .io
            .lock()
            .map_err(|e| ScriptError::HostError(format!("Script IO lock poisoned: {e}")))?;
        let response = io.roundtrip(&ScriptMessage::DrainGameplayCommands {
            instance_id: self.instance_id.clone(),
        })?;

        match response {
            ScriptMessage::GameplayCommands {
                instance_id,
                commands_json,
            } if instance_id == self.instance_id => {
                decode_gameplay_commands(&self.instance_id, &commands_json)
            }
            ScriptMessage::Error {
                code,
                operation,
                message,
                assembly_id,
            } => Err(ScriptError::ExecutionError(format!(
                "DrainGameplayCommands failed for '{}': {}",
                self.instance_id,
                protocol_error_message(&code, &operation, &message, assembly_id.as_deref())
            ))),
            other => Err(ScriptError::ExecutionError(format!(
                "Unexpected response to DrainGameplayCommands for '{}': {other:?}",
                self.instance_id
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Process host
// ---------------------------------------------------------------------------

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
    /// Loaded handles and the host-reflected concrete behaviour classes.
    assemblies: BTreeMap<String, (ScriptHandle, Vec<String>)>,
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
            assemblies: BTreeMap::new(),
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
            ScriptMessage::AssemblyLoaded {
                id: resp_id,
                mut classes,
            } => {
                if resp_id != id {
                    return Err(ScriptError::LoadFailed(format!(
                        "LoadAssembly response id '{resp_id}' did not match request id '{id}'"
                    )));
                }
                classes.sort();
                classes.dedup();
                let handle = ScriptHandle::new(&resp_id);
                self.assemblies.insert(resp_id, (handle.clone(), classes));
                Ok(handle)
            }
            ScriptMessage::Error {
                code,
                operation,
                message,
                assembly_id,
            } => Err(ScriptError::LoadFailed(protocol_error_message(
                &code,
                &operation,
                &message,
                assembly_id.as_deref(),
            ))),
            other => Err(ScriptError::LoadFailed(format!(
                "Unexpected response to LoadAssembly: {other:?}"
            ))),
        }
    }

    fn instantiate(
        &mut self,
        handle: &ScriptHandle,
        class_name: &str,
    ) -> Result<Box<dyn ScriptInstance>, ScriptError> {
        let verified = self.verified_classes(handle.id()).ok_or_else(|| {
            ScriptError::LoadFailed(format!(
                "Assembly '{}' is not loaded by ProcessHost",
                handle.id()
            ))
        })?;
        if verified
            .binary_search_by(|candidate| candidate.as_str().cmp(class_name))
            .is_err()
        {
            return Err(ScriptError::ExecutionError(format!(
                "Class '{class_name}' is not a reflection-verified Engine.EngineBehaviour in assembly '{}'",
                handle.id()
            )));
        }
        let instance_id = format!("inst-{:04x}", self.next_instance_id);
        self.next_instance_id += 1;

        // Tell the child process to create the instance
        let io = self.shared_io()?;
        let mut io_lock = io
            .lock()
            .map_err(|e| ScriptError::HostError(format!("Script IO lock poisoned: {e}")))?;
        let response = io_lock.roundtrip(&ScriptMessage::Instantiate {
            assembly_id: handle.id().to_string(),
            class_name: class_name.to_string(),
            instance_id: instance_id.clone(),
        })?;
        match response {
            ScriptMessage::MethodResult {
                instance_id: response_id,
                ..
            } if response_id == instance_id => {}
            ScriptMessage::Error {
                code,
                operation,
                message,
                assembly_id,
            } => {
                return Err(ScriptError::ExecutionError(protocol_error_message(
                    &code,
                    &operation,
                    &message,
                    assembly_id.as_deref(),
                )));
            }
            other => {
                return Err(ScriptError::ExecutionError(format!(
                    "Unexpected response to Instantiate: {other:?}"
                )));
            }
        }
        // Drop the lock before moving `io` into the instance
        drop(io_lock);

        Ok(Box::new(ProcessScriptInstance { instance_id, io }))
    }

    fn unload(&mut self, handle: &ScriptHandle) -> Result<(), ScriptError> {
        self.assemblies.remove(handle.id());
        Ok(())
    }

    fn verified_classes(&self, assembly_id: &str) -> Option<&[String]> {
        self.assemblies
            .get(assembly_id)
            .map(|(_, classes)| classes.as_slice())
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

fn protocol_error_message(
    code: &str,
    operation: &str,
    message: &str,
    assembly_id: Option<&str>,
) -> String {
    let assembly = assembly_id.map_or(String::new(), |id| format!(" assembly='{id}'"));
    format!("[{code}] operation='{operation}'{assembly}: {message}")
}

fn decode_gameplay_commands(
    instance_id: &str,
    commands_json: &str,
) -> Result<Vec<GameplayCommand>, ScriptError> {
    let raw_commands: Vec<serde_json::Value> =
        serde_json::from_str(commands_json).map_err(|error| {
            ScriptError::ExecutionError(format!(
                "DrainGameplayCommands returned invalid JSON for '{instance_id}': {error}"
            ))
        })?;
    let mut commands = Vec::with_capacity(raw_commands.len());
    for (index, raw_command) in raw_commands.into_iter().enumerate() {
        if raw_command.get("type").and_then(serde_json::Value::as_str) == Some("load_scene")
            && raw_command
                .get("scene_id")
                .and_then(serde_json::Value::as_str)
                .is_none()
        {
            return Err(ScriptError::ExecutionError(format!(
                "DrainGameplayCommands returned invalid command {index} for '{instance_id}': load_scene requires a string `scene_id`; use a key from game.project.json `scenes`"
            )));
        }
        let command: GameplayCommand = serde_json::from_value(raw_command).map_err(|error| {
            ScriptError::ExecutionError(format!(
                "DrainGameplayCommands returned invalid command {index} for '{instance_id}': {error}"
            ))
        })?;
        command.validate().map_err(|error| {
            ScriptError::ExecutionError(format!(
                "DrainGameplayCommands returned invalid command {index} for '{instance_id}': {error}"
            ))
        })?;
        commands.push(command);
    }
    Ok(commands)
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
    fn gameplay_command_decoder_preserves_legacy_and_explicit_entity_commands() {
        let commands = decode_gameplay_commands(
            "inst-0001",
            r#"[
                {"type":"set_transform","transform":{"translation":[1,2,3],"rotation":[0,0,0,1],"scale":[1,1,1]}},
                {"type":"set_entity_transform","entity_id":"enemy-01","transform":{"translation":[4,5,6],"rotation":[0,0,0,1],"scale":[2,2,2]}},
                {"type":"create_entity","entity_id":"spawned-01","transform":{"translation":[7,8,9],"rotation":[0,0,0,1],"scale":[1,1,1]}},
                {"type":"destroy_entity","entity_id":"enemy-01"},
                {"type":"destroy_self"},
                {"type":"load_scene","scene_id":"level_two"}
            ]"#,
        )
        .unwrap();
        assert!(matches!(
            &commands[0],
            GameplayCommand::SetTransform { transform }
                if transform.translation == [1.0, 2.0, 3.0]
        ));
        assert_eq!(
            commands[1],
            GameplayCommand::SetEntityTransform {
                entity_id: "enemy-01".into(),
                transform: crate::ScriptTransform {
                    translation: [4.0, 5.0, 6.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [2.0, 2.0, 2.0],
                }
            }
        );
        assert_eq!(
            commands[2],
            GameplayCommand::CreateEntity {
                entity_id: "spawned-01".into(),
                transform: crate::ScriptTransform {
                    translation: [7.0, 8.0, 9.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                }
            }
        );
        assert_eq!(
            commands[3],
            GameplayCommand::DestroyEntity {
                entity_id: "enemy-01".into()
            }
        );
        assert_eq!(commands[4], GameplayCommand::DestroySelf);
        assert_eq!(
            commands[5],
            GameplayCommand::LoadScene {
                scene_id: "level_two".into()
            }
        );
    }

    #[test]
    fn gameplay_command_decoder_accepts_managed_ui_class_commands() {
        let commands = decode_gameplay_commands(
            "inst-0001",
            r#"[
                {"type":"ui","command":{"type":"create_canvas","canvas_id":"hud","width":1280,"height":720}},
                {"type":"ui","command":{"type":"add_element","canvas_id":"hud","element_id":1,"element":{"kind":"panel","layout":{"anchor_min":[0,0],"anchor_max":[0,0],"offset_min":[24,24],"offset_max":[344,56]},"color":{"r":20,"g":20,"b":20,"a":210},"z_order":10}}},
                {"type":"ui","command":{"type":"set_slider_value","canvas_id":"hud","element_id":3,"value":0.75}}
            ]"#,
        )
        .unwrap();
        assert!(matches!(
            &commands[0],
            GameplayCommand::Ui {
                command: crate::GameplayUiCommand::CreateCanvas { canvas_id, .. }
            } if canvas_id == "hud"
        ));
        assert!(matches!(
            &commands[2],
            GameplayCommand::Ui {
                command: crate::GameplayUiCommand::SetSliderValue {
                    canvas_id,
                    element_id: 3,
                    value,
                }
            } if canvas_id == "hud" && (*value - 0.75).abs() < f32::EPSILON
        ));
        assert!(matches!(
            &commands[1],
            GameplayCommand::Ui {
                command: crate::GameplayUiCommand::AddElement {
                    canvas_id,
                    element_id: 1,
                    element: crate::GameplayUiElement::Panel { z_order: 10, .. }
                }
            } if canvas_id == "hud"
        ));
    }

    #[test]
    fn gameplay_command_decoder_rejects_unsafe_scene_ids_with_guidance() {
        for json in [
            r#"[{"type":"load_scene","scene_id":""}]"#,
            r#"[{"type":"load_scene","scene_id":"../outside"}]"#,
            r#"[{"type":"load_scene","scene_id":"levels/boss"}]"#,
            r#"[{"type":"load_scene"}]"#,
            r#"[{"type":"load_scene","scene_id":null}]"#,
        ] {
            let error = decode_gameplay_commands("inst-0001", json).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("invalid command 0"), "{message}");
            assert!(message.contains("game.project.json `scenes`"), "{message}");
        }
    }

    #[test]
    fn gameplay_command_decoder_rejects_unsafe_targets_and_non_finite_transforms() {
        for json in [
            r#"[{"type":"destroy_entity","entity_id":"../outside"}]"#,
            r#"[{"type":"set_entity_transform","entity_id":"enemy/child","transform":{"translation":[0,0,0],"rotation":[0,0,0,1],"scale":[1,1,1]}}]"#,
            r#"[{"type":"set_entity_transform","entity_id":"enemy","transform":{"translation":[0,0,0],"rotation":[0,0,0,0],"scale":[1,1,1]}}]"#,
            r#"[{"type":"create_entity","entity_id":"../spawn","transform":{"translation":[0,0,0],"rotation":[0,0,0,1],"scale":[1,1,1]}}]"#,
            r#"[{"type":"create_entity","entity_id":"spawn","transform":{"translation":[0,0,0],"rotation":[0,0,0,0],"scale":[1,1,1]}}]"#,
        ] {
            let error = decode_gameplay_commands("inst-0001", json).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("invalid command 0"), "{message}");
        }
    }

    #[test]
    fn process_host_context_encoder_preserves_gameplay_ui_events() {
        let mut context: GameplayContext =
            serde_json::from_str(r#"{"entity_id":"player","transform":null,"input_actions":{}}"#)
                .unwrap();
        context.ui_events = vec![crate::GameplayUiEvent {
            canvas_id: "main-menu".into(),
            element_id: 7,
            callback_id: Some("continue".into()),
            value: None,
        }];

        let context_json = encode_gameplay_context("inst-ui", &context).unwrap();
        let encoded: serde_json::Value = serde_json::from_str(&context_json).unwrap();
        assert_eq!(
            encoded["ui_events"],
            serde_json::json!([{
                "canvas_id": "main-menu",
                "element_id": 7,
                "callback_id": "continue"
            }])
        );
        assert_eq!(
            serde_json::from_str::<GameplayContext>(&context_json).unwrap(),
            context
        );
    }

    #[test]
    fn gameplay_command_decoder_accepts_validated_physics_queries() {
        let commands = decode_gameplay_commands(
            "inst-0001",
            r#"[
                {"type":"physics_query","query":{"kind":"raycast","query_id":7,"origin":[0,5,0],"direction":[0,-1,0],"max_distance":10}},
                {"type":"physics_query","query":{"kind":"overlap_sphere","query_id":8,"center":[0,0,0],"radius":2.5}}
            ]"#,
        )
        .unwrap();
        assert_eq!(
            commands[0],
            GameplayCommand::PhysicsQuery {
                query: crate::GameplayPhysicsQuery::Raycast {
                    query_id: 7,
                    origin: [0.0, 5.0, 0.0],
                    direction: [0.0, -1.0, 0.0],
                    max_distance: 10.0,
                }
            }
        );
        assert_eq!(
            commands[1],
            GameplayCommand::PhysicsQuery {
                query: crate::GameplayPhysicsQuery::OverlapSphere {
                    query_id: 8,
                    center: [0.0, 0.0, 0.0],
                    radius: 2.5,
                }
            }
        );
    }

    #[test]
    fn gameplay_command_decoder_rejects_invalid_physics_queries() {
        for json in [
            // Zero-length direction cannot define a ray.
            r#"[{"type":"physics_query","query":{"kind":"raycast","query_id":7,"origin":[0,5,0],"direction":[0,0,0],"max_distance":10}}]"#,
            // Non-positive travel distance.
            r#"[{"type":"physics_query","query":{"kind":"raycast","query_id":7,"origin":[0,5,0],"direction":[0,-1,0],"max_distance":0}}]"#,
            // Non-positive radius.
            r#"[{"type":"physics_query","query":{"kind":"overlap_sphere","query_id":8,"center":[0,0,0],"radius":-1}}]"#,
        ] {
            let error = decode_gameplay_commands("inst-0001", json).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("invalid command 0"), "{message}");
        }

        // Overflowing JSON numbers fail even earlier, at the parse boundary,
        // so non-finite query values cannot cross the wire at all.
        for json in [
            r#"[{"type":"physics_query","query":{"kind":"raycast","query_id":7,"origin":[0,5,0],"direction":[0,-1,0],"max_distance":1e999}}]"#,
            r#"[{"type":"physics_query","query":{"kind":"overlap_sphere","query_id":8,"center":[0,1e999,0],"radius":2.5}}]"#,
        ] {
            assert!(
                decode_gameplay_commands("inst-0001", json).is_err(),
                "{json}"
            );
        }
    }

    #[test]
    fn process_host_context_encoder_preserves_physics_query_results() {
        let mut context: GameplayContext =
            serde_json::from_str(r#"{"entity_id":"player","transform":null,"input_actions":{}}"#)
                .unwrap();
        context.physics_query_results = vec![
            crate::GameplayPhysicsQueryResult::RaycastHit {
                query_id: 7,
                entity_id: "cube-01".into(),
                point: [0.0, 0.5, 0.0],
                normal: [0.0, 1.0, 0.0],
                distance: 4.5,
            },
            crate::GameplayPhysicsQueryResult::RaycastMiss { query_id: 8 },
            crate::GameplayPhysicsQueryResult::OverlapSphere {
                query_id: 9,
                entity_ids: vec!["cube-01".into()],
            },
        ];

        let context_json = encode_gameplay_context("inst-physics", &context).unwrap();
        let encoded: serde_json::Value = serde_json::from_str(&context_json).unwrap();
        assert_eq!(
            encoded["physics_query_results"],
            serde_json::json!([
                {"kind":"raycast_hit","query_id":7,"entity_id":"cube-01","point":[0.0,0.5,0.0],"normal":[0.0,1.0,0.0],"distance":4.5},
                {"kind":"raycast_miss","query_id":8},
                {"kind":"overlap_sphere","query_id":9,"entity_ids":["cube-01"]}
            ])
        );
        assert_eq!(
            serde_json::from_str::<GameplayContext>(&context_json).unwrap(),
            context
        );
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
