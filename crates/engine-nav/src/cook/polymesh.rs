//! Polygon-mesh generation — ear-clipping triangulation of region
//! contours, followed by greedy convex merging (Hertel–Mehlhorn style)
//! to produce a low-polygon-count convex decomposition.
//!
//! The pipeline per contour:
//! 1. Ensure CCW winding.
//! 2. `ear_clip_triangulate`   → list of index-triplets.
//! 3. `merge_convex_polys`     → list of convex `PolyPolygon`s.

use glam::Vec3;
use crate::cook::config::CookError;
use crate::cook::contour::ContourSet;

// ── Data structures ───────────────────────────────────────────────────────

/// A polygon mesh produced from a set of region contours.
pub(crate) struct PolyMesh {
    pub verts: Vec<Vec3>,
    pub polys: Vec<PolyPolygon>,
    pub bmin: Vec3,
    pub cs: f32,
    pub ch: f32,
}

/// A single convex polygon in the mesh.
///
/// * `verts` — indices into the parent `PolyMesh::verts`.
/// * `neighbors` — indices into `PolyMesh::polys` of adjacent polygons.
/// * `area` — area type of the original region.
#[derive(Clone, Debug)]
pub(crate) struct PolyPolygon {
    pub verts: Vec<u16>,
    pub neighbors: Vec<u16>,
    pub area: u8,
}

// ── Main entry point ──────────────────────────────────────────────────────

/// Build a polygon mesh from all contours in `cs`.
///
/// `nvp` is the maximum vertices per polygon (typically 6).
pub(crate) fn build_poly_mesh(contours: &ContourSet, nvp: u32) -> Result<PolyMesh, CookError> {
    let mut all_verts: Vec<Vec3> = Vec::new();
    let mut all_polys: Vec<PolyPolygon> = Vec::new();

    for contour in &contours.conts {
        if contour.verts.len() < 3 {
            continue;
        }

        // Map contour vertices into the global vertex pool (de-dup).
        let mut local_to_global: Vec<u16> = Vec::with_capacity(contour.verts.len());

        for &v in &contour.verts {
            // Check for an existing vertex at the same position (within epsilon).
            let existing = all_verts.iter().position(|existing_v| {
                (existing_v.x - v.x).abs() < 0.001
                    && (existing_v.y - v.y).abs() < 0.001
                    && (existing_v.z - v.z).abs() < 0.001
            });

            match existing {
                Some(idx) => local_to_global.push(idx as u16),
                None => {
                    let idx = all_verts.len() as u16;
                    all_verts.push(v);
                    local_to_global.push(idx);
                }
            }
        }

        // Ensure CCW winding.
        let cw = is_cw_xz(&contour.verts);
        let verts_ccw: Vec<Vec3> = if cw {
            contour.verts.iter().copied().rev().collect()
        } else {
            contour.verts.clone()
        };
        // Also reverse the index mapping if we reversed vertices.
        let gbl_ccw: Vec<u16> = if cw {
            local_to_global.iter().copied().rev().collect()
        } else {
            local_to_global
        };

        // Ear-clip triangulate.
        let tris = ear_clip_triangulate(&verts_ccw);
        if tris.is_empty() {
            continue;
        }

        // Remap triangle indices to global vertex indices.
        let global_tris: Vec<(u32, u32, u32)> = tris
            .iter()
            .map(|&(a, b, c)| {
                (
                    gbl_ccw[a as usize] as u32,
                    gbl_ccw[b as usize] as u32,
                    gbl_ccw[c as usize] as u32,
                )
            })
            .collect();

        // Merge into convex polygons.
        let merged = merge_convex_polys(&global_tris, &all_verts, nvp);

        // Add to output, assigning sequential polygon indices.
        for poly_verts in &merged {
            let poly = PolyPolygon {
                verts: poly_verts.iter().map(|&i| i as u16).collect(),
                neighbors: Vec::new(), // filled below
                area: contour.area,
            };
            all_polys.push(poly);
        }
    }

    // ── Compute neighbour connectivity ────────────────────────────────────
    //
    // Two polygons are neighbours if they share at least 2 vertex indices.
    // This mirrors how `NavMesh::add_polygon` detects adjacency.
    for i in 0..all_polys.len() {
        for j in (i + 1)..all_polys.len() {
            let shared = all_polys[i]
                .verts
                .iter()
                .filter(|v| all_polys[j].verts.contains(v))
                .count();
            if shared >= 2 {
                all_polys[i].neighbors.push(j as u16);
                all_polys[j].neighbors.push(i as u16);
            }
        }
    }

    if all_polys.is_empty() {
        return Err(CookError::PolyMeshGenerationFailed(
            "no polygons generated from contours".into(),
        ));
    }

    Ok(PolyMesh {
        verts: all_verts,
        polys: all_polys,
        bmin: contours.bmin,
        cs: contours.cs,
        ch: contours.ch,
    })
}

