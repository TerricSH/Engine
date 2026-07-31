// Gravity Source Component Tests
// ══════════════════════════════════════════════════════════════════════════════

use crate::{
    resolve_effective_gravity, sum_source_gravity, GravityFalloff, GravityMode, GravitySource,
};

fn approx_vec3(actual: glam::Vec3, expected: glam::Vec3, label: &str) {
    assert!(
        (actual - expected).length() < 1e-5,
        "{label}: expected {expected:?}, got {actual:?}"
    );
}

#[test]
fn gravity_source_default_values() {
    let source = GravitySource::default();
    assert_eq!(source.mode, GravityMode::Directional);
    assert!(source.enabled);
    assert_eq!(source.strength, 9.81);
    assert_eq!(source.direction, glam::Vec3::new(0.0, -1.0, 0.0));
    assert_eq!(source.center, glam::Vec3::ZERO);
    assert_eq!(source.falloff, GravityFalloff::None);
    assert_eq!(source.max_radius, None);
}

#[test]
fn gravity_source_type_id() {
    assert_eq!(GravitySource::TYPE_ID, "engine.gravity_source");
    assert_ne!(GravitySource::TYPE_ID, RigidBody::TYPE_ID);
    assert_ne!(GravitySource::TYPE_ID, Collider::TYPE_ID);
}

