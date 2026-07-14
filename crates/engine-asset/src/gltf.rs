//! Strict static glTF 2.0 scene importer.
//!
//! The importer preserves document indices, expands every mesh primitive into
//! an explicit [`GltfPrimitive`], and resolves the selected scene's complete
//! world transforms. The first static implementation intentionally accepts
//! triangle lists only and rejects skinning and morph targets with structured
//! errors.

use std::path::{Path, PathBuf};

use glam::{Mat4, Vec2, Vec3};
use thiserror::Error;

use crate::mesh::MeshData;

/// PBR material properties extracted from a glTF material.
#[derive(Clone, Debug)]
pub struct GltfMaterial {
    /// Original glTF material index.
    pub material_index: usize,
    pub name: String,
    pub base_color: [f32; 4],
    /// Original glTF texture index, or `None`.
    pub base_color_texture: Option<usize>,
    pub metallic: f32,
    pub roughness: f32,
    /// Original glTF texture index, or `None`.
    pub metallic_roughness_texture: Option<usize>,
    pub emissive: [f32; 3],
    /// Original glTF texture index, or `None`.
    pub emissive_texture: Option<usize>,
    /// Original glTF texture index, or `None`.
    pub normal_texture: Option<usize>,
    pub alpha_mode: gltf::material::AlphaMode,
    pub alpha_cutoff: Option<f32>,
    pub double_sided: bool,
}

/// Pixel storage produced by the importer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GltfTextureFormat {
    Rgba8,
}

/// Sampler state attached to one glTF texture object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GltfSampler {
    /// Original sampler index. `None` means the glTF default sampler.
    pub sampler_index: Option<usize>,
    pub mag_filter: Option<gltf::texture::MagFilter>,
    pub min_filter: Option<gltf::texture::MinFilter>,
    pub wrap_s: gltf::texture::WrappingMode,
    pub wrap_t: gltf::texture::WrappingMode,
}

/// One decoded glTF texture object.
///
/// A texture is kept separate from its source image so two texture objects
/// sharing one image but using different samplers remain distinct.
#[derive(Clone, Debug)]
pub struct GltfTexture {
    /// Original glTF texture index.
    pub texture_index: usize,
    /// Original glTF image index.
    pub image_index: usize,
    pub sampler: GltfSampler,
    pub format: GltfTextureFormat,
    /// Tightly packed RGBA8 pixels.
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// One glTF mesh primitive with stable source mappings.
#[derive(Clone, Debug)]
pub struct GltfPrimitive {
    pub name: String,
    pub mesh: MeshData,
    pub material_index: Option<usize>,
    pub topology: gltf::mesh::Mode,
    pub source_mesh_index: usize,
    pub source_primitive_index: usize,
}

/// A node belonging to the selected glTF scene.
#[derive(Clone, Debug)]
pub struct GltfNode {
    /// Original glTF node index.
    pub source_node_index: usize,
    pub name: String,
    /// World-space transform accumulated from the complete parent chain.
    pub transform: Mat4,
    /// All primitive indices referenced by this node.
    pub primitive_indices: Vec<usize>,
    /// Compatibility alias for the first element of `primitive_indices`.
    pub mesh_index: Option<usize>,
    /// Compatibility alias for the first primitive's material.
    pub material_index: Option<usize>,
    /// Child indices into the owning scene's `nodes` vector.
    pub children: Vec<usize>,
}

/// The complete static contents of a glTF file after import.
#[derive(Clone, Debug)]
pub struct GltfScene {
    /// The selected default scene index, or the first scene when no default is declared.
    pub selected_scene_index: Option<usize>,
    pub primitives: Vec<GltfPrimitive>,
    /// Compatibility view containing one mesh per primitive in identical order.
    pub meshes: Vec<MeshData>,
    pub materials: Vec<GltfMaterial>,
    /// One entry per glTF texture, in original document order.
    pub textures: Vec<GltfTexture>,
    /// Only nodes reachable from the selected scene.
    pub nodes: Vec<GltfNode>,
    pub roots: Vec<usize>,
}

/// Structured failures from the strict static glTF importer.
#[derive(Debug, Error)]
pub enum GltfImportError {
    #[error("failed to open glTF {path}: {detail}", path = .path.display())]
    Open { path: PathBuf, detail: String },