// ── Winding detection ─────────────────────────────────────────────────────

/// Returns `true` if the polygon vertices are in clockwise order (XZ plane).
fn is_cw_xz(verts: &[Vec3]) -> bool {
    let mut area2: f32 = 0.0;
    let n = verts.len();
    for i in 0..n {
        let a = verts[i];
        let b = verts[(i + 1) % n];
        area2 += (b.x - a.x) * (b.z + a.z);
    }
    area2 > 0.0
}

// ── Ear-clipping triangulation ────────────────────────────────────────────

/// Triangulate a simple **CCW** polygon by ear-clipping.
///
/// Returns a list of vertex-index triplets `(a, b, c)` that define triangles.
/// The indices are into the input `verts` slice.
fn ear_clip_triangulate(verts: &[Vec3]) -> Vec<(u32, u32, u32)> {
    let n = verts.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![(0, 1, 2)];
    }

    // Build doubly-linked list.
    let mut prev: Vec<u32> = Vec::with_capacity(n);
    let mut next: Vec<u32> = Vec::with_capacity(n);
    for i in 0..n as u32 {
        prev.push(if i == 0 { n as u32 - 1 } else { i - 1 });
        next.push(if i == n as u32 - 1 { 0 } else { i + 1 });
    }

    // Pre-compute signed area to know the winding.
    let winding_ccw = !is_cw_xz(verts);

    let mut triangles = Vec::with_capacity(n.saturating_sub(2));
    let mut remaining = n as u32;
    let mut iterations = 0u32;
    let max_iter = (n as u32) * 4; // safety limit

    let mut idx: u32 = 0;
    while remaining > 3 && iterations < max_iter {
        iterations += 1;

        let p = prev[idx as usize];
        let nx = next[idx as usize];

        if is_ear(verts, &prev, &next, idx, p, nx, winding_ccw) {
            triangles.push((p, idx, nx));
            // Remove idx from the linked list.
            next[p as usize] = nx;
            prev[nx as usize] = p;
            remaining -= 1;
        }

        idx = next[idx as usize];
    }

    // Grab the final triangle.
    if remaining == 3 && triangles.len() + 1 == n - 2 {
        // Find the three remaining vertices.
        let v0 = idx;
        let v1 = next[v0 as usize];
        let v2 = next[v1 as usize];
        // Ensure we actually have the last three.
        if v2 != v0 {
            triangles.push((v0, v1, v2));
        }
    } else if remaining > 0 {
        // Fallback: walk the list and grab whatever's left.
        let mut seen = std::collections::HashSet::new();
        let mut cur = idx;
        while seen.insert(cur) {
            let nx = next[cur as usize];
            if nx == cur {
                break;
            }
            let nnx = next[nx as usize];
            if nnx != cur && nnx != nx {
                triangles.push((cur, nx, nnx));
            }
            cur = nnx;
            if cur == idx {
                break;
            }
        }
    }

    triangles
}

