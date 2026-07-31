//! Opaque native handles for three-dimensional and spherical path queries.

use std::ffi::{c_char, c_void, CStr};

use engine_nav::{
    SpaceCell, SpaceNavGrid, SpacePath, SphericalNavBuildConfig, SphericalNavGraph,
    SphericalNavObstacle, SphericalPath, SphericalSurfaceSample, SphericalTraversalArea,
};
use glam::{DVec3, Vec3};

#[no_mangle]
pub extern "C" fn space_nav_create(
    origin_x: f32,
    origin_y: f32,
    origin_z: f32,
    cells_x: u32,
    cells_y: u32,
    cells_z: u32,
    cell_size: f32,
) -> *mut c_void {
    SpaceNavGrid::new(
        Vec3::new(origin_x, origin_y, origin_z),
        [cells_x, cells_y, cells_z],
        cell_size,
    )
    .map(|grid| Box::into_raw(Box::new(grid)).cast())
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
/// Releases a sparse three-dimensional navigation grid.
///
/// # Safety
///
/// `grid` must be null or a live handle returned by `space_nav_create`. A
/// non-null handle may be destroyed exactly once and must not be in use.
pub unsafe extern "C" fn space_nav_destroy(grid: *mut c_void) {
    if !grid.is_null() {
        // SAFETY: Handles returned by `space_nav_create` own exactly one Box.
        drop(unsafe { Box::from_raw(grid.cast::<SpaceNavGrid>()) });
    }
}

#[no_mangle]
/// Changes the occupancy of one grid cell.
///
/// # Safety
///
/// `grid` must be a live, uniquely borrowed handle returned by
/// `space_nav_create`.
pub unsafe extern "C" fn space_nav_set_blocked(
    grid: *mut c_void,
    x: i32,
    y: i32,
    z: i32,
    blocked: bool,
) -> bool {
    grid_mut(grid).is_some_and(|grid| grid.set_blocked(SpaceCell::new(x, y, z), blocked))
}

#[no_mangle]
/// Finds a path through a sparse three-dimensional navigation grid.
///
/// # Safety
///
/// `grid` must be a live handle returned by `space_nav_create` and remain
/// readable for the duration of this call.
pub unsafe extern "C" fn space_nav_find_path(
    grid: *const c_void,
    from_x: f32,
    from_y: f32,
    from_z: f32,
    to_x: f32,
    to_y: f32,
    to_z: f32,
) -> *mut c_void {
    let Some(grid) = grid_ref(grid) else {
        return std::ptr::null_mut();
    };
    grid.find_path(
        Vec3::new(from_x, from_y, from_z),
        Vec3::new(to_x, to_y, to_z),
    )
    .map(|path| Box::into_raw(Box::new(path)).cast())
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
/// Releases a three-dimensional navigation path.
///
/// # Safety
///
/// `path` must be null or a live handle returned by `space_nav_find_path`. A
/// non-null handle may be destroyed exactly once and must not be in use.
pub unsafe extern "C" fn space_path_destroy(path: *mut c_void) {
    if !path.is_null() {
        // SAFETY: Handles returned by `space_nav_find_path` own exactly one Box.
        drop(unsafe { Box::from_raw(path.cast::<SpacePath>()) });
    }
}

#[no_mangle]
/// Returns the number of waypoints in a three-dimensional navigation path.
///
/// # Safety
///
/// `path` must be a live handle returned by `space_nav_find_path`.
pub unsafe extern "C" fn space_path_count(path: *const c_void) -> u32 {
    path_ref(path)
        .and_then(|path| u32::try_from(path.waypoints().len()).ok())
        .unwrap_or(0)
}

#[no_mangle]
/// Returns the length of a three-dimensional navigation path.
///
/// # Safety
///
/// `path` must be a live handle returned by `space_nav_find_path`.
pub unsafe extern "C" fn space_path_length(path: *const c_void) -> f32 {
    path_ref(path).map_or(f32::NAN, SpacePath::length)
}

#[no_mangle]
/// Copies one three-dimensional path waypoint into caller-owned outputs.
///
/// # Safety
///
/// `path` must be a live path handle. Each output must be null or point to
/// writable storage for one `f32`; all three must be valid for success.
pub unsafe extern "C" fn space_path_point(
    path: *const c_void,
    index: u32,
    out_x: *mut f32,
    out_y: *mut f32,
    out_z: *mut f32,
) -> bool {
    let Some(point) = path_ref(path).and_then(|path| path.waypoints().get(index as usize)) else {
        return false;
    };
    write_vec3(*point, out_x, out_y, out_z)
}

#[no_mangle]
pub extern "C" fn spherical_nav_create(
    center_x: f64,
    center_y: f64,
    center_z: f64,
    radius: f64,
    node_count: u32,
    neighbors_per_node: u32,
) -> *mut c_void {
    SphericalNavGraph::fibonacci(
        DVec3::new(center_x, center_y, center_z),
        radius,
        SphericalNavBuildConfig {
            node_count: node_count as usize,
            neighbors_per_node: neighbors_per_node as usize,
            ..Default::default()
        },
    )
    .map(|graph| Box::into_raw(Box::new(graph)).cast())
    .unwrap_or(std::ptr::null_mut())
}

/// Build a spherical graph projected onto the exact native planet terrain
/// query, so managed callers do not have to reproduce height mathematics.
///
/// # Safety
///
/// `query` must be a live handle returned by `planet_query_create` and remain
/// readable for the duration of graph construction.
#[no_mangle]
pub unsafe extern "C" fn spherical_nav_create_for_planet(
    query: *const c_void,
    node_count: u32,
    neighbors_per_node: u32,
) -> *mut c_void {
    // SAFETY: The caller guarantees that `query` is a live handle created by
    // `planet_query_create` for the duration of this call.
    let Some(query) = (unsafe { crate::planet_query::query_ref(query) }) else {
        return std::ptr::null_mut();
    };
    let center64 = query.center();
    let center = DVec3::from_array(center64);
    let radius = query.radius();
    if !center.is_finite() || !radius.is_finite() {
        return std::ptr::null_mut();
    }
    SphericalNavGraph::fibonacci_sampled(
        center,
        radius,
        SphericalNavBuildConfig {
            node_count: node_count as usize,
            neighbors_per_node: neighbors_per_node as usize,
            ..Default::default()
        },
        |direction| {
            let point = query.surface_point_from_direction([direction.x, direction.y, direction.z]);
            let position = DVec3::from_array(point);
            position.is_finite().then_some(SphericalSurfaceSample {
                position,
                traversal_cost: 1.0,
            })
        },
    )
    .map(|graph| Box::into_raw(Box::new(graph)).cast())
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
/// Releases a spherical navigation graph.
///
/// # Safety
///
/// `graph` must be null or a live handle returned by a spherical navigation
/// creation function. A non-null handle may be destroyed exactly once.
pub unsafe extern "C" fn spherical_nav_destroy(graph: *mut c_void) {
    if !graph.is_null() {
        // SAFETY: Handles returned by `spherical_nav_create` own exactly one Box.
        drop(unsafe { Box::from_raw(graph.cast::<SphericalNavGraph>()) });
    }
}

#[no_mangle]
/// Finds a great-circle path across a spherical navigation graph.
///
/// # Safety
///
/// `graph` must be a live spherical graph handle and remain readable for the
/// duration of this call.
pub unsafe extern "C" fn spherical_nav_find_path(
    graph: *const c_void,
    from_x: f64,
    from_y: f64,
    from_z: f64,
    to_x: f64,
    to_y: f64,
    to_z: f64,
) -> *mut c_void {
    let Some(graph) = spherical_graph_ref(graph) else {
        return std::ptr::null_mut();
    };
    graph
        .find_path(
            DVec3::new(from_x, from_y, from_z),
            DVec3::new(to_x, to_y, to_z),
        )
        .map(|path| Box::into_raw(Box::new(path)).cast())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
/// Releases a spherical navigation path.
///
/// # Safety
///
/// `path` must be null or a live handle returned by
/// `spherical_nav_find_path`. A non-null handle may be destroyed exactly once.
pub unsafe extern "C" fn spherical_path_destroy(path: *mut c_void) {
    if !path.is_null() {
        // SAFETY: Handles returned by `spherical_nav_find_path` own one Box.
        drop(unsafe { Box::from_raw(path.cast::<SphericalPath>()) });
    }
}

#[no_mangle]
/// Returns the waypoint count of a spherical navigation path.
///
/// # Safety
///
/// `path` must be a live handle returned by `spherical_nav_find_path`.
pub unsafe extern "C" fn spherical_path_count(path: *const c_void) -> u32 {
    spherical_path_ref(path)
        .and_then(|path| u32::try_from(path.waypoints().len()).ok())
        .unwrap_or(0)
}

#[no_mangle]
/// Returns the great-circle length of a spherical navigation path.
///
/// # Safety
///
/// `path` must be a live handle returned by `spherical_nav_find_path`.
pub unsafe extern "C" fn spherical_path_length(path: *const c_void) -> f64 {
    spherical_path_ref(path).map_or(f64::NAN, SphericalPath::length)
}

#[no_mangle]
/// Copies one spherical waypoint into caller-owned outputs.
///
/// # Safety
///
/// `path` must be a live spherical path handle. Each output must be null or
/// point to writable storage for one `f64`; all three must be valid for
/// success.
pub unsafe extern "C" fn spherical_path_point(
    path: *const c_void,
    index: u32,
    out_x: *mut f64,
    out_y: *mut f64,
    out_z: *mut f64,
) -> bool {
    let Some(point) =
        spherical_path_ref(path).and_then(|path| path.waypoints().get(index as usize))
    else {
        return false;
    };
    write_dvec3(*point, out_x, out_y, out_z)
}

#[no_mangle]
/// Inserts or replaces one dynamic impassable geodesic cap.
///
/// # Safety
///
/// `graph` must be a live, uniquely borrowed spherical graph handle.
/// `obstacle_id` must point to a readable NUL-terminated UTF-8 string.
pub unsafe extern "C" fn spherical_nav_upsert_obstacle(
    graph: *mut c_void,
    obstacle_id: *const c_char,
    direction_x: f64,
    direction_y: f64,
    direction_z: f64,
    angular_radius: f64,
) -> bool {
    let (Some(graph), Some(obstacle_id)) = (spherical_graph_mut(graph), ffi_string(obstacle_id))
    else {
        return false;
    };
    SphericalNavObstacle::new(
        obstacle_id,
        DVec3::new(direction_x, direction_y, direction_z),
        angular_radius,
    )
    .and_then(|obstacle| graph.upsert_obstacle(obstacle).map(|_| ()))
    .is_ok()
}

#[no_mangle]
/// Removes one dynamic spherical obstacle by stable identifier.
///
/// # Safety
///
/// `graph` must be a live, uniquely borrowed spherical graph handle.
/// `obstacle_id` must point to a readable NUL-terminated UTF-8 string.
pub unsafe extern "C" fn spherical_nav_remove_obstacle(
    graph: *mut c_void,
    obstacle_id: *const c_char,
) -> bool {
    let (Some(graph), Some(obstacle_id)) = (spherical_graph_mut(graph), ffi_string(obstacle_id))
    else {
        return false;
    };
    graph.remove_obstacle(&obstacle_id)
}

#[no_mangle]
/// Inserts or replaces one dynamic spherical traversal-cost area.
///
/// # Safety
///
/// `graph` must be a live, uniquely borrowed spherical graph handle.
/// `area_id` must point to a readable NUL-terminated UTF-8 string.
pub unsafe extern "C" fn spherical_nav_upsert_traversal_area(
    graph: *mut c_void,
    area_id: *const c_char,
    direction_x: f64,
    direction_y: f64,
    direction_z: f64,
    angular_radius: f64,
    cost_multiplier: f64,
) -> bool {
    let (Some(graph), Some(area_id)) = (spherical_graph_mut(graph), ffi_string(area_id)) else {
        return false;
    };
    SphericalTraversalArea::new(
        area_id,
        DVec3::new(direction_x, direction_y, direction_z),
        angular_radius,
        cost_multiplier,
    )
    .and_then(|area| graph.upsert_traversal_area(area))
    .is_ok()
}

#[no_mangle]
/// Removes one dynamic spherical traversal-cost area.
///
/// # Safety
///
/// `graph` must be a live, uniquely borrowed spherical graph handle.
/// `area_id` must point to a readable NUL-terminated UTF-8 string.
pub unsafe extern "C" fn spherical_nav_remove_traversal_area(
    graph: *mut c_void,
    area_id: *const c_char,
) -> bool {
    let (Some(graph), Some(area_id)) = (spherical_graph_mut(graph), ffi_string(area_id)) else {
        return false;
    };
    graph.remove_traversal_area(&area_id)
}

#[no_mangle]
/// Clears all dynamic spherical obstacles and traversal areas.
///
/// # Safety
///
/// `graph` must be a live, uniquely borrowed spherical graph handle.
pub unsafe extern "C" fn spherical_nav_clear_dynamic(graph: *mut c_void) -> bool {
    let Some(graph) = spherical_graph_mut(graph) else {
        return false;
    };
    graph.clear_dynamic_overrides();
    true
}

#[no_mangle]
/// Returns the dynamic-overlay revision of a spherical navigation graph.
///
/// # Safety
///
/// `graph` must be a live spherical graph handle and remain readable for the
/// duration of this call.
pub unsafe extern "C" fn spherical_nav_dynamic_revision(graph: *const c_void) -> u64 {
    spherical_graph_ref(graph).map_or(0, SphericalNavGraph::dynamic_revision)
}

fn grid_ref(grid: *const c_void) -> Option<&'static SpaceNavGrid> {
    (!grid.is_null()).then(|| unsafe { &*grid.cast::<SpaceNavGrid>() })
}

fn grid_mut(grid: *mut c_void) -> Option<&'static mut SpaceNavGrid> {
    (!grid.is_null()).then(|| unsafe { &mut *grid.cast::<SpaceNavGrid>() })
}

fn path_ref(path: *const c_void) -> Option<&'static SpacePath> {
    (!path.is_null()).then(|| unsafe { &*path.cast::<SpacePath>() })
}

fn spherical_graph_ref(graph: *const c_void) -> Option<&'static SphericalNavGraph> {
    (!graph.is_null()).then(|| unsafe { &*graph.cast::<SphericalNavGraph>() })
}

