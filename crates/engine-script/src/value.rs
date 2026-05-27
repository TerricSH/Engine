//! Shared value type for passing data between the engine and script hosts.

use std::collections::BTreeMap;

/// Values that can be passed between the engine and a script host.
///
/// These cover the common primitives and engine-specific types (vectors,
/// entity references, asset handles) that need to cross the script boundary.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptValue {
    /// No value / null.
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer.
    Int(i64),
    /// A 64-bit floating-point number.
    Float(f64),
    /// A UTF-8 string.
    String(String),
    /// A 3-component vector (`[x, y, z]`).
    Vec3([f32; 3]),
    /// A 4-component vector (`[x, y, z, w]`).
    Vec4([f32; 4]),
    /// An entity identifier.
    EntityId(String),
    /// A handle to an asset (opaque wrapper).
    AssetIdWrapper(String),
    /// An ordered list of values.
    Array(Vec<ScriptValue>),
    /// A map of named values.
    Map(BTreeMap<String, ScriptValue>),
}
