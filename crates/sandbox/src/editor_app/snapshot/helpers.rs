use super::*;

pub(super) fn hierarchy_snapshot(entities: &[EntityRecord]) -> Vec<HierarchyNodeDto> {
    fn children(
        entities: &[EntityRecord],
        parent: Option<&str>,
        visiting: &mut BTreeSet<String>,
    ) -> Vec<HierarchyNodeDto> {
        entities
            .iter()
            .filter(|entity| entity.parent.as_deref() == parent)
            .filter_map(|entity| {
                if !visiting.insert(entity.persistent_id.clone()) {
                    return None;
                }
                let children = children(entities, Some(&entity.persistent_id), visiting);
                visiting.remove(&entity.persistent_id);
                Some(HierarchyNodeDto {
                    id: entity.persistent_id.clone(),
                    name: entity
                        .name
                        .clone()
                        .unwrap_or_else(|| entity.persistent_id.clone()),
                    enabled: entity.enabled,
                    expanded: true,
                    prefab: prefab_asset(entity),
                    children,
                })
            })
            .collect()
    }

    children(entities, None, &mut BTreeSet::new())
}

fn prefab_asset(entity: &EntityRecord) -> Option<String> {
    let component = entity.components.get(PrefabInstanceRef::TYPE_ID)?;
    match component.fields.get("source_asset")? {
        Value::Asset(asset) => Some(asset.id.clone()),
        Value::Str(asset) => Some(asset.clone()),
        _ => None,
    }
}

pub(super) fn selection_snapshot(
    entities: &[EntityRecord],
    selected_ids: &[String],
    selected: Option<&str>,
) -> SelectionDto {
    let Some(selected) = selected else {
        return SelectionDto::default();
    };
    let Some(entity) = entities
        .iter()
        .find(|entity| entity.persistent_id == selected)
    else {
        return SelectionDto::default();
    };
    let mut entity_ids = selected_ids
        .iter()
        .filter(|id| entities.iter().any(|entity| &entity.persistent_id == *id))
        .cloned()
        .collect::<Vec<_>>();
    if !entity_ids.iter().any(|id| id == selected) {
        entity_ids.push(entity.persistent_id.clone());
    }
    let selected_entities = entity_ids
        .iter()
        .filter_map(|id| {
            entities
                .iter()
                .find(|candidate| &candidate.persistent_id == id)
        })
        .collect::<Vec<_>>();
    let transform = selected_entities
        .iter()
        .all(|selected_entity| selected_entity.components.contains_key("engine.transform"))
        .then(|| transform_snapshot(entity))
        .flatten();
    let components = entity
        .components
        .iter()
        .filter(|(type_id, _)| {
            selected_entities
                .iter()
                .all(|selected_entity| selected_entity.components.contains_key(*type_id))
        })
        .map(|(type_id, component)| {
            let descriptor = ComponentCatalog::descriptor(type_id);
            ComponentDto {
                type_id: type_id.clone(),
                display_name: descriptor
                    .map(|descriptor| descriptor.display_name.to_string())
                    .unwrap_or_else(|| type_id.clone()),
                enabled: component.enabled,
                removable: descriptor.is_none_or(|descriptor| descriptor.removable),
                resettable: descriptor.is_some(),
                remove_blocked_reason: selected_entities.iter().find_map(|selected_entity| {
                    selected_entity
                        .components
                        .keys()
                        .find_map(|dependent_type| {
                            let dependent = ComponentCatalog::descriptor(dependent_type)?;
                            dependent
                                .required_components
                                .contains(&type_id.as_str())
                                .then(|| {
                                    format!("{} requires this component", dependent.display_name)
                                })
                        })
                }),
                fields: component
                    .fields
                    .iter()
                    .map(|(name, value)| component_field_snapshot(type_id, name, value))
                    .collect(),
            }
        })
        .collect();
    SelectionDto {
        entity_ids,
        active_entity_id: Some(entity.persistent_id.clone()),
        display_name: Some(
            entity
                .name
                .clone()
                .unwrap_or_else(|| entity.persistent_id.clone()),
        ),
        active: Some(entity.enabled),
        transform,
        components,
    }
}

fn transform_snapshot(entity: &EntityRecord) -> Option<TransformDto> {
    let transform = entity.components.get("engine.transform")?;
    let position = match transform.fields.get("translation")? {
        Value::Vec3(value) => *value,
        _ => return None,
    };
    let rotation = match transform.fields.get("rotation")? {
        Value::Quat(value) => *value,
        _ => return None,
    };
    let scale = match transform.fields.get("scale")? {
        Value::Vec3(value) => *value,
        _ => return None,
    };
    let (x, y, z) = Quat::from_array(rotation).to_euler(EulerRot::XYZ);
    Some(TransformDto {
        position: position.into(),
        rotation_euler: Vec3Dto::from([x.to_degrees(), y.to_degrees(), z.to_degrees()]),
        scale: scale.into(),
    })
}

