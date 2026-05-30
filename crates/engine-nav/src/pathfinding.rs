use crate::navmesh::{NavError, NavMesh, PolygonIndex};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};
use tracing::debug;

// ---------------------------------------------------------------------------
// PathPoint / Path
// ---------------------------------------------------------------------------

/// A single waypoint along a computed [`Path`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathPoint {
    /// World-space position (polygon center or adjusted point).
    pub position: Vec3,
    /// The polygon this waypoint belongs to.
    pub polygon: PolygonIndex,
}

/// A computed path made up of ordered waypoints.
///
/// Paths are produced by [`Pathfinder`] and consumed by [`NavAgent`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Path {
    waypoints: Vec<PathPoint>,
}

impl Path {
    /// Create a new path from an ordered list of waypoints.
    pub fn new(waypoints: Vec<PathPoint>) -> Self {
        Self { waypoints }
    }

    /// View all waypoints.
    pub fn waypoints(&self) -> &[PathPoint] {
        &self.waypoints
    }

    /// Number of waypoints in this path.
    pub fn len(&self) -> usize {
        self.waypoints.len()
    }

    /// Whether the path is empty (no waypoints).
    pub fn is_empty(&self) -> bool {
        self.waypoints.is_empty()
    }

    /// First waypoint, if any.
    pub fn first(&self) -> Option<&PathPoint> {
        self.waypoints.first()
    }

    /// Last waypoint, if any.
    pub fn last(&self) -> Option<&PathPoint> {
        self.waypoints.last()
    }
}

// ---------------------------------------------------------------------------
// A* internal types
// ---------------------------------------------------------------------------

/// Priority-queue entry for the A* open set.
/// Ordered so that `BinaryHeap` pops the smallest `f_score` first.
#[derive(Clone)]
struct AStarNode {
    f_score: f32,
    polygon: PolygonIndex,
}

impl Eq for AStarNode {}

impl PartialEq for AStarNode {
    fn eq(&self, other: &Self) -> bool {
        self.polygon == other.polygon
    }
}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // BinaryHeap is a max-heap; reverse so smallest f_score is on top.
        Some(self.cmp(other))
    }
}

impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap; reverse so smallest f_score is on top.
        other
            .f_score
            .partial_cmp(&self.f_score)
            .unwrap_or(Ordering::Equal)
    }
}

// ---------------------------------------------------------------------------
// Pathfinder
// ---------------------------------------------------------------------------

/// A* pathfinder that operates on a [`NavMesh`].
///
/// Heuristic: **Euclidean distance on the XZ plane** between polygon centres.
/// Movement cost: `distance(poly_center, neighbor_center) × neighbor.cost`.
///
/// Currently returns polygon-centre waypoints.  A **funnel** post-processing
/// step (string-pulling) may be added in the future to tighten paths against
/// polygon edges, producing waypoints that hug the corridor walls rather
/// than jumping from centre to centre.
///
/// This is a stateless object; it can be reused freely.
pub struct Pathfinder;

impl Pathfinder {
    /// Create a new pathfinder.
    pub fn new() -> Self {
        Self
    }

    // -- Euclidean distance (XZ only) --------------------------------------

    fn distance_xz(&self, a: Vec3, b: Vec3) -> f32 {
        let dx = a.x - b.x;
        let dz = a.z - b.z;
        (dx * dx + dz * dz).sqrt()
    }

    // -- Public API --------------------------------------------------------

    /// Find a path from `from` to `to` on the given [`NavMesh`].
    ///
    /// The start and goal polygons are resolved automatically:
    ///  1. If the point is inside a polygon, that polygon is used.
    ///  2. Otherwise the nearest polygon (by centre distance) is used.
    pub fn find_path(&self, navmesh: &NavMesh, from: Vec3, to: Vec3) -> Result<Path, NavError> {
        let from_poly = navmesh
            .find_polygon_containing(from)
            .or_else(|| {
                let nearest = navmesh.find_nearest_polygon(from);
                debug!(
                    "from point not on navmesh — snapped to nearest polygon {:?}",
                    nearest
                );
                Some(nearest)
            })
            .ok_or_else(|| NavError::InvalidNavMesh("navmesh has no polygons".into()))?;

        let to_poly = navmesh
            .find_polygon_containing(to)
            .or_else(|| {
                let nearest = navmesh.find_nearest_polygon(to);
                debug!(
                    "to point not on navmesh — snapped to nearest polygon {:?}",
                    nearest
                );
                Some(nearest)
            })
            .ok_or_else(|| NavError::InvalidNavMesh("navmesh has no polygons".into()))?;

        self.find_path_polygon(navmesh, from_poly, to_poly)
    }

