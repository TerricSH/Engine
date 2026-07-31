use crate::commands::SetComponentField;
use engine_serialize::{PersistentId, Value};

use super::{AssetEntry, AssetKind};

/// Construct the real serialized-scene edit for a renderer asset.
///
/// This never mutates an ECS [`World`]. Callers execute the returned command
/// through `EditorScene`/`CommandHistory`, preserving undo and dirty tracking.
pub fn assignment_command(
    target_entity: PersistentId,
    asset: &AssetEntry,
) -> Option<SetComponentField> {
    let field = match asset.kind {
        AssetKind::Mesh => "mesh",
        AssetKind::Material => "material",
        AssetKind::Texture
        | AssetKind::Shader
        | AssetKind::Scene
        | AssetKind::Pipeline
        | AssetKind::Script
        | AssetKind::Audio
        | AssetKind::Font
        | AssetKind::Animation
        | AssetKind::Skeleton
        | AssetKind::NavMesh
        | AssetKind::Logic
        | AssetKind::Prefab
        | AssetKind::EnvironmentMap
        | AssetKind::MorphTargetSet
        | AssetKind::Unknown => return None,
    };
    Some(SetComponentField::new(
        target_entity,
        "engine.renderable".to_string(),
        field.to_string(),
        Value::Asset(asset.id.clone()),
    ))
}
