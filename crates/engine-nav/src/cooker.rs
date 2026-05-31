use crate::navmesh::{NavMesh, VertexIndex};
use glam::Vec3;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// NavMeshCooker
// ---------------------------------------------------------------------------

/// Cooks triangle-mesh geometry into a [`NavMesh`] with walkable convex
/// polygons.
///
/// The cooker applies the following pipeline:
///
/// 1. **Degenerate rejection** — zero-area triangles are skipped.
/// 2. **Steep rejection** — triangles whose normal makes an angle greater
///    than `walkable_slope` with the up vector (+Y) are discarded.
/// 3. **Vertex welding** — vertices within `WELD_EPSILON` world-units are
///    snapped together, eliminating T-junctions.
/// 4. **Coplanar grouping** — triangles lying on approximately the same
///    plane are collected together.
/// 5. **Greedy merging** — adjacent coplanar triangles are merged into
///    larger convex polygons (reducing polygon count for pathfinding).
/// 6. **NavMesh assembly** — each resulting convex polygon is added via
///    [`NavMesh::add_polygon`].
#[derive(Clone, Debug)]
pub struct NavMeshCooker {
    /// Agent radius — used for margin/padding (reserved for future use).
    pub agent_radius: f32,
    /// Agent height (reserved for future use).
    pub agent_height: f32,
    /// Maximum walkable step height (reserved for future use).
    pub agent_max_climb: f32,
    /// Maximum walkable slope in **degrees** (default: 45°).
    pub walkable_slope: f32,
}

impl Default for NavMeshCooker {
    fn default() -> Self {
        Self {
            agent_radius: 0.3,
            agent_height: 1.8,
            agent_max_climb: 0.5,
            walkable_slope: 45.0,
        }
    }
}

impl NavMeshCooker {
    /// Create a new cooker with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    // ------------------------------------------------------------------
    // Public API
    // ------------------------------------------------------------------

