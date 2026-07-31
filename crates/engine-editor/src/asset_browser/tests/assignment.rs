use std::collections::BTreeMap;

use crate::commands::Command;
use engine_scene::{ComponentRecord, DiagnosticsPolicy, EntityRecord, Scene, SceneSettings};
use engine_serialize::{AssetId, SchemaVersion, Value};

use super::super::*;

fn scene_with_renderable() -> Scene {
    let mut fields = BTreeMap::new();
    fields.insert("mesh".to_string(), Value::Asset(AssetId::new("old-mesh")));
    fields.insert(
        "material".to_string(),
        Value::Asset(AssetId::new("old-material")),
    );
    let mut components = BTreeMap::new();
    components.insert(
        "engine.renderable".to_string(),
        ComponentRecord {
            schema_version: SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields,
        },
    );
    Scene {
        schema_version: SchemaVersion::new(0, 1, 0),
        engine_version: "test".to_string(),
        scene_id: "test-scene".to_string(),
        name: "Test".to_string(),
        entities: vec![EntityRecord {
            persistent_id: "target".to_string(),
            parent: None,
            name: None,
            enabled: true,
            components,
        }],
        scene_settings: SceneSettings::default(),
        dependencies: Vec::new(),
        diagnostics_policy: DiagnosticsPolicy::Strict,
    }
}

#[test]
fn mesh_and_material_create_real_undoable_scene_commands() {
    for (kind, field) in [(AssetKind::Mesh, "mesh"), (AssetKind::Material, "material")] {
        let id = AssetId::with_path(format!("new-{field}"), format!("assets/{field}"));
        let entry = AssetEntry::new(id.clone(), kind);
        let mut command = assignment_command("target".to_string(), &entry).unwrap();
        let mut scene = scene_with_renderable();
        command.execute(&mut scene).unwrap();

        assert_eq!(
            scene.entities[0].components["engine.renderable"].fields[field],
            Value::Asset(id)
        );
        command.undo(&mut scene).unwrap();
        assert_eq!(
            scene.entities[0].components["engine.renderable"].fields[field],
            Value::Asset(AssetId::new(format!("old-{field}")))
        );
    }
}

#[test]
fn only_mesh_and_material_are_assignable_to_renderables() {
    for kind in [
        AssetKind::Texture,
        AssetKind::Shader,
        AssetKind::Scene,
        AssetKind::Pipeline,
        AssetKind::Script,
        AssetKind::Audio,
        AssetKind::Font,
        AssetKind::Animation,
        AssetKind::Skeleton,
        AssetKind::NavMesh,
        AssetKind::Logic,
        AssetKind::Unknown,
    ] {
        let asset = AssetEntry::new(AssetId::new(kind.label().to_lowercase()), kind);
        assert!(
            assignment_command("target".to_string(), &asset).is_none(),
            "{} must not be assignable to Renderable",
            kind.label()
        );
    }
}
