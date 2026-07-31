// Gravity Resolution (Combination Semantics) Tests
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn resolve_effective_gravity_falls_back_without_sources() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let sources: Vec<GravitySource> = Vec::new();
    assert_eq!(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        global
    );
    assert_eq!(sum_source_gravity(sources.iter(), glam::Vec3::ZERO), None);
}

#[test]
fn resolve_effective_gravity_falls_back_when_all_sources_out_of_range() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let sources =
        [GravitySource::point(glam::Vec3::new(1000.0, 0.0, 0.0), 50.0).with_max_radius(10.0)];
    assert_eq!(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        global
    );
}

#[test]
fn resolve_effective_gravity_sums_contributing_sources() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let sources = [
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 2.0),
        GravitySource::directional(glam::Vec3::new(0.0, 1.0, 0.0), 3.0),
    ];
    approx_vec3(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        glam::Vec3::new(2.0, 3.0, 0.0),
        "contributions from all in-range sources are summed",
    );
}

#[test]
fn resolve_effective_gravity_cancelling_sources_do_not_fall_back() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let sources = [
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 5.0),
        GravitySource::directional(glam::Vec3::new(-1.0, 0.0, 0.0), 5.0),
    ];
    assert_eq!(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        glam::Vec3::ZERO,
        "a zero-sum field is a real field: the global fallback stays suppressed"
    );
}

#[test]
fn resolve_effective_gravity_skips_disabled_sources() {
    let global = glam::Vec3::new(0.0, -9.81, 0.0);
    let mut disabled = GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 5.0);
    disabled.enabled = false;
    let sources = [disabled];
    assert_eq!(
        resolve_effective_gravity(sources.iter(), glam::Vec3::ZERO, global),
        global
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Gravity Source Serde Tests
// ══════════════════════════════════════════════════════════════════════════════

fn roundtrip_gravity_source(source: &GravitySource) -> GravitySource {
    let fields = crate::serde::serialize_gravity_source(source);
    let restored = crate::serde::deserialize_gravity_source(&fields);
    *restored
        .downcast::<GravitySource>()
        .expect("gravity source roundtrip type")
}

#[test]
fn gravity_source_serde_roundtrip_directional() {
    let source = GravitySource::directional(glam::Vec3::new(0.5, -0.5, 0.25), 3.25);
    assert_eq!(roundtrip_gravity_source(&source), source);
}

#[test]
fn gravity_source_serde_roundtrip_point_with_falloff_and_radius() {
    let source = GravitySource::point(glam::Vec3::new(4.0, 5.0, 6.0), 42.0)
        .with_falloff(GravityFalloff::InverseSquare)
        .with_max_radius(120.0);
    assert_eq!(roundtrip_gravity_source(&source), source);
}

#[test]
fn gravity_source_serde_omits_absent_max_radius() {
    let source = GravitySource::point(glam::Vec3::ZERO, 9.81);
    let fields = crate::serde::serialize_gravity_source(&source);
    assert!(!fields.contains_key("max_radius"));
    assert_eq!(
        fields.get("mode"),
        Some(&engine_serialize::Value::Enum("Point".into()))
    );
    assert_eq!(roundtrip_gravity_source(&source), source);
}

#[test]
fn gravity_source_deserialize_defaults_for_missing_fields() {
    let restored = crate::serde::deserialize_gravity_source(&std::collections::BTreeMap::new());
    assert_eq!(
        *restored.downcast::<GravitySource>().unwrap(),
        GravitySource::default()
    );
}

#[test]
fn gravity_source_deserialize_sanitizes_non_finite_values() {
    use engine_serialize::Value;
    let fields = std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("strength".into(), Value::Float32(f32::NAN)),
        ("direction".into(), Value::Vec3([f32::INFINITY, 0.0, 0.0])),
        ("center".into(), Value::Vec3([0.0, f32::NAN, 0.0])),
        ("falloff".into(), Value::Enum("Linear".into())),
        ("max_radius".into(), Value::Float32(f32::NEG_INFINITY)),
    ]);
    let restored = crate::serde::deserialize_gravity_source(&fields);
    let restored = restored.downcast::<GravitySource>().unwrap();
    assert_eq!(restored.mode, GravityMode::Point);
    assert_eq!(restored.strength, GravitySource::default().strength);
    assert_eq!(restored.direction, GravitySource::default().direction);
    assert_eq!(restored.center, glam::Vec3::ZERO);
    assert_eq!(restored.falloff, GravityFalloff::Linear);
    assert_eq!(restored.max_radius, None);
    assert!(
        restored.strength.is_finite()
            && restored.direction.is_finite()
            && restored.center.is_finite()
    );
}

#[test]
fn gravity_source_deserialize_rejects_non_positive_max_radius() {
    use engine_serialize::Value;
    for radius in [0.0, -1.0] {
        let fields = std::collections::BTreeMap::from([
            ("mode".into(), Value::Enum("Point".into())),
            ("max_radius".into(), Value::Float32(radius)),
        ]);
        let restored = crate::serde::deserialize_gravity_source(&fields);
        assert_eq!(
            restored.downcast::<GravitySource>().unwrap().max_radius,
            None,
            "max_radius {radius} must be treated as unlimited"
        );
    }
}

// ══════════════════════════════════════════════════════════════════════════════