    /// Build a [`NavMesh`] from a set of triangles.
    ///
    /// Each triangle is a triple of world-space [`Vec3`] positions.  The
    /// pipeline described on [`NavMeshCooker`] is applied before the
    /// polygons are inserted into the returned mesh.
    ///
    /// `default_cost` is the movement-cost multiplier assigned to every
    /// resulting polygon (1.0 for normal terrain).
    pub fn cook_from_triangles(
        &self,
        triangles: &[(Vec3, Vec3, Vec3)],
        default_cost: f32,
    ) -> NavMesh {
        // ------------------------------------------------------------------
        // Step 1 & 2 — filter degenerate and steep triangles
        // ------------------------------------------------------------------
        const DEGENERATE_AREA_EPS: f32 = 1e-10;
        const WELD_EPSILON: f32 = 0.001;
        const PLANE_NORMAL_EPS: f32 = 0.01; // radians (~0.57°)
        const PLANE_DIST_EPS: f32 = 0.01;

        let slope_rad = self.walkable_slope.to_radians();
        let up = Vec3::Y;

        // Pre-filtered triangles with their normals.
        struct CookTri {
            verts: [Vec3; 3],
            normal: Vec3,
        }

        let filtered: Vec<CookTri> = triangles
            .iter()
            .filter_map(|&(a, b, c)| {
                let e1 = b - a;
                let e2 = c - a;
                let n = e1.cross(e2);
                let twice_area = n.length();
                if twice_area < DEGENERATE_AREA_EPS {
                    return None; // skip degenerate
                }
                let normal = n / twice_area;
                // Normal could point up or down depending on winding;
                // check the acute angle to vertical.
                if normal.angle_between(up).min(normal.angle_between(-up)) > slope_rad {
                    return None; // too steep
                }
                Some(CookTri {
                    verts: [a, b, c],
                    normal,
                })
            })
            .collect();

        // ------------------------------------------------------------------
        // Step 3 — vertex welding
        // ------------------------------------------------------------------
        let mut welder = VertexWelder::new(WELD_EPSILON);
        let mut welded: Vec<(Vec3, [VertexIndex; 3])> = Vec::with_capacity(filtered.len());

        for ct in &filtered {
            // Ensure CCW winding (normal points up).  When the cross
            // product Y is negative the winding is CW, so we swap two
            // vertices and negate the stored normal to keep it up-facing.
            let cross = (ct.verts[1] - ct.verts[0]).cross(ct.verts[2] - ct.verts[0]);
            let (verts, normal) = if cross.y < 0.0 {
                ([ct.verts[0], ct.verts[2], ct.verts[1]], -ct.normal)
            } else {
                (ct.verts, ct.normal)
            };

            let i0 = welder.weld(verts[0]);
            let i1 = welder.weld(verts[1]);
            let i2 = welder.weld(verts[2]);

            welded.push((normal, [i0, i1, i2]));
        }

        let vertices = welder.into_vertices();
        if vertices.is_empty() {
            return NavMesh::new();
        }

        // ------------------------------------------------------------------
        // Step 4 — group by plane
        // ------------------------------------------------------------------
        let mut plane_groups: Vec<Vec<usize>> = Vec::new();
        let mut group_normals: Vec<Vec3> = Vec::new();
        let mut group_dists: Vec<f32> = Vec::new();

        for (idx, &(normal, ref tri)) in welded.iter().enumerate() {
            let d = normal.dot(vertices[tri[0].0 as usize]);

            // Search for an existing group that shares this plane.
            let mut matched = None;
            for g in 0..plane_groups.len() {
                if normal.angle_between(group_normals[g]) < PLANE_NORMAL_EPS
                    && (d - group_dists[g]).abs() < PLANE_DIST_EPS
                {
                    // Ensure all triangle vertices are approximately on the plane.
                    let on_plane = tri.iter().all(|&vi| {
                        let p = vertices[vi.0 as usize];
                        (normal.dot(p) - d).abs() < PLANE_DIST_EPS * 10.0
                    });
                    if on_plane {
                        matched = Some(g);
                        break;
                    }
                }
            }

            match matched {
                Some(g) => plane_groups[g].push(idx),
                None => {
                    plane_groups.push(vec![idx]);
                    group_normals.push(normal);
                    group_dists.push(d);
                }
            }
        }

        // ------------------------------------------------------------------
        // Step 5 & 6 — merge each planar group and add to NavMesh
        // ------------------------------------------------------------------
        let mut navmesh = NavMesh::new();

        // Register all welded vertices.
        let mut vi_map: Vec<VertexIndex> = Vec::with_capacity(vertices.len());
        for &v in &vertices {
            vi_map.push(navmesh.add_vertex(v));
        }

        for group in &plane_groups {
            let group_tris: Vec<[VertexIndex; 3]> =
                group.iter().map(|&i| welded[i].1).collect();

            let merged = merge_triangles_to_convex_polygons(&group_tris, &vertices);

            for poly_verts in &merged {
                let mapped: Vec<VertexIndex> =
                    poly_verts.iter().map(|&vi| vi_map[vi.0 as usize]).collect();
                navmesh.add_polygon(&mapped, default_cost);
            }
        }

        navmesh
    }

    /// Build a [`NavMesh`] from a heightfield grid.
    ///
    /// `height_data` should contain exactly `width × depth` entries, each
    /// representing the Y-coordinate of the terrain at grid cell `(x, z)`.
    ///
    /// | Parameter      | Meaning                                      |
    /// |----------------|----------------------------------------------|
    /// | `cell_size`    | World-space distance between adjacent cells  |
    /// | `cell_height`  | World-space Y per unit of height data        |
    ///
    /// Two triangles are generated per grid cell (a quad split along the
    /// diagonal), and the result is fed through [`cook_from_triangles`].
    pub fn cook_from_heightfield(
        &self,
        width: u32,
        depth: u32,
        height_data: &[f32],
        cell_size: f32,
        cell_height: f32,
        default_cost: f32,
    ) -> NavMesh {
        if width < 2 || depth < 2 || height_data.len() < (width * depth) as usize {
            return NavMesh::new();
        }

        let mut triangles = Vec::with_capacity(((width - 1) * (depth - 1) * 2) as usize);

        for z in 0..(depth - 1) {
            for x in 0..(width - 1) {
                let i00 = (z * width + x) as usize;
                let i10 = (z * width + (x + 1)) as usize;
                let i01 = ((z + 1) * width + x) as usize;
                let i11 = ((z + 1) * width + (x + 1)) as usize;

                let p00 = Vec3::new(
                    x as f32 * cell_size,
                    height_data[i00] * cell_height,
                    z as f32 * cell_size,
                );
                let p10 = Vec3::new(
                    (x + 1) as f32 * cell_size,
                    height_data[i10] * cell_height,
                    z as f32 * cell_size,
                );
                let p01 = Vec3::new(
                    x as f32 * cell_size,
                    height_data[i01] * cell_height,
                    (z + 1) as f32 * cell_size,
                );
                let p11 = Vec3::new(
                    (x + 1) as f32 * cell_size,
                    height_data[i11] * cell_height,
                    (z + 1) as f32 * cell_size,
                );

                // Two triangles per grid cell (split along \ diagonal).
                triangles.push((p00, p10, p01));
                triangles.push((p10, p11, p01));
            }
        }

        self.cook_from_triangles(&triangles, default_cost)
    }
}

