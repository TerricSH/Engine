use std::collections::{BTreeMap, HashSet};

use engine_animation::{
    AnimationPlayer, BoneIndex, BoneTransform, ExternalPoseOverride, RagdollComponent,
    RagdollJointType, RagdollMode, RagdollPart, RagdollPartRole, RagdollShape, SkeletonComponent,
};
use engine_physics::{
    BodyType, Collider, ColliderShape, JointLimits, JointType, PhysicsCommand, PhysicsJoint,
    RigidBody,
};
use engine_scene::components::Transform;
use engine_scene::{Entity, World};
use glam::{Mat4, Quat, Vec3};

use crate::game_loop::GameLoop;

pub(crate) fn reconcile_before_physics(game_loop: &mut GameLoop) {
    cleanup_orphaned_parts(game_loop);
    let owners = ragdoll_owners(game_loop);
    let mut commands = Vec::new();
    let mut diagnostics = Vec::new();

    for owner in owners {
        let Some((ragdoll, skeleton, owner_transform, cached_globals)) =
            owner_snapshot(game_loop, owner)
        else {
            continue;
        };
        if !ragdoll.enabled {
            continue;
        }
        if let Err(error) = ragdoll.validate_for_skeleton(&skeleton) {
            diagnostics.push((owner, error));
            continue;
        }
        let runtime_skeleton = engine_animation::skeleton::Skeleton::from_asset(&skeleton);
        let owner_matrix = entity_world_matrix_for_owner(game_loop, owner)
            .unwrap_or_else(|| transform_matrix(&owner_transform));
        let bone_globals = if cached_globals.len() == runtime_skeleton.bone_count() {
            cached_globals
        } else {
            runtime_skeleton
                .rest_pose()
                .global_transforms(&runtime_skeleton)
        };

        let graph = game_loop.runtime.with_world_mut(|world| {
            ensure_graph(
                world,
                owner,
                &ragdoll,
                &skeleton,
                owner_matrix,
                &bone_globals,
            )
        });
        let graph = match graph {
            Some(Ok(graph)) => graph,
            Some(Err(error)) => {
                diagnostics.push((owner, error));
                continue;
            }
            None => continue,
        };

        let desired_body_type = match ragdoll.mode {
            RagdollMode::Simulated => BodyType::Dynamic,
            RagdollMode::Animated | RagdollMode::Recovering => BodyType::Kinematic,
        };
        for (body_index, entity) in graph.body_entities.iter().copied().enumerate() {
            let current_type = game_loop
                .runtime
                .with_world(|world| world.get::<RigidBody>(entity).map(|body| body.body_type))
                .flatten();
            if current_type.as_ref() != Some(&desired_body_type) {
                commands.push(PhysicsCommand::SetBodyType {
                    entity,
                    body_type: desired_body_type,
                });
            }
            if ragdoll.mode == RagdollMode::Animated {
                let definition = &ragdoll.bodies[body_index];
                let Some(bone_index) = skeleton
                    .joints()
                    .iter()
                    .position(|joint| joint.name == definition.bone)
                else {
                    continue;
                };
                let matrix = owner_matrix
                    * bone_globals[bone_index].to_mat4()
                    * body_offset_matrix(definition);
                let (_, rotation, translation) = matrix.to_scale_rotation_translation();
                if translation.is_finite() && rotation.is_finite() {
                    commands.push(PhysicsCommand::SetBodyPosition {
                        entity,
                        position: translation,
                    });
                    commands.push(PhysicsCommand::SetBodyRotation { entity, rotation });
                }
            }
        }
        if ragdoll.mode == RagdollMode::Simulated && ragdoll.impulse_pending {
            let impulse = Vec3::from(ragdoll.pending_impulse);
            let body_count = graph.body_entities.len().max(1) as f32;
            if impulse != Vec3::ZERO {
                for entity in &graph.body_entities {
                    commands.push(PhysicsCommand::ApplyImpulse {
                        entity: *entity,
                        impulse: impulse / body_count,
                    });
                }
            }
            game_loop.runtime.with_world_mut(|world| {
                if let Some(component) = world.get_mut::<RagdollComponent>(owner) {
                    component.pending_impulse = [0.0; 3];
                    component.impulse_pending = false;
                }
            });
        }
    }

    if let Some(physics) = game_loop.physics.as_mut() {
        for command in commands {
            physics.queue_command(command);
        }
    }
    push_ragdoll_diagnostics(game_loop, diagnostics);
}

