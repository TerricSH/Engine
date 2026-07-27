//! Scene serialization hooks for animation ECS components.

use std::collections::BTreeMap;

use engine_serialize::{AssetId, Value};
use serde::{de::DeserializeOwned, Serialize};

use crate::{AnimationPlayer, IkTargetComponent, SkeletonComponent};

fn encode_blob<T: Serialize>(value: &T, component_name: &str) -> Value {
    match bincode::serialize(value) {
        Ok(bytes) => Value::List(
            bytes
                .into_iter()
                .map(|byte| Value::UInt(u64::from(byte)))
                .collect(),
        ),
        Err(error) => {
            tracing::error!(%error, component = component_name, "failed to serialize animation component");
            Value::List(Vec::new())
        }
    }
}

fn decode_blob<T: DeserializeOwned>(fields: &BTreeMap<String, Value>) -> Option<T> {
    let Value::List(values) = fields.get("runtime_data")? else {
        return None;
    };
    let bytes = values
        .iter()
        .map(|value| match value {
            Value::UInt(byte) => u8::try_from(*byte).ok(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    bincode::deserialize(&bytes).ok()
}

pub(crate) fn serialize_animation_player(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let Some(player) = component.downcast_ref::<AnimationPlayer>() else {
        tracing::error!("AnimationPlayer serialization received the wrong component type");
        return BTreeMap::new();
    };
    let mut fields = BTreeMap::new();
    if let Some(clip_asset) = &player.clip_asset {
        fields.insert("clip_asset".into(), Value::Asset(AssetId::new(clip_asset)));
    }
    fields.insert("playing".into(), Value::Bool(player.playing));
    fields.insert("looping".into(), Value::Bool(player.looping));
    fields.insert("speed".into(), Value::Float32(player.speed));
    fields.insert("current_time".into(), Value::Float32(player.current_time));
    fields.insert("layer".into(), Value::UInt(u64::from(player.layer)));
    fields.insert(
        "runtime_data".into(),
        encode_blob(player, "AnimationPlayer"),
    );
    fields
}

pub(crate) fn deserialize_animation_player(
    fields: &BTreeMap<String, Value>,
) -> Box<dyn std::any::Any> {
    let mut player: AnimationPlayer = decode_blob(fields).unwrap_or_default();
    match fields.get("clip_asset") {
        Some(Value::Asset(clip_asset)) => player.clip_asset = Some(clip_asset.id.clone()),
        Some(Value::Str(clip_asset)) => player.clip_asset = Some(clip_asset.clone()),
        _ => {}
    }
    if let Some(Value::Bool(playing)) = fields.get("playing") {
        player.playing = *playing;
    }
    if let Some(Value::Bool(looping)) = fields.get("looping") {
        player.looping = *looping;
    }
    if let Some(Value::Float32(speed)) = fields.get("speed") {
        player.speed = *speed;
    }
    if let Some(Value::Float32(current_time)) = fields.get("current_time") {
        player.current_time = *current_time;
    }
    if let Some(Value::UInt(layer)) = fields.get("layer") {
        if let Ok(layer) = u32::try_from(*layer) {
            player.layer = layer;
        }
    }
    Box::new(player)
}

pub(crate) fn serialize_skeleton_component(
    component: &dyn std::any::Any,
) -> BTreeMap<String, Value> {
    let Some(skeleton) = component.downcast_ref::<SkeletonComponent>() else {
        tracing::error!("SkeletonComponent serialization received the wrong component type");
        return BTreeMap::new();
    };
    let mut fields = BTreeMap::new();
    if let Some(asset) = &skeleton.skeleton_asset {
        fields.insert("skeleton_asset".into(), Value::Asset(AssetId::new(asset)));
    }
    if let Some(asset) = &skeleton.morph_target_set {
        fields.insert("morph_target_set".into(), Value::Asset(AssetId::new(asset)));
    }
    fields.insert(
        "morph_weights".into(),
        Value::List(
            skeleton
                .morph_weights
                .iter()
                .copied()
                .map(Value::Float32)
                .collect(),
        ),
    );
    fields.insert("bind_shape".into(), Value::Vec3(skeleton.bind_shape));
    fields
}

pub(crate) fn deserialize_skeleton_component(
    fields: &BTreeMap<String, Value>,
) -> Box<dyn std::any::Any> {
    let skeleton_asset = match fields.get("skeleton_asset") {
        Some(Value::Asset(asset)) => Some(asset.id.clone()),
        Some(Value::Str(asset)) => Some(asset.clone()),
        _ => None,
    };
    let bind_shape = match fields.get("bind_shape") {
        Some(Value::Vec3(shape)) if shape.iter().all(|value| value.is_finite()) => *shape,
        _ => [0.5; 3],
    };
    let morph_target_set = match fields.get("morph_target_set") {
        Some(Value::Asset(asset)) => Some(asset.id.clone()),
        Some(Value::Str(asset)) => Some(asset.clone()),
        _ => None,
    };
    let morph_weights = match fields.get("morph_weights") {
        Some(Value::List(weights)) => weights
            .iter()
            .filter_map(|weight| match weight {
                Value::Float32(weight) if weight.is_finite() => Some(weight.clamp(-1.0, 1.0)),
                _ => None,
            })
            .take(engine_renderer::MAX_MORPH_TARGETS)
            .collect(),
        _ => Vec::new(),
    };
    Box::new(SkeletonComponent {
        skeleton_asset,
        bind_shape,
        morph_target_set,
        morph_weights,
    })
}

pub(crate) fn serialize_ik_target(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let Some(target) = component.downcast_ref::<IkTargetComponent>() else {
        tracing::error!("IkTargetComponent serialization received the wrong component type");
        return BTreeMap::new();
    };
    let mut fields = BTreeMap::new();
    fields.insert("enabled".into(), Value::Bool(target.enabled));
    fields.insert("blend_weight".into(), Value::Float32(target.blend_weight));
    fields.insert(
        "runtime_data".into(),
        encode_blob(target, "IkTargetComponent"),
    );
    fields
}

pub(crate) fn deserialize_ik_target(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let mut target: IkTargetComponent = decode_blob(fields).unwrap_or_default();
    if let Some(Value::Bool(enabled)) = fields.get("enabled") {
        target.enabled = *enabled;
    }
    if let Some(Value::Float32(weight)) = fields.get("blend_weight") {
        target.blend_weight = weight.clamp(0.0, 1.0);
    }
    Box::new(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnimLayer, BoneIndex, IkEffector};
    use glam::Vec3;

    #[test]
    fn animation_player_scene_roundtrip_preserves_runtime_state() {
        let mut player = AnimationPlayer::with_clip("walk");
        player.current_time = 1.25;
        player.layers.push(AnimLayer::new("upper_body"));
        let fields = serialize_animation_player(&player);
        assert_eq!(
            fields.get("clip_asset"),
            Some(&Value::Asset(AssetId::new("walk")))
        );
        let restored = deserialize_animation_player(&fields);
        let restored = restored.downcast_ref::<AnimationPlayer>().unwrap();
        assert_eq!(restored.clip_asset.as_deref(), Some("walk"));
        assert_eq!(restored.current_time, 1.25);
        assert_eq!(restored.layers.len(), 2);
    }

    #[test]
    fn skeleton_scene_roundtrip_preserves_asset_and_shape() {
        let skeleton = SkeletonComponent {
            skeleton_asset: Some("hero.skeleton".into()),
            bind_shape: [0.4, 1.0, 0.3],
            morph_target_set: Some("hero.morphs".into()),
            morph_weights: vec![0.25, 0.75],
        };
        let fields = serialize_skeleton_component(&skeleton);
        assert_eq!(
            fields.get("skeleton_asset"),
            Some(&Value::Asset(AssetId::new("hero.skeleton")))
        );
        let restored = deserialize_skeleton_component(&fields);
        let restored = restored.downcast_ref::<SkeletonComponent>().unwrap();
        assert_eq!(restored.skeleton_asset, skeleton.skeleton_asset);
        assert_eq!(restored.bind_shape, skeleton.bind_shape);
        assert_eq!(restored.morph_target_set, skeleton.morph_target_set);
        assert_eq!(restored.morph_weights, skeleton.morph_weights);
    }

    #[test]
    fn animation_player_accepts_legacy_string_asset_reference() {
        let fields = BTreeMap::from([("clip_asset".into(), Value::Str("walk".into()))]);

        let restored = deserialize_animation_player(&fields);
        let restored = restored.downcast_ref::<AnimationPlayer>().unwrap();

        assert_eq!(restored.clip_asset.as_deref(), Some("walk"));
    }

    #[test]
    fn skeleton_accepts_legacy_string_asset_reference() {
        let fields =
            BTreeMap::from([("skeleton_asset".into(), Value::Str("hero.skeleton".into()))]);

        let restored = deserialize_skeleton_component(&fields);
        let restored = restored.downcast_ref::<SkeletonComponent>().unwrap();

        assert_eq!(restored.skeleton_asset.as_deref(), Some("hero.skeleton"));
    }

    #[test]
    fn ik_scene_roundtrip_preserves_effectors() {
        let mut target = IkTargetComponent::new();
        target.add_effector(IkEffector::new("hand", BoneIndex(0), Vec3::X));
        target.blend_weight = 0.75;
        let fields = serialize_ik_target(&target);
        let restored = deserialize_ik_target(&fields);
        let restored = restored.downcast_ref::<IkTargetComponent>().unwrap();
        assert_eq!(restored.effectors.len(), 1);
        assert_eq!(restored.blend_weight, 0.75);
    }
}