// ===========================================================================
// Vertex welding
// ===========================================================================

/// Snaps vertices within `epsilon` distance to the same index.
struct VertexWelder {
    vertices: Vec<Vec3>,
    epsilon_sq: f32,
}

impl VertexWelder {
    fn new(epsilon: f32) -> Self {
        Self {
            vertices: Vec::new(),
            epsilon_sq: epsilon * epsilon,
        }
    }

    fn weld(&mut self, pos: Vec3) -> VertexIndex {
        for (i, v) in self.vertices.iter().enumerate() {
            if v.distance_squared(pos) < self.epsilon_sq {
                return VertexIndex(i as u32);
            }
        }
        let idx = self.vertices.len() as u32;
        self.vertices.push(pos);
        VertexIndex(idx)
    }

    fn into_vertices(self) -> Vec<Vec3> {
        self.vertices
    }
}

// ===========================================================================
// Coplanar merging helpers
// ===========================================================================

/// Greedily merge a group of coplanar triangles into larger convex polygons.
///
/// All triangles in `tris` must lie on approximately the same plane.  The
/// function starts with each triangle as its own polygon and iteratively
/// merges any pair that shares a full edge *and* whose union remains convex.
fn merge_triangles_to_convex_polygons(
    tris: &[[VertexIndex; 3]],
    vertices: &[Vec3],
) -> Vec<Vec<VertexIndex>> {
    if tris.is_empty() {
        return Vec::new();
    }

    // Seed the polygon list with one polygon per triangle.
    let mut polys: Vec<Vec<VertexIndex>> = tris.iter().map(|t| t.to_vec()).collect();

    // Greedy merge loop.
    loop {
        // Build edge → polygon-owner map for the current set.
        let owner_map = build_edge_owner_map(&polys);

        let mut merged_any = false;

        // Iterate over edges shared by exactly two polygons.
        let keys: Vec<(u32, u32)> = owner_map
            .iter()
            .filter(|(_, owners)| owners.len() == 2)
            .map(|(e, _)| *e)
            .collect();

        for edge in &keys {
            let owners = &owner_map[edge];
            debug_assert!(owners.len() == 2);
            let i = owners[0];
            let j = owners[1];
            if i == j {
                continue;
            }

            if let Some(merged) = try_merge_convex(&polys[i], &polys[j], vertices) {
                // Replace i and j with the merged polygon.
                // Remove higher index first to keep indices valid.
                if i < j {
                    polys.remove(j);
                    polys.remove(i);
                } else {
                    polys.remove(i);
                    polys.remove(j);
                }
                polys.push(merged);
                merged_any = true;
                break; // restart edge scan
            }
        }

        if !merged_any {
            break;
        }
    }

    polys
}

/// Build a map from edge (sorted vertex-index pair) to the list of polygon
/// indices that contain that edge.
fn build_edge_owner_map(polys: &[Vec<VertexIndex>]) -> HashMap<(u32, u32), Vec<usize>> {
    let mut map: HashMap<(u32, u32), Vec<usize>> = HashMap::new();

    for (pi, poly) in polys.iter().enumerate() {
        let n = poly.len();
        for j in 0..n {
            let a = poly[j].0;
            let b = poly[(j + 1) % n].0;
            let edge = if a < b { (a, b) } else { (b, a) };
            map.entry(edge).or_default().push(pi);
        }
    }

    map
}

