//! Canonical prefab cooker.

use std::path::Path;

use engine_serialize::SchemaVersion;

use super::{write_cooked_artifact, AssetType, CookError, CookResult};

/// Cook a human-readable `*.prefab.ron` source into the validated binary
/// payload consumed by `engine_scene::prefab_loader`.
pub fn cook_prefab(source: &Path, output: &Path) -> Result<CookResult, CookError> {
    let source_bytes = std::fs::read(source)?;
    let mut payload = Vec::new();
    engine_scene::prefab_cooker(&source_bytes, &mut payload).map_err(CookError::InvalidAsset)?;
    write_cooked_artifact(
        output,
        AssetType::Prefab.kind_code(),
        &payload,
        SchemaVersion::new(0, 1, 0),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use engine_scene::{ComponentRecord, EntityRecord, Prefab};
    use engine_serialize::{AssetId, SchemaVersion};

    use super::*;
    use crate::cook::read_cooked_artifact;

    fn valid_prefab() -> Prefab {
        let mut prefab = Prefab::new(AssetId::new("prefab-test"));
        prefab.add_entity(EntityRecord {
            persistent_id: "root".into(),
            parent: None,
            name: Some("Root".into()),
            enabled: true,
            components: BTreeMap::from([(
                "engine.transform".into(),
                ComponentRecord {
                    schema_version: SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::new(),
                },
            )]),
        });
        prefab
    }

    #[test]
    fn ron_source_cooks_to_runtime_prefab_payload() {
        let root = std::env::temp_dir().join(format!(
            "engine_prefab_cook_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("test.prefab.ron");
        let output = root.join("test.cooked");
        std::fs::write(
            &source,
            engine_scene::serialize_prefab_source(&valid_prefab()).unwrap(),
        )
        .unwrap();

        cook_prefab(&source, &output).unwrap();
        let artifact = read_cooked_artifact(&output).unwrap();
        assert_eq!(artifact.header.asset_kind, AssetType::Prefab.kind_code());
        let loaded = engine_scene::prefab_loader(&artifact.payload).unwrap();
        assert_eq!(
            loaded.downcast::<Prefab>().unwrap(),
            Box::new(valid_prefab())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_or_empty_prefab_source_is_rejected_without_artifact() {
        let root = std::env::temp_dir().join(format!(
            "engine_prefab_cook_invalid_{}_{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("invalid.prefab.ron");
        let output = root.join("invalid.cooked");
        std::fs::write(&source, "(source_asset:(id:\"prefab-bad\"))").unwrap();

        assert!(cook_prefab(&source, &output).is_err());
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
