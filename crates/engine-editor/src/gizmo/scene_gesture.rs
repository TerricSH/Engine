use super::*;

// ---------------------------------------------------------------------------
// Scene-level undoable Transform gestures
// ---------------------------------------------------------------------------

pub(super) const TRANSFORM_COMPONENT_TYPE: &str = "engine.transform";

/// Active scene-level gizmo gesture. The persistent ID is captured when the
/// drag begins, so later selection changes cannot redirect the edit.
#[derive(Clone, Debug)]
pub(crate) struct SceneGizmoDrag {
    entity_id: PersistentId,
    original: TransformFieldSnapshot,
}

/// Exact serialized Transform fields. `None` is significant: cancelling or
/// undoing a gesture restores an originally omitted default field by removing
/// it again, instead of materialising a synthetic value.
#[derive(Clone, Debug, PartialEq)]
struct TransformFieldSnapshot {
    translation: Option<Value>,
    rotation: Option<Value>,
    scale: Option<Value>,
}

#[derive(Clone, Copy, Debug)]
struct EffectiveTransform {
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

#[derive(Clone, Copy, Debug)]
struct ResolvedWorldTransform {
    matrix: Mat4,
    rotation: Quat,
}

impl ResolvedWorldTransform {
    const IDENTITY: Self = Self {
        matrix: Mat4::IDENTITY,
        rotation: Quat::IDENTITY,
    };
}

impl TransformFieldSnapshot {
    fn effective(&self) -> Option<EffectiveTransform> {
        let translation = match &self.translation {
            Some(Value::Vec3(value)) => Vec3::from_array(*value),
            None => Vec3::ZERO,
            Some(_) => return None,
        };
        let rotation = match &self.rotation {
            Some(Value::Quat(value)) => Quat::from_array(*value),
            None => Quat::IDENTITY,
            Some(_) => return None,
        };
        let scale = match &self.scale {
            Some(Value::Vec3(value)) => Vec3::from_array(*value),
            None => Vec3::ONE,
            Some(_) => return None,
        };
        if !translation.is_finite()
            || !rotation.is_finite()
            || rotation.length_squared() <= f32::EPSILON
            || !scale.is_finite()
        {
            return None;
        }
        Some(EffectiveTransform {
            translation,
            rotation,
            scale,
        })
    }
}

/// One command represents one complete pointer gesture, even when translate,
/// rotate, and scale fields were previewed repeatedly during the drag.
struct SetTransformGesture {
    entity_id: PersistentId,
    before: TransformFieldSnapshot,
    after: TransformFieldSnapshot,
}

impl Command for SetTransformGesture {
    fn name(&self) -> &str {
        "Transform Gizmo Drag"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        apply_transform_snapshot(scene, &self.entity_id, &self.after)
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        apply_transform_snapshot(scene, &self.entity_id, &self.before)
    }
}

impl EditorScene {
    /// Read the selected entity's effective local Transform for gizmo drawing.
    ///
    /// Omitted serialized fields use the same defaults as the scene loader:
    /// zero translation, identity rotation, and unit scale. A stale selection,
    /// missing Transform, malformed field type, non-finite value, or zero
    /// quaternion returns `None`. The returned values are the serialized local
    /// TRS components; hierarchy resolution is performed internally when a
    /// world-space gizmo delta is previewed. `parent` is therefore `None`.
    pub fn selected_transform_for_gizmo(&self) -> Option<Transform> {
        let entity_id = self.selected_entity.as_ref()?;
        let transform = capture_transform_snapshot(&self.scene, entity_id)
            .ok()?
            .effective()?;
        Some(Transform {
            translation: transform.translation,
            rotation: transform.rotation,
            scale: transform.scale,
            parent: None,
        })
    }

    /// Begin a Transform gizmo gesture for the currently selected entity.
    ///
    /// Returns `false` without mutating state when there is no selection, the
    /// persistent ID is stale, the entity has no Transform, the serialized
    /// Transform is malformed, or another gesture is already active.
    pub fn begin_transform_gizmo_drag(&mut self) -> bool {
        if self.gizmo_drag.is_some() {
            return false;
        }
        let Some(entity_id) = self.selected_entity.clone() else {
            return false;
        };
        let Ok(original) = capture_transform_snapshot(&self.scene, &entity_id) else {
            return false;
        };
        if original.effective().is_none() {
            return false;
        }
        self.gizmo_drag = Some(SceneGizmoDrag {
            entity_id,
            original,
        });
        true
    }