    /// Find a path from one polygon to another using A*.
    ///
    /// Returns an ordered list of polygon-centre waypoints.
    /// When the start and goal are the same polygon a single-element path
    /// (containing the polygon centre) is returned.
    pub fn find_path_polygon(
        &self,
        navmesh: &NavMesh,
        from_poly: PolygonIndex,
        to_poly: PolygonIndex,
    ) -> Result<Path, NavError> {
        if navmesh.polygon_count() == 0 {
            return Err(NavError::InvalidNavMesh("navmesh has no polygons".into()));
        }

        // Trivial case: start == goal.
        if from_poly == to_poly {
            let center = navmesh
                .polygon_center(from_poly)
                .ok_or(NavError::InvalidNavMesh("invalid from polygon".into()))?;
            return Ok(Path::new(vec![PathPoint {
                position: center,
                polygon: from_poly,
            }]));
        }

        let goal_center = navmesh
            .polygon_center(to_poly)
            .ok_or(NavError::InvalidNavMesh("invalid goal polygon".into()))?;

        // -- A* state ------------------------------------------------------
        let mut g_score: BTreeMap<PolygonIndex, f32> = BTreeMap::new();
        let mut came_from: BTreeMap<PolygonIndex, PolygonIndex> = BTreeMap::new();
        let mut open_heap: BinaryHeap<AStarNode> = BinaryHeap::new();
        let mut open_set: Vec<PolygonIndex> = Vec::new();

        let start_center = navmesh
            .polygon_center(from_poly)
            .ok_or(NavError::InvalidNavMesh("invalid start polygon".into()))?;

        g_score.insert(from_poly, 0.0);
        open_heap.push(AStarNode {
            f_score: self.distance_xz(start_center, goal_center),
            polygon: from_poly,
        });
        open_set.push(from_poly);

        // -- Search loop ---------------------------------------------------
        while let Some(node) = open_heap.pop() {
            // Lazy removal: skip if this entry is stale.
            if let Some(&best_g) = g_score.get(&node.polygon) {
                let f_expected = best_g
                    + self.distance_xz(
                        navmesh.polygon_center(node.polygon).unwrap_or(start_center),
                        goal_center,
                    );
                // Allow small epsilon for floating-point drift.
                if node.f_score > f_expected + 0.0001 {
                    continue;
                }
            }

            // Remove from the tracking vec so re-insertion is fast.
            if let Some(pos) = open_set.iter().position(|p| *p == node.polygon) {
                open_set.swap_remove(pos);
            }

            // Goal check.
            if node.polygon == to_poly {
                let path = self.reconstruct_path(navmesh, &came_from, from_poly, to_poly)?;
                debug!(
                    "Path found: {} waypoints from {:?} to {:?}",
                    path.len(),
                    from_poly,
                    to_poly
                );
                return Ok(path);
            }

            let current_g = g_score[&node.polygon];

            for &neighbor in navmesh.polygon_neighbors(node.polygon).iter() {
                let neighbor_center = match navmesh.polygon_center(neighbor) {
                    Some(c) => c,
                    None => continue,
                };
                let current_center = match navmesh.polygon_center(node.polygon) {
                    Some(c) => c,
                    None => continue,
                };

                // Edge cost = distance × polygon cost multiplier.
                let poly_cost = navmesh
                    .polygons
                    .get(neighbor.0 as usize)
                    .map(|p| p.cost)
                    .unwrap_or(1.0);

                let edge_cost = self.distance_xz(current_center, neighbor_center) * poly_cost;
                let tentative_g = current_g + edge_cost;

                let prev_g = g_score.get(&neighbor).copied().unwrap_or(f32::MAX);

                if tentative_g < prev_g {
                    g_score.insert(neighbor, tentative_g);
                    came_from.insert(neighbor, node.polygon);

                    let h = self.distance_xz(neighbor_center, goal_center);
                    open_heap.push(AStarNode {
                        f_score: tentative_g + h,
                        polygon: neighbor,
                    });

                    if !open_set.contains(&neighbor) {
                        open_set.push(neighbor);
                    }
                }
            }
        }

        debug!(
            "No path found from polygon {:?} to {:?}",
            from_poly, to_poly
        );
        Err(NavError::NoPathFound)
    }

    // -- Helpers -----------------------------------------------------------

    /// Reconstruct the waypoint list from the `came_from` map.
    fn reconstruct_path(
        &self,
        navmesh: &NavMesh,
        came_from: &BTreeMap<PolygonIndex, PolygonIndex>,
        from_poly: PolygonIndex,
        to_poly: PolygonIndex,
    ) -> Result<Path, NavError> {
        let mut polys = Vec::new();
        let mut cur = to_poly;

        while cur != from_poly {
            polys.push(cur);
            cur = *came_from.get(&cur).ok_or(NavError::NoPathFound)?;
        }
        polys.push(from_poly);
        polys.reverse();

        let waypoints: Vec<PathPoint> = polys
            .iter()
            .filter_map(|&p| {
                navmesh.polygon_center(p).map(|pos| PathPoint {
                    position: pos,
                    polygon: p,
                })
            })
            .collect();

        if waypoints.is_empty() {
            return Err(NavError::NoPathFound);
        }

        Ok(Path::new(waypoints))
    }
}

impl Default for Pathfinder {
    fn default() -> Self {
        Self::new()
    }
}
