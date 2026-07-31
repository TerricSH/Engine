// Gravity Source Script-Bridge Round-Trip Tests
//
// Mirror the merge -> deserialize -> serialize validation the script
// component bridge applies (commit e49b6fd): a write is accepted only when
// every provided field survives the round-trip unchanged.
// ══════════════════════════════════════════════════════════════════════════════

fn script_style_merge_write(
    base: &std::collections::BTreeMap<String, engine_serialize::Value>,
    write: &std::collections::BTreeMap<String, engine_serialize::Value>,
) -> Result<GravitySource, Vec<String>> {
    let mut merged = base.clone();
    for (name, value) in write {
        merged.insert(name.clone(), value.clone());
    }
    let candidate = crate::serde::deserialize_gravity_source(&merged);
    let reserialized = crate::serde::serialize_gravity_source(candidate.as_ref());
    let rejected: Vec<String> = write
        .keys()
        .filter(|name| reserialized.get(*name) != merged.get(*name))
        .cloned()
        .collect();
    if rejected.is_empty() {
        Ok(*candidate.downcast::<GravitySource>().unwrap())
    } else {
        Err(rejected)
    }
}

#[test]
fn gravity_source_script_merge_write_roundtrips() {
    use engine_serialize::Value;
    let base = crate::serde::serialize_gravity_source(&GravitySource::default());
    let write = std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("center".into(), Value::Vec3([10.0, 0.0, -4.0])),
        ("strength".into(), Value::Float32(25.0)),
        ("falloff".into(), Value::Enum("InverseSquare".into())),
        ("max_radius".into(), Value::Float32(80.0)),
    ]);
    let applied = script_style_merge_write(&base, &write).expect("valid write is accepted");
    assert_eq!(applied.mode, GravityMode::Point);
    assert_eq!(applied.center, glam::Vec3::new(10.0, 0.0, -4.0));
    assert_eq!(applied.strength, 25.0);
    assert_eq!(applied.falloff, GravityFalloff::InverseSquare);
    assert_eq!(applied.max_radius, Some(80.0));
    // Fields the write did not mention keep their previous values.
    assert_eq!(applied.direction, GravitySource::default().direction);
    assert!(applied.enabled);

    // A second write over the updated snapshot toggles the source off.
    let updated = crate::serde::serialize_gravity_source(&applied);
    let toggle = std::collections::BTreeMap::from([("enabled".into(), Value::Bool(false))]);
    let toggled = script_style_merge_write(&updated, &toggle).expect("toggle write is accepted");
    assert!(!toggled.enabled);
    assert_eq!(toggled.mode, GravityMode::Point);
}

