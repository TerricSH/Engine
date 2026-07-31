use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use engine_asset::cook::{AssetType, SourceManifest};
use engine_asset::AssetRegistry;
use engine_scene::{
    sample_scene, Component, ComponentRecord, EntityRecord, PrefabInstanceRef, Scene,
};
use engine_serialize::{AssetId, SchemaVersion, Value};

use super::instantiation::prefab_instance_component;
use super::source::PREFAB_TRANSACTION_COUNTER;
use super::*;
use crate::commands::EntityPasteParent;
use crate::EditorScene;

fn transform() -> ComponentRecord {
    ComponentRecord {
        schema_version: SchemaVersion::new(0, 1, 0),
        enabled: true,
        fields: BTreeMap::from([
            ("translation".into(), Value::Vec3([0.0, 0.0, 0.0])),
            ("rotation".into(), Value::Quat([0.0, 0.0, 0.0, 1.0])),
            ("scale".into(), Value::Vec3([1.0, 1.0, 1.0])),
        ]),
    }
}

fn authoring_scene() -> Scene {
    let mut scene = sample_scene();
    scene.scene_settings.active_camera = None;
    scene.entities = vec![
        EntityRecord {
            persistent_id: "vehicle".into(),
            parent: None,
            name: Some("Vehicle".into()),
            enabled: true,
            components: BTreeMap::from([("engine.transform".into(), transform())]),
        },
        EntityRecord {
            persistent_id: "wheel".into(),
            parent: Some("vehicle".into()),
            name: Some("Wheel".into()),
            enabled: true,
            components: BTreeMap::from([("engine.transform".into(), transform())]),
        },
    ];
    scene
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "engine_editor_prefab_{name}_{}_{}",
        std::process::id(),
        PREFAB_TRANSACTION_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn scene_subtree_becomes_self_contained_prefab() {
    let mut scene = authoring_scene();
    scene.entities[0].components.insert(
        PrefabInstanceRef::TYPE_ID.into(),
        prefab_instance_component(&AssetId::new("prefab-old"), "old", "vehicle"),
    );
    let prefab =
        prefab_from_scene_subtree(&scene, &"vehicle".into(), AssetId::new("prefab-vehicle"))
            .unwrap();
    assert_eq!(prefab.hierarchy.len(), 2);
    assert!(prefab.hierarchy[0].parent.is_none());
    assert!(prefab
        .hierarchy
        .iter()
        .all(|record| !record.components.contains_key(PrefabInstanceRef::TYPE_ID)));
}

#[test]
fn create_is_manifest_and_source_transaction() {
    let root = temp_root("create");
    let manifest_path = root.join("game.manifest");
    let request = PrefabAssetCreateRequest {
        source_root: &root,
        manifest_path: &manifest_path,
        relative_source_path: Path::new("Prefabs/vehicle.prefab.ron"),
        asset_id: AssetId::new("prefab-vehicle"),
    };
    let created =
        create_prefab_asset_from_scene(&authoring_scene(), &"vehicle".into(), request).unwrap();
    assert_eq!(
        load_prefab_source(&created.source_path).unwrap(),
        created.prefab
    );
    let manifest: SourceManifest =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(manifest.assets[0].asset_type, AssetType::Prefab);

    let manifest_before = std::fs::read(&manifest_path).unwrap();
    let duplicate = PrefabAssetCreateRequest {
        source_root: &root,
        manifest_path: &manifest_path,
        relative_source_path: Path::new("Prefabs/other.prefab.ron"),
        asset_id: AssetId::new("prefab-vehicle"),
    };
    assert!(
        create_prefab_asset_from_scene(&authoring_scene(), &"vehicle".into(), duplicate).is_err()
    );
    assert_eq!(std::fs::read(&manifest_path).unwrap(), manifest_before);
    assert!(!root.join("Prefabs/other.prefab.ron").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn instantiate_and_unpack_are_atomic_undo_commands() {
    let prefab = prefab_from_scene_subtree(
        &authoring_scene(),
        &"vehicle".into(),
        AssetId::new("prefab-vehicle"),
    )
    .unwrap();
    let mut destination = sample_scene();
    destination.scene_settings.active_camera = None;
    let plan =
        prepare_prefab_instantiation(&destination, &prefab, None, EntityPasteParent::SceneRoot)
            .unwrap();
    let root_id = plan.root_entity_id().clone();
    let inserted = plan.entity_ids().to_vec();
    let original_count = destination.entities.len();
    let mut editor = EditorScene::new(destination);
    editor.execute(plan.into_command()).unwrap();
    assert_eq!(editor.scene.entities.len(), original_count + 2);
    assert!(inserted.iter().all(|id| editor
        .scene
        .entities
        .iter()
        .find(|entity| &entity.persistent_id == id)
        .unwrap()
        .components
        .contains_key(PrefabInstanceRef::TYPE_ID)));

    let unpack =
        prepare_unpack_prefab(&editor.scene, &root_id, PrefabUnpackMode::Completely).unwrap();
    assert_eq!(unpack.entity_ids().len(), 2);
    editor.execute(unpack.into_command()).unwrap();
    assert!(inserted.iter().all(|id| !editor
        .scene
        .entities
        .iter()
        .find(|entity| &entity.persistent_id == id)
        .unwrap()
        .components
        .contains_key(PrefabInstanceRef::TYPE_ID)));
    editor.undo().unwrap();
    assert!(inserted.iter().all(|id| editor
        .scene
        .entities
        .iter()
        .find(|entity| &entity.persistent_id == id)
        .unwrap()
        .components
        .contains_key(PrefabInstanceRef::TYPE_ID)));
}

#[test]
fn loaded_registry_is_a_real_instantiation_source() {
    let prefab = prefab_from_scene_subtree(
        &authoring_scene(),
        &"vehicle".into(),
        AssetId::new("prefab-vehicle"),
    )
    .unwrap();
    let mut assets = AssetRegistry::new();
    assets.insert_typed(AssetId::new("prefab-vehicle"), prefab);
    let scene = sample_scene();
    let plan = prepare_prefab_instantiation_from_registry(
        &scene,
        &assets,
        &AssetId::new("prefab-vehicle"),
        EntityPasteParent::SceneRoot,
    )
    .unwrap();
    assert_eq!(plan.entity_ids().len(), 2);
}
