//! Game-agnostic gravity sources.
//!
//! A [`GravitySource`] component shapes the gravity experienced by dynamic
//! rigid bodies so games can build planet-style point gravity or directional
//! gravity fields instead of relying on the single global gravity vector.
//!
//! # Modes
//!
//! - [`GravityMode::Directional`] — constant acceleration along
//!   [`GravitySource::direction`] scaled by [`GravitySource::strength`],
//!   affecting every dynamic body in the world.
//! - [`GravityMode::Point`] — acceleration towards [`GravitySource::center`]
//!   scaled by [`GravitySource::strength`] and an optional distance
//!   [`GravityFalloff`], limited by an optional
//!   [`GravitySource::max_radius`]. `strength` is the acceleration at one
//!   metre from the centre.
//!
//! All component positions (`center`) are world-space values.
//!
//! # Combination semantics
//!
//! Effective gravity is resolved per dynamic body per fixed physics step:
//!
//! 1. Every enabled source whose field reaches the body contributes an
//!    acceleration vector (see [`GravitySource::contribution`]).
//! 2. When at least one source contributes, the contributions are **summed**
//!    (superposition) and the sum — even when it cancels to zero — replaces
//!    the global gravity vector for that body. The body's own
//!    `RigidBody::gravity_scale` still multiplies the result.
//! 3. When no source contributes (no sources exist, all are disabled, or the
//!    body is outside every point source's `max_radius`), the body falls back
//!    to the configured global gravity exactly as before.
//!
//! Sources on disabled entities never contribute. Bodies under at least one
//! contributing source are kept awake while the source drives them; bodies
//! that fall back to global gravity keep rapier's normal sleep behaviour.

use serde::{Deserialize, Serialize};

use engine_scene::Component;

/// Distance (in metres) at or below which a body is treated as resting at the
/// exact centre of a point source. The pull direction is undefined there, so
/// the contribution is a zero vector (which still suppresses the global
/// gravity fallback). This also bounds the [`GravityFalloff::InverseSquare`]
/// singularity to `strength / GRAVITY_SOURCE_MIN_DISTANCE²`.
pub const GRAVITY_SOURCE_MIN_DISTANCE: f32 = 1.0e-3;

// ── GravityMode ─────────────────────────────────────────────────────────────

/// How a [`GravitySource`] shapes its gravitational field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GravityMode {
    /// Constant acceleration everywhere along `direction * strength`.
    #[default]
    Directional,
    /// Acceleration towards `center`, attenuated by `falloff` and limited by
    /// `max_radius`.
    Point,
}

// ── GravityFalloff ──────────────────────────────────────────────────────────

/// Distance attenuation model for [`GravityMode::Point`] sources.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GravityFalloff {
    /// Full strength at every distance inside `max_radius`.
    #[default]
    None,
    /// Linear ramp from full strength at the centre to zero at `max_radius`.
    /// Without a `max_radius` this behaves like [`GravityFalloff::None`].
    Linear,
    /// Inverse-square attenuation: `strength / d²` where `d` is the distance
    /// to the centre clamped below by [`GRAVITY_SOURCE_MIN_DISTANCE`].
    /// `strength` is therefore the acceleration one metre from the centre.
    InverseSquare,
}

// ── GravitySource ───────────────────────────────────────────────────────────

/// Gravity source component.
///
/// Attach to any entity to shape the gravity of dynamic rigid bodies around
/// it. Serialisable — the scene format and the script component bridge share
/// the registered serde hooks, and the physics step re-reads the component
/// every fixed step so runtime edits (including script writes) take effect on
/// the next step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GravitySource {
    /// Whether this source shapes gravity: directional field or point field.
    pub mode: GravityMode,
    /// Whether the source currently contributes. Disabled sources are
    /// skipped entirely, as are sources on disabled entities.
    pub enabled: bool,
    /// Field strength in m/s². For point sources with
    /// [`GravityFalloff::InverseSquare`] this is the acceleration one metre
    /// from the centre. Negative values repel instead of attracting.
    pub strength: f32,
    /// Pull direction for [`GravityMode::Directional`]. Normalised at
    /// resolution time; must be finite and non-zero to contribute.
    pub direction: glam::Vec3,
    /// World-space centre for [`GravityMode::Point`].
    pub center: glam::Vec3,
    /// Distance attenuation for [`GravityMode::Point`].
    pub falloff: GravityFalloff,
    /// Range limit in metres for [`GravityMode::Point`]. Bodies farther than
    /// this receive no contribution. `None` means unlimited range. Values
    /// that are non-finite or `<= 0` are treated as `None`.
    pub max_radius: Option<f32>,
}

impl Default for GravitySource {
    fn default() -> Self {
        Self {
            mode: GravityMode::Directional,
            enabled: true,
            strength: 9.81,
            direction: glam::Vec3::new(0.0, -1.0, 0.0),
            center: glam::Vec3::ZERO,
            falloff: GravityFalloff::None,
            max_radius: None,
        }
    }
}

impl Component for GravitySource {
    const TYPE_ID: &'static str = "engine.gravity_source";
}

