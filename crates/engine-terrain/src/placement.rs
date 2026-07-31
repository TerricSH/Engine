use std::collections::BTreeMap;

use bincode::Options;
use engine_scene::{
    components::Transform, Component, ComponentExtension, ComponentMeta, ComponentRegistry,
    ComponentStorageDyn, ScriptAccess, SparseSet,
};
use engine_serialize::Value;
use glam::{DMat3, DQuat, DVec3, Quat, Vec3};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{PlanetTangentFrame, PlanetTerrainQuery};

/// Persistent, game-agnostic attachment of an entity to a generated planet.
///
/// The component stores a logical radial direction instead of a local f32
/// position. The terrain runtime can therefore rebuild the entity transform
/// after a floating-origin shift or terrain recipe revision without scripts
/// reproducing surface mathematics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanetSurfaceAnchor {
    pub enabled: bool,
    /// Persistent ID of the owning `TerrainVolume` entity.
    ///
    /// Empty preserves legacy single-planet scenes. Hosts may resolve that
    /// shorthand only when exactly one enabled cube-sphere volume exists.
    pub terrain_volume_id: String,
    pub direction: [f64; 3],
    pub heading_radians: f64,
    pub altitude_offset: f64,
    pub footprint_radius: f64,
    pub max_slope_radians: f64,
    pub max_height_delta: f64,
    pub support_samples: u16,
    pub blocks_navigation: bool,
}

impl Default for PlanetSurfaceAnchor {
    fn default() -> Self {
        Self {
            enabled: true,
            terrain_volume_id: String::new(),
            direction: [0.0, 1.0, 0.0],
            heading_radians: 0.0,
            altitude_offset: 0.0,
            footprint_radius: 1.0,
            max_slope_radians: 35.0_f64.to_radians(),
            max_height_delta: 0.5,
            support_samples: 12,
            blocks_navigation: true,
        }
    }
}

impl Component for PlanetSurfaceAnchor {
    const TYPE_ID: &'static str = "engine.planet_surface_anchor";
}

/// Fully resolved surface basis and placement diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanetSurfacePlacement {
    pub position: [f64; 3],
    pub normal: [f64; 3],
    pub right: [f64; 3],
    pub forward: [f64; 3],
    /// Quaternion in `[x, y, z, w]` order.
    pub rotation: [f64; 4],
    pub radial_direction: [f64; 3],
    pub angular_radius: f64,
    pub maximum_slope_radians: f64,
    pub support_height_span: f64,
}

