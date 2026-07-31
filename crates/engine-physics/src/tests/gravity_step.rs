// Gravity Source Physics Step Tests
// ══════════════════════════════════════════════════════════════════════════════

fn ecs_with_dynamic_body(position: glam::Vec3) -> (World, Entity) {
    let mut ecs = World::new();
    let entity = ecs.create_entity();
    ecs.add_component(
        entity,
        Transform {
            translation: position,
            ..Default::default()
        },
    );
    ecs.add_component(entity, RigidBody::default());
    ecs.add_component(entity, Collider::default());
    (ecs, entity)
}

fn step_n(world: &mut PhysicsWorld, ecs: &mut World, frames: usize) {
    for _ in 0..frames {
        world.step(1.0 / 60.0, ecs);
    }
}

#[test]
fn gravity_source_point_pulls_body_toward_center() {
    // No global gravity: the point source is the only acceleration.
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::new(0.0, 10.0, 0.0));
    let planet = ecs.create_entity();
    ecs.add_component(planet, GravitySource::point(glam::Vec3::ZERO, 9.81));

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    assert!(
        transform.translation.y < 10.0,
        "body should fall towards the planet centre: y={}",
        transform.translation.y
    );
    assert!(
        transform.translation.x.abs() < 1e-4 && transform.translation.z.abs() < 1e-4,
        "pull is purely radial: {:?}",
        transform.translation
    );
}

#[test]
fn gravity_source_directional_overrides_global_direction() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    assert!(
        transform.translation.x > 0.01,
        "body should accelerate along the directional field: {:?}",
        transform.translation
    );
    assert!(
        transform.translation.y.abs() < 1e-6,
        "the directional field replaces the global -Y gravity for this body: {:?}",
        transform.translation
    );
}

#[test]
fn gravity_source_out_of_range_body_keeps_global_gravity() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::new(0.0, 10.0, 0.0));
    let planet = ecs.create_entity();
    ecs.add_component(
        planet,
        GravitySource::point(glam::Vec3::new(1000.0, 0.0, 0.0), 50.0).with_max_radius(10.0),
    );

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    assert!(
        transform.translation.y < 10.0,
        "body outside every source range still falls under global gravity: y={}",
        transform.translation.y
    );
    assert!(
        transform.translation.x.abs() < 1e-4,
        "no sideways pull from the out-of-range source: {:?}",
        transform.translation
    );
}

#[test]
fn gravity_source_cancelling_fields_keep_body_in_place() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field_a = ecs.create_entity();
    ecs.add_component(
        field_a,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );
    let field_b = ecs.create_entity();
    ecs.add_component(
        field_b,
        GravitySource::directional(glam::Vec3::new(-1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    approx_vec3(
        transform.translation,
        glam::Vec3::ZERO,
        "cancelling fields suppress the global fallback and leave the body in place",
    );
}

#[test]
fn gravity_source_respects_body_gravity_scale() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    ecs.add_component(
        body,
        RigidBody {
            gravity_scale: 0.5,
            ..RigidBody::default()
        },
    );
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 30);
    let half_displacement = ecs.get::<Transform>(body).unwrap().translation.x;

    // Reference run at full gravity scale.
    let mut world_full = PhysicsWorld::new(glam::Vec3::ZERO);
    let (mut ecs_full, body_full) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field_full = ecs_full.create_entity();
    ecs_full.add_component(
        field_full,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );
    step_n(&mut world_full, &mut ecs_full, 30);
    let full_displacement = ecs_full.get::<Transform>(body_full).unwrap().translation.x;

    assert!(full_displacement > 0.01);
    let ratio = half_displacement / full_displacement;
    assert!(
        (ratio - 0.5).abs() < 1e-3,
        "gravity_scale 0.5 should halve the source-driven acceleration: ratio={ratio}"
    );
}

#[test]
fn gravity_source_removed_restores_global_gravity() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 10);
    let driven = ecs.get::<Transform>(body).unwrap().translation;
    assert!(driven.x > 0.0, "field drives the body sideways: {driven:?}");
    assert_eq!(driven.y, 0.0, "global gravity is replaced while driven");

    // Remove the source: the body falls back to the configured global
    // gravity on the next step.
    ecs.remove_component::<GravitySource>(field);
    step_n(&mut world, &mut ecs, 30);

    let restored = ecs.get::<Transform>(body).unwrap().translation;
    assert!(
        restored.y < 0.0,
        "body resumes global -Y gravity after the source is removed: {restored:?}"
    );
}

#[test]
fn gravity_source_disabled_component_restores_global_gravity() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 10);
    assert!(ecs.get::<Transform>(body).unwrap().translation.x > 0.0);

    ecs.get_mut::<GravitySource>(field).unwrap().enabled = false;
    step_n(&mut world, &mut ecs, 30);

    let restored = ecs.get::<Transform>(body).unwrap().translation;
    assert!(
        restored.y < 0.0,
        "disabling the source restores global gravity: {restored:?}"
    );
}

#[test]
fn gravity_source_live_edit_takes_effect_next_step() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::ZERO);
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 10);
    let after_first_phase = ecs.get::<Transform>(body).unwrap().translation.x;
    assert!(after_first_phase > 0.0);

    // Edit the component in place (this is what a script write through the
    // component bridge commits): the reversed field applies from the very
    // next physics step without recreating the body.
    ecs.get_mut::<GravitySource>(field).unwrap().direction = glam::Vec3::new(-1.0, 0.0, 0.0);
    step_n(&mut world, &mut ecs, 20);

    let after_second_phase = ecs.get::<Transform>(body).unwrap().translation.x;
    assert!(
        after_second_phase < after_first_phase,
        "live field edit reverses the body's motion: {after_first_phase} -> {after_second_phase}"
    );
}

#[test]
fn gravity_source_does_not_move_static_bodies() {
    let mut world = PhysicsWorld::new(glam::Vec3::ZERO);
    let mut ecs = World::new();
    let body = ecs.create_entity();
    ecs.add_component(body, Transform::default());
    ecs.add_component(
        body,
        RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
    );
    ecs.add_component(body, Collider::default());
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );

    step_n(&mut world, &mut ecs, 30);

    approx_vec3(
        ecs.get::<Transform>(body).unwrap().translation,
        glam::Vec3::ZERO,
        "static bodies ignore gravity sources",
    );
}

#[test]
fn gravity_source_on_disabled_entity_does_not_contribute() {
    let mut world = PhysicsWorld::new(glam::Vec3::new(0.0, -9.81, 0.0));
    let (mut ecs, body) = ecs_with_dynamic_body(glam::Vec3::new(0.0, 10.0, 0.0));
    let field = ecs.create_entity();
    ecs.add_component(
        field,
        GravitySource::directional(glam::Vec3::new(1.0, 0.0, 0.0), 9.81),
    );
    ecs.set_enabled(field, false);

    step_n(&mut world, &mut ecs, 30);

    let transform = ecs.get::<Transform>(body).unwrap();
    assert!(
        transform.translation.y < 10.0 && transform.translation.x.abs() < 1e-4,
        "sources on disabled entities are ignored; global gravity applies: {:?}",
        transform.translation
    );
}
