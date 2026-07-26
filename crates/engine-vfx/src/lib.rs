#![forbid(unsafe_code)]

mod components;
mod serde;

pub use components::{
    Decal, Particle, ParticleEmitter, BUILTIN_VFX_MATERIAL_ID, BUILTIN_VFX_QUAD_MESH_ID,
};

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use engine_renderer::{AxisAlignedBox, RenderFrameInput, RenderableItem};
use engine_scene::{
    camera_relative_render_origin, entity_world_position, entity_world_transform, Component,
    ComponentExtension, ComponentMeta, ComponentRegistry, ComponentStorageDyn, ScriptAccess,
    SparseSet, World,
};
use engine_serialize::AssetId;
use glam::{Mat3, Mat4, Vec3};

/// Register scene-serializable particle-emitter and decal components.
pub fn register_vfx_extensions(component_registry: &mut ComponentRegistry) {
    let _ = component_registry.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: ParticleEmitter::TYPE_ID,
            display_name: "Particle Emitter",
            schema_version: (0, 1, 0),
            has_editor: true,
            script_access: ScriptAccess::ReadWrite,
        },
        storage_factory: || -> Box<dyn ComponentStorageDyn> {
            Box::new(SparseSet::<ParticleEmitter>::new())
        },
        serialize: Some(serde::serialize_particle_emitter),
        deserialize: Some(serde::deserialize_particle_emitter),
    });
    let _ = component_registry.register_fields_validator(
        ParticleEmitter::TYPE_ID,
        serde::validate_particle_emitter_fields,
    );

    let _ = component_registry.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: Decal::TYPE_ID,
            display_name: "Decal",
            schema_version: (0, 1, 0),
            has_editor: true,
            script_access: ScriptAccess::ReadWrite,
        },
        storage_factory: || -> Box<dyn ComponentStorageDyn> { Box::new(SparseSet::<Decal>::new()) },
        serialize: Some(serde::serialize_decal),
        deserialize: Some(serde::deserialize_decal),
    });
    let _ =
        component_registry.register_fields_validator(Decal::TYPE_ID, serde::validate_decal_fields);
}

/// Advance every enabled emitter and finite-lifetime decal.
pub fn update_vfx(world: &mut World, dt: f32) {
    let dt = if dt.is_finite() {
        dt.clamp(0.0, 0.1)
    } else {
        0.0
    };
    let origins = world
        .query::<ParticleEmitter>()
        .map(|(entity, _)| {
            (
                entity,
                entity_world_position(world, entity).unwrap_or(Vec3::ZERO),
            )
        })
        .collect::<HashMap<_, _>>();

    for (entity, emitter) in world.query_mut::<ParticleEmitter>() {
        let acceleration = emitter.acceleration;
        emitter.particles_mut().retain_mut(|particle| {
            particle.age += dt;
            if particle.age >= particle.lifetime {
                return false;
            }
            particle.velocity += acceleration * dt;
            particle.position += particle.velocity * dt;
            particle.rotation += particle.angular_velocity * dt;
            true
        });

        let origin = origins.get(&entity).copied().unwrap_or(Vec3::ZERO);
        let spawn_count = emitter.take_spawn_budget(dt);
        for _ in 0..spawn_count {
            let lifetime_range = (emitter.lifetime_min, emitter.lifetime_max);
            let speed_range = (emitter.speed_min, emitter.speed_max);
            let angular_velocity_range =
                (emitter.angular_velocity_min, emitter.angular_velocity_max);
            let lifetime = emitter.random_range(lifetime_range.0, lifetime_range.1);
            let speed = emitter.random_range(speed_range.0, speed_range.1);
            let angular_velocity =
                emitter.random_range(angular_velocity_range.0, angular_velocity_range.1);
            let direction = random_direction_in_cone(emitter);
            let rotation = emitter.random_range(0.0, std::f32::consts::TAU);
            emitter.particles_mut().push(Particle {
                position: origin,
                velocity: direction * speed,
                age: 0.0,
                lifetime,
                rotation,
                angular_velocity,
            });
        }
    }

    for (_, decal) in world.query_mut::<Decal>() {
        decal.tick(dt);
    }
}