/// Check whether vertex `i` is an ear of the polygon.
///
/// A vertex is an ear if:
/// 1. The interior angle is convex (same sign as polygon winding).
/// 2. No other vertex of the polygon lies strictly inside the triangle
///    `(prev[i], i, next[i])`.
fn is_ear(
    verts: &[Vec3],
    _prev: &[u32],
    next: &[u32],
    i: u32,
    p: u32,
    n: u32,
    winding_ccw: bool,
) -> bool {
    let a = verts[p as usize];
    let b = verts[i as usize];
    let c = verts[n as usize];

    // 1. Convexity test (interior angle < 180°).
    // Cross product of (b - a) × (c - b) on the XZ plane.
    let cross = (b.x - a.x) * (c.z - b.z) - (b.z - a.z) * (c.x - b.x);

    if winding_ccw {
        if cross <= 1e-8 {
            return false; // reflex or collinear
        }
    } else {
        if cross >= -1e-8 {
            return false;
        }
    }

    // 2. No other vertex inside triangle (a, b, c).
    for j in 0..verts.len() {
        if j == p as usize || j == i as usize || j == n as usize {
            continue;
        }
        // Skip vertices that have already been removed (self-loop in linked list).
        let j_idx = j as u32;
        if next[j_idx as usize] == j_idx {
            continue;
        }

        let q = verts[j];
        if point_in_triangle_xz(a, b, c, q, winding_ccw) {
            return false;
        }
    }

    true
}

/// Test whether point `q` is strictly inside the CCW triangle `(a, b, c)`
/// (or strictly inside the CW triangle, depending on `ccw`).
fn point_in_triangle_xz(a: Vec3, b: Vec3, c: Vec3, q: Vec3, ccw: bool) -> bool {
    // Edge functions.
    let e0 = (b.x - a.x) * (q.z - a.z) - (b.z - a.z) * (q.x - a.x); // cross(b-a, q-a)
    let e1 = (c.x - b.x) * (q.z - b.z) - (c.z - b.z) * (q.x - b.x); // cross(c-b, q-b)
    let e2 = (a.x - c.x) * (q.z - c.z) - (a.z - c.z) * (q.x - c.x); // cross(a-c, q-c)

    if ccw {
        // Strictly inside: all positive (tolerance for float errors).
        e0 > -1e-8 && e1 > -1e-8 && e2 > -1e-8
    } else {
        // CW: all negative.
        e0 < 1e-8 && e1 < 1e-8 && e2 < 1e-8
    }
}

// ── Convex merging (Hertel–Mehlhorn style) ────────────────────────────────

