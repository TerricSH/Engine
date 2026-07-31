    use super::*;

    #[test]
    fn hierarchy_snapshot_preserves_parent_order_and_nesting() {
        let scene = engine_scene::sample_scene();
        let snapshot = hierarchy_snapshot(&scene.entities);
        assert!(!snapshot.is_empty());
        let ids = snapshot
            .iter()
            .map(|entity| entity.id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), snapshot.len());
    }

    #[test]
    fn multi_selection_exposes_only_components_shared_by_every_entity() {
        let scene = engine_scene::sample_scene();
        let first = &scene.entities[0];
        let second = scene
            .entities
            .iter()
            .skip(1)
            .find(|entity| entity.components.keys().ne(first.components.keys()))
            .expect("sample scene must contain entities with different components");
        let selected_ids = vec![first.persistent_id.clone(), second.persistent_id.clone()];

        let snapshot = selection_snapshot(
            &scene.entities,
            &selected_ids,
            Some(first.persistent_id.as_str()),
        );
        let expected = first
            .components
            .keys()
            .filter(|type_id| second.components.contains_key(*type_id))
            .cloned()
            .collect::<BTreeSet<_>>();
        let actual = snapshot
            .components
            .iter()
            .map(|component| component.type_id.clone())
            .collect::<BTreeSet<_>>();

        assert_eq!(snapshot.entity_ids, selected_ids);
        assert_eq!(actual, expected);
    }

    #[test]
    fn integer_inspector_values_keep_full_precision() {
        let field = component_field_snapshot("engine.camera", "mask", &Value::UInt(u64::MAX));
        assert_eq!(field.value, json!(u64::MAX.to_string()));
        assert_eq!(field.engine_value, Value::UInt(u64::MAX));
    }

    #[test]
    fn react_asset_fields_expose_compatible_kinds_and_complete_asset_ids() {
        let mesh = component_field_snapshot(
            "engine.renderable",
            "mesh",
            &Value::Asset(AssetId::with_path("hero", "models/hero.glb")),
        );
        let material = component_field_snapshot(
            "engine.renderable",
            "material",
            &Value::Asset(AssetId::new("hero-material")),
        );
        assert_eq!(mesh.accepted_asset_kinds, &["model"]);
        assert_eq!(material.accepted_asset_kinds, &["material"]);

        let value = serde_json::to_value(AssetDto {
            id: "hero".into(),
            asset_id: AssetId::with_path("hero", "models/hero.glb"),
            name: "Hero".into(),
            path: "models/hero.glb".into(),
            kind: "model",
            loaded: true,
            cooked: true,
            manifest_declared: true,
        })
        .unwrap();
        assert_eq!(value["assetId"]["id"], json!("hero"));
        assert_eq!(value["assetId"]["logical_path"], json!("models/hero.glb"));
    }

    #[test]
    fn every_asset_kind_has_a_stable_react_category() {
        let kinds = [
            AssetKind::Mesh,
            AssetKind::Texture,
            AssetKind::Shader,
            AssetKind::Scene,
            AssetKind::Material,
            AssetKind::Pipeline,
            AssetKind::Script,
            AssetKind::Audio,
            AssetKind::Font,
            AssetKind::Animation,
            AssetKind::Skeleton,
            AssetKind::NavMesh,
            AssetKind::Logic,
            AssetKind::Prefab,
            AssetKind::EnvironmentMap,
            AssetKind::MorphTargetSet,
            AssetKind::Unknown,
        ];
        assert!(kinds.iter().all(|kind| !asset_kind_name(*kind).is_empty()));
        assert_eq!(asset_kind_name(AssetKind::EnvironmentMap), "texture");
        assert_eq!(asset_kind_name(AssetKind::MorphTargetSet), "model");
    }

    #[test]
    fn viewport_snapshot_exposes_complete_scene_camera_and_gizmo_state() {
        let viewport = ViewportDto {
            scene_camera: SceneCameraDto {
                pitch: 20.0,
                yaw: 45.0,
                distance: 10.0,
                target: [1.0, 2.0, 3.0],
                orthographic: false,
                speed: 5.0,
            },
            gizmos_visible: true,
            snapping_enabled: true,
        };
        let value = serde_json::to_value(viewport).unwrap();
        assert_eq!(value["sceneCamera"]["target"], json!([1.0, 2.0, 3.0]));
        assert_eq!(value["sceneCamera"]["speed"], json!(5.0));
        assert_eq!(value["gizmosVisible"], json!(true));
        assert_eq!(value["snappingEnabled"], json!(true));
    }