/// Try to merge two convex polygons that share a full edge.
///
/// Returns `Some(merged_polygon)` if the merge produces a convex polygon,
/// or `None` if the polygons do not share an edge or the result would be
/// non-convex.
fn try_merge_convex(
    p1: &[VertexIndex],
    p2: &[VertexIndex],
    vertices: &[Vec3],
) -> Option<Vec<VertexIndex>> {
    // Find the two shared vertices.
    let shared: Vec<VertexIndex> = p1.iter().filter(|v| p2.contains(v)).copied().collect();
    if shared.len() != 2 {
        return None;
    }
    let s0 = shared[0];
    let s1 = shared[1];

    // Verify the shared vertices form a full edge in both polygons.
    if !vertices_form_edge(s0, s1, p1) || !vertices_form_edge(s0, s1, p2) {
        return None;
    }

    let p1_s0 = p1.iter().position(|v| *v == s0).unwrap();
    let p1_s1 = p1.iter().position(|v| *v == s1).unwrap();
    let p2_s0 = p2.iter().position(|v| *v == s0).unwrap();
    let p2_s1 = p2.iter().position(|v| *v == s1).unwrap();

    // Try both possible orientations around the shared edge.
    // Ordering A: walk p1 s0 → s1, then p2 (after s1) → (before s0)
    let ma = build_merged_ring(p1, p2, p1_s0, p1_s1, p2_s1, p2_s0);
    if is_convex_xz(&ma, vertices) && ma.len() >= 3 {
        // Remove trailing duplicate of the first vertex, if any.
        let mut cleaned = ma;
        if cleaned.len() > 1 && cleaned[cleaned.len() - 1] == cleaned[0] {
            cleaned.pop();
        }
        return Some(clean_duplicates(&cleaned));
    }

    // Ordering B: walk p1 s1 → s0, then p2 (after s0) → (before s1)
    let mb = build_merged_ring(p1, p2, p1_s1, p1_s0, p2_s0, p2_s1);
    if is_convex_xz(&mb, vertices) && mb.len() >= 3 {
        let mut cleaned = mb;
        if cleaned.len() > 1 && cleaned[cleaned.len() - 1] == cleaned[0] {
            cleaned.pop();
        }
        return Some(clean_duplicates(&cleaned));
    }

    None
}

/// Build the merged vertex ring by walking two polygons.
///
/// 1. Walk `p1` forward from `p1_start` (inclusive) to `p1_end` (inclusive).
/// 2. Walk `p2` forward from the vertex **after** `p2_start_after` (inclusive)
///    to `p2_end_before` (exclusive).
fn build_merged_ring(
    p1: &[VertexIndex],
    p2: &[VertexIndex],
    p1_start: usize,
    p1_end: usize,
    p2_start_after: usize,
    p2_end_before: usize,
) -> Vec<VertexIndex> {
    let n1 = p1.len();
    let n2 = p2.len();
    let mut ring = Vec::with_capacity(n1 + n2);

    // Walk p1 from p1_start → p1_end (inclusive).
    let mut i = p1_start;
    loop {
        ring.push(p1[i]);
        if i == p1_end {
            break;
        }
        i = (i + 1) % n1;
    }

    // Walk p2 from (p2_start_after + 1) → p2_end_before (exclusive).
    if n2 >= 2 {
        let mut i = (p2_start_after + 1) % n2;
        loop {
            if i == p2_end_before {
                break;
            }
            ring.push(p2[i]);
            i = (i + 1) % n2;
        }
    }

    ring
}

/// Check whether two vertices are consecutive in a polygon (share an edge).
fn vertices_form_edge(a: VertexIndex, b: VertexIndex, poly: &[VertexIndex]) -> bool {
    let n = poly.len();
    if n < 2 {
        return false;
    }
    let Some(pa) = poly.iter().position(|v| *v == a) else {
        return false;
    };
    let Some(pb) = poly.iter().position(|v| *v == b) else {
        return false;
    };
    (pa + 1) % n == pb || (pb + 1) % n == pa
}

/// Check that a polygon is convex in the XZ plane (right-handed +Y up).
///
/// All cross products of consecutive edge vectors must have the same sign
/// (all positive or all negative).  Collinear edges are skipped.
fn is_convex_xz(poly: &[VertexIndex], vertices: &[Vec3]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }

    let mut sign: Option<f32> = None;
    for i in 0..n {
        let a = vertices[poly[i].0 as usize];
        let b = vertices[poly[(i + 1) % n].0 as usize];
        let c = vertices[poly[(i + 2) % n].0 as usize];

        // 2D cross product (edge_i × edge_{i+1}) on the XZ plane.
        let cross = (b.x - a.x) * (c.z - b.z) - (b.z - a.z) * (c.x - b.x);

        if cross.abs() < 1e-8 {
            continue; // collinear → skip
        }
        let s = cross.signum();
        match sign {
            None => sign = Some(s),
            Some(prev) if prev != s => return false,
            _ => {}
        }
    }

    true
}

