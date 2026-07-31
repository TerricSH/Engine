use super::*;

pub(super) fn flatten_node(
    node: &gltf::Node<'_>,
    parent_transform: Mat4,
    nodes: &mut Vec<GltfNode>,
    roots: &mut Vec<usize>,
    mesh_to_primitives: &[Vec<usize>],
    is_root: bool,
) -> usize {
    let local_transform = Mat4::from_cols_array_2d(&node.transform().matrix());
    let transform = parent_transform * local_transform;
    let primitive_indices = node
        .mesh()
        .and_then(|mesh| mesh_to_primitives.get(mesh.index()))
        .cloned()
        .unwrap_or_default();
    let morph_weights = node
        .weights()
        .or_else(|| node.mesh().and_then(|mesh| mesh.weights()))
        .map(<[f32]>::to_vec)
        .unwrap_or_default();
    let node_index = nodes.len();

    nodes.push(GltfNode {
        source_node_index: node.index(),
        name: node.name().unwrap_or("node").to_string(),
        transform,
        primitive_indices,
        skin_index: node.skin().map(|skin| skin.index()),
        morph_weights,
        children: Vec::new(),
    });
    if is_root {
        roots.push(node_index);
    }

    for child in node.children() {
        let child_index = flatten_node(&child, transform, nodes, roots, mesh_to_primitives, false);
        nodes[node_index].children.push(child_index);
    }
    node_index
}

pub(super) fn compute_bounds(positions: &[Vec3]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for position in positions {
        min = min.min(*position);
        max = max.max(*position);
    }
    (min, max)
}

pub(super) fn generate_vertex_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let [a, b, c] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        let (Some(&pa), Some(&pb), Some(&pc)) =
            (positions.get(a), positions.get(b), positions.get(c))
        else {
            continue;
        };
        let face = (pb - pa).cross(pc - pa);
        if !face.is_finite() || face.length_squared() <= f32::EPSILON {
            continue;
        }
        normals[a] += face;
        normals[b] += face;
        normals[c] += face;
    }
    normals
        .into_iter()
        .map(|normal| normal.try_normalize().unwrap_or(Vec3::Y))
        .collect()
}