fn random_direction_in_cone(emitter: &mut ParticleEmitter) -> Vec3 {
    let axis = emitter.direction.normalize_or_zero();
    let axis = if axis == Vec3::ZERO { Vec3::Y } else { axis };
    let cos_limit = emitter.spread_angle_radians.cos();
    let cos_theta = 1.0 - emitter.random_unit() * (1.0 - cos_limit);
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = emitter.random_range(0.0, std::f32::consts::TAU);
    let tangent = axis.any_orthonormal_vector();
    let bitangent = axis.cross(tangent);
    (axis * cos_theta + tangent * (sin_theta * phi.cos()) + bitangent * (sin_theta * phi.sin()))
        .normalize_or_zero()
}

/// Append visible particle billboards and mesh decals to a scene frame.
pub fn extract_vfx(world: &World, input: &mut RenderFrameInput) {
    let relative_origin = camera_relative_render_origin(world).unwrap_or(Vec3::ZERO);
    let camera_world = input
        .views
        .first()
        .map(|view| Mat4::from_cols_array(&view.view_matrix).inverse())
        .unwrap_or(Mat4::IDENTITY);
    let camera_rotation = Mat3::from_mat4(camera_world);
    let camera_right = camera_rotation.x_axis.normalize_or_zero();
    let camera_up = camera_rotation.y_axis.normalize_or_zero();
    let camera_back = camera_rotation.z_axis.normalize_or_zero();

    let mut visible = 0_u32;
    let mut culled = 0_u32;
    for (entity, emitter) in world.query::<ParticleEmitter>() {
        if !emitter.enabled {
            continue;
        }
        let entity_id = world.persistent_id(entity).map(str::to_owned);
        for (particle_index, particle) in emitter.particles().iter().enumerate() {
            let progress = (particle.age / particle.lifetime).clamp(0.0, 1.0);
            let size = emitter.start_size + (emitter.end_size - emitter.start_size) * progress;
            if size <= 0.0 {
                continue;
            }
            let position = particle.position - relative_origin;
            let bounds = centered_bounds(position, Vec3::splat(size * 0.75));
            if !visible_in_any_view(input, &emitter.render_layer, &bounds) {
                culled += 1;
                continue;
            }
            let (sin, cos) = particle.rotation.sin_cos();
            let right = (camera_right * cos + camera_up * sin) * size;
            let up = (-camera_right * sin + camera_up * cos) * size;
            let world_matrix = Mat4::from_cols(
                right.extend(0.0),
                up.extend(0.0),
                camera_back.extend(0.0),
                position.extend(1.0),
            );
            let mesh = AssetId::new(&emitter.mesh_asset);
            let material = AssetId::new(&emitter.material_asset);
            input.drawables.push(RenderableItem {
                entity: entity_id
                    .as_ref()
                    .map(|id| format!("{id}#particle-{particle_index}")),
                sort_key: batch_sort_key(&material, &mesh),
                mesh,
                material,
                world_transform: world_matrix.to_cols_array(),
                bounds,
                render_layer: emitter.render_layer.clone(),
                cast_shadows: false,
            });
            visible += 1;
        }
    }

    for (entity, decal) in world.query::<Decal>() {
        if !decal.enabled || decal.expired() {
            continue;
        }
        let Some(base_world) = entity_world_transform(world, entity) else {
            continue;
        };
        let render_world = Mat4::from_translation(-relative_origin)
            * base_world
            * Mat4::from_translation(Vec3::Z * decal.normal_bias)
            * Mat4::from_scale(Vec3::new(decal.size[0], decal.size[1], 1.0));
        let position = render_world.transform_point3(Vec3::ZERO);
        let half = 0.5 * decal.size[0].max(decal.size[1]);
        let bounds = centered_bounds(position, Vec3::splat(half));
        if !visible_in_any_view(input, &decal.render_layer, &bounds) {
            culled += 1;
            continue;
        }
        let mesh = AssetId::new(&decal.mesh_asset);
        let material = AssetId::new(&decal.material_asset);
        input.drawables.push(RenderableItem {
            entity: world.persistent_id(entity).map(str::to_owned),
            sort_key: batch_sort_key(&material, &mesh),
            mesh,
            material,
            world_transform: render_world.to_cols_array(),
            bounds,
            render_layer: decal.render_layer.clone(),
            cast_shadows: false,
        });
        visible += 1;
    }

    if let Some(stats) = &mut input.extraction_stats {
        stats.visible_drawables = stats.visible_drawables.saturating_add(visible);
        stats.culled_drawables = stats.culled_drawables.saturating_add(culled);
    }
    input.drawables.sort_by_key(|item| item.sort_key);
}