pub(crate) fn reconcile_after_physics(game_loop: &mut GameLoop, dt: f32) {
    let owners = ragdoll_owners(game_loop);
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let mut diagnostics = Vec::new();

    for owner in owners {
        let Some((ragdoll, skeleton, owner_transform, cached_globals)) =
            owner_snapshot(game_loop, owner)
        else {
            continue;
        };
        if !ragdoll.enabled || ragdoll.mode == RagdollMode::Animated {
            game_loop.runtime.with_world_mut(|world| {
                if let Some(player) = world.get_mut::<AnimationPlayer>(owner) {
                    player.external_pose_override = None;
                }
            });
            continue;
        }
        if let Err(error) = ragdoll.validate_for_skeleton(&skeleton) {
            diagnostics.push((owner, error));
            continue;
        }

        let runtime_skeleton = engine_animation::skeleton::Skeleton::from_asset(&skeleton);
        let base_globals = if cached_globals.len() == runtime_skeleton.bone_count() {
            cached_globals
        } else {
            runtime_skeleton
                .rest_pose()
                .global_transforms(&runtime_skeleton)
        };
        let base_locals = globals_to_locals(&runtime_skeleton, &base_globals);
        let owner_inverse = entity_world_matrix_for_owner(game_loop, owner)
            .unwrap_or_else(|| transform_matrix(&owner_transform))
            .inverse();
        if !owner_inverse.is_finite() {
            diagnostics.push((owner, "ragdoll owner transform is not invertible".into()));
            continue;
        }

        let body_transforms = game_loop
            .runtime
            .with_world(|world| {
                ragdoll
                    .bodies
                    .iter()
                    .filter_map(|definition| {
                        let id = ragdoll.generated_body_ids.get(&definition.bone)?;
                        let entity = world.entity_by_persistent_id(id)?;
                        let transform = world.get::<Transform>(entity)?;
                        Some((definition.bone.clone(), transform.clone()))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        let mut physical_globals = BTreeMap::new();
        for definition in &ragdoll.bodies {
            let Some(transform) = body_transforms.get(&definition.bone) else {
                continue;
            };
            let Some(bone_index) = skeleton
                .joints()
                .iter()
                .position(|joint| joint.name == definition.bone)
            else {
                continue;
            };
            let body_world = transform_matrix(transform);
            let bone_model = owner_inverse * body_world * body_offset_matrix(definition).inverse();
            if bone_model.is_finite() {
                physical_globals.insert(bone_index, bone_transform_from_matrix(bone_model));
            }
        }
        if physical_globals.is_empty() {
            continue;
        }

        let mut final_globals = Vec::with_capacity(runtime_skeleton.bone_count());
        for (bone_index, local) in base_locals
            .iter()
            .copied()
            .enumerate()
            .take(runtime_skeleton.bone_count())
        {
            if let Some(physical) = physical_globals.get(&bone_index) {
                final_globals.push(*physical);
                continue;
            }
            let global = match runtime_skeleton.parent_of(BoneIndex(bone_index as u16)) {
                Some(parent) => final_globals[parent.0 as usize] * local,
                None => local,
            };
            final_globals.push(global);
        }
        let local_pose = globals_to_locals(&runtime_skeleton, &final_globals);
        let (weight, next_mode, next_elapsed) = match ragdoll.mode {
            RagdollMode::Simulated => (1.0, RagdollMode::Simulated, 0.0),
            RagdollMode::Recovering if ragdoll.recovery_duration <= f32::EPSILON => {
                (0.0, RagdollMode::Animated, 0.0)
            }
            RagdollMode::Recovering => {
                let elapsed = (ragdoll.recovery_elapsed + dt).min(ragdoll.recovery_duration);
                let weight = 1.0 - elapsed / ragdoll.recovery_duration;
                let mode = if weight <= 0.0 {
                    RagdollMode::Animated
                } else {
                    RagdollMode::Recovering
                };
                (weight, mode, elapsed)
            }
            RagdollMode::Animated => unreachable!(),
        };

        game_loop.runtime.with_world_mut(|world| {
            if let Some(player) = world.get_mut::<AnimationPlayer>(owner) {
                player.external_pose_override = (weight > 0.0).then_some(ExternalPoseOverride {
                    local_transforms: local_pose,
                    weight,
                });
            }
            if let Some(component) = world.get_mut::<RagdollComponent>(owner) {
                component.mode = next_mode;
                component.recovery_elapsed = next_elapsed;
            }
        });
    }
    push_ragdoll_diagnostics(game_loop, diagnostics);
}

pub(crate) fn set_active(
    game_loop: &mut GameLoop,
    entity_id: &str,
    active: bool,
    recovery_duration: f32,
    impulse: Vec3,
) -> Result<Vec<String>, String> {
    if !recovery_duration.is_finite() || recovery_duration < 0.0 {
        return Err("ragdoll recovery duration must be finite and non-negative".into());
    }
    if !impulse.is_finite() {
        return Err("ragdoll impulse must be finite".into());
    }
    let entity = game_loop
        .runtime
        .with_world(|world| world.entity_by_persistent_id(entity_id))
        .flatten()
        .ok_or_else(|| format!("ragdoll target '{entity_id}' does not exist"))?;
    let (authored, skeleton_id) = game_loop
        .runtime
        .with_world(|world| {
            let ragdoll = world
                .get::<RagdollComponent>(entity)
                .ok_or_else(|| format!("entity '{entity_id}' has no Ragdoll component"))?;
            if !ragdoll.enabled {
                return Err(format!(
                    "entity '{entity_id}' has a disabled Ragdoll component"
                ));
            }
            if world.get::<Transform>(entity).is_none() {
                return Err(format!("entity '{entity_id}' has no Transform component"));
            }
            let skeleton_id = world
                .get::<SkeletonComponent>(entity)
                .ok_or_else(|| format!("entity '{entity_id}' has no Skeleton component"))?
                .skeleton_asset
                .clone()
                .ok_or_else(|| format!("entity '{entity_id}' has no skeleton asset assigned"))?;
            Ok((ragdoll.clone(), skeleton_id))
        })
        .ok_or_else(|| "no active world".to_string())??;
    let skeleton = game_loop
        .runtime
        .extension_asset::<engine_animation::Skeleton>(
            "skeleton",
            &engine_serialize::AssetId::new(skeleton_id.clone()),
        )
        .ok_or_else(|| {
            format!("entity '{entity_id}' references unavailable skeleton asset '{skeleton_id}'")
        })?;
    authored
        .validate_for_skeleton(skeleton.get())
        .map_err(|error| format!("entity '{entity_id}' has an invalid ragdoll: {error}"))?;

    let body_ids = game_loop
        .runtime
        .with_world_mut(|world| -> Result<Vec<String>, String> {
            let ragdoll = world
                .get_mut::<RagdollComponent>(entity)
                .ok_or_else(|| format!("entity '{entity_id}' has no Ragdoll component"))?;
            if active {
                ragdoll.mode = RagdollMode::Simulated;
                ragdoll.recovery_elapsed = 0.0;
                ragdoll.pending_impulse = impulse.to_array();
                ragdoll.impulse_pending = impulse != Vec3::ZERO;
            } else if recovery_duration <= f32::EPSILON {
                ragdoll.mode = RagdollMode::Animated;
                ragdoll.recovery_duration = 0.0;
                ragdoll.recovery_elapsed = 0.0;
                ragdoll.pending_impulse = [0.0; 3];
                ragdoll.impulse_pending = false;
            } else {
                ragdoll.mode = RagdollMode::Recovering;
                ragdoll.recovery_duration = recovery_duration;
                ragdoll.recovery_elapsed = 0.0;
                ragdoll.pending_impulse = [0.0; 3];
                ragdoll.impulse_pending = false;
            }
            Ok(ragdoll
                .generated_body_ids
                .values()
                .cloned()
                .collect::<Vec<_>>())
        })
        .ok_or_else(|| "no active world".to_string())??;

    Ok(body_ids)
}

struct RagdollGraph {
    body_entities: Vec<Entity>,
}

fn ensure_graph(
    world: &mut World,
    owner: Entity,
    authored: &RagdollComponent,
    skeleton: &engine_animation::Skeleton,
    owner_matrix: Mat4,
    bone_globals: &[BoneTransform],
) -> Result<RagdollGraph, String> {
    let owner_id = world
        .persistent_id(owner)
        .ok_or_else(|| "ragdoll owner needs a persistent ID".to_string())?
        .to_owned();
    let mut ragdoll = authored.clone();
    let authored_bones = ragdoll
        .bodies
        .iter()
        .map(|body| body.bone.clone())
        .collect::<HashSet<_>>();
    ragdoll
        .generated_body_ids
        .retain(|bone, _| authored_bones.contains(bone));
    ragdoll
        .generated_joint_ids
        .truncate(ragdoll.constraints.len());
    let mut created = Vec::new();
    let result = (|| {
        let mut body_entities = Vec::with_capacity(ragdoll.bodies.len());
        for (body_index, definition) in ragdoll.bodies.iter().enumerate() {
            let body_id = ragdoll
                .generated_body_ids
                .get(&definition.bone)
                .cloned()
                .unwrap_or_else(|| format!("{owner_id}.__ragdoll.body.{body_index}"));
            let entity = match world.entity_by_persistent_id(&body_id) {
                Some(entity) => {
                    let owned = world.get::<RagdollPart>(entity).is_some_and(|part| {
                        part.owner_id == owner_id
                            && part.role == RagdollPartRole::Body
                            && part.key == definition.bone
                    });
                    if !owned {
                        return Err(format!(
                            "ragdoll generated body ID '{body_id}' conflicts with another entity"
                        ));
                    }
                    if world.get::<Transform>(entity).is_none()
                        || world.get::<RigidBody>(entity).is_none()
                        || world.get::<Collider>(entity).is_none()
                    {
                        return Err(format!(
                            "ragdoll generated body '{body_id}' is missing required physics components"
                        ));
                    }
                    entity
                }
                None => {
                    let entity = world
                        .create_persistent_entity(body_id.clone())
                        .map_err(|error| error.to_string())?;
                    created.push(entity);
                    let bone_index = skeleton
                        .joints()
                        .iter()
                        .position(|joint| joint.name == definition.bone)
                        .ok_or_else(|| {
                            format!(
                                "ragdoll bone '{}' disappeared during generation",
                                definition.bone
                            )
                        })?;
                    let matrix = owner_matrix
                        * bone_globals[bone_index].to_mat4()
                        * body_offset_matrix(definition);
                    let (_, rotation, translation) = matrix.to_scale_rotation_translation();
                    world.add_component(
                        entity,
                        Transform {
                            translation,
                            rotation,
                            scale: Vec3::ONE,
                            parent: None,
                        },
                    );
                    world.add_component(
                        entity,
                        RigidBody {
                            body_type: if ragdoll.mode == RagdollMode::Simulated {
                                BodyType::Dynamic
                            } else {
                                BodyType::Kinematic
                            },
                            mass: definition.mass,
                            linear_damping: definition.linear_damping,
                            angular_damping: definition.angular_damping,
                            ..RigidBody::default()
                        },
                    );
                    world.add_component(
                        entity,
                        Collider {
                            shape: match definition.shape {
                                RagdollShape::Ball { radius } => ColliderShape::Ball { radius },
                                RagdollShape::Capsule {
                                    half_height,
                                    radius,
                                } => ColliderShape::Capsule {
                                    half_height,
                                    radius,
                                },
                                RagdollShape::Box { half_extents } => ColliderShape::Cuboid {
                                    hx: half_extents[0],
                                    hy: half_extents[1],
                                    hz: half_extents[2],
                                },
                            },
                            ..Collider::default()
                        },
                    );
                    world.add_component(
                        entity,
                        RagdollPart {
                            owner_id: owner_id.clone(),
                            role: RagdollPartRole::Body,
                            key: definition.bone.clone(),
                        },
                    );
                    entity
                }
            };
            ragdoll
                .generated_body_ids
                .insert(definition.bone.clone(), body_id);
            body_entities.push(entity);
        }

        for (constraint_index, constraint) in ragdoll.constraints.iter().enumerate() {
            let joint_id = ragdoll
                .generated_joint_ids
                .get(constraint_index)
                .cloned()
                .unwrap_or_else(|| format!("{owner_id}.__ragdoll.joint.{constraint_index}"));
            let parent_id = ragdoll
                .generated_body_ids
                .get(&constraint.parent_bone)
                .ok_or_else(|| {
                    format!(
                        "ragdoll constraint references missing body '{}'",
                        constraint.parent_bone
                    )
                })?
                .clone();
            let child_id = ragdoll
                .generated_body_ids
                .get(&constraint.child_bone)
                .ok_or_else(|| {
                    format!(
                        "ragdoll constraint references missing body '{}'",
                        constraint.child_bone
                    )
                })?
                .clone();
            let desired_joint = PhysicsJoint {
                body_a: parent_id,
                body_b: child_id,
                joint_type: match constraint.joint_type {
                    RagdollJointType::Fixed => JointType::Fixed,
                    RagdollJointType::Revolute => JointType::Revolute,
                    RagdollJointType::Spherical => JointType::Spherical,
                },
                anchor_a: constraint.anchor_parent,
                anchor_b: constraint.anchor_child,
                axis: constraint.axis,
                limits: constraint.limits.map(|[min, max]| JointLimits {
                    min,
                    max,
                    stiffness: 0.0,
                    damping: 0.0,
                }),
                break_force: constraint.break_force,
                break_torque: constraint.break_torque,
                ..PhysicsJoint::default()
            };
            match world.entity_by_persistent_id(&joint_id) {
                Some(entity) => {
                    let owned = world.get::<RagdollPart>(entity).is_some_and(|part| {
                        part.owner_id == owner_id
                            && part.role == RagdollPartRole::Joint
                            && part.key == constraint.child_bone
                    });
                    if !owned {
                        return Err(format!(
                            "ragdoll generated joint ID '{joint_id}' conflicts with another entity"
                        ));
                    }
                    let Some(component) = world.get_mut::<PhysicsJoint>(entity) else {
                        return Err(format!(
                            "ragdoll generated joint '{joint_id}' is missing its PhysicsJoint component"
                        ));
                    };
                    if component != &desired_joint {
                        *component = desired_joint.clone();
                    }
                }
                None => {
                    let entity = world
                        .create_persistent_entity(joint_id.clone())
                        .map_err(|error| error.to_string())?;
                    created.push(entity);
                    world.add_component(entity, desired_joint);
                    world.add_component(
                        entity,
                        RagdollPart {
                            owner_id: owner_id.clone(),
                            role: RagdollPartRole::Joint,
                            key: constraint.child_bone.clone(),
                        },
                    );
                }
            }
            if ragdoll.generated_joint_ids.len() <= constraint_index {
                ragdoll.generated_joint_ids.push(joint_id);
            } else {
                ragdoll.generated_joint_ids[constraint_index] = joint_id;
            }
        }
        Ok(RagdollGraph { body_entities })
    })();

    match result {
        Ok(graph) => {
            if let Some(component) = world.get_mut::<RagdollComponent>(owner) {
                component.generated_body_ids = ragdoll.generated_body_ids;
                component.generated_joint_ids = ragdoll.generated_joint_ids;
            }
            Ok(graph)
        }
        Err(error) => {
            for entity in created.into_iter().rev() {
                world.destroy_entity(entity);
            }
            Err(error)
        }
    }
}

fn cleanup_orphaned_parts(game_loop: &mut GameLoop) {
    game_loop.runtime.with_world_mut(|world| {
        let stale = world
            .query::<RagdollPart>()
            .filter_map(|(entity, part)| {
                let owner = world.entity_by_persistent_id(&part.owner_id)?;
                let ragdoll = world.get::<RagdollComponent>(owner)?;
                let id = world.persistent_id(entity)?;
                let retained = match part.role {
                    RagdollPartRole::Body => ragdoll
                        .generated_body_ids
                        .get(&part.key)
                        .is_some_and(|expected| expected == id),
                    RagdollPartRole::Joint => ragdoll
                        .generated_joint_ids
                        .iter()
                        .any(|expected| expected == id),
                };
                (!retained).then_some(entity)
            })
            .collect::<Vec<_>>();
        let ownerless = world
            .query::<RagdollPart>()
            .filter_map(|(entity, part)| {
                world
                    .entity_by_persistent_id(&part.owner_id)
                    .is_none()
                    .then_some(entity)
            })
            .collect::<Vec<_>>();
        for entity in stale.into_iter().chain(ownerless) {
            world.destroy_entity(entity);
        }
    });
}

fn ragdoll_owners(game_loop: &GameLoop) -> Vec<Entity> {
    game_loop
        .runtime
        .with_world(|world| {
            world
                .query::<RagdollComponent>()
                .map(|(entity, _)| entity)
                .collect()
        })
        .unwrap_or_default()
}

fn owner_snapshot(
    game_loop: &GameLoop,
    owner: Entity,
) -> Option<(
    RagdollComponent,
    engine_animation::Skeleton,
    Transform,
    Vec<BoneTransform>,
)> {
    let (ragdoll, skeleton_id, transform, cached) = game_loop.runtime.with_world(|world| {
        let ragdoll = world.get::<RagdollComponent>(owner)?.clone();
        let skeleton_id = world
            .get::<SkeletonComponent>(owner)?
            .skeleton_asset
            .clone()?;
        let transform = world.get::<Transform>(owner)?.clone();
        let cached = world
            .get::<AnimationPlayer>(owner)
            .map(|player| player.cached_bone_transforms.clone())
            .unwrap_or_default();
        Some((ragdoll, skeleton_id, transform, cached))
    })??;
    let skeleton = game_loop
        .runtime
        .extension_asset::<engine_animation::Skeleton>(
            "skeleton",
            &engine_serialize::AssetId::new(skeleton_id),
        )?
        .get()
        .clone();
    Some((ragdoll, skeleton, transform, cached))
}

fn entity_world_matrix_for_owner(game_loop: &GameLoop, entity: Entity) -> Option<Mat4> {
    game_loop
        .runtime
        .with_world(|world| entity_world_matrix(world, entity))
        .flatten()
}

fn entity_world_matrix(world: &World, entity: Entity) -> Option<Mat4> {
    let mut current = Some(entity);
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    while let Some(entity) = current {
        if !seen.insert(entity) {
            return None;
        }
        let transform = world.get::<Transform>(entity)?;
        chain.push(transform.clone());
        current = transform.parent;
    }
    Some(
        chain
            .into_iter()
            .rev()
            .fold(Mat4::IDENTITY, |world, local| {
                world * transform_matrix(&local)
            }),
    )
}

fn transform_matrix(transform: &Transform) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    )
}

fn body_offset_matrix(body: &engine_animation::RagdollBody) -> Mat4 {
    let rotation = normalized_quat(body.local_rotation);
    Mat4::from_rotation_translation(rotation, Vec3::from(body.local_translation))
}

fn normalized_quat(value: [f32; 4]) -> Quat {
    let rotation = Quat::from_array(value);
    if rotation.is_finite() && rotation.length_squared() > f32::EPSILON {
        rotation.normalize()
    } else {
        Quat::IDENTITY
    }
}

fn bone_transform_from_matrix(matrix: Mat4) -> BoneTransform {
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    BoneTransform {
        translation,
        rotation: if rotation.is_finite() {
            rotation.normalize()
        } else {
            Quat::IDENTITY
        },
        scale,
    }
}

fn globals_to_locals(
    skeleton: &engine_animation::skeleton::Skeleton,
    globals: &[BoneTransform],
) -> Vec<BoneTransform> {
    globals
        .iter()
        .enumerate()
        .map(
            |(index, global)| match skeleton.parent_of(BoneIndex(index as u16)) {
                Some(parent) => bone_transform_from_matrix(
                    globals[parent.0 as usize].to_mat4().inverse() * global.to_mat4(),
                ),
                None => *global,
            },
        )
        .collect()
}

fn push_ragdoll_diagnostics(game_loop: &mut GameLoop, diagnostics: Vec<(Entity, String)>) {
    for (entity, message) in diagnostics {
        let entity_id = game_loop
            .runtime
            .with_world(|world| world.persistent_id(entity).map(str::to_owned))
            .flatten();
        let mut diagnostic = engine_serialize::Diagnostic::new(
            "RAGDOLL_RECONCILE_FAILED",
            engine_serialize::DiagnosticSeverity::Error,
            "ragdoll",
            message,
        );
        diagnostic.entity = entity_id;
        game_loop
            .runtime
            .diagnostics_collector_mut()
            .push_scene_diags(vec![diagnostic]);
    }
}