    #[error("failed to load buffers for glTF {path}: {detail}", path = .path.display())]
    BufferLoad { path: PathBuf, detail: String },

    #[error(
        "glTF texture {texture_index} references image {image_index} at {image_source}, but decoding failed: {detail}"
    )]
    TextureDecode {
        texture_index: usize,
        image_index: usize,
        image_source: String,
        detail: String,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} uses unsupported topology {topology:?}; only triangle-list is supported"
    )]
    UnsupportedTopology {
        mesh_index: usize,
        primitive_index: usize,
        topology: gltf::mesh::Mode,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} uses morph targets, which are unsupported by the static importer"
    )]
    UnsupportedMorphTargets {
        mesh_index: usize,
        primitive_index: usize,
    },

    #[error(
        "glTF node {node_index} ({node_name}) uses skin {skin_index}, which is unsupported by the static importer"
    )]
    UnsupportedSkin {
        node_index: usize,
        node_name: String,
        skin_index: usize,
    },

    #[error("glTF mesh {mesh_index} primitive {primitive_index} has no POSITION attribute")]
    MissingPositions {
        mesh_index: usize,
        primitive_index: usize,
    },

    #[error("glTF mesh {mesh_index} primitive {primitive_index} has an empty POSITION attribute")]
    EmptyPositions {
        mesh_index: usize,
        primitive_index: usize,
    },
}

/// Load one static scene from a glTF 2.0 file.
pub fn load_gltf_scene(path: &Path) -> Result<GltfScene, GltfImportError> {
    let parsed = gltf::Gltf::open(path).map_err(|error| GltfImportError::Open {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    let gltf::Gltf { document, blob } = parsed;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let buffers = gltf::import_buffers(&document, Some(base), blob).map_err(|error| {
        GltfImportError::BufferLoad {
            path: path.to_path_buf(),
            detail: error.to_string(),
        }
    })?;

    validate_static_nodes(&document)?;

    let materials = document
        .materials()
        .map(|material| extract_material(&material))
        .collect();
    let textures = load_textures(&document, base, &buffers)?;

    let mesh_count = document.meshes().len();
    let mut primitives = Vec::new();
    let mut mesh_to_primitives = vec![Vec::new(); mesh_count];

    for source_mesh in document.meshes() {
        let source_mesh_index = source_mesh.index();
        let source_mesh_name = source_mesh.name().unwrap_or("mesh");
        for (source_primitive_index, primitive) in source_mesh.primitives().enumerate() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                return Err(GltfImportError::UnsupportedTopology {
                    mesh_index: source_mesh_index,
                    primitive_index: source_primitive_index,
                    topology: primitive.mode(),
                });
            }
            if primitive.morph_targets().next().is_some() {
                return Err(GltfImportError::UnsupportedMorphTargets {
                    mesh_index: source_mesh_index,
                    primitive_index: source_primitive_index,
                });
            }

            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            let positions: Vec<Vec3> = reader
                .read_positions()
                .ok_or(GltfImportError::MissingPositions {
                    mesh_index: source_mesh_index,
                    primitive_index: source_primitive_index,
                })?
                .map(Vec3::from_array)
                .collect();
            if positions.is_empty() {
                return Err(GltfImportError::EmptyPositions {
                    mesh_index: source_mesh_index,
                    primitive_index: source_primitive_index,
                });
            }

            let normals = reader
                .read_normals()
                .map(|values| values.map(Vec3::from_array).collect())
                .unwrap_or_else(|| vec![Vec3::Y; positions.len()]);
            let uvs = reader
                .read_tex_coords(0)
                .map(|values| values.into_f32().map(Vec2::from_array).collect())
                .unwrap_or_default();
            let indices = reader
                .read_indices()
                .map(|values| values.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());
            let material_index = primitive.material().index();
            let primitive_index = primitives.len();
            let bounds = compute_bounds(&positions);

            primitives.push(GltfPrimitive {
                name: format!("{source_mesh_name}_{source_primitive_index}"),
                mesh: MeshData {
                    positions,
                    normals,
                    uvs,
                    indices,
                    bounds,
                    joints: Vec::new(),
                    weights: Vec::new(),
                },
                material_index,
                topology: gltf::mesh::Mode::Triangles,
                source_mesh_index,
                source_primitive_index,
            });
            mesh_to_primitives[source_mesh_index].push(primitive_index);
        }
    }

    let selected_scene = document
        .default_scene()
        .or_else(|| document.scenes().next());
    let selected_scene_index = selected_scene.as_ref().map(gltf::Scene::index);
    let mut nodes = Vec::new();
    let mut roots = Vec::new();
    if let Some(scene) = selected_scene {
        for node in scene.nodes() {
            flatten_node(
                &node,
                Mat4::IDENTITY,
                &mut nodes,
                &mut roots,
                &primitives,
                &mesh_to_primitives,
                true,
            );
        }
    }

    let meshes = primitives
        .iter()
        .map(|primitive| primitive.mesh.clone())
        .collect();

    Ok(GltfScene {
        selected_scene_index,
        primitives,
        meshes,
        materials,
        textures,
        nodes,
        roots,
    })
}

