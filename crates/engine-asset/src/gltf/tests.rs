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
fn step_animation_bakes_held_values_before_discontinuities() {
    let (times, values) = bake_vec3_animation_track(
        &[0.0, 1.0, 2.0],
        &[[0.0; 3], [10.0; 3], [20.0; 3]],
        gltf::animation::Interpolation::Step,
    );
    assert_eq!(times.len(), 5);
    assert_eq!(
        values,
        vec![[0.0; 3], [0.0; 3], [10.0; 3], [10.0; 3], [20.0; 3]]
    );
    assert_eq!(times[0], 0.0);
    assert!(times[1] < 1.0 && times[1] > 0.0);
    assert_eq!(times[2], 1.0);
    assert!(times[3] < 2.0 && times[3] > 1.0);
    assert_eq!(times[4], 2.0);
}

#[test]
fn cubic_vec3_animation_is_resampled_with_hermite_tangents() {
    let (times, values) = bake_vec3_animation_track(
        &[0.0, 1.0],
        &[[0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [1.0; 3], [0.0; 3]],
        gltf::animation::Interpolation::CubicSpline,
    );
    assert_eq!(times.len(), 61);
    assert!((times[15] - 0.25).abs() < 1.0e-6);
    assert!((values[15][0] - 0.15625).abs() < 1.0e-5);
    assert_eq!(values[60], [1.0; 3]);
}

#[test]
fn cubic_quaternion_animation_normalizes_every_baked_key() {
    let (_, values) = bake_quaternion_animation_track(
        &[0.0, 1.0],
        &[
            [0.0; 4],
            [0.0, 0.0, 0.0, 2.0],
            [0.0; 4],
            [0.0; 4],
            [0.0, 0.0, 2.0, 0.0],
            [0.0; 4],
        ],
        gltf::animation::Interpolation::CubicSpline,
    );
    assert_eq!(values.first().copied(), Some([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(values.last().copied(), Some([0.0, 0.0, 1.0, 0.0]));
    assert!(values.iter().all(|value| {
        let length_squared = value
            .iter()
            .map(|component| component * component)
            .sum::<f32>();
        (length_squared - 1.0).abs() < 1.0e-5
    }));
}

#[test]
fn gltf_cubic_spline_channel_imports_as_baked_linear_keys() {
    let dir = temp_dir("cubic-animation");
    let gltf_path = dir.join("cubic.gltf");
    let bin_path = dir.join("cubic.bin");
    let mut bytes = Vec::new();
    for time in [0.0f32, 1.0] {
        bytes.extend_from_slice(&time.to_le_bytes());
    }
    for value in [
        [0.0f32; 3],
        [0.0f32; 3],
        [0.0f32; 3],
        [0.0f32; 3],
        [1.0f32; 3],
        [0.0f32; 3],
    ] {
        for component in value {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    assert_eq!(bytes.len(), 80);
    fs::write(&bin_path, bytes).expect("write cubic animation buffer");
    fs::write(
            &gltf_path,
            r#"{
                "asset": { "version": "2.0" },
                "buffers": [{ "uri": "cubic.bin", "byteLength": 80 }],
                "bufferViews": [
                    { "buffer": 0, "byteOffset": 0, "byteLength": 8 },
                    { "buffer": 0, "byteOffset": 8, "byteLength": 72 }
                ],
                "accessors": [
                    { "bufferView": 0, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1] },
                    { "bufferView": 1, "componentType": 5126, "count": 6, "type": "VEC3" }
                ],
                "nodes": [{ "name": "Animated" }],
                "animations": [{
                    "name": "Ease",
                    "samplers": [{ "input": 0, "output": 1, "interpolation": "CUBICSPLINE" }],
                    "channels": [{ "sampler": 0, "target": { "node": 0, "path": "translation" } }]
                }],
                "scenes": [{ "nodes": [0] }],
                "scene": 0
            }"#,
        )
        .expect("write cubic glTF");

    let scene = load_gltf_scene(&gltf_path).expect("CUBICSPLINE glTF should load");
    let channel = &scene.animations[0].channels[0];
    assert_eq!(channel.times.len(), 61);
    let GltfAnimationValues::Translations(values) = &channel.values else {
        panic!("translation values expected");
    };
    assert!((values[15][0] - 0.15625).abs() < 1.0e-5);
    assert_eq!(values[60], [1.0; 3]);

    fs::remove_dir_all(dir).expect("remove test directory");
}

#[test]
fn load_triangle_gltf_exposes_canonical_primitive_data() {
    let scene = load_gltf_scene(&model_path("triangle.gltf")).expect("triangle.gltf should load");
    assert_eq!(scene.primitives.len(), 1);
    assert_eq!(scene.materials.len(), 0);
    assert_eq!(scene.nodes.len(), 1);
    assert_eq!(scene.roots, vec![0]);
    assert_eq!(scene.nodes[0].primitive_indices, vec![0]);
    assert_eq!(scene.primitives[0].mesh.positions.len(), 3);
    assert!((scene.primitives[0].mesh.positions[0].x + 1.0).abs() < 0.001);
}

#[test]
fn missing_normals_are_generated_from_triangle_geometry() {
    let normals = generate_vertex_normals(&[Vec3::ZERO, Vec3::X, Vec3::Y], &[0, 1, 2]);
    assert_eq!(normals.len(), 3);
    assert!(normals.iter().all(|normal| normal.z > 0.99));
}

#[test]
fn skinned_gltf_preserves_joint_weights_and_node_skin_binding() {
    let dir = temp_dir("skinned-mesh");
    let gltf_path = dir.join("skinned.gltf");
    let bin_path = dir.join("skinned.bin");
    let mut bytes = Vec::new();
    for position in [[-1.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for value in position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for _ in 0..3 {
        bytes.extend_from_slice(&[0, 1, 1, 1]);
    }
    for _ in 0..3 {
        for value in [0.75f32, 0.25, 0.0, 0.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for index in [0u16, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes.extend_from_slice(&[0, 0]);
    for _ in 0..2 {
        for column in 0..4 {
            for row in 0..4 {
                let value = if column == row { 1.0f32 } else { 0.0 };
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    for time in [0.0f32, 1.0] {
        bytes.extend_from_slice(&time.to_le_bytes());
    }
    for translation in [[0.0f32, 1.0, 0.0], [0.0, 2.0, 0.0]] {
        for value in translation {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    assert_eq!(bytes.len(), 264);
    fs::write(&bin_path, bytes).expect("write skinned buffer");
    fs::write(
            &gltf_path,
            r#"{
                "asset": { "version": "2.0" },
                "buffers": [{ "uri": "skinned.bin", "byteLength": 264 }],
                "bufferViews": [
                    { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
                    { "buffer": 0, "byteOffset": 36, "byteLength": 12 },
                    { "buffer": 0, "byteOffset": 48, "byteLength": 48 },
                    { "buffer": 0, "byteOffset": 96, "byteLength": 6 },
                    { "buffer": 0, "byteOffset": 104, "byteLength": 128 },
                    { "buffer": 0, "byteOffset": 232, "byteLength": 8 },
                    { "buffer": 0, "byteOffset": 240, "byteLength": 24 }
                ],
                "accessors": [
                    { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [-1, 0, 0], "max": [1, 1, 0] },
                    { "bufferView": 1, "componentType": 5121, "count": 3, "type": "VEC4" },
                    { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4" },
                    { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" },
                    { "bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4" },
                    { "bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1] },
                    { "bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC3" }
                ],
                "meshes": [{
                    "name": "SkinnedTriangle",
                    "primitives": [{
                        "attributes": { "POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2 },
                        "indices": 3
                    }]
                }],
                "nodes": [
                    { "name": "Mesh", "mesh": 0, "skin": 0 },
                    { "name": "RootJoint", "children": [2] },
                    { "name": "ChildJoint" }
                ],
                "skins": [{ "name": "Rig", "joints": [2, 1], "skeleton": 1, "inverseBindMatrices": 4 }],
                "animations": [{
                    "name": "Raise",
                    "samplers": [{ "input": 5, "output": 6, "interpolation": "LINEAR" }],
                    "channels": [{ "sampler": 0, "target": { "node": 2, "path": "translation" } }]
                }],
                "scenes": [{ "nodes": [0, 1] }],
                "scene": 0
            }"#,
        )
        .expect("write skinned glTF");

    let scene = load_gltf_scene(&gltf_path).expect("skinned glTF should load");
    assert_eq!(scene.primitives.len(), 1);
    assert_eq!(scene.primitives[0].mesh.joints, vec![[1, 0, 0, 0]; 3]);
    assert_eq!(
        scene.primitives[0].mesh.weights,
        vec![[0.75, 0.25, 0.0, 0.0]; 3]
    );
    assert_eq!(scene.nodes[0].skin_index, Some(0));
    assert_eq!(scene.nodes[1].skin_index, None);
    assert_eq!(scene.skins.len(), 1);
    assert_eq!(scene.skins[0].name, "Rig");
    assert_eq!(scene.skins[0].joint_remap, vec![1, 0]);
    assert_eq!(scene.skins[0].joints[0].name, "RootJoint");
    assert_eq!(scene.skins[0].joints[0].parent_index, None);
    assert_eq!(scene.skins[0].joints[1].name, "ChildJoint");
    assert_eq!(scene.skins[0].joints[1].parent_index, Some(0));
    assert_eq!(scene.animations.len(), 1);
    assert_eq!(scene.animations[0].name, "Raise");
    assert_eq!(scene.animations[0].duration, 1.0);
    assert_eq!(
        scene.animations[0].channels[0].values,
        GltfAnimationValues::Translations(vec![[0.0, 1.0, 0.0], [0.0, 2.0, 0.0]])
    );

    fs::remove_dir_all(dir).expect("remove test directory");
}

#[test]
fn resource_chain_preserves_all_indices_and_instances() {
    let scene = load_gltf_scene(&model_path("resource-chain.gltf"))
        .expect("resource-chain.gltf should load");

    assert_eq!(scene.selected_scene_index, Some(1));
    assert_eq!(scene.primitives.len(), 2);
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
    let json =
        fs::read_to_string(source)
            .expect("read fixture")
            .replacen("\"mode\": 4", "\"mode\": 1", 1);
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

#[test]
fn position_morph_targets_are_imported_and_cooked() {
    let dir = temp_dir("morph-target");
    let gltf_path = dir.join("morph.gltf");
    let buffer_path = dir.join("morph.bin");
    let positions = [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let deltas = [[0.0_f32, 0.0, 0.0], [0.0, 0.2, 0.0], [0.0, 0.0, 0.3]];
    let mut bytes = Vec::new();
    for value in positions.into_iter().chain(deltas) {
        for component in value {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    fs::write(&buffer_path, bytes).expect("write morph buffer");
    fs::write(
        &gltf_path,
        r#"{
  "asset": {"version": "2.0"},
  "buffers": [{"uri": "morph.bin", "byteLength": 72}],
  "bufferViews": [
    {"buffer": 0, "byteOffset": 0, "byteLength": 36},
    {"buffer": 0, "byteOffset": 36, "byteLength": 36}
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
     "min": [0, 0, 0], "max": [1, 1, 0]},
    {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}
  ],
  "meshes": [{"weights": [0.4], "primitives": [{
    "attributes": {"POSITION": 0},
    "targets": [{"POSITION": 1}]
  }]}],
  "nodes": [{"mesh": 0}],
  "scenes": [{"nodes": [0]}],
  "scene": 0
}"#,
    )
    .expect("write morph glTF");

    let scene = load_gltf_scene(&gltf_path).expect("morph glTF should load");
    assert_eq!(scene.primitives[0].morph_targets.len(), 1);
    assert_eq!(
        scene.primitives[0].morph_targets[0].position_deltas[1],
        Vec3::new(0.0, 0.2, 0.0)
    );
    assert_eq!(scene.nodes[0].morph_weights, vec![0.4]);

    let output = dir.join("morph.cooked");
    crate::cook::cook_morph_target_set(&gltf_path, &output, None).expect("cook morph target set");
    let artifact = crate::cook::read_cooked_artifact(&output).unwrap();
    let cooked = crate::cook::decode_cooked_morph_target_set(&artifact).unwrap();
    assert_eq!(cooked.vertex_count, 3);
    assert_eq!(cooked.targets.len(), 1);
    fs::remove_dir_all(dir).expect("remove test directory");
}
