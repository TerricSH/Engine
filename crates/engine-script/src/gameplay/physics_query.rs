use serde::{Deserialize, Serialize};

use super::validation::validate_entity_id;

/// Optional filter applied to a script physics query.
///
/// All fields are optional; an absent filter (or an all-default filter)
/// reproduces the original query behaviour: every collision layer matches,
/// sensor (trigger) colliders are skipped, and no entity is excluded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayPhysicsQueryFilter {
    /// Only colliders whose `collision_group` shares at least one bit with
    /// this mask are candidates. `None` matches every layer. A zero mask
    /// matches nothing and is rejected as a validation error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_mask: Option<u32>,
    /// Include sensor (trigger) colliders in the query. Defaults to `false`,
    /// preserving the long-standing sensor-excluding behaviour.
    #[serde(default)]
    pub include_sensors: bool,
    /// Persistent id of an entity whose colliders the query ignores, for
    /// example self-exclusion for character casts. The id must be a valid
    /// entity id and — enforced by the runtime, which owns the world — must
    /// name an existing entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_entity: Option<String>,
}

impl GameplayPhysicsQueryFilter {
    /// Validate untrusted filter data received from a script host.
    ///
    /// This checks the self-contained invariants (non-zero mask, well-formed
    /// entity id); the runtime separately rejects `exclude_entity` ids that
    /// do not name an existing entity.
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.layer_mask, Some(0)) {
            return Err("physics query layer_mask must be non-zero".into());
        }
        if let Some(exclude_entity) = &self.exclude_entity {
            validate_entity_id(exclude_entity)?;
        }
        Ok(())
    }
}

/// Active physics query requested by a script through the gameplay bridge.
///
/// Queries travel as deferred gameplay commands: the engine validates and
/// executes them against the physics world at the frame boundary and
/// delivers the matching [`GameplayPhysicsQueryResult`] with the next
/// frame's snapshot. Scripts correlate requests and results through the
/// caller-chosen `query_id`; scripts never receive raw ECS handles or
/// backend objects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameplayPhysicsQuery {
    /// Cast a ray and report the closest collider hit, if any.
    Raycast {
        /// Script-chosen correlator echoed back with the result.
        query_id: u32,
        /// World-space ray origin.
        origin: [f32; 3],
        /// Ray direction. Does not need to be normalised; the engine
        /// normalises before querying. Must not be zero length.
        direction: [f32; 3],
        /// Maximum travel distance, clamped to [`MAX_PHYSICS_QUERY_DISTANCE`].
        max_distance: f32,
        /// Optional candidate filter (layer mask, sensors, self-exclusion).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<GameplayPhysicsQueryFilter>,
    },
    /// Sweep a sphere along a direction and report the closest hit, if any.
    SphereCast {
        /// Script-chosen correlator echoed back with the result.
        query_id: u32,
        /// World-space centre of the sphere at the start of the sweep.
        origin: [f32; 3],
        /// Sphere radius, clamped to [`MAX_PHYSICS_QUERY_DISTANCE`].
        radius: f32,
        /// Sweep direction. Does not need to be normalised; the engine
        /// normalises before querying. Must not be zero length.
        direction: [f32; 3],
        /// Maximum travel distance, clamped to [`MAX_PHYSICS_QUERY_DISTANCE`].
        max_distance: f32,
        /// Optional candidate filter (layer mask, sensors, self-exclusion).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<GameplayPhysicsQueryFilter>,
    },
    /// Find every collider overlapping a world-space sphere.
    OverlapSphere {
        /// Script-chosen correlator echoed back with the result.
        query_id: u32,
        /// World-space sphere centre.
        center: [f32; 3],
        /// Sphere radius, clamped to [`MAX_PHYSICS_QUERY_DISTANCE`].
        radius: f32,
        /// Optional candidate filter (layer mask, sensors, self-exclusion).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<GameplayPhysicsQueryFilter>,
    },
}

impl GameplayPhysicsQuery {
    /// The script-chosen correlator carried by this query.
    pub fn query_id(&self) -> u32 {
        match self {
            Self::Raycast { query_id, .. }
            | Self::SphereCast { query_id, .. }
            | Self::OverlapSphere { query_id, .. } => *query_id,
        }
    }

    /// The optional candidate filter carried by this query.
    pub fn filter(&self) -> Option<&GameplayPhysicsQueryFilter> {
        match self {
            Self::Raycast { filter, .. }
            | Self::SphereCast { filter, .. }
            | Self::OverlapSphere { filter, .. } => filter.as_ref(),
        }
    }