fn validate_static_nodes(document: &gltf::Document) -> Result<(), GltfImportError> {
    for node in document.nodes() {
        if let Some(skin) = node.skin() {
            return Err(GltfImportError::UnsupportedSkin {
                node_index: node.index(),
                node_name: node.name().unwrap_or("node").to_string(),
                skin_index: skin.index(),
            });
        }
    }
    Ok(())
}

fn extract_material(material: &gltf::Material<'_>) -> GltfMaterial {
    let pbr = material.pbr_metallic_roughness();
    GltfMaterial {
        material_index: material
            .index()
            .expect("document.materials() never yields the default material"),
        name: material.name().unwrap_or("material").to_string(),
        base_color: pbr.base_color_factor(),
        base_color_texture: pbr.base_color_texture().map(|info| info.texture().index()),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        metallic_roughness_texture: pbr
            .metallic_roughness_texture()
            .map(|info| info.texture().index()),
        emissive: material.emissive_factor(),
        emissive_texture: material
            .emissive_texture()
            .map(|info| info.texture().index()),
        normal_texture: material.normal_texture().map(|info| info.texture().index()),
        alpha_mode: material.alpha_mode(),
        alpha_cutoff: material.alpha_cutoff(),
        double_sided: material.double_sided(),
    }
}

fn load_textures(
    document: &gltf::Document,
    base: &Path,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<GltfTexture>, GltfImportError> {
    let mut textures = Vec::with_capacity(document.textures().len());
    for texture in document.textures() {
        let texture_index = texture.index();
        let image = texture.source();
        let image_index = image.index();
        let source = image.source();
        let source_label = image_source_label(&source, base);
        let decoded =
            gltf::image::Data::from_source(source, Some(base), buffers).map_err(|error| {
                GltfImportError::TextureDecode {
                    texture_index,
                    image_index,
                    image_source: source_label,
                    detail: error.to_string(),
                }
            })?;
        let sampler = texture.sampler();
        textures.push(decode_gltf_image(
            decoded,
            texture_index,
            image_index,
            GltfSampler {
                sampler_index: sampler.index(),
                mag_filter: sampler.mag_filter(),
                min_filter: sampler.min_filter(),
                wrap_s: sampler.wrap_s(),
                wrap_t: sampler.wrap_t(),
            },
        ));
    }
    Ok(textures)
}

fn image_source_label(source: &gltf::image::Source<'_>, base: &Path) -> String {
    match source {
        gltf::image::Source::Uri { uri, .. } if uri.starts_with("data:") => {
            "embedded data URI".to_string()
        }
        gltf::image::Source::Uri { uri, .. } => base.join(uri).display().to_string(),
        gltf::image::Source::View { view, .. } => format!("bufferView {}", view.index()),
    }
}

fn flatten_node(
    node: &gltf::Node<'_>,
    parent_transform: Mat4,
    nodes: &mut Vec<GltfNode>,
    roots: &mut Vec<usize>,
    primitives: &[GltfPrimitive],
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
    let mesh_index = primitive_indices.first().copied();
    let material_index = mesh_index.and_then(|index| primitives[index].material_index);
    let node_index = nodes.len();

    nodes.push(GltfNode {
        source_node_index: node.index(),
        name: node.name().unwrap_or("node").to_string(),
        transform,
        primitive_indices,
        mesh_index,
        material_index,
        children: Vec::new(),
    });
    if is_root {
        roots.push(node_index);
    }

    for child in node.children() {
        let child_index = flatten_node(
            &child,
            transform,
            nodes,
            roots,
            primitives,
            mesh_to_primitives,
            false,
        );
        nodes[node_index].children.push(child_index);
    }
    node_index
}

fn compute_bounds(positions: &[Vec3]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for position in positions {
        min = min.min(*position);
        max = max.max(*position);
    }
    (min, max)
}

fn decode_gltf_image(
    image: gltf::image::Data,
    texture_index: usize,
    image_index: usize,
    sampler: GltfSampler,
) -> GltfTexture {
    let pixel_count = image.width as usize * image.height as usize;
    let mut data = Vec::with_capacity(pixel_count * 4);
    match image.format {
        gltf::image::Format::R8 => {
            for &red in &image.pixels {
                data.extend_from_slice(&[red, red, red, 255]);
            }
        }
        gltf::image::Format::R8G8 => {
            for pixel in image.pixels.chunks_exact(2) {
                data.extend_from_slice(&[pixel[0], pixel[1], 0, 255]);
            }
        }
        gltf::image::Format::R8G8B8 => {
            for pixel in image.pixels.chunks_exact(3) {
                data.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        gltf::image::Format::R8G8B8A8 => data = image.pixels,
        gltf::image::Format::R16 => {
            for pixel in image.pixels.chunks_exact(2) {
                let red = u16_to_u8(pixel);
                data.extend_from_slice(&[red, red, red, 255]);
            }
        }
        gltf::image::Format::R16G16 => {
            for pixel in image.pixels.chunks_exact(4) {
                data.extend_from_slice(&[u16_to_u8(pixel), u16_to_u8(&pixel[2..]), 0, 255]);
            }
        }
        gltf::image::Format::R16G16B16 => {
            for pixel in image.pixels.chunks_exact(6) {
                data.extend_from_slice(&[
                    u16_to_u8(pixel),
                    u16_to_u8(&pixel[2..]),
                    u16_to_u8(&pixel[4..]),
                    255,
                ]);
            }
        }
        gltf::image::Format::R16G16B16A16 => {
            for pixel in image.pixels.chunks_exact(8) {
                data.extend_from_slice(&[
                    u16_to_u8(pixel),
                    u16_to_u8(&pixel[2..]),
                    u16_to_u8(&pixel[4..]),
                    u16_to_u8(&pixel[6..]),
                ]);
            }
        }
        gltf::image::Format::R32G32B32FLOAT => {
            for pixel in image.pixels.chunks_exact(12) {
                data.extend_from_slice(&[
                    f32_to_u8(pixel),
                    f32_to_u8(&pixel[4..]),
                    f32_to_u8(&pixel[8..]),
                    255,
                ]);
            }
        }
        gltf::image::Format::R32G32B32A32FLOAT => {
            for pixel in image.pixels.chunks_exact(16) {
                data.extend_from_slice(&[
                    f32_to_u8(pixel),
                    f32_to_u8(&pixel[4..]),
                    f32_to_u8(&pixel[8..]),
                    f32_to_u8(&pixel[12..]),
                ]);
            }
        }
    }
    debug_assert_eq!(data.len(), pixel_count * 4);

    GltfTexture {
        texture_index,
        image_index,
        sampler,
        format: GltfTextureFormat::Rgba8,
        data,
        width: image.width,
        height: image.height,
    }
}

fn u16_to_u8(bytes: &[u8]) -> u8 {
    (u16::from_ne_bytes([bytes[0], bytes[1]]) / 257) as u8
}

fn f32_to_u8(bytes: &[u8]) -> u8 {
    (f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).clamp(0.0, 1.0) * 255.0).round()
        as u8
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use glam::{Quat, Vec3};

    use super::*;

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn model_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/models")
            .join(name)
    }

    fn temp_dir(test_name: &str) -> PathBuf {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "engine-asset-{test_name}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test directory");
        path
    }

    fn assert_mat4_close(actual: Mat4, expected: Mat4) {
        for (actual, expected) in actual
            .to_cols_array()
            .into_iter()
            .zip(expected.to_cols_array())
        {
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }
    }

    #[test]
    fn load_triangle_gltf_keeps_compatibility_view() {
        let scene =
            load_gltf_scene(&model_path("triangle.gltf")).expect("triangle.gltf should load");
        assert_eq!(scene.primitives.len(), 1);
        assert_eq!(scene.meshes.len(), 1);
        assert_eq!(scene.materials.len(), 0);
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.roots, vec![0]);
        assert_eq!(scene.nodes[0].primitive_indices, vec![0]);
        assert_eq!(scene.nodes[0].mesh_index, Some(0));
        assert_eq!(scene.meshes[0].positions.len(), 3);
        assert!((scene.meshes[0].positions[0].x + 1.0).abs() < 0.001);
    }

    #[test]
    fn legacy_mesh_entry_points_use_the_strict_primitive_chain() {
        let path = model_path("resource-chain.gltf");
        let first = crate::mesh::load_mesh_from_gltf(&path).expect("load first primitive");
        let all = crate::mesh::load_meshes_from_gltf(&path).expect("load all primitives");

        assert_eq!(first.positions.len(), 3);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0, "SharedMesh_0");
        assert_eq!(all[1].0, "SharedMesh_1");
    }

    #[test]
    fn resource_chain_preserves_all_indices_and_instances() {
        let scene = load_gltf_scene(&model_path("resource-chain.gltf"))
            .expect("resource-chain.gltf should load");

        assert_eq!(scene.selected_scene_index, Some(1));
        assert_eq!(scene.primitives.len(), 2);
        assert_eq!(scene.meshes.len(), 2);
        assert_eq!(scene.materials.len(), 2);
        assert_eq!(scene.textures.len(), 2);
        assert_eq!(scene.nodes.len(), 2, "scene 0 decoy must not be selected");
        assert_eq!(scene.roots, vec![0]);

        for (index, primitive) in scene.primitives.iter().enumerate() {
            assert_eq!(primitive.source_mesh_index, 0);
            assert_eq!(primitive.source_primitive_index, index);
            assert_eq!(primitive.material_index, Some(index));
            assert_eq!(primitive.topology, gltf::mesh::Mode::Triangles);
            assert_eq!(primitive.mesh.positions.len(), 3);
            assert_eq!(primitive.mesh.uvs.len(), 3);
        }

        let material0 = &scene.materials[0];
        assert_eq!(material0.material_index, 0);
        assert_eq!(material0.base_color_texture, Some(0));
        assert_eq!(material0.metallic, 0.25);
        assert_eq!(material0.roughness, 0.75);
        assert_eq!(material0.alpha_mode, gltf::material::AlphaMode::Mask);
        assert_eq!(material0.alpha_cutoff, Some(0.4));
        assert!(material0.double_sided);

        let material1 = &scene.materials[1];
        assert_eq!(material1.material_index, 1);
        assert_eq!(material1.base_color_texture, Some(1));
        assert_eq!(material1.alpha_mode, gltf::material::AlphaMode::Blend);
        assert!(!material1.double_sided);

        let texture0 = &scene.textures[0];
        let texture1 = &scene.textures[1];
        assert_eq!((texture0.texture_index, texture0.image_index), (0, 0));
        assert_eq!((texture1.texture_index, texture1.image_index), (1, 0));
        assert_eq!(texture0.format, GltfTextureFormat::Rgba8);
        assert_eq!(texture0.data, texture1.data);
        assert_eq!(texture0.data, [255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(texture0.sampler.sampler_index, Some(0));
        assert_eq!(texture1.sampler.sampler_index, Some(1));
        assert_eq!(
            texture0.sampler.mag_filter,
            Some(gltf::texture::MagFilter::Nearest)
        );
        assert_eq!(
            texture1.sampler.mag_filter,
            Some(gltf::texture::MagFilter::Linear)
        );
        assert_ne!(texture0.sampler.wrap_s, texture1.sampler.wrap_s);

        assert_eq!(scene.nodes[0].name, "RootInstance");
        assert_eq!(scene.nodes[1].name, "ChildInstance");
        assert_eq!(scene.nodes[0].primitive_indices, vec![0, 1]);
        assert_eq!(scene.nodes[1].primitive_indices, vec![0, 1]);
        assert_eq!(scene.nodes[0].mesh_index, Some(0));
        assert_eq!(scene.nodes[1].mesh_index, Some(0));
        assert_eq!(scene.nodes[0].children, vec![1]);

        let root_transform = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 1.0, 1.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(1.0, 2.0, 3.0),
        );
        let child_local = Mat4::from_scale_rotation_translation(
            Vec3::new(0.5, 2.0, 1.0),
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            Vec3::new(1.0, 1.0, 0.0),
        );
        assert_mat4_close(scene.nodes[0].transform, root_transform);
        assert_mat4_close(scene.nodes[1].transform, root_transform * child_local);
    }

    #[test]
    fn corrupt_second_texture_reports_original_indices_and_source() {
        let dir = temp_dir("corrupt-texture");
        let gltf_path = dir.join("corrupt.gltf");
        let broken_image = dir.join("broken.png");
        let json = r#"{
            "asset": { "version": "2.0" },
            "images": [
                { "uri": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=" },
                { "uri": "broken.png" }
            ],
            "textures": [ { "source": 0 }, { "source": 1 } ]
        }"#;
        fs::write(&gltf_path, json).expect("write test glTF");
        fs::write(&broken_image, b"not a png").expect("write corrupt image");

        let error = load_gltf_scene(&gltf_path).expect_err("second texture must fail");
        match error {
            GltfImportError::TextureDecode {
                texture_index,
                image_index,
                image_source,
                ..
            } => {
                assert_eq!(texture_index, 1);
                assert_eq!(image_index, 1);
                assert!(image_source.ends_with("broken.png"));
            }
            other => panic!("unexpected error: {other}"),
        }
        fs::remove_dir_all(dir).expect("remove test directory");
    }

    #[test]
    fn non_triangle_primitive_is_rejected_structurally() {
        let dir = temp_dir("non-triangle");
        let source = model_path("resource-chain.gltf");
        let gltf_path = dir.join("resource-chain.gltf");
        let json = fs::read_to_string(source).expect("read fixture").replacen(
            "\"mode\": 4",
            "\"mode\": 1",
            1,
        );
        fs::write(&gltf_path, json).expect("write modified fixture");
        fs::copy(
            model_path("resource-chain.bin"),
            dir.join("resource-chain.bin"),
        )
        .expect("copy buffer");
        fs::copy(
            model_path("resource-chain.png"),
            dir.join("resource-chain.png"),
        )
        .expect("copy texture");

        let error = load_gltf_scene(&gltf_path).expect_err("line primitive must fail");
        assert!(matches!(
            error,
            GltfImportError::UnsupportedTopology {
                mesh_index: 0,
                primitive_index: 0,
                topology: gltf::mesh::Mode::Lines,
            }
        ));
        fs::remove_dir_all(dir).expect("remove test directory");
    }
}
