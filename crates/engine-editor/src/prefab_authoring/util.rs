use std::collections::{BTreeMap, BTreeSet};

use engine_scene::{Component, EntityRecord, PrefabInstanceRef};
use engine_serialize::{PersistentId, Value};

pub(super) fn instance_id(entity: &EntityRecord) -> Option<String> {
    let component = entity.components.get(PrefabInstanceRef::TYPE_ID)?;
    match component.fields.get("instance_id") {
        Some(Value::Str(instance_id)) if !instance_id.is_empty() => Some(instance_id.clone()),
        _ => None,
    }
}

pub(super) fn remap_entity_values(
    value: &mut Value,
    id_map: &BTreeMap<PersistentId, PersistentId>,
) {
    match value {
        Value::Entity(entity_id) => {
            if let Some(remapped) = id_map.get(entity_id) {
                *entity_id = remapped.clone();
            }
        }
        Value::List(values) => {
            for value in values {
                remap_entity_values(value, id_map);
            }
        }
        Value::Map(values) => {
            for value in values.values_mut() {
                remap_entity_values(value, id_map);
            }
        }
        _ => {}
    }
}

pub(super) fn allocate_unique_string(base: &str, used: &mut BTreeSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    for suffix in 2_u64.. {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("the u64 identifier suffix space cannot be exhausted in memory")
}

pub(super) fn portable_token(value: &str) -> String {
    let token = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    token.trim_matches('-').to_string()
}
