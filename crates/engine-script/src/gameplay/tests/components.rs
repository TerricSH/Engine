use super::*;

#[test]
fn component_value_roundtrips_every_scene_value_variant_except_entities() {
    use engine_serialize::Value as SceneValue;

    let cases = vec![
        SceneValue::Bool(true),
        SceneValue::Int(-7),
        SceneValue::UInt(u32::MAX as u64),
        SceneValue::Float32(0.75),
        SceneValue::Float64(2.5),
        SceneValue::Str("hello".into()),
        SceneValue::Enum("Dynamic".into()),
        SceneValue::Asset(engine_serialize::AssetId::new("audio.beep")),
        SceneValue::Vec3([1.0, 2.0, 3.0]),
        SceneValue::Quat([0.0, 0.0, 0.0, 1.0]),
        SceneValue::Color([0.1, 0.2, 0.3, 1.0]),
        SceneValue::List(vec![SceneValue::Float32(1.0), SceneValue::Bool(false)]),
        SceneValue::Map(BTreeMap::from([(
            "shape".into(),
            SceneValue::Map(BTreeMap::from([(
                "radius".into(),
                SceneValue::Float32(0.5),
            )])),
        )])),
    ];
    for scene_value in cases {
        let wire = GameplayComponentValue::from_scene_value(&scene_value)
            .expect("supported scene value converts to the wire form");
        let json = serde_json::to_string(&wire).unwrap();
        let decoded: GameplayComponentValue = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, wire, "wire JSON round-trip failed for {json}");
        let restored = decoded.to_scene_value();
        if let SceneValue::Float64(value) = scene_value {
            // The wire carries f32; Float64 sources land in Float32.
            assert_eq!(restored, SceneValue::Float32(value as f32));
        } else {
            assert_eq!(restored, scene_value, "scene round-trip mismatch");
        }
    }

    assert!(
        GameplayComponentValue::from_scene_value(&SceneValue::Entity("cube-01".into())).is_none()
    );
}

#[test]
fn component_value_has_a_stable_tagged_json_contract() {
    let value = GameplayComponentValue::Map(BTreeMap::from([
        ("volume".into(), GameplayComponentValue::Float(0.5)),
        ("playing".into(), GameplayComponentValue::Bool(true)),
        (
            "color".into(),
            GameplayComponentValue::Color([1.0, 0.5, 0.25, 1.0]),
        ),
    ]));
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"map","value":{"color":{"type":"color","value":[1.0,0.5,0.25,1.0]},"playing":{"type":"bool","value":true},"volume":{"type":"float","value":0.5}}}"#
    );
}

#[test]
fn untrusted_component_values_reject_non_finite_oversized_and_deep_payloads() {
    assert!(GameplayComponentValue::Float(f32::NAN).validate(0).is_err());
    assert!(GameplayComponentValue::Vec3([0.0, f32::INFINITY, 0.0])
        .validate(0)
        .is_err());
    assert!(GameplayComponentValue::Str("x".repeat(4097))
        .validate(0)
        .is_err());
    assert!(GameplayComponentValue::Str("bad\u{0007}".into())
        .validate(0)
        .is_err());
    assert!(GameplayComponentValue::List(vec![
        GameplayComponentValue::Bool(true);
        MAX_COMPONENT_LIST_ITEMS + 1
    ])
    .validate(0)
    .is_err());

    let mut deep = GameplayComponentValue::Bool(true);
    for _ in 0..=MAX_COMPONENT_VALUE_DEPTH {
        deep = GameplayComponentValue::List(vec![deep]);
    }
    assert!(deep.validate(0).is_err());

    let shallow = GameplayComponentValue::Map(BTreeMap::from([(
        "shape".into(),
        GameplayComponentValue::Map(BTreeMap::from([(
            "radius".into(),
            GameplayComponentValue::Float(0.5),
        )])),
    )]));
    assert!(shallow.validate(0).is_ok());
}

#[test]
fn component_type_keys_and_field_names_follow_the_identifier_contract() {
    for valid in [
        "engine.audio_source",
        "engine.physics.rigid_body",
        "engine.camera",
    ] {
        assert!(validate_component_type_key(valid).is_ok(), "{valid}");
    }
    for invalid in ["", "../evil", "engine audio", "engine/audio", "组件"] {
        assert!(validate_component_type_key(invalid).is_err(), "{invalid}");
    }
    assert!(validate_component_field_name("clip_asset").is_ok());
    assert!(validate_component_field_name("").is_err());
    assert!(validate_component_field_name("bad name").is_err());
}

#[test]
fn component_commands_have_a_stable_validated_json_contract() {
    let query = GameplayCommand::ComponentQuery {
        query: GameplayComponentQuery {
            query_id: 11,
            entity_id: "speaker-01".into(),
            component_type: "engine.audio_source".into(),
        },
    };
    assert_eq!(
        serde_json::to_string(&query).unwrap(),
        r#"{"type":"component_query","query":{"query_id":11,"entity_id":"speaker-01","component_type":"engine.audio_source"}}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(&serde_json::to_string(&query).unwrap()).unwrap(),
        query
    );
    assert!(query.validate().is_ok());

    let set = GameplayCommand::SetComponent {
        entity_id: "speaker-01".into(),
        component_type: "engine.audio_source".into(),
        fields: BTreeMap::from([
            ("volume".into(), GameplayComponentValue::Float(0.25)),
            ("playing".into(), GameplayComponentValue::Bool(true)),
        ]),
    };
    assert_eq!(
        serde_json::to_string(&set).unwrap(),
        r#"{"type":"set_component","entity_id":"speaker-01","component_type":"engine.audio_source","fields":{"playing":{"type":"bool","value":true},"volume":{"type":"float","value":0.25}}}"#
    );
    assert_eq!(
        serde_json::from_str::<GameplayCommand>(&serde_json::to_string(&set).unwrap()).unwrap(),
        set
    );
    assert!(set.validate().is_ok());

    for invalid in [
        GameplayCommand::ComponentQuery {
            query: GameplayComponentQuery {
                query_id: 1,
                entity_id: "../outside".into(),
                component_type: "engine.camera".into(),
            },
        },
        GameplayCommand::ComponentQuery {
            query: GameplayComponentQuery {
                query_id: 1,
                entity_id: "cube-01".into(),
                component_type: "engine camera".into(),
            },
        },
        GameplayCommand::SetComponent {
            entity_id: "cube-01".into(),
            component_type: "engine.camera".into(),
            fields: BTreeMap::from([("near".into(), GameplayComponentValue::Float(f32::INFINITY))]),
        },
        GameplayCommand::SetComponent {
            entity_id: "cube-01".into(),
            component_type: "engine.camera".into(),
            fields: BTreeMap::from([("bad field".into(), GameplayComponentValue::Float(1.0))]),
        },
    ] {
        assert!(invalid.validate().is_err());
    }

    let oversized = GameplayCommand::SetComponent {
        entity_id: "cube-01".into(),
        component_type: "engine.camera".into(),
        fields: (0..=MAX_COMPONENT_FIELDS)
            .map(|index| {
                (
                    format!("field_{index}"),
                    GameplayComponentValue::Bool(false),
                )
            })
            .collect(),
    };
    assert!(oversized.validate().is_err());
}