/// Greedily merge triangles into convex polygons with at most `nvp`
/// vertices per polygon.
///
/// Each input triangle is `(v0, v1, v2)` where the indices refer to `verts`.
/// The function starts with each triangle as its own polygon and iteratively
/// merges any pair that shares a full edge and whose union remains convex.
fn merge_convex_polys(
    tris: &[(u32, u32, u32)],
    verts: &[Vec3],
    nvp: u32,
) -> Vec<Vec<u32>> {
    if tris.is_empty() {
        return Vec::new();
    }

    // Seed with triangles.
    let mut polys: Vec<Vec<u32>> = tris
        .iter()
        .map(|&(a, b, c)| vec![a, b, c])
        .collect();

    // Greedy merge loop.
    loop {
        // Build edge → polygon-owner map.
        let owner_map = build_edge_owner_map(&polys);

        // Find an edge shared by exactly 2 polygons.
        let candidates: Vec<((u32, u32), Vec<usize>)> = owner_map
            .into_iter()
            .filter(|(_, owners)| owners.len() == 2)
            .collect();

        let mut merged_any = false;

        for (_edge, owners) in &candidates {
            let i = owners[0];
            let j = owners[1];
            if i == j {
                continue;
            }

            if let Some(merged) = try_merge(&polys[i], &polys[j], verts, nvp) {
                // Replace i and j with the merged polygon.
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

    // Filter degenerate polygons.
    polys.retain(|p| p.len() >= 3);
    polys
}

/// Build a map from edge (sorted vertex-index pair) to the list of polygon
/// indices that contain that edge.
fn build_edge_owner_map(polys: &[Vec<u32>]) -> std::collections::HashMap<(u32, u32), Vec<usize>> {
    let mut map: std::collections::HashMap<(u32, u32), Vec<usize>> = std::collections::HashMap::new();

    for (pi, poly) in polys.iter().enumerate() {
        let n = poly.len();
        for j in 0..n {
            let a = poly[j];
            let b = poly[(j + 1) % n];
            let edge = if a < b { (a, b) } else { (b, a) };
            map.entry(edge).or_default().push(pi);
        }
    }

    map
}

/// Try to merge two convex polygons that share a full edge.
///
/// Returns `Some(merged)` if the merge is valid (result is convex and
/// does not exceed `nvp` vertices).
fn try_merge(
    p1: &[u32],
    p2: &[u32],
    verts: &[Vec3],
    nvp: u32,
) -> Option<Vec<u32>> {
    // Check vertex count limit.
    let total_verts = p1.len() + p2.len() - 2; // shared edge counted twice
    if total_verts > nvp as usize {
        return None;
    }

    // Find shared vertices.
    let shared: Vec<u32> = p1.iter().filter(|v| p2.contains(v)).copied().collect();
    if shared.len() != 2 {
        return None;
    }
    let s0 = shared[0];
    let s1 = shared[1];

    // Verify they form a full edge in both polygons.
    if !verts_form_edge_u32(s0, s1, p1) || !verts_form_edge_u32(s0, s1, p2) {
        return None;
    }

    let p1_s0 = p1.iter().position(|&v| v == s0).unwrap();
    let p1_s1 = p1.iter().position(|&v| v == s1).unwrap();
    let p2_s0 = p2.iter().position(|&v| v == s0).unwrap();
    let p2_s1 = p2.iter().position(|&v| v == s1).unwrap();

    // Try both orientations.
    let ma = build_merged_ring(p1, p2, p1_s0, p1_s1, p2_s1, p2_s0);
    let ma_clean = clean_duplicates_u32(&ma);
    if is_convex_xz_u32(&ma_clean, verts) && ma_clean.len() >= 3 {
        return Some(ma_clean);
    }

    let mb = build_merged_ring(p1, p2, p1_s1, p1_s0, p2_s0, p2_s1);
    let mb_clean = clean_duplicates_u32(&mb);
    if is_convex_xz_u32(&mb_clean, verts) && mb_clean.len() >= 3 {
        return Some(mb_clean);
    }

    None
}

/// Build the merged vertex ring by walking two polygons.
fn build_merged_ring(
    p1: &[u32],
    p2: &[u32],
    p1_start: usize,
    p1_end: usize,
    p2_start_after: usize,
    p2_end_before: usize,
) -> Vec<u32> {
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
fn verts_form_edge_u32(a: u32, b: u32, poly: &[u32]) -> bool {
    let n = poly.len();
    if n < 2 {
        return false;
    }
    let Some(pa) = poly.iter().position(|&v| v == a) else {
        return false;
    };
    let Some(pb) = poly.iter().position(|&v| v == b) else {
        return false;
    };
    (pa + 1) % n == pb || (pb + 1) % n == pa
}

/// Check that a polygon is convex in the XZ plane (all cross products have
/// the same sign).
fn is_convex_xz_u32(poly: &[u32], verts: &[Vec3]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }

    let mut sign: Option<f32> = None;
    for i in 0..n {
        let a = verts[poly[i] as usize];
        let b = verts[poly[(i + 1) % n] as usize];
        let c = verts[poly[(i + 2) % n] as usize];

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

/// Remove consecutive duplicate vertices from a polygon.
fn clean_duplicates_u32(poly: &[u32]) -> Vec<u32> {
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

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::contour::Contour;

    fn make_contour(verts: Vec<Vec3>, reg: u16, area: u8) -> Contour {
        Contour { verts, reg, area }
    }

    fn make_contour_set(conts: Vec<Contour>) -> ContourSet {
        ContourSet {
            conts,
            width: 10,
            height: 10,
            bmin: Vec3::ZERO,
            cs: 1.0,
            ch: 1.0,
            border_size: 0,
        }
    }

    #[test]
    fn single_triangle_in_one_tri_out() {
        let c = make_contour(
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            1,
            1,
        );
        let cs = make_contour_set(vec![c]);
        let result = build_poly_mesh(&cs, 6);
        assert!(result.is_ok());
        let pm = result.unwrap();
        assert_eq!(pm.polys.len(), 1, "single triangle → 1 poly");
        assert_eq!(pm.polys[0].verts.len(), 3, "poly should be a triangle");
    }

    #[test]
    fn quad_becomes_four_verts() {
        // A square region → should produce 1 quad after merge.
        let c = make_contour(
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 2.0),
                Vec3::new(0.0, 0.0, 2.0),
            ],
            1,
            1,
        );
        let cs = make_contour_set(vec![c]);
        let result = build_poly_mesh(&cs, 6);
        assert!(result.is_ok());
        let pm = result.unwrap();
        // The ear-clip produces 2 triangles → merged into 1 quad.
        assert!(
            pm.polys.len() >= 1,
            "should produce at least 1 polygon"
        );
        // The quad should have 4 vertices.
        assert!(
            pm.polys[0].verts.len() == 4,
            "merged quad should have 4 vertices, got {}",
            pm.polys[0].verts.len()
        );
    }

    #[test]
    fn l_shape_triangulates() {
        // A simple L-shaped contour.
        let c = make_contour(
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(2.0, 0.0, 1.0),
                Vec3::new(3.0, 0.0, 1.0),
                Vec3::new(3.0, 0.0, 3.0),
                Vec3::new(0.0, 0.0, 3.0),
            ],
            1,
            1,
        );
        let cs = make_contour_set(vec![c]);
        let result = build_poly_mesh(&cs, 6);
        assert!(result.is_ok());
        let pm = result.unwrap();
        assert!(
            pm.polys.len() >= 1,
            "L-shape should produce at least 1 polygon, got {}",
            pm.polys.len()
        );
        // Total polygon vertices should cover the shape.
        let total_verts: usize = pm.polys.iter().map(|p| p.verts.len()).sum();
        assert!(total_verts >= 4, "should have at least 4 poly verts");
    }

    #[test]
    fn nvp_limits_polygon_size() {
        // A hexagon → with nvp=4 should produce at least 2 polys.
        let c = make_contour(
            vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(3.0, 0.0, 0.0),
                Vec3::new(4.0, 0.0, 1.0),
                Vec3::new(4.0, 0.0, 3.0),
                Vec3::new(3.0, 0.0, 4.0),
                Vec3::new(0.0, 0.0, 4.0),
            ],
            1,
            1,
        );
        let cs = make_contour_set(vec![c]);
        let pm = build_poly_mesh(&cs, 4).unwrap();
        // With nvp=4, all polygons should have ≤ 4 verts.
        for poly in &pm.polys {
            assert!(
                poly.verts.len() <= 4,
                "poly has {} verts, exceeds nvp=4",
                poly.verts.len()
            );
        }
        // Should produce multiple polygons.
        assert!(
            pm.polys.len() >= 2,
            "hexagon with nvp=4 should produce multiple polys, got {}",
            pm.polys.len()
        );
    }

    #[test]
    fn empty_contour_set_returns_error() {
        let cs = make_contour_set(vec![]);
        let result = build_poly_mesh(&cs, 6);
        assert!(result.is_err());
    }

    #[test]
    fn ear_clip_triangle() {
        let verts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 2.0),
        ];
        let tris = ear_clip_triangulate(&verts);
        assert_eq!(tris.len(), 1, "triangle → 1 ear-triple");
        assert_eq!(tris[0], (0, 1, 2));
    }

    #[test]
    fn merge_quads_from_tris() {
        let verts = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 2.0),
            Vec3::new(0.0, 0.0, 2.0),
        ];
        // Two triangles forming a quad.
        let tris = vec![(0u32, 1u32, 3u32), (1u32, 2u32, 3u32)];
        let merged = merge_convex_polys(&tris, &verts, 6);
        assert_eq!(merged.len(), 1, "two quad tris → 1 merged poly");
        assert_eq!(
            merged[0].len(),
            4,
            "merged poly should have 4 verts, got {}",
            merged[0].len()
        );
    }
}