fn visible_in_any_view(
    input: &RenderFrameInput,
    render_layer: &str,
    bounds: &AxisAlignedBox,
) -> bool {
    let Some(layer_bit) = engine_scene::render_layer_bit(render_layer) else {
        return false;
    };
    let layer_mask = 1_u32 << layer_bit;
    let center = [
        (bounds.min[0] + bounds.max[0]) * 0.5,
        (bounds.min[1] + bounds.max[1]) * 0.5,
        (bounds.min[2] + bounds.max[2]) * 0.5,
    ];
    let half = [
        (bounds.max[0] - bounds.min[0]) * 0.5,
        (bounds.max[1] - bounds.min[1]) * 0.5,
        (bounds.max[2] - bounds.min[2]) * 0.5,
    ];
    input.views.iter().any(|view| {
        view.render_layer_mask & layer_mask != 0
            && view.frustum.as_ref().is_none_or(|planes| {
                let planes = planes.map(glam::Vec4::from_array);
                engine_scene::aabb_in_frustum(center, half, &planes)
            })
    })
}

fn centered_bounds(center: Vec3, half_extents: Vec3) -> AxisAlignedBox {
    AxisAlignedBox {
        min: (center - half_extents).to_array(),
        max: (center + half_extents).to_array(),
    }
}

fn batch_sort_key(material: &AssetId, mesh: &AssetId) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    material.hash(&mut hasher);
    mesh.hash(&mut hasher);
    hasher.finish()
}

