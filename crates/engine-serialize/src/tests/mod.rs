use crate::{AssetId, Diagnostic, DiagnosticSeverity, SchemaVersion, Value};

// ── AssetId tests ────────────────────────────────────────────────────────

#[test]
fn asset_id_new_creates_without_path() {
    let id = AssetId::new("mesh-cube");
    assert_eq!(id.id, "mesh-cube");
    assert_eq!(id.logical_path, None);
}

#[test]
fn asset_id_with_path_sets_logical_path() {
    let id = AssetId::with_path("my-thing", "custom/thing.bin");
    assert_eq!(id.id, "my-thing");
    assert_eq!(id.logical_path, Some("custom/thing.bin".to_string()));
}

#[test]
fn asset_id_equality() {
    let a = AssetId::new("mesh-cube");
    let b = AssetId::new("mesh-cube");
    let c = AssetId::new("other");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn asset_id_ordering() {
    let a = AssetId::new("alpha");
    let b = AssetId::new("beta");
    assert!(a < b);
}

#[test]
fn asset_id_with_path_equality() {
    let a = AssetId::with_path("id", "path/a.bin");
    let b = AssetId::with_path("id", "path/a.bin");
    let c = AssetId::with_path("id", "other.bin");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

// ── SchemaVersion tests ──────────────────────────────────────────────────

#[test]
fn schema_version_new_creates_version() {
    let v = SchemaVersion::new(1, 2, 3);
    assert_eq!(v.major, 1);
    assert_eq!(v.minor, 2);
    assert_eq!(v.patch, 3);
}

#[test]
fn schema_version_ordering() {
    let v1 = SchemaVersion::new(1, 0, 0);
    let v2 = SchemaVersion::new(2, 0, 0);
    let v3 = SchemaVersion::new(1, 1, 0);
    let v4 = SchemaVersion::new(1, 0, 1);
    assert!(v1 < v2);
    assert!(v1 < v3);
    assert!(v1 < v4);
    assert!(v3 < v2);
}

#[test]
fn schema_version_equality() {
    assert_eq!(SchemaVersion::new(1, 0, 0), SchemaVersion::new(1, 0, 0));
    assert_ne!(SchemaVersion::new(1, 0, 0), SchemaVersion::new(1, 0, 1));
}

#[test]
fn schema_version_default_is_zero() {
    let v = SchemaVersion::default();
    assert_eq!(v, SchemaVersion::new(0, 0, 0));
}

// ── Diagnostic tests ─────────────────────────────────────────────────────

#[test]
fn diagnostic_new_creates_basic_diagnostic() {
    let d = Diagnostic::new(
        "C001",
        DiagnosticSeverity::Error,
        "core",
        "something failed",
    );
    assert_eq!(d.code, "C001");
    assert_eq!(d.severity, DiagnosticSeverity::Error);
    assert_eq!(d.system, "core");
    assert_eq!(d.message, "something failed");
    assert!(d.contract.is_none());
    assert!(d.path.is_none());
    assert!(!d.recoverable);
}

#[test]
fn diagnostic_contract_sets_contract_and_version() {
    let d = Diagnostic::new("C001", DiagnosticSeverity::Warning, "sys", "msg")
        .contract("my-contract", "1.0.0");
    assert_eq!(d.contract, Some("my-contract".to_string()));
    assert_eq!(d.version, Some("1.0.0".to_string()));
}

#[test]
fn diagnostic_path_sets_path() {
    let d = Diagnostic::new("C002", DiagnosticSeverity::Info, "sys", "msg").path("some/path.txt");
    assert_eq!(d.path, Some("some/path.txt".to_string()));
}

#[test]
fn diagnostic_builder_chain() {
    let d = Diagnostic::new("C003", DiagnosticSeverity::Fatal, "sys", "fatal error")
        .contract("contract-a", "2.0.0")
        .path("path/to/file");
    assert_eq!(d.contract, Some("contract-a".to_string()));
    assert_eq!(d.path, Some("path/to/file".to_string()));
}

#[test]
fn diagnostic_severity_variants() {
    assert_eq!(format!("{:?}", DiagnosticSeverity::Info), "Info");
    assert_eq!(format!("{:?}", DiagnosticSeverity::Warning), "Warning");
    assert_eq!(format!("{:?}", DiagnosticSeverity::Error), "Error");
    assert_eq!(format!("{:?}", DiagnosticSeverity::Fatal), "Fatal");
}

// ── Value tests ──────────────────────────────────────────────────────────

#[test]
fn value_bool_variant() {
    assert_eq!(Value::Bool(true), Value::Bool(true));
    assert_ne!(Value::Bool(true), Value::Bool(false));
}

#[test]
fn value_int_variant() {
    assert_eq!(Value::Int(42), Value::Int(42));
    assert_eq!(format!("{:?}", Value::Int(-1)), "Int(-1)");
}

#[test]
fn value_uint_variant() {
    assert_eq!(Value::UInt(u64::MAX), Value::UInt(u64::MAX));
}

#[test]
fn value_float_variants() {
    let f32_val = Value::Float32(std::f32::consts::PI);
    let f64_val = Value::Float64(2.71);
    assert_eq!(
        format!("{:?}", f32_val),
        format!("Float32({:?})", std::f32::consts::PI)
    );
    assert_eq!(format!("{:?}", f64_val), "Float64(2.71)");
}

#[test]
fn value_str_variant() {
    let s = Value::Str("hello".to_string());
    assert_eq!(s, Value::Str("hello".to_string()));
    assert_ne!(s, Value::Str("world".to_string()));
}

#[test]
fn value_vec3_variant() {
    let v = Value::Vec3([1.0, 2.0, 3.0]);
    assert_eq!(v, Value::Vec3([1.0, 2.0, 3.0]));
}

#[test]
fn value_quat_variant() {
    let q = Value::Quat([0.0, 0.0, 0.0, 1.0]);
    assert_eq!(q, Value::Quat([0.0, 0.0, 0.0, 1.0]));
}

#[test]
fn value_color_variant() {
    let c = Value::Color([1.0, 0.0, 0.0, 1.0]);
    assert_eq!(c, Value::Color([1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn value_asset_variant() {
    let a = Value::Asset(AssetId::new("mesh-cube"));
    assert_eq!(a, Value::Asset(AssetId::new("mesh-cube")));
}

#[test]
fn value_entity_variant() {
    let e = Value::Entity("ent-123".to_string());
    assert_eq!(e, Value::Entity("ent-123".to_string()));
}

#[test]
fn value_enum_variant() {
    let e = Value::Enum("Red".to_string());
    assert_eq!(e, Value::Enum("Red".to_string()));
}

#[test]
fn value_list_variant() {
    let list = Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
    assert_eq!(
        list,
        Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    assert_ne!(list, Value::List(vec![]));
}

#[test]
fn value_map_variant() {
    use std::collections::BTreeMap;
    let mut map = BTreeMap::new();
    map.insert("key".to_string(), Value::Str("val".to_string()));
    let m = Value::Map(map);
    assert_eq!(format!("{:?}", m), "Map({\"key\": Str(\"val\")})");
}