#[test]
fn gravity_source_constructors() {
    let directional = GravitySource::directional(glam::Vec3::new(2.0, 0.0, 0.0), 3.5);
    assert_eq!(directional.mode, GravityMode::Directional);
    assert_eq!(directional.direction, glam::Vec3::new(2.0, 0.0, 0.0));
    assert_eq!(directional.strength, 3.5);

    let point = GravitySource::point(glam::Vec3::new(1.0, 2.0, 3.0), 12.0)
        .with_falloff(GravityFalloff::InverseSquare)
        .with_max_radius(50.0);
    assert_eq!(point.mode, GravityMode::Point);
    assert_eq!(point.center, glam::Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(point.falloff, GravityFalloff::InverseSquare);
    assert_eq!(point.max_radius, Some(50.0));
}

#[test]
fn gravity_source_contribution_directional_normalizes() {
    let source = GravitySource::directional(glam::Vec3::new(0.0, 3.0, 0.0), 6.0);
    let contribution = source
        .contribution(glam::Vec3::new(100.0, -50.0, 25.0))
        .expect("directional source reaches every position");
    approx_vec3(
        contribution,
        glam::Vec3::new(0.0, 6.0, 0.0),
        "directional contribution is normalised direction times strength",
    );
}

#[test]
fn gravity_source_contribution_directional_rejects_zero_direction() {
    let source = GravitySource::directional(glam::Vec3::ZERO, 9.81);
    assert_eq!(source.contribution(glam::Vec3::ZERO), None);
}

#[test]
fn gravity_source_contribution_disabled_source() {
    let mut source = GravitySource::directional(glam::Vec3::new(0.0, -1.0, 0.0), 9.81);
    source.enabled = false;
    assert_eq!(source.contribution(glam::Vec3::ZERO), None);
}

#[test]
fn gravity_source_contribution_point_no_falloff() {
    let source = GravitySource::point(glam::Vec3::ZERO, 9.81);
    let contribution = source
        .contribution(glam::Vec3::new(10.0, 0.0, 0.0))
        .expect("body is inside the field");
    approx_vec3(
        contribution,
        glam::Vec3::new(-9.81, 0.0, 0.0),
        "point source pulls towards the centre at full strength",
    );
}

#[test]
fn gravity_source_contribution_point_linear_falloff() {
    let source = GravitySource::point(glam::Vec3::ZERO, 10.0)
        .with_falloff(GravityFalloff::Linear)
        .with_max_radius(20.0);
    let contribution = source
        .contribution(glam::Vec3::new(10.0, 0.0, 0.0))
        .expect("body is inside max_radius");
    approx_vec3(
        contribution,
        glam::Vec3::new(-5.0, 0.0, 0.0),
        "linear falloff halves the strength at half the radius",
    );

    // At exactly max_radius the ramp reaches zero, but the body is still in
    // range, so the fallback stays suppressed.
    let edge = source.contribution(glam::Vec3::new(20.0, 0.0, 0.0));
    assert_eq!(edge, Some(glam::Vec3::ZERO));
}

#[test]
fn gravity_source_contribution_point_linear_without_radius_is_constant() {
    let source = GravitySource::point(glam::Vec3::ZERO, 10.0).with_falloff(GravityFalloff::Linear);
    let contribution = source
        .contribution(glam::Vec3::new(123.0, 0.0, 0.0))
        .expect("no range limit");
    approx_vec3(
        contribution,
        glam::Vec3::new(-10.0, 0.0, 0.0),
        "linear falloff without max_radius behaves like no falloff",
    );
}

#[test]
fn gravity_source_contribution_point_inverse_square() {
    let source =
        GravitySource::point(glam::Vec3::ZERO, 20.0).with_falloff(GravityFalloff::InverseSquare);
    // At 2 m: strength / d^2 = 20 / 4 = 5 towards the centre.
    let contribution = source
        .contribution(glam::Vec3::new(0.0, 2.0, 0.0))
        .expect("body is inside the field");
    approx_vec3(
        contribution,
        glam::Vec3::new(0.0, -5.0, 0.0),
        "inverse-square falloff quarters strength at double distance",
    );
    // At 1 m the acceleration equals strength exactly.
    let at_one_metre = source
        .contribution(glam::Vec3::new(1.0, 0.0, 0.0))
        .expect("body is inside the field");
    approx_vec3(
        at_one_metre,
        glam::Vec3::new(-20.0, 0.0, 0.0),
        "inverse-square strength is the acceleration at one metre",
    );
}

#[test]
fn gravity_source_contribution_point_outside_max_radius() {
    let source = GravitySource::point(glam::Vec3::ZERO, 9.81).with_max_radius(5.0);
    assert_eq!(source.contribution(glam::Vec3::new(5.1, 0.0, 0.0)), None);
    assert!(source
        .contribution(glam::Vec3::new(4.9, 0.0, 0.0))
        .is_some());
}

#[test]
fn gravity_source_contribution_point_at_centre_is_zero() {
    let source = GravitySource::point(glam::Vec3::new(1.0, 2.0, 3.0), 9.81);
    assert_eq!(
        source.contribution(glam::Vec3::new(1.0, 2.0, 3.0)),
        Some(glam::Vec3::ZERO),
        "a body at the exact centre floats instead of falling back to global gravity"
    );
}

#[test]
fn gravity_source_contribution_negative_strength_repels() {
    let source = GravitySource::point(glam::Vec3::ZERO, -4.0);
    let contribution = source
        .contribution(glam::Vec3::new(2.0, 0.0, 0.0))
        .expect("body is inside the field");
    approx_vec3(
        contribution,
        glam::Vec3::new(4.0, 0.0, 0.0),
        "negative strength pushes bodies away from the centre",
    );
}

#[test]
fn gravity_source_contribution_rejects_non_finite_configuration() {
    let mut source = GravitySource::directional(glam::Vec3::new(0.0, -1.0, 0.0), f32::NAN);
    assert_eq!(source.contribution(glam::Vec3::ZERO), None);

    source = GravitySource::directional(glam::Vec3::new(f32::INFINITY, 0.0, 0.0), 1.0);
    assert_eq!(source.contribution(glam::Vec3::ZERO), None);

    let mut point = GravitySource::point(glam::Vec3::ZERO, 9.81);
    point.center = glam::Vec3::new(f32::NAN, 0.0, 0.0);
    assert_eq!(point.contribution(glam::Vec3::ZERO), None);

    // Non-positive or non-finite max_radius values are treated as unlimited.
    point = GravitySource::point(glam::Vec3::ZERO, 9.81).with_max_radius(-3.0);
    assert!(point
        .contribution(glam::Vec3::new(100.0, 0.0, 0.0))
        .is_some());
    point.max_radius = Some(f32::NAN);
    assert!(point
        .contribution(glam::Vec3::new(100.0, 0.0, 0.0))
        .is_some());
}

// ══════════════════════════════════════════════════════════════════════════════