impl GravitySource {
    /// Create a directional gravity field (for example wind-tunnel or
    /// sideways-gravity sections).
    pub fn directional(direction: glam::Vec3, strength: f32) -> Self {
        Self {
            mode: GravityMode::Directional,
            direction,
            strength,
            ..Self::default()
        }
    }

    /// Create a point gravity source (planet-style gravity) centred at the
    /// world-space `center`.
    pub fn point(center: glam::Vec3, strength: f32) -> Self {
        Self {
            mode: GravityMode::Point,
            center,
            strength,
            ..Self::default()
        }
    }

    /// Set the distance falloff model (point mode only).
    pub fn with_falloff(mut self, falloff: GravityFalloff) -> Self {
        self.falloff = falloff;
        self
    }

    /// Set the range limit in metres (point mode only).
    pub fn with_max_radius(mut self, max_radius: f32) -> Self {
        self.max_radius = Some(max_radius);
        self
    }

    /// The acceleration this source exerts on a body at `body_position`.
    ///
    /// Returns `None` when the source does not reach the body: the source is
    /// disabled, its configured values are unusable (non-finite, or a zero
    /// direction), or the body lies outside `max_radius`. A body at the exact
    /// centre of a point source receives `Some(Vec3::ZERO)` — it is inside
    /// the field, so the global gravity fallback stays suppressed, but the
    /// pull direction is undefined.
    pub fn contribution(&self, body_position: glam::Vec3) -> Option<glam::Vec3> {
        if !self.enabled || !self.strength.is_finite() {
            return None;
        }
        match self.mode {
            GravityMode::Directional => {
                if !self.direction.is_finite() {
                    return None;
                }
                let direction = self.direction.normalize_or_zero();
                if direction == glam::Vec3::ZERO {
                    return None;
                }
                Some(direction * self.strength)
            }
            GravityMode::Point => {
                if !self.center.is_finite() {
                    return None;
                }
                let offset = self.center - body_position;
                let distance = offset.length();
                if !distance.is_finite() {
                    return None;
                }
                let max_radius = self.effective_max_radius();
                if let Some(radius) = max_radius {
                    if distance > radius {
                        return None;
                    }
                }
                if distance <= GRAVITY_SOURCE_MIN_DISTANCE {
                    return Some(glam::Vec3::ZERO);
                }
                let factor = match self.falloff {
                    GravityFalloff::None => 1.0,
                    GravityFalloff::Linear => match max_radius {
                        Some(radius) => (1.0 - distance / radius).max(0.0),
                        None => 1.0,
                    },
                    GravityFalloff::InverseSquare => 1.0 / (distance * distance),
                };
                Some(offset / distance * (self.strength * factor))
            }
        }
    }

    /// The validated range limit, or `None` when the authored value is
    /// missing, non-finite, or non-positive.
    pub fn effective_max_radius(&self) -> Option<f32> {
        match self.max_radius {
            Some(radius) if radius.is_finite() && radius > 0.0 => Some(radius),
            _ => None,
        }
    }
}

// ── Resolution ──────────────────────────────────────────────────────────────

/// Sum the contributions of every source that reaches `body_position`.
///
/// Returns `None` when no source contributes, signalling that the body keeps
/// the configured global gravity.
pub fn sum_source_gravity<'a>(
    sources: impl IntoIterator<Item = &'a GravitySource>,
    body_position: glam::Vec3,
) -> Option<glam::Vec3> {
    let mut sum = glam::Vec3::ZERO;
    let mut any = false;
    for source in sources {
        if let Some(contribution) = source.contribution(body_position) {
            sum += contribution;
            any = true;
        }
    }
    any.then_some(sum)
}

/// Resolve the effective gravity acceleration for a body at `body_position`.
///
/// Contributions from all in-range sources are summed (superposition); when
/// no source contributes the body falls back to `global_gravity`. The
/// returned value is an acceleration — multiply by the body's
/// `RigidBody::gravity_scale` to honour per-body scaling.
pub fn resolve_effective_gravity<'a>(
    sources: impl IntoIterator<Item = &'a GravitySource>,
    body_position: glam::Vec3,
    global_gravity: glam::Vec3,
) -> glam::Vec3 {
    sum_source_gravity(sources, body_position).unwrap_or(global_gravity)
}

/// Translate every [`GravitySource::center`] in the world by `offset`.
///
/// `center` is stored in the same origin-relative f32 world space as
/// `Transform::translation`, so world-origin shifts must move it by the same
/// `-delta` to keep logical source positions unchanged. Directional sources
/// ignore `center`, but shifting it unconditionally keeps the sweep simple
/// and the stored data consistent. Disabled entities are included: their
/// stored centers are world-space state like any other.
///
/// Returns the number of sources shifted.
pub fn shift_gravity_source_centers(world: &mut engine_scene::World, offset: glam::Vec3) -> usize {
    let mut shifted = 0;
    for (_, source) in world.query_all_mut::<GravitySource>() {
        source.center += offset;
        shifted += 1;
    }
    shifted
}