    /// Validate untrusted query data received from a script host.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(filter) = self.filter() {
            filter.validate()?;
        }
        match self {
            Self::Raycast {
                origin,
                direction,
                max_distance,
                ..
            } => {
                if !origin.iter().all(|value| value.is_finite()) {
                    return Err("raycast origin must contain only finite values".into());
                }
                validate_sweep_direction(direction, "raycast")?;
                if !max_distance.is_finite() || *max_distance <= 0.0 {
                    return Err("raycast max_distance must be finite and greater than zero".into());
                }
                Ok(())
            }
            Self::SphereCast {
                origin,
                radius,
                direction,
                max_distance,
                ..
            } => {
                if !origin.iter().all(|value| value.is_finite()) {
                    return Err("sphere cast origin must contain only finite values".into());
                }
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err("sphere cast radius must be finite and greater than zero".into());
                }
                validate_sweep_direction(direction, "sphere cast")?;
                if !max_distance.is_finite() || *max_distance <= 0.0 {
                    return Err(
                        "sphere cast max_distance must be finite and greater than zero".into(),
                    );
                }
                Ok(())
            }
            Self::OverlapSphere { center, radius, .. } => {
                if !center.iter().all(|value| value.is_finite()) {
                    return Err("overlap sphere center must contain only finite values".into());
                }
                if !radius.is_finite() || *radius <= 0.0 {
                    return Err("overlap sphere radius must be finite and greater than zero".into());
                }
                Ok(())
            }
        }
    }
}

/// Shared validation for raycast / sphere-cast sweep directions.
fn validate_sweep_direction(direction: &[f32; 3], query_kind: &str) -> Result<(), String> {
    if !direction.iter().all(|value| value.is_finite()) {
        return Err(format!(
            "{query_kind} direction must contain only finite values"
        ));
    }
    if direction.iter().map(|value| value * value).sum::<f32>() <= f32::EPSILON {
        return Err(format!("{query_kind} direction must not be zero length"));
    }
    Ok(())
}

/// Outcome of a script physics query, delivered with the next frame
/// snapshot following the frame that issued the query.
///
/// Results are frame-local: they appear in exactly one snapshot and are not
/// repeated. Every result echoes the issuing query's `query_id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayInteractionSnapshot {
    pub prompt: String,
    pub action: String,
    pub max_distance: f32,
    pub grabbable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameplayPhysicsQueryResult {
    /// A raycast found a collider attached to the given persistent entity.
    RaycastHit {
        /// Correlator from the issuing query.
        query_id: u32,
        /// Persistent entity id of the closest hit — never a raw ECS handle.
        entity_id: String,
        /// World-space intersection point.
        point: [f32; 3],
        /// World-space surface normal at the intersection.
        normal: [f32; 3],
        /// Distance from the ray origin to the intersection.
        distance: f32,
        /// Present only when the hit carries an enabled Interactable whose
        /// authored maximum distance includes this hit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interaction: Option<GameplayInteractionSnapshot>,
    },
    /// A raycast found no collider within range.
    RaycastMiss {
        /// Correlator from the issuing query.
        query_id: u32,
    },
    /// A sphere cast found a collider attached to the given persistent
    /// entity. Carries the same payload as [`Self::RaycastHit`]: the
    /// world-space contact point on the hit collider, its outward normal,
    /// and the sweep travel distance.
    SphereCastHit {
        /// Correlator from the issuing query.
        query_id: u32,
        /// Persistent entity id of the closest hit — never a raw ECS handle.
        entity_id: String,
        /// World-space contact point on the hit collider.
        point: [f32; 3],
        /// World-space outward surface normal of the hit collider.
        normal: [f32; 3],
        /// Travel distance from the sweep origin to the impact.
        distance: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interaction: Option<GameplayInteractionSnapshot>,
    },
    /// A sphere cast found no collider within range.
    SphereCastMiss {
        /// Correlator from the issuing query.
        query_id: u32,
    },
    /// Persistent entity ids overlapped by a sphere query, sorted and
    /// bounded to [`MAX_PHYSICS_OVERLAP_RESULTS`].
    OverlapSphere {
        /// Correlator from the issuing query.
        query_id: u32,
        /// Overlapping persistent entity ids — never raw ECS handles.
        entity_ids: Vec<String>,
    },
}