#[test]
fn gravity_source_script_write_rejects_unknown_fields_and_bad_enums() {
    use engine_serialize::Value;
    let base = crate::serde::serialize_gravity_source(&GravitySource::default());

    let unknown = std::collections::BTreeMap::from([("gravity".into(), Value::Float32(1.0))]);
    assert_eq!(
        script_style_merge_write(&base, &unknown),
        Err(vec!["gravity".to_string()])
    );

    let bad_mode = std::collections::BTreeMap::from([("mode".into(), Value::Enum("Warp".into()))]);
    assert_eq!(
        script_style_merge_write(&base, &bad_mode),
        Err(vec!["mode".to_string()])
    );

    let bad_falloff =
        std::collections::BTreeMap::from([("falloff".into(), Value::Enum("Wobbly".into()))]);
    assert_eq!(
        script_style_merge_write(&base, &bad_falloff),
        Err(vec!["falloff".to_string()])
    );

    let wrong_type =
        std::collections::BTreeMap::from([("strength".into(), Value::Str("heavy".into()))]);
    assert_eq!(
        script_style_merge_write(&base, &wrong_type),
        Err(vec!["strength".to_string()])
    );

    // Rejected writes never partially apply: the base snapshot is untouched.
    let unchanged = crate::serde::deserialize_gravity_source(&base);
    assert_eq!(
        *unchanged.downcast::<GravitySource>().unwrap(),
        GravitySource::default()
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// Gravity Source Scene Load / Validation Tests
// ══════════════════════════════════════════════════════════════════════════════

fn physics_test_registry() -> std::sync::Arc<engine_scene::ComponentRegistry> {
    let mut registry = engine_scene::ComponentRegistry::new();
    registry.register_core();
    crate::register_physics_extensions(&mut registry, None);
    std::sync::Arc::new(registry)
}

fn gravity_test_scene(
    fields: std::collections::BTreeMap<String, engine_serialize::Value>,
) -> engine_scene::Scene {
    engine_scene::Scene {
        schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
        engine_version: "0.1.0".to_string(),
        scene_id: "gravity-scene".to_string(),
        name: "Gravity Scene".to_string(),
        entities: vec![engine_scene::EntityRecord {
            persistent_id: "planet-01".to_string(),
            parent: None,
            name: Some("Planet".to_string()),
            enabled: true,
            components: std::collections::BTreeMap::from([(
                "engine.gravity_source".to_string(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields,
                },
            )]),
        }],
        scene_settings: engine_scene::SceneSettings::default(),
        dependencies: Vec::new(),
        diagnostics_policy: engine_scene::DiagnosticsPolicy::Strict,
    }
}

#[test]
fn gravity_source_scene_load_roundtrip() {
    use engine_serialize::Value;
    let scene = gravity_test_scene(std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("enabled".into(), Value::Bool(true)),
        ("strength".into(), Value::Float32(30.0)),
        ("center".into(), Value::Vec3([0.0, -100.0, 0.0])),
        ("falloff".into(), Value::Enum("InverseSquare".into())),
        ("max_radius".into(), Value::Float32(500.0)),
    ]));
    let registry = physics_test_registry();
    let world = engine_scene::World::try_from_scene_with_registry(&scene, registry)
        .expect("gravity source scene loads through the strict registry path");

    let entity = world.entity_by_persistent_id("planet-01").unwrap();
    let source = world
        .get::<GravitySource>(entity)
        .expect("source materialized");
    assert_eq!(source.mode, GravityMode::Point);
    assert_eq!(source.strength, 30.0);
    assert_eq!(source.center, glam::Vec3::new(0.0, -100.0, 0.0));
    assert_eq!(source.falloff, GravityFalloff::InverseSquare);
    assert_eq!(source.max_radius, Some(500.0));

    // Saving the world reproduces the authored component fields.
    let saved = world.to_scene();
    let record = &saved.entities[0].components["engine.gravity_source"];
    assert_eq!(
        record.fields.get("mode"),
        Some(&Value::Enum("Point".into()))
    );
    assert_eq!(
        record.fields.get("falloff"),
        Some(&Value::Enum("InverseSquare".into()))
    );
    assert_eq!(
        record.fields.get("max_radius"),
        Some(&Value::Float32(500.0))
    );
}

#[test]
fn gravity_source_scene_load_sanitizes_non_finite_fields() {
    use engine_serialize::Value;
    let scene = gravity_test_scene(std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("strength".into(), Value::Float32(f32::NAN)),
        ("center".into(), Value::Vec3([f32::INFINITY, 0.0, 0.0])),
    ]));
    let registry = physics_test_registry();
    let world = engine_scene::World::try_from_scene_with_registry(&scene, registry)
        .expect("scene with non-finite source data still loads");
    let entity = world.entity_by_persistent_id("planet-01").unwrap();
    let source = world.get::<GravitySource>(entity).unwrap();
    assert!(source.strength.is_finite());
    assert!(source.center.is_finite());
}

#[test]
fn gravity_source_authoring_validation_accepts_component() {
    use engine_serialize::Value;
    let scene = gravity_test_scene(std::collections::BTreeMap::from([
        ("mode".into(), Value::Enum("Point".into())),
        ("strength".into(), Value::Float32(12.5)),
        ("center".into(), Value::Vec3([1.0, 2.0, 3.0])),
        ("falloff".into(), Value::Enum("Linear".into())),
        ("max_radius".into(), Value::Float32(25.0)),
    ]));
    let registry = physics_test_registry();
    engine_scene::validate_scene_for_authoring(&scene, Some(&registry))
        .expect("authoring preflight accepts gravity source components");
}

#[test]
fn gravity_source_registers_with_script_binding() {
    let registry = physics_test_registry();
    let extension = registry
        .get(GravitySource::TYPE_ID)
        .expect("gravity source registered");
    assert!(extension.meta.has_editor);
    assert!(extension.meta.has_script_binding());
    assert_eq!(
        extension.meta.script_access,
        engine_scene::ScriptAccess::ReadWrite
    );
    assert!(extension.serialize.is_some());
    assert!(extension.deserialize.is_some());
}

// ══════════════════════════════════════════════════════════════════════════════