/// Rebase world-space live particle positions during an origin shift.
pub fn shift_world_positions(world: &mut World, offset: Vec3) -> usize {
    let mut shifted = 0;
    for (_, emitter) in world.query_all_mut::<ParticleEmitter>() {
        for particle in emitter.particles_mut() {
            particle.position += offset;
            shifted += 1;
        }
    }
    shifted
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_renderer::{ClearFlags, Rect, RenderView, ViewCompose};
    use engine_scene::components::Transform;

    fn test_input() -> RenderFrameInput {
        let mut input = RenderFrameInput::empty(1);
        input.views.push(RenderView {
            view_id: 0,
            camera_entity: None,
            viewport: Rect::FULL,
            viewport_rect_normalized: Rect::FULL,
            view_matrix: Mat4::IDENTITY.to_cols_array(),
            projection_matrix: Mat4::IDENTITY.to_cols_array(),
            clear_flags: ClearFlags::ColorAndDepth,
            clear_color: [0.0; 4],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: ViewCompose::Base {
                clear: ClearFlags::ColorAndDepth,
                clear_color: [0.0; 4],
            },
            stack_order: 0,
            frustum: None,
        });
        input
    }

    #[test]
    fn burst_emitter_updates_caps_and_expires_particles() {
        let mut world = World::new();
        let entity = world.create_entity();
        world.add_component(entity, Transform::default());
        let emitter = ParticleEmitter {
            burst_count: 10,
            max_particles: 4,
            emission_rate: 0.0,
            lifetime_min: 0.05,
            lifetime_max: 0.05,
            ..ParticleEmitter::default()
        };
        world.add_component(entity, emitter);

        update_vfx(&mut world, 0.01);
        assert_eq!(
            world
                .get::<ParticleEmitter>(entity)
                .unwrap()
                .particles()
                .len(),
            4
        );
        update_vfx(&mut world, 0.1);
        assert!(world
            .get::<ParticleEmitter>(entity)
            .unwrap()
            .particles()
            .is_empty());
    }

    #[test]
    fn extraction_emits_billboards_and_decals() {
        let mut world = World::new();
        let emitter_entity = world.create_entity();
        world.add_component(emitter_entity, Transform::default());
        world.add_component(
            emitter_entity,
            ParticleEmitter {
                burst_count: 1,
                emission_rate: 0.0,
                ..ParticleEmitter::default()
            },
        );
        let decal_entity = world.create_entity();
        world.add_component(
            decal_entity,
            Transform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                ..Transform::default()
            },
        );
        world.add_component(decal_entity, Decal::default());
        update_vfx(&mut world, 0.01);

        let mut input = test_input();
        extract_vfx(&world, &mut input);
        assert_eq!(input.drawables.len(), 2);
        assert!(input.drawables.iter().all(|item| !item.cast_shadows));
        assert!(input
            .drawables
            .iter()
            .all(|item| item.mesh.id == BUILTIN_VFX_QUAD_MESH_ID));
    }

    #[test]
    fn component_serde_does_not_persist_live_particles_or_decal_age() {
        let mut emitter = ParticleEmitter {
            burst_count: 1,
            emission_rate: 0.0,
            ..ParticleEmitter::default()
        };
        let mut world = World::new();
        let entity = world.create_entity();
        world.add_component(entity, Transform::default());
        world.add_component(entity, emitter.clone());
        update_vfx(&mut world, 0.01);
        emitter = world.get::<ParticleEmitter>(entity).unwrap().clone();
        assert_eq!(emitter.particles().len(), 1);

        let fields = serde::serialize_particle_emitter(&emitter);
        let decoded = serde::deserialize_particle_emitter(&fields);
        let decoded = decoded.downcast_ref::<ParticleEmitter>().unwrap();
        assert!(decoded.particles().is_empty());
        assert_eq!(decoded.burst_count, 1);
    }

    #[test]
    fn origin_shift_rebases_live_world_space_particles() {
        let mut world = World::new();
        let entity = world.create_entity();
        world.add_component(
            entity,
            Transform {
                translation: Vec3::new(20.0, 0.0, 0.0),
                ..Transform::default()
            },
        );
        world.add_component(
            entity,
            ParticleEmitter {
                burst_count: 1,
                emission_rate: 0.0,
                ..ParticleEmitter::default()
            },
        );
        update_vfx(&mut world, 0.01);
        assert_eq!(
            world.get::<ParticleEmitter>(entity).unwrap().particles()[0].position,
            Vec3::new(20.0, 0.0, 0.0)
        );

        assert_eq!(
            shift_world_positions(&mut world, Vec3::new(-10.0, 0.0, 0.0)),
            1
        );
        assert_eq!(
            world.get::<ParticleEmitter>(entity).unwrap().particles()[0].position,
            Vec3::new(10.0, 0.0, 0.0)
        );
    }

    #[test]
    fn registered_vfx_components_roundtrip_through_scene_storage() {
        let mut registry = ComponentRegistry::new();
        registry.register_core();
        register_vfx_extensions(&mut registry);
        let registry = std::sync::Arc::new(registry);

        let mut world = World::new();
        world.set_shared_component_registry(std::sync::Arc::clone(&registry));
        let entity = world
            .create_persistent_entity("vfx-entity")
            .expect("unique id");
        world.add_component(entity, Transform::default());
        world.add_component(
            entity,
            ParticleEmitter {
                emission_rate: 42.0,
                material_asset: "material-smoke".to_string(),
                ..ParticleEmitter::default()
            },
        );
        world.add_component(
            entity,
            Decal {
                size: [2.0, 3.0],
                material_asset: "material-impact".to_string(),
                ..Decal::default()
            },
        );

        let scene = world.to_scene();
        let restored =
            World::try_from_scene_with_registry(&scene, registry).expect("strict VFX reload");
        let entity = restored.entity_by_persistent_id("vfx-entity").unwrap();
        assert_eq!(
            restored
                .get::<ParticleEmitter>(entity)
                .unwrap()
                .emission_rate,
            42.0
        );
        assert_eq!(restored.get::<Decal>(entity).unwrap().size, [2.0, 3.0]);
        let record = &scene.entities[0];
        assert!(matches!(
            record.components["engine.vfx.particle_emitter"]
                .fields
                .get("material_asset"),
            Some(engine_serialize::Value::Asset(asset)) if asset.id == "material-smoke"
        ));
        assert!(matches!(
            record.components["engine.vfx.decal"]
                .fields
                .get("material_asset"),
            Some(engine_serialize::Value::Asset(asset)) if asset.id == "material-impact"
        ));
    }
}