impl PlanetSurfacePlacement {
    /// Convert the logical f64 placement to the engine's origin-relative ECS
    /// transform. This is the only point at which construction placement loses
    /// precision to the renderer/physics f32 representation.
    pub fn to_transform(self, world_origin: [f64; 3]) -> Result<Transform, PlanetPlacementError> {
        let local = DVec3::from_array(self.position) - DVec3::from_array(world_origin);
        let rotation = DQuat::from_array(self.rotation);
        if !local.is_finite() || !rotation.is_finite() {
            return Err(PlanetPlacementError::OutsideLocalRange);
        }
        let local = local.as_vec3();
        let rotation = Quat::from_xyzw(
            rotation.x as f32,
            rotation.y as f32,
            rotation.z as f32,
            rotation.w as f32,
        );
        if !local.is_finite() || !rotation.is_finite() {
            return Err(PlanetPlacementError::OutsideLocalRange);
        }
        Ok(Transform {
            translation: local,
            rotation: rotation.normalize(),
            scale: Vec3::ONE,
            parent: None,
        })
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum PlanetPlacementError {
    #[error("surface anchor direction must be a finite non-zero vector")]
    InvalidDirection,
    #[error("surface anchor parameters are outside their finite supported ranges")]
    InvalidParameters,
    #[error("terrain slope {actual_radians} exceeds the allowed {maximum_radians} radians")]
    SlopeExceeded {
        actual_radians: f64,
        maximum_radians: f64,
    },
    #[error("terrain support height span {actual} exceeds the allowed {maximum}")]
    HeightSpanExceeded { actual: f64, maximum: f64 },
    #[error("surface footprint overlaps reserved construction '{existing_id}'")]
    Occupied { existing_id: String },
    #[error("logical placement cannot be represented relative to the current world origin")]
    OutsideLocalRange,
}

impl PlanetSurfaceAnchor {
    pub fn validate(&self, planet_radius: f64) -> Result<(), PlanetPlacementError> {
        let direction = DVec3::from_array(self.direction);
        if !direction.is_finite() || direction.length_squared() <= f64::EPSILON {
            return Err(PlanetPlacementError::InvalidDirection);
        }
        if !planet_radius.is_finite()
            || planet_radius <= 0.0
            || !self.heading_radians.is_finite()
            || !self.altitude_offset.is_finite()
            || !self.footprint_radius.is_finite()
            || self.footprint_radius < 0.0
            || self.footprint_radius >= std::f64::consts::PI * planet_radius
            || !self.max_slope_radians.is_finite()
            || !(0.0..=std::f64::consts::FRAC_PI_2).contains(&self.max_slope_radians)
            || !self.max_height_delta.is_finite()
            || self.max_height_delta < 0.0
            || !(4..=64).contains(&self.support_samples)
        {
            return Err(PlanetPlacementError::InvalidParameters);
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        query: &PlanetTerrainQuery,
    ) -> Result<PlanetSurfacePlacement, PlanetPlacementError> {
        self.validate(query.radius())?;
        let radial = DVec3::from_array(self.direction).normalize();
        let center = DVec3::from_array(query.center());
        let frame = query.tangent_frame(radial.to_array());
        let placement = placement_basis(frame, radial, self.heading_radians, self.altitude_offset);

        let mut minimum_radius = f64::INFINITY;
        let mut maximum_radius = f64::NEG_INFINITY;
        let mut maximum_slope = slope_radians(frame.normal, radial);
        let angular_radius = self.footprint_radius / query.radius();
        if self.footprint_radius > 0.0 {
            for sample_index in 0..usize::from(self.support_samples) {
                let angle =
                    std::f64::consts::TAU * sample_index as f64 / f64::from(self.support_samples);
                let tangent = DVec3::from_array(frame.east) * angle.cos()
                    + DVec3::from_array(frame.north) * angle.sin();
                let sample_direction =
                    (radial * angular_radius.cos() + tangent * angular_radius.sin()).normalize();
                let sample_frame = query.tangent_frame(sample_direction.to_array());
                let sample_radius =
                    (DVec3::from_array(sample_frame.surface_point) - center).length();
                minimum_radius = minimum_radius.min(sample_radius);
                maximum_radius = maximum_radius.max(sample_radius);
                maximum_slope =
                    maximum_slope.max(slope_radians(sample_frame.normal, sample_direction));
            }
        } else {
            let radius = (DVec3::from_array(frame.surface_point) - center).length();
            minimum_radius = radius;
            maximum_radius = radius;
        }
        let height_span = maximum_radius - minimum_radius;
        if maximum_slope > self.max_slope_radians {
            return Err(PlanetPlacementError::SlopeExceeded {
                actual_radians: maximum_slope,
                maximum_radians: self.max_slope_radians,
            });
        }
        if height_span > self.max_height_delta {
            return Err(PlanetPlacementError::HeightSpanExceeded {
                actual: height_span,
                maximum: self.max_height_delta,
            });
        }
        Ok(PlanetSurfacePlacement {
            angular_radius,
            maximum_slope_radians: maximum_slope,
            support_height_span: height_span,
            ..placement
        })
    }
}

fn placement_basis(
    frame: PlanetTangentFrame,
    radial: DVec3,
    heading_radians: f64,
    altitude_offset: f64,
) -> PlanetSurfacePlacement {
    let normal = DVec3::from_array(frame.normal).normalize();
    let east = DVec3::from_array(frame.east).normalize();
    let north = DVec3::from_array(frame.north).normalize();
    let forward = (north * heading_radians.cos() + east * heading_radians.sin()).normalize();
    let right = normal.cross(forward).normalize();
    let forward = right.cross(normal).normalize();
    let rotation = DQuat::from_mat3(&DMat3::from_cols(right, normal, forward)).normalize();
    let position = DVec3::from_array(frame.surface_point) + normal * altitude_offset;
    PlanetSurfacePlacement {
        position: position.to_array(),
        normal: normal.to_array(),
        right: right.to_array(),
        forward: forward.to_array(),
        rotation: rotation.to_array(),
        radial_direction: radial.to_array(),
        angular_radius: 0.0,
        maximum_slope_radians: 0.0,
        support_height_span: 0.0,
    }
}

fn slope_radians(normal: [f64; 3], radial: DVec3) -> f64 {
    DVec3::from_array(normal)
        .normalize_or_zero()
        .dot(radial)
        .clamp(-1.0, 1.0)
        .acos()
}

/// Serializable geodesic construction footprint used for reservation and
/// dynamic navigation blocking. Angular caps avoid cube-face seam special
/// cases and remain stable across floating-origin shifts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanetConstructionFootprint {
    pub direction: [f64; 3],
    pub angular_radius: f64,
    pub blocks_navigation: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlanetSurfaceVolumeKey {
    #[default]
    Legacy,
    Persistent(String),
    Runtime {
        index: u32,
        generation: u32,
    },
}

impl PlanetSurfaceVolumeKey {
    pub fn from_persistent_id(id: impl Into<String>) -> Self {
        let id = id.into();
        if id.is_empty() {
            Self::Legacy
        } else {
            Self::Persistent(id)
        }
    }

    pub const fn from_runtime_entity(index: u32, generation: u32) -> Self {
        Self::Runtime { index, generation }
    }

    pub fn persistent_id(&self) -> Option<&str> {
        match self {
            Self::Persistent(id) => Some(id),
            Self::Legacy | Self::Runtime { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlanetSurfaceOwnerKey {
    Persistent(String),
    Runtime { index: u32, generation: u32 },
}

impl PlanetSurfaceOwnerKey {
    pub fn persistent(id: impl Into<String>) -> Self {
        Self::Persistent(id.into())
    }

    pub const fn from_runtime_entity(index: u32, generation: u32) -> Self {
        Self::Runtime { index, generation }
    }

    pub fn display_id(&self) -> String {
        match self {
            Self::Persistent(id) => id.clone(),
            Self::Runtime { index, generation } => format!("runtime:{index}:{generation}"),
        }
    }

    /// Injective string form for APIs that require flat string identities.
    /// The explicit domain and persistent-ID byte length prevent an authored
    /// ID from impersonating an anonymous runtime entity.
    pub fn stable_key(&self) -> String {
        match self {
            Self::Persistent(id) => format!("persistent:{}:{id}", id.len()),
            Self::Runtime { index, generation } => {
                format!("runtime-entity:{index}:{generation}")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct PlanetSurfaceReservationKey {
    volume: PlanetSurfaceVolumeKey,
    owner: PlanetSurfaceOwnerKey,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanetSurfaceReservation<'a> {
    pub volume: &'a PlanetSurfaceVolumeKey,
    pub owner: &'a PlanetSurfaceOwnerKey,
    pub footprint: &'a PlanetConstructionFootprint,
}

/// Deterministic construction occupancy map. Games own costs and build rules;
/// the engine owns overlap, reservation identity and persistence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanetSurfaceOccupancy {
    reservations: BTreeMap<PlanetSurfaceReservationKey, PlanetConstructionFootprint>,
}

const OCCUPANCY_MAGIC: [u8; 8] = *b"PLNOCC01";
const OCCUPANCY_VERSION: u8 = 1;

#[derive(Serialize, Deserialize)]
struct PlanetSurfaceOccupancyEnvelope {
    magic: [u8; 8],
    version: u8,
    reservations: Vec<PlanetSurfaceReservationEntry>,
}

#[derive(Serialize, Deserialize)]
struct PlanetSurfaceReservationEntry {
    volume: PlanetSurfaceVolumeKey,
    owner: PlanetSurfaceOwnerKey,
    footprint: PlanetConstructionFootprint,
}

#[derive(Serialize)]
struct PlanetSurfaceOccupancyEnvelopeRef<'a> {
    magic: [u8; 8],
    version: u8,
    reservations: Vec<PlanetSurfaceReservationEntryRef<'a>>,
}

#[derive(Serialize)]
struct PlanetSurfaceReservationEntryRef<'a> {
    volume: &'a PlanetSurfaceVolumeKey,
    owner: &'a PlanetSurfaceOwnerKey,
    footprint: &'a PlanetConstructionFootprint,
}

#[derive(Serialize, Deserialize)]
struct LegacyPlanetSurfaceOccupancy {
    reservations: BTreeMap<String, PlanetConstructionFootprint>,
}

impl Serialize for PlanetSurfaceOccupancy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        PlanetSurfaceOccupancyEnvelopeRef {
            magic: OCCUPANCY_MAGIC,
            version: OCCUPANCY_VERSION,
            reservations: self
                .reservations
                .iter()
                .map(|(key, footprint)| PlanetSurfaceReservationEntryRef {
                    volume: &key.volume,
                    owner: &key.owner,
                    footprint,
                })
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PlanetSurfaceOccupancy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            #[derive(Deserialize)]
            #[serde(untagged)]
            enum HumanReadableOccupancy {
                Current(PlanetSurfaceOccupancyEnvelope),
                Legacy(LegacyPlanetSurfaceOccupancy),
            }

            return match HumanReadableOccupancy::deserialize(deserializer)? {
                HumanReadableOccupancy::Current(envelope) => {
                    Self::from_envelope(envelope).map_err(serde::de::Error::custom)
                }
                HumanReadableOccupancy::Legacy(legacy) => Ok(Self::from_legacy(legacy)),
            };
        }
        let envelope = PlanetSurfaceOccupancyEnvelope::deserialize(deserializer)?;
        Self::from_envelope(envelope).map_err(serde::de::Error::custom)
    }
}

impl PlanetSurfaceOccupancy {
    fn from_envelope(envelope: PlanetSurfaceOccupancyEnvelope) -> Result<Self, &'static str> {
        if envelope.magic != OCCUPANCY_MAGIC || envelope.version != OCCUPANCY_VERSION {
            return Err("unsupported planet surface occupancy envelope");
        }
        let mut reservations = BTreeMap::new();
        for entry in envelope.reservations {
            let key = PlanetSurfaceReservationKey {
                volume: entry.volume,
                owner: entry.owner,
            };
            if reservations.insert(key, entry.footprint).is_some() {
                return Err("duplicate planet surface reservation identity");
            }
        }
        Ok(Self { reservations })
    }

    fn from_legacy(legacy: LegacyPlanetSurfaceOccupancy) -> Self {
        Self {
            reservations: legacy
                .reservations
                .into_iter()
                .map(|(entity_id, footprint)| {
                    (
                        PlanetSurfaceReservationKey {
                            volume: PlanetSurfaceVolumeKey::Legacy,
                            owner: PlanetSurfaceOwnerKey::Persistent(entity_id),
                        },
                        footprint,
                    )
                })
                .collect(),
        }
    }

    /// Serialize the versioned occupancy envelope.
    pub fn to_bincode(&self) -> Result<Vec<u8>, Box<bincode::ErrorKind>> {
        strict_bincode_options().serialize(self)
    }

    /// Decode either the versioned envelope or the original single-planet
    /// bincode struct. Generic `bincode::deserialize` only accepts the current
    /// envelope; persisted engine data should use this compatibility entry
    /// point.
    pub fn from_bincode_compatible(bytes: &[u8]) -> Result<Self, Box<bincode::ErrorKind>> {
        let current_format = magic_prefix_matches(bytes, &OCCUPANCY_MAGIC);
        if current_format {
            return strict_bincode_options().deserialize(bytes);
        }
        let legacy = strict_bincode_options().deserialize(bytes)?;
        Ok(Self::from_legacy(legacy))
    }

    pub fn reservations(&self) -> impl ExactSizeIterator<Item = PlanetSurfaceReservation<'_>> + '_ {
        self.reservations
            .iter()
            .map(|(key, footprint)| PlanetSurfaceReservation {
                volume: &key.volume,
                owner: &key.owner,
                footprint,
            })
    }

    pub fn reserve(
        &mut self,
        id: impl Into<String>,
        placement: PlanetSurfacePlacement,
        blocks_navigation: bool,
        padding_radians: f64,
    ) -> Result<(), PlanetPlacementError> {
        self.reserve_scoped(
            PlanetSurfaceVolumeKey::Legacy,
            PlanetSurfaceOwnerKey::Persistent(id.into()),
            placement,
            blocks_navigation,
            padding_radians,
        )
    }

    /// Reserve a footprint in one planet's independent angular namespace.
    pub fn reserve_for_volume(
        &mut self,
        terrain_volume_id: impl Into<String>,
        id: impl Into<String>,
        placement: PlanetSurfacePlacement,
        blocks_navigation: bool,
        padding_radians: f64,
    ) -> Result<(), PlanetPlacementError> {
        self.reserve_scoped(
            PlanetSurfaceVolumeKey::from_persistent_id(terrain_volume_id),
            PlanetSurfaceOwnerKey::Persistent(id.into()),
            placement,
            blocks_navigation,
            padding_radians,
        )
    }

    pub fn reserve_scoped(
        &mut self,
        volume: PlanetSurfaceVolumeKey,
        owner: PlanetSurfaceOwnerKey,
        placement: PlanetSurfacePlacement,
        blocks_navigation: bool,
        padding_radians: f64,
    ) -> Result<(), PlanetPlacementError> {
        let direction = DVec3::from_array(placement.radial_direction).normalize();
        if matches!(&owner, PlanetSurfaceOwnerKey::Persistent(id) if id.is_empty())
            || !direction.is_finite()
            || !padding_radians.is_finite()
            || padding_radians < 0.0
        {
            return Err(PlanetPlacementError::InvalidParameters);
        }
        let angular_radius = placement.angular_radius + padding_radians;
        let key = PlanetSurfaceReservationKey { volume, owner };
        for (existing_key, footprint) in &self.reservations {
            if existing_key.volume != key.volume {
                continue;
            }
            if existing_key.owner == key.owner {
                continue;
            }
            let existing = DVec3::from_array(footprint.direction).normalize_or_zero();
            let separation = direction.dot(existing).clamp(-1.0, 1.0).acos();
            if separation + 1.0e-12 < angular_radius + footprint.angular_radius {
                return Err(PlanetPlacementError::Occupied {
                    existing_id: existing_key.owner.display_id(),
                });
            }
        }
        self.reservations.insert(
            key,
            PlanetConstructionFootprint {
                direction: direction.to_array(),
                angular_radius,
                blocks_navigation,
            },
        );
        Ok(())
    }

    pub fn release(&mut self, id: &str) -> bool {
        self.release_scoped(
            &PlanetSurfaceVolumeKey::Legacy,
            &PlanetSurfaceOwnerKey::Persistent(id.to_string()),
        )
    }

    pub fn release_for_volume(&mut self, terrain_volume_id: &str, id: &str) -> bool {
        self.release_scoped(
            &PlanetSurfaceVolumeKey::from_persistent_id(terrain_volume_id),
            &PlanetSurfaceOwnerKey::Persistent(id.to_string()),
        )
    }

    pub fn release_scoped(
        &mut self,
        volume: &PlanetSurfaceVolumeKey,
        owner: &PlanetSurfaceOwnerKey,
    ) -> bool {
        self.reservations
            .remove(&PlanetSurfaceReservationKey {
                volume: volume.clone(),
                owner: owner.clone(),
            })
            .is_some()
    }

    pub fn navigation_blockers(&self) -> impl Iterator<Item = &PlanetConstructionFootprint> + '_ {
        self.reservations
            .values()
            .filter(|footprint| footprint.blocks_navigation)
    }

    pub fn reservations_for_volume<'a>(
        &'a self,
        terrain_volume_id: &str,
    ) -> impl Iterator<Item = (String, &'a PlanetConstructionFootprint)> + 'a {
        let volume = PlanetSurfaceVolumeKey::from_persistent_id(terrain_volume_id);
        self.reservations
            .iter()
            .filter_map(move |(key, footprint)| {
                (key.volume == volume).then_some((key.owner.display_id(), footprint))
            })
    }

    pub fn reservations_for_scope<'a>(
        &'a self,
        volume: &'a PlanetSurfaceVolumeKey,
    ) -> impl Iterator<Item = (&'a PlanetSurfaceOwnerKey, &'a PlanetConstructionFootprint)> + 'a
    {
        self.reservations
            .iter()
            .filter_map(move |(key, footprint)| {
                (&key.volume == volume).then_some((&key.owner, footprint))
            })
    }

    pub fn navigation_blockers_for_volume<'a>(
        &'a self,
        terrain_volume_id: &str,
    ) -> impl Iterator<Item = (String, &'a PlanetConstructionFootprint)> + 'a {
        self.reservations_for_volume(terrain_volume_id)
            .filter(|(_, footprint)| footprint.blocks_navigation)
    }

    pub fn navigation_blockers_for_scope<'a>(
        &'a self,
        volume: &'a PlanetSurfaceVolumeKey,
    ) -> impl Iterator<Item = (&'a PlanetSurfaceOwnerKey, &'a PlanetConstructionFootprint)> + 'a
    {
        self.reservations_for_scope(volume)
            .filter(|(_, footprint)| footprint.blocks_navigation)
    }

    pub fn contains_direction(&self, direction: [f64; 3]) -> bool {
        self.contains_direction_for_volume("", direction)
    }

    pub fn contains_direction_for_volume(
        &self,
        terrain_volume_id: &str,
        direction: [f64; 3],
    ) -> bool {
        self.contains_direction_for_scope(
            &PlanetSurfaceVolumeKey::from_persistent_id(terrain_volume_id),
            direction,
        )
    }

    pub fn contains_direction_for_scope(
        &self,
        volume: &PlanetSurfaceVolumeKey,
        direction: [f64; 3],
    ) -> bool {
        let direction = DVec3::from_array(direction).normalize_or_zero();
        direction != DVec3::ZERO
            && self.reservations.iter().any(|(key, footprint)| {
                if &key.volume != volume {
                    return false;
                }
                let reserved = DVec3::from_array(footprint.direction).normalize_or_zero();
                direction.dot(reserved).clamp(-1.0, 1.0).acos() <= footprint.angular_radius
            })
    }
}

fn magic_prefix_matches(bytes: &[u8], magic: &[u8]) -> bool {
    !bytes.is_empty()
        && (bytes.starts_with(magic) || (bytes.len() < magic.len() && magic.starts_with(bytes)))
}

fn strict_bincode_options() -> impl Options + Copy {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
}

pub(crate) fn register_planet_surface_anchor(registry: &mut ComponentRegistry) {
    let registered = registry
        .register(ComponentExtension {
            meta: ComponentMeta {
                type_id: PlanetSurfaceAnchor::TYPE_ID,
                display_name: "Planet Surface Anchor",
                schema_version: (0, 1, 0),
                has_editor: true,
                script_access: ScriptAccess::ReadWrite,
            },
            storage_factory: || -> Box<dyn ComponentStorageDyn> {
                Box::new(SparseSet::<PlanetSurfaceAnchor>::new())
            },
            serialize: Some(serialize_anchor),
            deserialize: Some(deserialize_anchor),
        })
        .is_ok();
    if registered {
        let _ = registry
            .register_fields_validator(PlanetSurfaceAnchor::TYPE_ID, validate_anchor_fields);
    }
}

fn serialize_anchor(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let anchor = component
        .downcast_ref::<PlanetSurfaceAnchor>()
        .expect("PlanetSurfaceAnchor expected");
    serialize_planet_surface_anchor_fields(anchor)
}

/// Serialize an authored surface anchor through the canonical scene schema.
///
/// Editor and host tooling use this function instead of duplicating the
/// component registry's field layout.
pub fn serialize_planet_surface_anchor_fields(
    anchor: &PlanetSurfaceAnchor,
) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("enabled".into(), Value::Bool(anchor.enabled)),
        (
            "terrain_volume_id".into(),
            Value::Str(anchor.terrain_volume_id.clone()),
        ),
        (
            "direction".into(),
            Value::List(anchor.direction.into_iter().map(Value::Float64).collect()),
        ),
        (
            "heading_radians".into(),
            Value::Float64(anchor.heading_radians),
        ),
        (
            "altitude_offset".into(),
            Value::Float64(anchor.altitude_offset),
        ),
        (
            "footprint_radius".into(),
            Value::Float64(anchor.footprint_radius),
        ),
        (
            "max_slope_radians".into(),
            Value::Float64(anchor.max_slope_radians),
        ),
        (
            "max_height_delta".into(),
            Value::Float64(anchor.max_height_delta),
        ),
        (
            "support_samples".into(),
            Value::UInt(u64::from(anchor.support_samples)),
        ),
        (
            "blocks_navigation".into(),
            Value::Bool(anchor.blocks_navigation),
        ),
    ])
}

