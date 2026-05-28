//! Wire protocol between the engine and a script host sub-process.
//!
//! The [`ScriptMessage`] enum defines a JSON-line protocol that the engine uses
//! to communicate with a child process running the .NET/C# script runtime. Each
//! message is a single line of JSON terminated by `\n`.
//!
//! This is the "process-based stub" for CoreCLR hosting. A real implementation
//! would host CoreCLR in-process via `hostfxr` / `nethost` native APIs; this
//! file provides the wire format so that the engine side is ready regardless of
//! the hosting strategy.

use serde::{Deserialize, Serialize};

use crate::value::ScriptValue;

/// Messages sent between the engine and a script host sub-process.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ScriptMessage {
    /// Request: load an assembly into the script runtime.
    #[serde(rename = "LoadAssembly")]
    LoadAssembly {
        /// User-assigned identifier for this assembly.
        id: String,
        /// Base64-encoded assembly bytes (e.g. a .NET PE file).
        data_base64: String,
    },

    /// Response: assembly has been loaded.
    #[serde(rename = "AssemblyLoaded")]
    AssemblyLoaded {
        /// The identifier from the corresponding `LoadAssembly` request.
        id: String,
        /// The fully-qualified type names found in the assembly.
        types: Vec<String>,
    },

    /// Request: create an instance of a script type.
    #[serde(rename = "Instantiate")]
    Instantiate {
        /// The assembly that contains the type.
        assembly_id: String,
        /// The fully-qualified class name to instantiate.
        class_name: String,
        /// A unique id for this instance (chosen by the engine).
        instance_id: String,
    },

    /// Request: call a lifecycle or user-defined method.
    #[serde(rename = "CallMethod")]
    CallMethod {
        /// The instance to invoke on.
        instance_id: String,
        /// The method name (e.g. `"OnUpdate"`).
        method: String,
        /// Arguments to pass to the method.
        args: Vec<ScriptValue>,
    },

    /// Response: method returned a value.
    #[serde(rename = "MethodResult")]
    MethodResult {
        /// The instance from the corresponding `CallMethod` request.
        instance_id: String,
        /// The return value from the method.
        result: ScriptValue,
    },

    /// Error response (for any request that failed).
    #[serde(rename = "Error")]
    Error {
        /// Human-readable error description.
        message: String,
    },

    /// Request: write a field value on an instance.
    #[serde(rename = "SetField")]
    SetField {
        /// The target instance.
        instance_id: String,
        /// Field name.
        name: String,
        /// Value to write.
        value: ScriptValue,
    },

    /// Request: read a field value from an instance.
    #[serde(rename = "GetField")]
    GetField {
        /// The target instance.
        instance_id: String,
        /// Field name.
        name: String,
    },

    /// Response: field value (in reply to `GetField`).
    #[serde(rename = "FieldValue")]
    FieldValue {
        /// The instance from the corresponding `GetField` request.
        instance_id: String,
        /// Field name.
        name: String,
        /// The current value, or `None` if the field does not exist.
        value: Option<ScriptValue>,
    },

    /// Request: shut down the script runtime gracefully.
    #[serde(rename = "Shutdown")]
    Shutdown,
}

impl ScriptMessage {
    /// Serialise this message to a JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialise a message from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_load_assembly_roundtrip() {
        let msg = ScriptMessage::LoadAssembly {
            id: "asm-001".into(),
            data_base64: "AAAABBBBCCCC".into(),
        };
        let json = msg.to_json().unwrap();
        let back = ScriptMessage::from_json(&json).unwrap();
        assert!(matches!(back, ScriptMessage::LoadAssembly { id, .. } if id == "asm-001"));
    }

    #[test]
    fn protocol_assembly_loaded_roundtrip() {
        let msg = ScriptMessage::AssemblyLoaded {
            id: "asm-001".into(),
            types: vec!["MyApp.MyScript".into()],
        };
        let json = msg.to_json().unwrap();
        let back = ScriptMessage::from_json(&json).unwrap();
        assert!(matches!(back, ScriptMessage::AssemblyLoaded { id, .. } if id == "asm-001"));
    }

    #[test]
    fn protocol_instantiate_roundtrip() {
        let msg = ScriptMessage::Instantiate {
            assembly_id: "asm-001".into(),
            class_name: "MyScript".into(),
            instance_id: "inst-001".into(),
        };
        let json = msg.to_json().unwrap();
        let back = ScriptMessage::from_json(&json).unwrap();
        assert!(matches!(back, ScriptMessage::Instantiate { instance_id, .. } if instance_id == "inst-001"));
    }

    #[test]
    fn protocol_call_method_roundtrip() {
        let msg = ScriptMessage::CallMethod {
            instance_id: "inst-001".into(),
            method: "OnUpdate".into(),
            args: vec![ScriptValue::Float(0.016)],
        };
        let json = msg.to_json().unwrap();
        let back = ScriptMessage::from_json(&json).unwrap();
        assert!(matches!(back, ScriptMessage::CallMethod { method, .. } if method == "OnUpdate"));
    }

    #[test]
    fn protocol_error_roundtrip() {
        let msg = ScriptMessage::Error {
            message: "Something went wrong".into(),
        };
        let json = msg.to_json().unwrap();
        let back = ScriptMessage::from_json(&json).unwrap();
        assert!(matches!(back, ScriptMessage::Error { message } if message == "Something went wrong"));
    }

    #[test]
    fn protocol_set_field_roundtrip() {
        let msg = ScriptMessage::SetField {
            instance_id: "inst-001".into(),
            name: "speed".into(),
            value: ScriptValue::Float(100.0),
        };
        let json = msg.to_json().unwrap();
        let back = ScriptMessage::from_json(&json).unwrap();
        assert!(matches!(back, ScriptMessage::SetField { name, .. } if name == "speed"));
    }

    #[test]
    fn protocol_get_field_roundtrip() {
        let msg = ScriptMessage::GetField {
            instance_id: "inst-001".into(),
            name: "speed".into(),
        };
        let json = msg.to_json().unwrap();
        let back = ScriptMessage::from_json(&json).unwrap();
        assert!(matches!(back, ScriptMessage::GetField { name, .. } if name == "speed"));
    }

    #[test]
    fn protocol_field_value_roundtrip() {
        let msg = ScriptMessage::FieldValue {
            instance_id: "inst-001".into(),
            name: "speed".into(),
            value: Some(ScriptValue::Float(200.0)),
        };
        let json = msg.to_json().unwrap();
        let back = ScriptMessage::from_json(&json).unwrap();
        assert!(matches!(back, ScriptMessage::FieldValue { value: Some(ScriptValue::Float(v)), .. } if (v - 200.0).abs() < f64::EPSILON));
    }

    #[test]
    fn protocol_shutdown_roundtrip() {
        let msg = ScriptMessage::Shutdown;
        let json = msg.to_json().unwrap();
        let back = ScriptMessage::from_json(&json).unwrap();
        assert!(matches!(back, ScriptMessage::Shutdown));
    }
}