pub(super) fn component_field_snapshot(
    component_type: &str,
    name: &str,
    value: &Value,
) -> ComponentFieldDto {
    let (value_type, plain_value) = match value {
        Value::Bool(value) => ("boolean", json!(value)),
        Value::Int(value) => ("number", json!(value.to_string())),
        Value::UInt(value) => ("number", json!(value.to_string())),
        Value::Float32(value) => ("number", json!(value)),
        Value::Float64(value) => ("number", json!(value)),
        Value::Str(value) => ("string", json!(value)),
        Value::Vec3(value) => ("vec3", json!(Vec3Dto::from(*value))),
        Value::Quat(value) => ("vec4", json!(value)),
        Value::Color(value) => ("color", json!(value)),
        Value::Asset(value) => ("asset", json!(value.id)),
        Value::Entity(value) => ("string", json!(value)),
        Value::Enum(value) => ("enum", json!(value)),
        Value::List(value) => (
            "list",
            serde_json::to_value(value).unwrap_or(JsonValue::Null),
        ),
        Value::Map(value) => (
            "map",
            serde_json::to_value(value).unwrap_or(JsonValue::Null),
        ),
    };
    ComponentFieldDto {
        path: name.to_string(),
        label: humanize(name),
        value: plain_value,
        value_type,
        engine_value: value.clone(),
        accepted_asset_kinds: match (component_type, name) {
            ("engine.renderable", "mesh") => &["model"],
            ("engine.renderable", "material") => &["material"],
            ("engine.audio_source", "clip_asset") => &["audio"],
            ("engine.nav_agent", "navmesh_ref") => &["navmesh"],
            _ => &[],
        },
    }
}

fn humanize(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut uppercase = true;
    for character in value.chars() {
        if matches!(character, '_' | '-') {
            output.push(' ');
            uppercase = true;
        } else if uppercase {
            output.extend(character.to_uppercase());
            uppercase = false;
        } else {
            output.push(character);
        }
    }
    output
}

pub(super) fn asset_snapshot(entry: &AssetEntry) -> AssetDto {
    AssetDto {
        id: entry.id.id.clone(),
        asset_id: entry.id.clone(),
        name: entry.display_name(),
        path: entry
            .browser_path()
            .map(str::to_string)
            .unwrap_or_else(|| entry.id.id.clone()),
        kind: asset_kind_name(entry.kind),
        loaded: entry.loaded,
        cooked: entry.cooked,
        manifest_declared: entry.manifest_declared,
    }
}

pub(super) fn asset_kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Scene => "scene",
        AssetKind::Prefab => "prefab",
        AssetKind::Mesh
        | AssetKind::Skeleton
        | AssetKind::Animation
        | AssetKind::MorphTargetSet => "model",
        AssetKind::Material => "material",
        AssetKind::Texture | AssetKind::EnvironmentMap => "texture",
        AssetKind::Audio => "audio",
        AssetKind::Script => "script",
        AssetKind::Shader | AssetKind::Pipeline => "shader",
        AssetKind::NavMesh => "navmesh",
        AssetKind::Font | AssetKind::Logic | AssetKind::Unknown => "other",
    }
}

pub(super) fn diagnostics_snapshot(scene: Option<&EditorScene>) -> Vec<ConsoleEntryDto> {
    scene
        .into_iter()
        .flat_map(|scene| scene.diagnostics.all_entries().iter())
        .enumerate()
        .map(|(index, entry)| {
            let diagnostic = &entry.diagnostic;
            ConsoleEntryDto {
                id: format!("{index}:{}", diagnostic.code),
                timestamp: format!("-{:.3}s", entry.timestamp.elapsed().as_secs_f32()),
                level: match diagnostic.severity {
                    DiagnosticSeverity::Info => "info",
                    DiagnosticSeverity::Warning => "warning",
                    DiagnosticSeverity::Error | DiagnosticSeverity::Fatal => "error",
                },
                source: diagnostic.system.clone(),
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
                path: diagnostic.path.clone(),
                entity: diagnostic.entity.clone(),
                suggested_action: diagnostic.suggested_action.clone(),
            }
        })
        .collect()
}

pub(super) fn frame_stats_snapshot(
    stats: &engine_editor::performance::FrameStats,
) -> FrameStatsDto {
    FrameStatsDto {
        frame_time_ms: stats.frame_time_ms,
        draw_calls: stats.draw_calls,
        triangles: stats.triangles,
        physics_bodies: stats.physics_bodies,
        animation_count: stats.animation_count,
        nav_agents: stats.nav_agents,
        asset_count: stats.asset_count,
    }
}
