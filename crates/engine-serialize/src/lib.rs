#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl SchemaVersion {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

pub type EngineVersion = String;
pub type ContractVersion = String;
pub type PersistentId = String;
pub type ComponentTypeId = String;
pub type PropertyPath = String;
pub type HashDigest = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AssetId {
    pub id: String,
    pub logical_path: Option<String>,
}

impl AssetId {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            logical_path: None,
        }
    }

    pub fn with_path(id: impl Into<String>, logical_path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            logical_path: Some(logical_path.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
    Fatal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub system: String,
    pub contract: Option<String>,
    pub version: Option<String>,
    pub message: String,
    pub path: Option<String>,
    pub entity: Option<PersistentId>,
    pub asset: Option<AssetId>,
    pub package_id: Option<String>,
    pub recoverable: bool,
    pub suggested_action: Option<String>,
    pub fields: BTreeMap<String, String>,
    pub related: Vec<Diagnostic>,
}

impl Diagnostic {
    pub fn new(
        code: impl Into<String>,
        severity: DiagnosticSeverity,
        system: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity,
            system: system.into(),
            contract: None,
            version: None,
            message: message.into(),
            path: None,
            entity: None,
            asset: None,
            package_id: None,
            recoverable: false,
            suggested_action: None,
            fields: BTreeMap::new(),
            related: Vec::new(),
        }
    }

    pub fn contract(mut self, contract: impl Into<String>, version: impl Into<String>) -> Self {
        self.contract = Some(contract.into());
        self.version = Some(version.into());
        self
    }

    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float32(f32),
    Float64(f64),
    Str(String),
    Vec3([f32; 3]),
    Quat([f32; 4]),
    Color([f32; 4]),
    Asset(AssetId),
    Entity(PersistentId),
    Enum(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}