    /// Apply one incremental gizmo delta as an uncommitted scene preview.
    ///
    /// Preview edits do not dirty command history. Call
    /// [`commit_transform_gizmo_drag`](Self::commit_transform_gizmo_drag) once
    /// on pointer release, or
    /// [`cancel_transform_gizmo_drag`](Self::cancel_transform_gizmo_drag) to
    /// restore the exact pre-drag values.
    pub fn preview_transform_gizmo_drag(&mut self, system: &GizmoSystem, delta: Vec3) -> bool {
        if !delta.is_finite() {
            return false;
        }
        let Some(entity_id) = self
            .gizmo_drag
            .as_ref()
            .map(|session| session.entity_id.clone())
        else {
            return false;
        };
        let Ok(snapshot) = capture_transform_snapshot(&self.scene, &entity_id) else {
            return false;
        };
        let Some(mut transform) = snapshot.effective() else {
            return false;
        };

        let field = match system.mode {
            GizmoMode::Translate => {
                let Some(parent_world) = resolve_parent_world_transform(&self.scene, &entity_id)
                else {
                    return false;
                };
                let determinant = parent_world.matrix.determinant();
                if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
                    return false;
                }
                let inverse_parent = parent_world.matrix.inverse();
                let local_delta = inverse_parent.transform_vector3(delta);
                if !inverse_parent.is_finite() || !local_delta.is_finite() {
                    return false;
                }
                transform.translation += local_delta;
                ("translation", Value::Vec3(transform.translation.to_array()))
            }
            GizmoMode::Rotate => {
                let Some(axis) = system.drag_axis else {
                    return false;
                };
                let Some(parent_world) = resolve_parent_world_transform(&self.scene, &entity_id)
                else {
                    return false;
                };
                let angle = match axis {
                    GizmoAxis::X => delta.x,
                    GizmoAxis::Y => delta.y,
                    GizmoAxis::Z => delta.z,
                };
                let local_rotation = transform.rotation.normalize();
                let world_rotation = (parent_world.rotation * local_rotation).normalize();
                let axis_rotation = Quat::from_axis_angle(axis.direction(), angle);
                let next_world_rotation = match system.space {
                    GizmoSpace::Local => world_rotation * axis_rotation,
                    GizmoSpace::Global => axis_rotation * world_rotation,
                }
                .normalize();
                transform.rotation =
                    (parent_world.rotation.inverse() * next_world_rotation).normalize();
                ("rotation", Value::Quat(transform.rotation.to_array()))
            }
            GizmoMode::Scale => {
                // Scale handles always represent the entity's local axes and
                // write its local scale components. This remains well-defined
                // under rotated/non-uniform parents, unlike a world-aligned
                // scale that would generally require storing shear.
                transform.scale *= Vec3::ONE + delta;
                transform.scale = transform.scale.max(Vec3::splat(0.001));
                ("scale", Value::Vec3(transform.scale.to_array()))
            }
        };

        set_transform_field(&mut self.scene, &entity_id, field.0, field.1).is_ok()
    }

    /// Commit the active preview as exactly one undoable command.
    ///
    /// Returns `Ok(false)` when no gesture is active or the Transform did not
    /// change. The command uses the real persistent entity ID captured at drag
    /// start, never a fabricated ECS index.
    pub fn commit_transform_gizmo_drag(&mut self) -> Result<bool, EditorError> {
        let Some(session) = self.gizmo_drag.take() else {
            return Ok(false);
        };
        let current = capture_transform_snapshot(&self.scene, &session.entity_id)?;
        if current == session.original {
            return Ok(false);
        }
        // The pointer preview is not committed authoring state. Restore the
        // exact gesture origin first so EditorScene::execute owns the complete
        // atomic transition and can roll back to it on validation failure.
        apply_transform_snapshot(&mut self.scene, &session.entity_id, &session.original)?;
        self.execute(Box::new(SetTransformGesture {
            entity_id: session.entity_id,
            before: session.original,
            after: current,
        }))?;
        Ok(true)
    }

    /// Cancel the active gesture and restore the exact pre-drag Transform.
    ///
    /// Cancellation never creates an undo entry or marks history dirty.
    pub fn cancel_transform_gizmo_drag(&mut self) -> bool {
        let Some(session) = self.gizmo_drag.take() else {
            return false;
        };
        apply_transform_snapshot(&mut self.scene, &session.entity_id, &session.original).is_ok()
    }

    /// Whether a scene-level Transform gesture is currently active.
    pub fn is_transform_gizmo_drag_active(&self) -> bool {
        self.gizmo_drag.is_some()
    }