fn deserialize_anchor(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let mut anchor = PlanetSurfaceAnchor::default();
    if let Some(Value::Bool(value)) = fields.get("enabled") {
        anchor.enabled = *value;
    }
    if let Some(Value::Str(value)) = fields.get("terrain_volume_id") {
        anchor.terrain_volume_id = value.clone();
    }
    if let Some(Value::List(values)) = fields.get("direction") {
        if let [Value::Float64(x), Value::Float64(y), Value::Float64(z)] = values.as_slice() {
            anchor.direction = [*x, *y, *z];
        }
    }
    for (name, target) in [
        ("heading_radians", &mut anchor.heading_radians),
        ("altitude_offset", &mut anchor.altitude_offset),
        ("footprint_radius", &mut anchor.footprint_radius),
        ("max_slope_radians", &mut anchor.max_slope_radians),
        ("max_height_delta", &mut anchor.max_height_delta),
    ] {
        if let Some(Value::Float64(value)) = fields.get(name) {
            *target = *value;
        }
    }
    if let Some(Value::UInt(value)) = fields.get("support_samples") {
        anchor.support_samples = u16::try_from(*value).unwrap_or(u16::MAX);
    }
    if let Some(Value::Bool(value)) = fields.get("blocks_navigation") {
        anchor.blocks_navigation = *value;
    }
    Box::new(anchor)
}

fn validate_anchor_fields(fields: &BTreeMap<String, Value>) -> Result<(), String> {
    let anchor = deserialize_anchor(fields)
        .downcast::<PlanetSurfaceAnchor>()
        .map_err(|_| "surface anchor deserializer returned an incompatible value".to_string())?;
    let normalized = serialize_anchor(anchor.as_ref());
    let rejected = fields
        .iter()
        .filter_map(|(name, value)| (normalized.get(name) != Some(value)).then_some(name.clone()))
        .collect::<Vec<_>>();
    if !rejected.is_empty() {
        return Err(format!(
            "unknown or incorrectly typed fields: {}",
            rejected.join(", ")
        ));
    }
    anchor.validate(1.0e12).map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "placement/tests.rs"]
mod tests;
