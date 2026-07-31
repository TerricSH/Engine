//! Native planetary math queries for managed and embedded script hosts.

use std::ffi::c_void;

use engine_terrain::{
    PlanetCoordinates, PlanetPlacementError, PlanetSurfaceAnchor, PlanetTangentFrame,
    PlanetTerrainQuery, TerrainTopology, TerrainVolume,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiPlanetTerrainConfig {
    pub center_x: f64,
    pub center_y: f64,
    pub center_z: f64,
    pub radius: f64,
    pub height_scale: f32,
    pub seed: u64,
    pub octaves: u32,
    pub frequency: f32,
    pub lacunarity: f32,
    pub gain: f32,
    pub domain_warp_amplitude: f32,
    pub domain_warp_frequency: f32,
}

impl Default for FfiPlanetTerrainConfig {
    fn default() -> Self {
        let volume = TerrainVolume::default();
        Self {
            center_x: 0.0,
            center_y: 0.0,
            center_z: 0.0,
            radius: volume.planet_radius,
            height_scale: volume.height_scale,
            seed: volume.seed,
            octaves: volume.octaves,
            frequency: volume.frequency,
            lacunarity: volume.lacunarity,
            gain: volume.gain,
            domain_warp_amplitude: volume.domain_warp_amplitude,
            domain_warp_frequency: volume.domain_warp_frequency,
        }
    }
}

impl FfiPlanetTerrainConfig {
    fn into_volume(self) -> TerrainVolume {
        TerrainVolume {
            topology: TerrainTopology::CubeSphere,
            planet_center: [self.center_x, self.center_y, self.center_z],
            planet_radius: self.radius,
            height_scale: self.height_scale,
            seed: self.seed,
            octaves: self.octaves,
            frequency: self.frequency,
            lacunarity: self.lacunarity,
            gain: self.gain,
            domain_warp_amplitude: self.domain_warp_amplitude,
            domain_warp_frequency: self.domain_warp_frequency,
            ..TerrainVolume::default()
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FfiVector3d {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl From<[f64; 3]> for FfiVector3d {
    fn from(value: [f64; 3]) -> Self {
        Self {
            x: value[0],
            y: value[1],
            z: value[2],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FfiPlanetCoordinates {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FfiPlanetTangentFrame {
    pub surface_point: FfiVector3d,
    pub normal: FfiVector3d,
    pub east: FfiVector3d,
    pub north: FfiVector3d,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiPlanetSurfaceAnchor {
    pub direction: FfiVector3d,
    pub heading_radians: f64,
    pub altitude_offset: f64,
    pub footprint_radius: f64,
    pub max_slope_radians: f64,
    pub max_height_delta: f64,
    pub support_samples: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FfiPlanetSurfacePlacement {
    pub position: FfiVector3d,
    pub normal: FfiVector3d,
    pub right: FfiVector3d,
    pub forward: FfiVector3d,
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub rotation_z: f64,
    pub rotation_w: f64,
    pub radial_direction: FfiVector3d,
    pub angular_radius: f64,
    pub maximum_slope_radians: f64,
    pub support_height_span: f64,
}

#[no_mangle]
/// Creates a native planetary query from an ABI-stable configuration.
///
/// # Safety
///
/// `config` must be null or point to a readable `FfiPlanetTerrainConfig` for
/// the duration of this call.
pub unsafe extern "C" fn planet_query_create(config: *const FfiPlanetTerrainConfig) -> *mut c_void {
    if config.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: The caller supplies a readable config for the duration of this call.
    let config = unsafe { *config };
    PlanetTerrainQuery::new(&config.into_volume())
        .map(|query| Box::into_raw(Box::new(query)).cast())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
/// Releases a native planetary query.
///
/// # Safety
///
/// `query` must be null or a live handle returned by `planet_query_create`.
/// A non-null handle may be destroyed exactly once and must not be in use.
pub unsafe extern "C" fn planet_query_destroy(query: *mut c_void) {
    if query.is_null() {
        return;
    }
    // SAFETY: Handles returned by `planet_query_create` own exactly one Box.
    drop(unsafe { Box::from_raw(query.cast::<PlanetTerrainQuery>()) });
}

#[no_mangle]
/// Samples procedural terrain height along a planet-relative direction.
///
/// # Safety
///
/// `query` must be a live handle returned by `planet_query_create`.
pub unsafe extern "C" fn planet_query_height(
    query: *const c_void,
    direction_x: f64,
    direction_y: f64,
    direction_z: f64,
) -> f64 {
    query_ref(query).map_or(f64::NAN, |query| {
        query.height_along_direction([direction_x, direction_y, direction_z])
    })
}

#[no_mangle]
/// Projects a direction onto the generated planetary surface.
///
/// # Safety
///
/// `query` must be a live query handle. `output` must be null or point to
/// writable storage for one `FfiVector3d`.
pub unsafe extern "C" fn planet_query_surface_point(
    query: *const c_void,
    direction_x: f64,
    direction_y: f64,
    direction_z: f64,
    output: *mut FfiVector3d,
) -> bool {
    let Some(query) = query_ref(query) else {
        return false;
    };
    if output.is_null() {
        return false;
    }
    // SAFETY: `output` was null-checked and the caller guarantees writable storage.
    unsafe {
        *output = query
            .surface_point_from_direction([direction_x, direction_y, direction_z])
            .into();
    }
    true
}

#[no_mangle]
/// Returns signed terrain-relative altitude at a world-space position.
///
/// # Safety
///
/// `query` must be a live handle returned by `planet_query_create`.
pub unsafe extern "C" fn planet_query_altitude(
    query: *const c_void,
    world_x: f64,
    world_y: f64,
    world_z: f64,
) -> f64 {
    query_ref(query).map_or(f64::NAN, |query| {
        query.altitude([world_x, world_y, world_z])
    })
}

#[no_mangle]
/// Converts a world-space position to latitude, longitude, and altitude.
///
/// # Safety
///
/// `query` must be a live query handle. `output` must be null or point to
/// writable storage for one `FfiPlanetCoordinates`.
pub unsafe extern "C" fn planet_query_coordinates(
    query: *const c_void,
    world_x: f64,
    world_y: f64,
    world_z: f64,
    output: *mut FfiPlanetCoordinates,
) -> bool {
    let Some(query) = query_ref(query) else {
        return false;
    };
    if output.is_null() {
        return false;
    }
    let coordinates = query.coordinates([world_x, world_y, world_z]);
    // SAFETY: `output` was null-checked and the caller guarantees writable storage.
    unsafe {
        *output = FfiPlanetCoordinates {
            latitude: coordinates.latitude,
            longitude: coordinates.longitude,
            altitude: coordinates.altitude,
        };
    }
    true
}

#[no_mangle]
/// Converts latitude, longitude, and altitude to a world-space position.
///
/// # Safety
///
/// `query` must be a live query handle. `output` must be null or point to
/// writable storage for one `FfiVector3d`.
pub unsafe extern "C" fn planet_query_world_from_coordinates(
    query: *const c_void,
    coordinates: FfiPlanetCoordinates,
    output: *mut FfiVector3d,
) -> bool {
    let Some(query) = query_ref(query) else {
        return false;
    };
    if output.is_null() {
        return false;
    }
    // SAFETY: `output` was null-checked and the caller guarantees writable storage.
    unsafe {
        *output = query
            .world_from_coordinates(PlanetCoordinates {
                latitude: coordinates.latitude,
                longitude: coordinates.longitude,
                altitude: coordinates.altitude,
            })
            .into();
    }
    true
}

#[no_mangle]
/// Resolves a terrain-aware tangent frame for a surface direction.
///
/// # Safety
///
/// `query` must be a live query handle. `output` must be null or point to
/// writable storage for one `FfiPlanetTangentFrame`.
pub unsafe extern "C" fn planet_query_tangent_frame(
    query: *const c_void,
    direction_x: f64,
    direction_y: f64,
    direction_z: f64,
    output: *mut FfiPlanetTangentFrame,
) -> bool {
    let Some(query) = query_ref(query) else {
        return false;
    };
    if output.is_null() {
        return false;
    }
    let PlanetTangentFrame {
        surface_point,
        normal,
        east,
        north,
    } = query.tangent_frame([direction_x, direction_y, direction_z]);
    // SAFETY: `output` was null-checked and the caller guarantees writable storage.
    unsafe {
        *output = FfiPlanetTangentFrame {
            surface_point: surface_point.into(),
            normal: normal.into(),
            east: east.into(),
            north: north.into(),
        };
    }
    true
}

/// Resolves a slope-checked construction transform and geodesic footprint.
///
/// Returns zero on success, `-1` for an invalid handle/output pointer, `-2`
/// for invalid settings, `-3` for excessive slope, and `-4` for an unsupported
/// height span.
///
/// # Safety
///
/// `query` must be a live query handle. `output` must be null or point to
/// writable storage for one `FfiPlanetSurfacePlacement`.
#[no_mangle]
pub unsafe extern "C" fn planet_query_resolve_surface_placement(
    query: *const c_void,
    anchor: FfiPlanetSurfaceAnchor,
    output: *mut FfiPlanetSurfacePlacement,
) -> i32 {
    let Some(query) = query_ref(query) else {
        return -1;
    };
    if output.is_null() {
        return -1;
    }
    let Ok(support_samples) = u16::try_from(anchor.support_samples) else {
        return -2;
    };
    let anchor = PlanetSurfaceAnchor {
        direction: [anchor.direction.x, anchor.direction.y, anchor.direction.z],
        heading_radians: anchor.heading_radians,
        altitude_offset: anchor.altitude_offset,
        footprint_radius: anchor.footprint_radius,
        max_slope_radians: anchor.max_slope_radians,
        max_height_delta: anchor.max_height_delta,
        support_samples,
        ..PlanetSurfaceAnchor::default()
    };
    let placement = match anchor.resolve(query) {
        Ok(placement) => placement,
        Err(PlanetPlacementError::InvalidDirection | PlanetPlacementError::InvalidParameters) => {
            return -2;
        }
        Err(PlanetPlacementError::SlopeExceeded { .. }) => return -3,
        Err(PlanetPlacementError::HeightSpanExceeded { .. }) => return -4,
        Err(PlanetPlacementError::Occupied { .. } | PlanetPlacementError::OutsideLocalRange) => {
            return -2;
        }
    };
    // SAFETY: `output` was null-checked and the caller guarantees writable storage.
    unsafe {
        *output = FfiPlanetSurfacePlacement {
            position: placement.position.into(),
            normal: placement.normal.into(),
            right: placement.right.into(),
            forward: placement.forward.into(),
            rotation_x: placement.rotation[0],
            rotation_y: placement.rotation[1],
            rotation_z: placement.rotation[2],
            rotation_w: placement.rotation[3],
            radial_direction: placement.radial_direction.into(),
            angular_radius: placement.angular_radius,
            maximum_slope_radians: placement.maximum_slope_radians,
            support_height_span: placement.support_height_span,
        };
    }
    0
}

#[no_mangle]
/// Measures great-circle arc distance between two world-space positions.
///
/// # Safety
///
/// `query` must be a live handle returned by `planet_query_create`.
pub unsafe extern "C" fn planet_query_great_circle_distance(
    query: *const c_void,
    from_x: f64,
    from_y: f64,
    from_z: f64,
    to_x: f64,
    to_y: f64,
    to_z: f64,
) -> f64 {
    query_ref(query).map_or(f64::NAN, |query| {
        query.great_circle_distance([from_x, from_y, from_z], [to_x, to_y, to_z])
    })
}

pub(crate) unsafe fn query_ref<'a>(query: *const c_void) -> Option<&'a PlanetTerrainQuery> {
    if query.is_null() {
        return None;
    }
    // SAFETY: Public callers must pass a live handle created by this module.
    Some(unsafe { &*query.cast::<PlanetTerrainQuery>() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_query_uses_native_planet_math_and_round_trips() {
        let config = FfiPlanetTerrainConfig {
            radius: 1_000.0,
            height_scale: 25.0,
            ..Default::default()
        };
        let query = unsafe { planet_query_create(&config) };
        assert!(!query.is_null());
        let mut surface = FfiVector3d::default();
        assert!(unsafe { planet_query_surface_point(query, 0.0, 1.0, 0.0, &mut surface) });
        assert!(surface.y > 975.0 && surface.y < 1_025.0);
        let altitude = unsafe { planet_query_altitude(query, surface.x, surface.y, surface.z) };
        assert!(altitude.abs() < 1.0e-8);
        let mut placement = FfiPlanetSurfacePlacement::default();
        let placement_status = unsafe {
            planet_query_resolve_surface_placement(
                query,
                FfiPlanetSurfaceAnchor {
                    direction: FfiVector3d {
                        x: 0.0,
                        y: 1.0,
                        z: 0.0,
                    },
                    heading_radians: 0.3,
                    altitude_offset: 2.0,
                    footprint_radius: 0.0,
                    max_slope_radians: std::f64::consts::FRAC_PI_2,
                    max_height_delta: 100.0,
                    support_samples: 12,
                },
                &mut placement,
            )
        };
        assert_eq!(placement_status, 0);
        assert!(placement.position.y > surface.y);
        assert!((placement.angular_radius - 0.0).abs() < f64::EPSILON);
        unsafe { planet_query_destroy(query) };
    }
}
