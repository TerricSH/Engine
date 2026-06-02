use crate::navmesh::{NavError, NavMesh, PolygonIndex};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashSet};
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
        let mut open_set: std::collections::HashSet<PolygonIndex> =
            std::collections::HashSet::new();

        let start_center = navmesh
            .polygon_center(from_poly)
            .ok_or(NavError::InvalidNavMesh("invalid start polygon".into()))?;

        g_score.insert(from_poly, 0.0);
        open_heap.push(AStarNode {
            f_score: self.distance_xz(start_center, goal_center),
            polygon: from_poly,
        });
        open_set.insert(from_poly);

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

            // Remove from the tracking set so re-insertion is fast.
            open_set.remove(&node.polygon);

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
                        open_set.insert(neighbor);
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

    // -- Funnel / string-pulling smoothing ----------------------------------

    /// Find a path and apply funnel (string-pulling) smoothing to produce
    /// waypoints that hug polygon edges rather than jumping centre-to-centre.
    ///
    /// This is the recommended entry-point for agent navigation.  The
    /// returned path has the same semantic meaning as
    /// [`find_path`](Self::find_path) but with tighter, more natural-looking
    /// waypoints.
    pub fn find_path_smoothed(
        &self,
        navmesh: &NavMesh,
        from: Vec3,
        to: Vec3,
    ) -> Result<Path, NavError> {
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

        let corridor = self.find_corridor(navmesh, from_poly, to_poly)?;
        self.string_pull(navmesh, &corridor, to)
    }

    /// Return the ordered list of polygon indices forming a path corridor.
    fn find_corridor(
        &self,
        navmesh: &NavMesh,
        from_poly: PolygonIndex,
        to_poly: PolygonIndex,
    ) -> Result<Vec<PolygonIndex>, NavError> {
        if navmesh.polygon_count() == 0 {
            return Err(NavError::InvalidNavMesh("navmesh has no polygons".into()));
        }
        if from_poly == to_poly {
            return Ok(vec![from_poly]);
        }

        let goal_center = navmesh
            .polygon_center(to_poly)
            .ok_or(NavError::InvalidNavMesh("invalid goal polygon".into()))?;

        let mut g_score: BTreeMap<PolygonIndex, f32> = BTreeMap::new();
        let mut came_from: BTreeMap<PolygonIndex, PolygonIndex> = BTreeMap::new();
        let mut open_heap: BinaryHeap<AStarNode> = BinaryHeap::new();
        let mut open_set: HashSet<PolygonIndex> = HashSet::new();

        let start_center = navmesh
            .polygon_center(from_poly)
            .ok_or(NavError::InvalidNavMesh("invalid start polygon".into()))?;

        g_score.insert(from_poly, 0.0);
        open_heap.push(AStarNode {
            f_score: self.distance_xz(start_center, goal_center),
            polygon: from_poly,
        });
        open_set.insert(from_poly);

        while let Some(node) = open_heap.pop() {
            if let Some(&best_g) = g_score.get(&node.polygon) {
                let f_expected = best_g
                    + self.distance_xz(
                        navmesh.polygon_center(node.polygon).unwrap_or(start_center),
                        goal_center,
                    );
                if node.f_score > f_expected + 0.0001 {
                    continue;
                }
            }
            open_set.remove(&node.polygon);

            if node.polygon == to_poly {
                // Reconstruct corridor.
                let mut corridor = Vec::new();
                let mut cur = to_poly;
                while cur != from_poly {
                    corridor.push(cur);
                    cur = *came_from.get(&cur).ok_or(NavError::NoPathFound)?;
                }
                corridor.push(from_poly);
                corridor.reverse();
                return Ok(corridor);
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
                        open_set.insert(neighbor);
                    }
                }
            }
        }
        Err(NavError::NoPathFound)
    }

    /// Apply the simple-stabbing funnel algorithm to the polygon corridor,
    /// returning a tightened path that hugs shared edges.
    ///
    /// Based on the classic "string pulling" method for navmesh waypoints.
    fn string_pull(
        &self,
        navmesh: &NavMesh,
        corridor: &[PolygonIndex],
        goal: Vec3,
    ) -> Result<Path, NavError> {
        if corridor.is_empty() {
            return Err(NavError::NoPathFound);
        }
        if corridor.len() == 1 {
            return navmesh
                .polygon_center(corridor[0])
                .map(|c| {
                    Path::new(vec![PathPoint {
                        position: c,
                        polygon: corridor[0],
                    }])
                })
                .ok_or(NavError::InvalidNavMesh("invalid single polygon".into()));
        }

        // ── 1. Build portals (shared edges) between adjacent corridor polygons ──
        //
        // Each portal is an edge (left, right) where the edge is the shared
        // boundary between polygon[i] and polygon[i+1].  We order the two
        // vertices so that `left` is to the *left* of the path direction
        // (XZ cross product) and `right` is to the right.

        struct Portal {
            left: Vec3,
            right: Vec3,
        }

        let mut portals: Vec<Portal> = Vec::with_capacity(corridor.len() - 1);

        for pair in corridor.windows(2) {
            let a = pair[0];
            let b = pair[1];
            if let Some((v0, v1)) = navmesh.shared_edge(a, b) {
                let p0 = navmesh.vertex(v0).copied().unwrap_or(Vec3::ZERO);
                let p1 = navmesh.vertex(v1).copied().unwrap_or(Vec3::ZERO);

                // Determine ordering: the edge midpoint → next polygon centre
                // direction tells us which side is left/right.
                let edge_mid = (p0 + p1) * 0.5;
                let next_center = navmesh.polygon_center(b).unwrap_or(goal);
                let to_next = (next_center - edge_mid).normalize_or_zero();
                // If the direction is degenerate (centre == edge midpoint),
                // fall back to using the edge's own direction.
                let to_next = if to_next == Vec3::ZERO {
                    (p1 - p0).normalize_or_zero()
                } else {
                    to_next
                };
                let edge_dir = (p1 - p0).normalize_or_zero();
                let cross_z = edge_dir.x * to_next.z - edge_dir.z * to_next.x;

                if cross_z > 0.0 {
                    portals.push(Portal {
                        left: p0,
                        right: p1,
                    });
                } else {
                    portals.push(Portal {
                        left: p1,
                        right: p0,
                    });
                }
            } else {
                // Polygons not adjacent — should not happen for a valid corridor.
                // Fall back to polygon centres.
                let c = navmesh.polygon_center(a).unwrap_or(Vec3::ZERO);
                portals.push(Portal { left: c, right: c });
            }
        }

        // ── 2. Funnel walk ─────────────────────────────────────────────────
        //
        // Start at the start point (polygon[0] centre, or ideally the actual
        // agent position — we use the centre here for generality).
        let start_pt = navmesh.polygon_center(corridor[0]).unwrap_or(Vec3::ZERO);

        /// Internal: a waypoint being built during the funnel walk.
        /// Carries both position and the index of the corridor polygon
        /// the point lies in (or an adjacent polygon for edge points).
        struct Waypoint {
            pos: Vec3,
            poly: PolygonIndex,
        }

        let last_poly = *corridor.last().unwrap_or(&PolygonIndex(0));
        let mut output: Vec<Waypoint> = Vec::new();
        output.push(Waypoint {
            pos: start_pt,
            poly: corridor[0],
        });

        // Current apex position.
        let mut apex = start_pt;

        // Funnel state: left and right boundaries, tracked by portal index.
        let mut left_idx: usize = 0;
        let mut right_idx: usize = 0;
        let mut portal_left = portals[0].left;
        let mut portal_right = portals[0].right;

        // Walk portals with an explicit index (so we can restart on collapse).
        let mut i: usize = 1;
        while i < portals.len() {
            let new_left = portals[i].left;
            let new_right = portals[i].right;

            // ── Update right boundary ──────────────────────────────────────
            let r_cross = cross_xz(portal_right - apex, new_right - apex);
            if r_cross >= 0.0 {
                let dist_new = (new_right - apex).length_squared();
                let dist_cur = (portal_right - apex).length_squared();
                if dist_new < dist_cur || r_cross > 0.0 {
                    portal_right = new_right;
                    right_idx = i;
                }
            }

            // ── Update left boundary ───────────────────────────────────────
            let l_cross = cross_xz(portal_left - apex, new_left - apex);
            if l_cross <= 0.0 {
                let dist_new = (new_left - apex).length_squared();
                let dist_cur = (portal_left - apex).length_squared();
                if dist_new < dist_cur || l_cross < 0.0 {
                    portal_left = new_left;
                    left_idx = i;
                }
            }

            // ── Check for funnel collapse ──────────────────────────────────
            let cross_rl = cross_xz(portal_right - apex, portal_left - apex);
            if cross_rl >= 0.0 {
                // Right boundary crossed left → add right vertex as waypoint.
                // The vertex lies on the portal at right_idx, which is the
                // shared edge between corridor[right_idx] and corridor[right_idx+1].
                output.push(Waypoint {
                    pos: portal_right,
                    poly: corridor[right_idx],
                });
                apex = portal_right;
                // Restart from right_idx.
                left_idx = right_idx;
                portal_left = apex;
                portal_right = apex;
                i = right_idx + 1;
                continue;
            }

            let cross_lr = cross_xz(portal_left - apex, portal_right - apex);
            if cross_lr <= 0.0 {
                // Left boundary crossed right → add left vertex as waypoint.
                output.push(Waypoint {
                    pos: portal_left,
                    poly: corridor[left_idx],
                });
                apex = portal_left;
                right_idx = left_idx;
                portal_left = apex;
                portal_right = apex;
                i = left_idx + 1;
                continue;
            }

            i += 1;
        }

        // ── 3. Emit final point ────────────────────────────────────────────
        //
        // Add the goal point (end of the last corridor polygon).
        if output.last().map(|w| w.pos) != Some(goal) {
            output.push(Waypoint {
                pos: goal,
                poly: last_poly,
            });
        }

        // Convert to PathPoints.
        let waypoints: Vec<PathPoint> = output
            .into_iter()
            .map(|w| PathPoint {
                position: w.pos,
                polygon: w.poly,
            })
            .collect();

        Ok(Path::new(waypoints))
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

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// 2D cross product (XZ plane) of vectors `a` and `b`.
/// Returns a scalar: positive if `b` is counter-clockwise from `a`.
fn cross_xz(a: Vec3, b: Vec3) -> f32 {
    a.x * b.z - a.z * b.x
}