    /// Persistent entity captured by the active Transform gesture.
    ///
    /// Hosts can compare this with their current selection before syncing a
    /// preview into a runtime world. A selection change never retargets an
    /// already-active gesture.
    pub fn active_transform_gizmo_entity(&self) -> Option<&PersistentId> {
        self.gizmo_drag.as_ref().map(|session| &session.entity_id)
    }
}

fn resolve_parent_world_transform(
    scene: &Scene,
    entity_id: &PersistentId,
) -> Option<ResolvedWorldTransform> {
    let entity = scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == *entity_id)?;
    let Some(parent_id) = entity.parent.as_ref() else {
        return Some(ResolvedWorldTransform::IDENTITY);
    };
    resolve_entity_world_transform(scene, parent_id, &mut Vec::new())
}

fn resolve_entity_world_transform(
    scene: &Scene,
    entity_id: &PersistentId,
    visiting: &mut Vec<PersistentId>,
) -> Option<ResolvedWorldTransform> {
    if visiting.iter().any(|visited| visited == entity_id) {
        return None;
    }
    let entity = scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == *entity_id)?;

    // Match runtime transform extraction: an entity without Transform is an
    // identity root, even if malformed input gave it a parent of its own.
    let Some(component) = entity.components.get(TRANSFORM_COMPONENT_TYPE) else {
        return Some(ResolvedWorldTransform::IDENTITY);
    };
    let local = TransformFieldSnapshot {
        translation: component.fields.get("translation").cloned(),
        rotation: component.fields.get("rotation").cloned(),
        scale: component.fields.get("scale").cloned(),
    }
    .effective()?;
    let local_rotation = local.rotation.normalize();
    let local_matrix =
        Mat4::from_scale_rotation_translation(local.scale, local_rotation, local.translation);
    if !local_matrix.is_finite() {
        return None;
    }

    visiting.push(entity_id.clone());
    let parent_world = match entity.parent.as_ref() {
        Some(parent_id) => resolve_entity_world_transform(scene, parent_id, visiting),
        None => Some(ResolvedWorldTransform::IDENTITY),
    };
    visiting.pop();
    let parent_world = parent_world?;
    let matrix = parent_world.matrix * local_matrix;
    let rotation = (parent_world.rotation * local_rotation).normalize();
    (matrix.is_finite() && rotation.is_finite())
        .then_some(ResolvedWorldTransform { matrix, rotation })
}

fn capture_transform_snapshot(
    scene: &Scene,
    entity_id: &PersistentId,
) -> Result<TransformFieldSnapshot, EditorError> {
    let entity = scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == *entity_id)
        .ok_or_else(|| EditorError::EntityNotFound(entity_id.clone()))?;
    let component = entity
        .components
        .get(TRANSFORM_COMPONENT_TYPE)
        .ok_or_else(|| EditorError::ComponentNotFound(TRANSFORM_COMPONENT_TYPE.into()))?;
    Ok(TransformFieldSnapshot {
        translation: component.fields.get("translation").cloned(),
        rotation: component.fields.get("rotation").cloned(),
        scale: component.fields.get("scale").cloned(),
    })
}

fn apply_transform_snapshot(
    scene: &mut Scene,
    entity_id: &PersistentId,
    snapshot: &TransformFieldSnapshot,
) -> Result<(), EditorError> {
    let entity = crate::commands::find_entity_mut(scene, entity_id)?;
    let component = entity
        .components
        .get_mut(TRANSFORM_COMPONENT_TYPE)
        .ok_or_else(|| EditorError::ComponentNotFound(TRANSFORM_COMPONENT_TYPE.into()))?;
    set_optional_field(component, "translation", snapshot.translation.clone());
    set_optional_field(component, "rotation", snapshot.rotation.clone());
    set_optional_field(component, "scale", snapshot.scale.clone());
    Ok(())
}

fn set_optional_field(
    component: &mut engine_scene::ComponentRecord,
    field: &str,
    value: Option<Value>,
) {
    if let Some(value) = value {
        component.fields.insert(field.to_string(), value);
    } else {
        component.fields.remove(field);
    }
}

pub(super) fn set_transform_field(
    scene: &mut Scene,
    entity_id: &PersistentId,
    field: &str,
    value: Value,
) -> Result<(), EditorError> {
    let entity = crate::commands::find_entity_mut(scene, entity_id)?;
    let component = entity
        .components
        .get_mut(TRANSFORM_COMPONENT_TYPE)
        .ok_or_else(|| EditorError::ComponentNotFound(TRANSFORM_COMPONENT_TYPE.into()))?;
    component.fields.insert(field.to_string(), value);
    Ok(())
}