/// Remove consecutive duplicate vertices from a polygon ring.
fn clean_duplicates(poly: &[VertexIndex]) -> Vec<VertexIndex> {
    if poly.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(poly.len());
    out.push(poly[0]);
    for &v in &poly[1..] {
        if v != *out.last().unwrap() {
            out.push(v);
        }
    }
    // Also check wraparound duplicate.
    if out.len() > 1 && out[out.len() - 1] == out[0] {
        out.pop();
    }
    out
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PolygonIndex;

    // ------------------------------------------------------------------
    // Helper: default cooker
    // ------------------------------------------------------------------
    fn cooker() -> NavMeshCooker {
        NavMeshCooker::default()
    }

    // ------------------------------------------------------------------
    // Empty input
    // ------------------------------------------------------------------
    #[test]
    fn empty_triangles_produces_empty_mesh() {
        let mesh = cooker().cook_from_triangles(&[], 1.0);
        assert_eq!(mesh.polygon_count(), 0);
        assert_eq!(mesh.vertex_count(), 0);
    }

    // ------------------------------------------------------------------
    // Single triangle
    // ------------------------------------------------------------------
    #[test]
    fn single_triangle() {
        let tri = (
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );
        let mesh = cooker().cook_from_triangles(&[tri], 1.0);
        assert_eq!(mesh.polygon_count(), 1);
        assert_eq!(mesh.vertex_count(), 3);

        // The polygon centre should be the centroid.
        let centre = mesh.polygon_center(PolygonIndex(0)).unwrap();
        let expected = Vec3::new(1.0 / 3.0, 0.0, 1.0 / 3.0);
        assert!(
            (centre - expected).length() < 0.001,
            "expected {:?}, got {:?}",
            expected,
            centre
        );
    }

    // ------------------------------------------------------------------
    // Degenerate triangle (zero area)
    // ------------------------------------------------------------------
    #[test]
    fn degenerate_triangle_is_skipped() {
        let tri = (
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0), // collinear → zero area
        );
        let mesh = cooker().cook_from_triangles(&[tri], 1.0);
        assert_eq!(mesh.polygon_count(), 0);
    }

    // ------------------------------------------------------------------
    // Steep triangle rejection
    // ------------------------------------------------------------------
    #[test]
    fn steep_triangle_is_rejected() {
        // Vertical triangle (normal 90° from up).
        let tri = (
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        );
        let mesh = cooker().cook_from_triangles(&[tri], 1.0);
        assert_eq!(mesh.polygon_count(), 0);
    }

    #[test]
    fn walkable_slope_accepts_gentle_triangles() {
        // 30° slope — default walkable_slope is 45°, so this should pass.
        let angle_rad = 30.0_f32.to_radians();
        let x = angle_rad.cos();
        let y = angle_rad.sin(); // height component
        let tri = (
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(x, y, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );
        let mesh = cooker().cook_from_triangles(&[tri], 1.0);
        assert_eq!(mesh.polygon_count(), 1);
    }

    // ------------------------------------------------------------------
    // Disjoint triangles (no shared edge)
    // ------------------------------------------------------------------
    #[test]
    fn disjoint_triangles_stay_separate() {
        let tris = [
            (
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
            (
                Vec3::new(5.0, 0.0, 5.0),
                Vec3::new(6.0, 0.0, 5.0),
                Vec3::new(5.0, 0.0, 6.0),
            ),
        ];
        let mesh = cooker().cook_from_triangles(&tris, 1.0);
        // Two separate triangles → two polygons, no neighbours.
        assert_eq!(mesh.polygon_count(), 2);
        assert!(mesh.polygon_neighbors(PolygonIndex(0)).is_empty());
        assert!(mesh.polygon_neighbors(PolygonIndex(1)).is_empty());
    }

    // ------------------------------------------------------------------
    // Coplanar merge
    // ------------------------------------------------------------------
    #[test]
    fn adjacent_coplanar_triangles_merge_into_quad() {
        // Two triangles that form a unit square.
        let tris = [
            (
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
            (
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
        ];
        let mesh = cooker().cook_from_triangles(&tris, 1.0);
        // Should merge into a single quad (4 vertices).
        assert_eq!(
            mesh.polygon_count(),
            1,
            "coplanar adjacent triangles should merge into one polygon"
        );
        assert_eq!(
            mesh.vertex_count(),
            4,
            "merged quad should have 4 unique vertices"
        );
    }

    #[test]
    fn coplanar_merge_preserves_convexity() {
        // A strip of triangles forming a convex shape — these should all
        // merge into a single polygon.
        //
        // Layout (XZ):  A------B
        //               | \    |
        //               |  \   |
        //               |   \  |
        //               |    \ |
        //               C------D
        //
        // Two triangles sharing the diagonal: (A,B,C) and (B,D,C)
        let tris = [
            (
                Vec3::new(0.0, 0.0, 0.0), // A
                Vec3::new(2.0, 0.0, 0.0), // B
                Vec3::new(0.0, 0.0, 1.0), // C
            ),
            (
                Vec3::new(2.0, 0.0, 0.0), // B
                Vec3::new(2.0, 0.0, 1.0), // D
                Vec3::new(0.0, 0.0, 1.0), // C
            ),
        ];
        let mesh = cooker().cook_from_triangles(&tris, 1.0);
        // Merges into a single convex quad.
        assert_eq!(
            mesh.polygon_count(),
            1,
            "coplanar strip should merge into a single polygon"
        );
        assert_eq!(mesh.vertex_count(), 4);
    }

    #[test]
    fn non_coplanar_triangles_dont_merge() {
        // Two triangles on different but walkable planes.
        let tris = [
            (
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
            (
                Vec3::new(2.0, 0.5, 0.0),  // raised by 0.5
                Vec3::new(3.0, 0.5, 0.0),
                Vec3::new(2.0, 0.5, 1.0),
            ),
        ];
        let mesh = cooker().cook_from_triangles(&tris, 1.0);
        // Two separate triangles on different planes.
        assert_eq!(mesh.polygon_count(), 2);
    }

    // ------------------------------------------------------------------
    // Vertex welding
    // ------------------------------------------------------------------
    #[test]
    fn vertex_welding_eliminates_t_junctions() {
        // Two triangles that share an edge but with slightly different
        // vertex positions for that edge.
        let tris = [
            (
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
            (
                Vec3::new(1.000_001, 0.0, 0.0), // nearly the same
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
        ];
        let mesh = cooker().cook_from_triangles(&tris, 1.0);
        // After welding, should have 4 unique vertices.
        assert_eq!(
            mesh.vertex_count(),
            4,
            "welding should snap nearly coincident vertices"
        );
        // Should merge into a single quad.
        assert_eq!(mesh.polygon_count(), 1);
    }

    // ------------------------------------------------------------------
    // Heightfield conversion
    // ------------------------------------------------------------------
    #[test]
    fn heightfield_empty_below_minimum_size() {
        let mesh = cooker().cook_from_heightfield(1, 1, &[0.0], 1.0, 1.0, 1.0);
        assert_eq!(mesh.polygon_count(), 0);
    }

    #[test]
    fn heightfield_single_cell() {
        // 2×2 heightfield = 1 cell = 2 triangles → should merge to 1 quad
        // on a flat plane.
        let heights = [0.0, 0.0, 0.0, 0.0];
        let mesh = cooker().cook_from_heightfield(2, 2, &heights, 1.0, 1.0, 1.0);
        assert_eq!(
            mesh.polygon_count(),
            1,
            "flat 2x2 heightfield should produce 1 merged quad"
        );
        assert_eq!(mesh.vertex_count(), 4);
    }

    #[test]
    fn heightfield_larger_grid() {
        // 4×4 flat heightfield → many coplanar triangles → merged into
        // a single large polygon (or a few depending on anything that
        // prevents merging — with the default 0.01 plane tolerance the
        // whole grid should merge into one polygon).
        let heights = [0.0_f32; 16];
        let mesh = cooker().cook_from_heightfield(4, 4, &heights, 1.0, 1.0, 1.0);
        assert!(mesh.polygon_count() >= 1);
        // width×depth grid = width×depth unique grid points after welding.
        assert_eq!(mesh.vertex_count(), 16);
    }

    #[test]
    fn heightfield_with_varying_heights() {
        // A 2×2 heightfield with a ridge: two cells at different heights.
        // The two triangles in the lower cell should merge; the two in
        // the higher cell should also merge (both are flat planes).
        let heights = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
        // 4×3 grid = 3×2 cells = 6 triangles on two height levels.
        let mesh = cooker().cook_from_heightfield(4, 3, &heights, 1.0, 1.0, 1.0);
        // Each flat strip should merge its triangles.
        // The exact polygon count depends on planar grouping and merging,
        // but should be far fewer than 6 raw triangles.
        assert!(
            mesh.polygon_count() <= 4,
            "expected at most 4 merged polygons from two flat strips, got {}",
            mesh.polygon_count()
        );
    }

    // ------------------------------------------------------------------
    // Polygon neighbour connectivity
    // ------------------------------------------------------------------
    #[test]
    fn neighbouring_polygons_detect_connectivity() {
        // Two roof-like quads on different planes sharing a ridge edge.
        //
        //   Left slope (y=0→1)         Right slope (y=1→0)
        //   A-------B                    C-------D
        //    \     /                      \     /
        //     \   /                        \   /
        //      \ /                          \ /
        //       C-------D                    E-------F
        //
        // Quads share the ridge edge C-D at (0,1,1)-(2,1,1).
        let tris = [
            // Left slope: A(0,0,0), B(2,0,0), C(0,1,1), D(2,1,1)
            (
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 1.0),
            ),
            (
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 1.0, 1.0),
                Vec3::new(0.0, 1.0, 1.0),
            ),
            // Right slope: C(0,1,1), D(2,1,1), E(0,0,2), F(2,0,2)
            (
                Vec3::new(0.0, 1.0, 1.0),
                Vec3::new(2.0, 1.0, 1.0),
                Vec3::new(0.0, 0.0, 2.0),
            ),
            (
                Vec3::new(2.0, 1.0, 1.0),
                Vec3::new(2.0, 0.0, 2.0),
                Vec3::new(0.0, 0.0, 2.0),
            ),
        ];

        let cooker = NavMeshCooker {
            walkable_slope: 60.0, // both slopes are ~45°, so they pass
            ..NavMeshCooker::default()
        };
        let mesh = cooker.cook_from_triangles(&tris, 1.0);

        // Each slope merges into one quad → 2 polygons total.
        assert_eq!(
            mesh.polygon_count(),
            2,
            "two differently-sloped quads should remain separate"
        );

        // Check they are neighbours (share vertices C and D at the ridge).
        let n0 = mesh.polygon_neighbors(PolygonIndex(0));
        let n1 = mesh.polygon_neighbors(PolygonIndex(1));
        assert!(
            n0.contains(&PolygonIndex(1)),
            "quad 0 should be neighbour of quad 1"
        );
        assert!(
            n1.contains(&PolygonIndex(0)),
            "quad 1 should be neighbour of quad 0"
        );
    }

    // ------------------------------------------------------------------
    // Edge: all-steep input → empty mesh
    // ------------------------------------------------------------------
    #[test]
    fn all_steep_yields_empty_mesh() {
        let tris = [
            (
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 1.0),
            ),
            (
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 1.0),
                Vec3::new(1.0, 1.0, 0.0),
            ),
        ];
        let mesh = cooker().cook_from_triangles(&tris, 1.0);
        assert_eq!(mesh.polygon_count(), 0);
    }

    // ------------------------------------------------------------------
    // Custom walkable slope
    // ------------------------------------------------------------------
    #[test]
    fn custom_walkable_slope_filters_accordingly() {
        // 20° slope triangle.
        let angle_rad = 20.0_f32.to_radians();
        let x = angle_rad.cos();
        let y = angle_rad.sin();
        let tri = (
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(x, y, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );

        let strict = NavMeshCooker {
            walkable_slope: 10.0, // too strict for a 20° slope
            ..NavMeshCooker::default()
        };
        assert_eq!(strict.cook_from_triangles(&[tri], 1.0).polygon_count(), 0);

        let lenient = NavMeshCooker {
            walkable_slope: 30.0, // lenient enough
            ..NavMeshCooker::default()
        };
        assert_eq!(lenient.cook_from_triangles(&[tri], 1.0).polygon_count(), 1);
    }
}