fn spherical_graph_mut(graph: *mut c_void) -> Option<&'static mut SphericalNavGraph> {
    (!graph.is_null()).then(|| unsafe { &mut *graph.cast::<SphericalNavGraph>() })
}

fn spherical_path_ref(path: *const c_void) -> Option<&'static SphericalPath> {
    (!path.is_null()).then(|| unsafe { &*path.cast::<SphericalPath>() })
}

fn write_vec3(point: Vec3, out_x: *mut f32, out_y: *mut f32, out_z: *mut f32) -> bool {
    if out_x.is_null() || out_y.is_null() || out_z.is_null() {
        return false;
    }
    // SAFETY: Outputs were null-checked and public callers guarantee writable pointers.
    unsafe {
        *out_x = point.x;
        *out_y = point.y;
        *out_z = point.z;
    }
    true
}

fn write_dvec3(point: DVec3, out_x: *mut f64, out_y: *mut f64, out_z: *mut f64) -> bool {
    if out_x.is_null() || out_y.is_null() || out_z.is_null() {
        return false;
    }
    // SAFETY: Outputs were null-checked and public callers guarantee writable pointers.
    unsafe {
        *out_x = point.x;
        *out_y = point.y;
        *out_z = point.z;
    }
    true
}

fn ffi_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // SAFETY: Public callers must provide a readable NUL-terminated UTF-8 string.
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .ok()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_navigation_handles_return_native_waypoints() {
        let grid = space_nav_create(0.0, 0.0, 0.0, 4, 4, 4, 1.0);
        assert!(!grid.is_null());
        let path = unsafe { space_nav_find_path(grid, 0.5, 0.5, 0.5, 3.5, 3.5, 3.5) };
        assert!(!path.is_null());
        assert_eq!(unsafe { space_path_count(path) }, 2);
        assert!(unsafe { space_path_length(path) } > 5.0);
        unsafe {
            space_path_destroy(path);
            space_nav_destroy(grid);
        }
    }

    #[test]
    fn spherical_navigation_can_share_the_native_planet_surface_query() {
        let config = crate::planet_query::FfiPlanetTerrainConfig {
            center_x: 1.0e12,
            center_y: -2.0e12,
            center_z: 3.0e12,
            radius: 1_000.0,
            height_scale: 20.0,
            ..Default::default()
        };
        let query = unsafe { crate::planet_query::planet_query_create(&config) };
        assert!(!query.is_null());
        let graph = unsafe { spherical_nav_create_for_planet(query, 256, 8) };
        assert!(!graph.is_null());
        let path = unsafe {
            spherical_nav_find_path(
                graph,
                config.center_x + 1_000.0,
                config.center_y,
                config.center_z,
                config.center_x - 1_000.0,
                config.center_y,
                config.center_z,
            )
        };
        assert!(!path.is_null());
        assert!(unsafe { spherical_path_count(path) } > 3);
        let mut x = 0.0;
        let mut y = 0.0;
        let mut z = 0.0;
        assert!(unsafe { spherical_path_point(path, 0, &mut x, &mut y, &mut z) });
        assert!((x - config.center_x - 1_000.0).abs() < 1.0e-6);
        assert!((y - config.center_y).abs() < 1.0e-6);
        assert!((z - config.center_z).abs() < 1.0e-6);

        let obstacle = std::ffi::CString::new("landing-pad").unwrap();
        assert!(unsafe {
            spherical_nav_upsert_obstacle(graph, obstacle.as_ptr(), 1.0, 0.0, 0.0, 0.05)
        });
        assert!(unsafe { spherical_nav_dynamic_revision(graph) } > 0);
        let blocked = unsafe {
            spherical_nav_find_path(
                graph,
                config.center_x + 1_000.0,
                config.center_y,
                config.center_z,
                config.center_x - 1_000.0,
                config.center_y,
                config.center_z,
            )
        };
        assert!(blocked.is_null());
        assert!(unsafe { spherical_nav_remove_obstacle(graph, obstacle.as_ptr()) });
        unsafe {
            spherical_path_destroy(path);
            spherical_nav_destroy(graph);
            crate::planet_query::planet_query_destroy(query);
        }
    }
}
