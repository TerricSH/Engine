use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

use engine_asset::gltf::{load_gltf_scene, GltfScene, GltfTextureFormat};
use engine_renderer::{
    AssetId, AxisAlignedBox, ClearFlags, ColorSpace, Diagnostic, LightItem, LightKind,
    MaterialUpload, Rect, RenderFrameInput, RenderView, RenderableItem, Renderer, ShadowMode,
    TextureUpload, TextureUploadFormat, Transparency, UploadReceipt, ViewCompose,
};
use glam::{Mat4, Vec3};
use platform::winit::window::Window;
use platform::{EventFlow, PlatformEvent, WindowApp, WindowDescriptor};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use render_vulkan::device_impl::VulkanDevice;
use render_vulkan::scene_renderer::SceneRenderer;

const DEFAULT_MODEL: &str = "assets/models/resource-chain.gltf";

fn default_model_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(DEFAULT_MODEL)
}

struct ModelResources {
    drawables: Vec<RenderableItem>,
    camera_target: Vec3,
    camera_radius: f32,
}

struct ModelViewerApp {
    renderer: Option<Renderer>,
    drawables: Vec<RenderableItem>,
    frames: u64,
    max_frames: Option<u64>,
    last_frame_time: Instant,
    camera_angle: f32,
    camera_target: Vec3,
    camera_radius: f32,
    width: u32,
    height: u32,
    failed: Arc<AtomicBool>,
}

fn upload_or_exit(
    operation: &str,
    result: Result<UploadReceipt, Vec<Diagnostic>>,
) -> UploadReceipt {
    match result {
        Ok(receipt) => {
            super::log_upload_receipt(operation, &receipt);
            receipt
        }
        Err(diagnostics) => {
            super::log_renderer_diagnostics(operation, &diagnostics);
            std::process::exit(1);
        }
    }
}

fn texture_asset_id(index: usize) -> AssetId {
    AssetId::new(format!("model.texture.{index}"))
}

fn material_asset_id(index: usize) -> AssetId {
    AssetId::new(format!("model.material.{index}"))
}

fn mesh_asset_id(index: usize) -> AssetId {
    AssetId::new(format!("model.mesh.{index}"))
}

fn reject_unrepresentable_material_data(scene: &GltfScene) {
    for material in &scene.materials {
        let unsupported_texture = material
            .metallic_roughness_texture
            .map(|index| ("metallic-roughness", index))
            .or_else(|| material.normal_texture.map(|index| ("normal", index)))
            .or_else(|| material.emissive_texture.map(|index| ("emissive", index)));
        if let Some((slot, texture_index)) = unsupported_texture {
            tracing::error!(
                code = "SBX_MODEL_UNSUPPORTED_TEXTURE_SLOT",
                material_index = material.material_index,
                slot,
                texture_index,
                "the portable material contract cannot preserve this glTF texture slot"
            );
            std::process::exit(1);
        }
        if material.emissive.iter().any(|value| *value != 0.0) {
            tracing::error!(
                code = "SBX_MODEL_UNSUPPORTED_EMISSIVE",
                material_index = material.material_index,
                emissive = ?material.emissive,
                "the portable material contract cannot preserve emissive factors"
            );
            std::process::exit(1);
        }
    }
}

fn upload_textures(renderer: &mut Renderer, scene: &GltfScene) {
    let mut color_spaces = vec![None; scene.textures.len()];
    for material in &scene.materials {
        if let Some(texture_index) = material.base_color_texture {
            let Some(slot) = color_spaces.get_mut(texture_index) else {
                tracing::error!(
                    code = "SBX_MODEL_TEXTURE_INDEX",
                    material_index = material.material_index,
                    texture_index,
                    "base-color texture index is outside the imported texture table"
                );
                std::process::exit(1);
            };
            *slot = Some(ColorSpace::Srgb);
        }
    }

    for texture in &scene.textures {
        if texture.format != GltfTextureFormat::Rgba8 {
            tracing::error!(
                code = "SBX_MODEL_TEXTURE_FORMAT",
                texture_index = texture.texture_index,
                format = ?texture.format,
                "model-viewer requires decoded RGBA8 texture data"
            );
            std::process::exit(1);
        }
        let Some(color_slot) = color_spaces.get(texture.texture_index) else {
            tracing::error!(
                code = "SBX_MODEL_TEXTURE_INDEX",
                texture_index = texture.texture_index,
                "imported texture index is outside the stable texture table"
            );
            std::process::exit(1);
        };
        let color_space = color_slot.unwrap_or(ColorSpace::Linear);
        if color_slot.is_none() {
            tracing::warn!(
                code = "SBX_MODEL_UNREFERENCED_TEXTURE",
                texture_index = texture.texture_index,
                projected_color_space = "Linear",
                "texture has no supported semantic; using the linear upload convention"
            );
        }

        let mip_levels =
            super::rgba8_mip_chain(texture.width, texture.height, &texture.data, color_space);
        let sampler = super::gltf_sampler_descriptor(texture.sampler);
        let mut hash_source = Vec::new();
        hash_source.extend_from_slice(&texture.width.to_le_bytes());
        hash_source.extend_from_slice(&texture.height.to_le_bytes());
        hash_source.extend_from_slice(format!("{color_space:?}:{sampler:?}").as_bytes());
        for level in &mip_levels {
            hash_source.extend_from_slice(&level.width.to_le_bytes());
            hash_source.extend_from_slice(&level.height.to_le_bytes());
            hash_source.extend_from_slice(&level.bytes);
        }
        let texture_id = texture_asset_id(texture.texture_index);
        let upload = TextureUpload {
            texture_id: texture_id.clone(),
            width: texture.width,
            height: texture.height,
            format: TextureUploadFormat::Rgba8,
            color_space,
            mip_levels,
            sampler,
            content_hash: super::hash_upload_parts(&[&hash_source]),
        };
        upload_or_exit(
            &format!("model-viewer upload texture '{}'", texture_id.id),
            renderer.upload_texture(upload),
        );
    }
}

fn upload_materials(renderer: &mut Renderer, scene: &GltfScene) -> AssetId {
    let default_material_id = AssetId::new("model.material.default");
    let default_upload = MaterialUpload {
        material_id: default_material_id.clone(),
        base_color: [1.0; 4],
        metallic: 0.0,
        roughness: 1.0,
        ambient_occlusion: 1.0,
        base_color_texture: None,
        transparency: Transparency::Opaque,
        double_sided: false,
        content_hash: super::hash_upload_parts(&[b"model.material.default.v1"]),
    };
    upload_or_exit(
        "model-viewer upload default material",
        renderer.upload_material(default_upload),
    );

    for material in &scene.materials {
        let source_alpha_mode = format!("{:?}", material.alpha_mode);
        if source_alpha_mode != "Opaque" || material.double_sided {
            tracing::warn!(
                code = "SBX_MODEL_MATERIAL_PROJECTED",
                material_index = material.material_index,
                source_alpha_mode,
                source_alpha_cutoff = ?material.alpha_cutoff,
                source_double_sided = material.double_sided,
                projected_alpha_mode = "Opaque",
                projected_double_sided = false,
                "sandbox is explicitly projecting unsupported glTF raster state to the portable backend contract"
            );
        }

        let material_id = material_asset_id(material.material_index);
        let base_color_texture = material.base_color_texture.map(texture_asset_id);
        let mut hash_source = Vec::new();
        for value in material
            .base_color
            .iter()
            .chain([&material.metallic, &material.roughness])
        {
            hash_source.extend_from_slice(&value.to_le_bytes());
        }
        if let Some(texture_id) = &base_color_texture {
            hash_source.extend_from_slice(texture_id.id.as_bytes());
        }
        hash_source.extend_from_slice(source_alpha_mode.as_bytes());
        hash_source.push(u8::from(material.double_sided));
        let upload = MaterialUpload {
            material_id: material_id.clone(),
            base_color: material.base_color,
            metallic: material.metallic,
            roughness: material.roughness,
            ambient_occlusion: 1.0,
            base_color_texture,
            transparency: Transparency::Opaque,
            double_sided: false,
            content_hash: super::hash_upload_parts(&[&hash_source]),
        };
        upload_or_exit(
            &format!("model-viewer upload material '{}'", material_id.id),
            renderer.upload_material(upload),
        );
    }
    default_material_id
}

fn upload_meshes(renderer: &mut Renderer, scene: &GltfScene) {
    for (primitive_index, primitive) in scene.primitives.iter().enumerate() {
        let mesh_id = mesh_asset_id(primitive_index);
        let upload = super::mesh_upload_from_data(mesh_id.id.clone(), &primitive.mesh);
        upload_or_exit(
            &format!("model-viewer upload mesh '{}'", mesh_id.id),
            renderer.upload_mesh(upload),
        );
    }
}

fn instantiate_selected_scene(scene: &GltfScene, default_material_id: &AssetId) -> ModelResources {
    let mut world_min = Vec3::splat(f32::INFINITY);
    let mut world_max = Vec3::splat(f32::NEG_INFINITY);
    let mut drawables = Vec::new();

    for node in &scene.nodes {
        for &primitive_index in &node.primitive_indices {
            let Some(primitive) = scene.primitives.get(primitive_index) else {
                tracing::error!(
                    code = "SBX_MODEL_PRIMITIVE_INDEX",
                    node_index = node.source_node_index,
                    primitive_index,
                    "selected scene node references a missing imported primitive"
                );
                std::process::exit(1);
            };
            let (local_min, local_max) = primitive.mesh.bounds;
            for x in [local_min.x, local_max.x] {
                for y in [local_min.y, local_max.y] {
                    for z in [local_min.z, local_max.z] {
                        let point = node.transform.transform_point3(Vec3::new(x, y, z));
                        world_min = world_min.min(point);
                        world_max = world_max.max(point);
                    }
                }
            }
            let material = primitive
                .material_index
                .map(material_asset_id)
                .unwrap_or_else(|| default_material_id.clone());
            drawables.push(RenderableItem {
                entity: Some(format!(
                    "gltf.node.{}.primitive.{}",
                    node.source_node_index, primitive_index
                )),
                mesh: mesh_asset_id(primitive_index),
                material,
                // The importer already accumulated the complete selected-scene
                // parent chain. Keep the matrix verbatim; decomposing into ECS
                // TRS would lose shear from rotated non-uniform ancestors.
                world_transform: node.transform.to_cols_array(),
                bounds: AxisAlignedBox {
                    min: local_min.to_array(),
                    max: local_max.to_array(),
                },
                render_layer: "default".into(),
                cast_shadows: true,
                sort_key: drawables.len() as u64,
            });
        }
    }

    if drawables.is_empty() {
        tracing::error!(
            code = "SBX_MODEL_EMPTY_SCENE",
            selected_scene = ?scene.selected_scene_index,
            "selected glTF scene contains no drawable primitives"
        );
        std::process::exit(1);
    }
    let camera_target = (world_min + world_max) * 0.5;
    let camera_radius = ((world_max - world_min).length() * 1.5).max(3.0);
    ModelResources {
        drawables,
        camera_target,
        camera_radius,
    }
}

impl WindowApp for ModelViewerApp {
    fn on_create(&mut self, window: Arc<Window>) {
        let size = window.inner_size();
        self.width = size.width.max(1);
        self.height = size.height.max(1);

        let default_model = default_model_path();
        let model_path = match super::parse_model_path(&default_model.to_string_lossy()) {
            Ok(path) => path,
            Err(error) => {
                tracing::error!(
                    code = "SBX_MODEL_CLI",
                    message = error,
                    "invalid model-viewer arguments"
                );
                std::process::exit(2);
            }
        };
        let scene = match load_gltf_scene(std::path::Path::new(&model_path)) {
            Ok(scene) => scene,
            Err(error) => {
                tracing::error!(
                    code = "SBX_MODEL_IMPORT",
                    path = model_path,
                    error = %error,
                    "glTF load failed"
                );
                std::process::exit(1);
            }
        };
        tracing::info!(
            path = model_path,
            selected_scene = ?scene.selected_scene_index,
            primitives = scene.primitives.len(),
            materials = scene.materials.len(),
            textures = scene.textures.len(),
            nodes = scene.nodes.len(),
            "glTF scene loaded"
        );
        reject_unrepresentable_material_data(&scene);

        let display_handle = match window.display_handle() {
            Ok(handle) => handle.as_raw(),
            Err(error) => {
                tracing::error!(code = "SBX_MODEL_DISPLAY_HANDLE", error = %error);
                std::process::exit(1);
            }
        };
        let window_handle = match window.window_handle() {
            Ok(handle) => handle.as_raw(),
            Err(error) => {
                tracing::error!(code = "SBX_MODEL_WINDOW_HANDLE", error = %error);
                std::process::exit(1);
            }
        };
        let device = match VulkanDevice::new(
            display_handle,
            window_handle,
            self.width,
            self.height,
            std::env::var("ENGINE_VK_VALIDATION").is_ok(),
            None,
        ) {
            Ok(device) => device,
            Err(error) => {
                tracing::error!(
                    code = "SBX_MODEL_DEVICE",
                    error = %error,
                    "VulkanDevice creation failed"
                );
                std::process::exit(1);
            }
        };
        let backend = SceneRenderer::new(device, self.width, self.height);
        let mut renderer = Renderer::new_with_backend(Box::new(backend));

        upload_textures(&mut renderer, &scene);
        let default_material_id = upload_materials(&mut renderer, &scene);
        upload_meshes(&mut renderer, &scene);
        let resources = instantiate_selected_scene(&scene, &default_material_id);

        self.camera_target = resources.camera_target;
        self.camera_radius = resources.camera_radius;
        self.drawables = resources.drawables;
        self.renderer = Some(renderer);
        tracing::info!(
            drawables = self.drawables.len(),
            target = ?self.camera_target,
            radius = self.camera_radius,
            "model-viewer resource chain initialized"
        );
    }

    fn on_event(&mut self, window: &Window, event: PlatformEvent) -> EventFlow {
        if self.failed.load(Ordering::Acquire) {
            return EventFlow::Exit;
        }
        match event {
            PlatformEvent::Redraw => {
                let delta_seconds = self.last_frame_time.elapsed().as_secs_f32();
                self.last_frame_time = Instant::now();
                self.camera_angle += delta_seconds * 0.3;

                if let Some(renderer) = self.renderer.as_mut() {
                    let eye = self.camera_target
                        + Vec3::new(
                            self.camera_angle.sin() * self.camera_radius,
                            self.camera_radius * 0.45,
                            self.camera_angle.cos() * self.camera_radius,
                        );
                    let view = Mat4::look_at_rh(eye, self.camera_target, Vec3::Y);
                    let projection = Mat4::perspective_rh(
                        std::f32::consts::FRAC_PI_4,
                        self.width as f32 / self.height as f32,
                        0.05,
                        (self.camera_radius * 20.0).max(100.0),
                    );
                    let mut input = RenderFrameInput::empty(self.frames);
                    input.views.push(RenderView {
                        view_id: 0,
                        camera_entity: None,
                        viewport: Rect::FULL,
                        viewport_rect_normalized: Rect::FULL,
                        view_matrix: view.to_cols_array(),
                        projection_matrix: projection.to_cols_array(),
                        clear_flags: ClearFlags::ColorAndDepth,
                        clear_color: [0.02, 0.025, 0.04, 1.0],
                        render_layer_mask: u32::MAX,
                        msaa_samples: 1,
                        compose: ViewCompose::Base {
                            clear: ClearFlags::ColorAndDepth,
                            clear_color: [0.02, 0.025, 0.04, 1.0],
                        },
                        stack_order: 0,
                        frustum: None,
                    });
                    input.drawables.clone_from(&self.drawables);
                    input.lights.push(LightItem {
                        entity: Some("model-viewer.sun".into()),
                        kind: LightKind::Directional,
                        color: [1.0, 0.97, 0.9],
                        intensity: 4.0,
                        range: self.camera_radius * 10.0,
                        position: [0.0; 3],
                        direction: [-0.45, -0.8, -0.35],
                        spot_angles: None,
                        shadow_mode: ShadowMode::Hard,
                    });
                    match renderer.draw_scene(&input) {
                        Ok(stats) => tracing::info!(
                            frame = self.frames,
                            draw_calls = stats.draw_calls,
                            triangles = stats.triangles,
                            drawables = self.drawables.len(),
                            "model-viewer frame rendered"
                        ),
                        Err(diagnostics) => {
                            super::log_renderer_diagnostics("model-viewer draw", &diagnostics);
                            self.failed.store(true, Ordering::Release);
                            return EventFlow::Exit;
                        }
                    }
                }

                self.frames += 1;
                if self.max_frames.is_some_and(|limit| self.frames >= limit) {
                    tracing::info!(frames = self.frames, "frame limit reached; exiting");
                    return EventFlow::Exit;
                }
                window.request_redraw();
            }
            PlatformEvent::Resized { width, height } => {
                if let Some(renderer) = self.renderer.as_mut() {
                    if let Err(diagnostics) = renderer.resize(width, height) {
                        super::log_renderer_diagnostics("model-viewer resize", &diagnostics);
                        self.failed.store(true, Ordering::Release);
                        return EventFlow::Exit;
                    }
                }
                self.width = width;
                self.height = height;
            }
            PlatformEvent::CloseRequested => return EventFlow::Exit,
            _ => {}
        }
        EventFlow::Continue
    }
}

pub fn run() {
    let max_frames = super::parse_frame_limit();
    if max_frames == Some(0) {
        tracing::error!(
            code = "SBX_MODEL_FRAME_LIMIT",
            "--frames must be greater than zero"
        );
        std::process::exit(2);
    }
    let failed = Arc::new(AtomicBool::new(false));
    let app = ModelViewerApp {
        renderer: None,
        drawables: Vec::new(),
        frames: 0,
        max_frames,
        last_frame_time: Instant::now(),
        camera_angle: 0.0,
        camera_target: Vec3::ZERO,
        camera_radius: 3.0,
        width: 1280,
        height: 720,
        failed: Arc::clone(&failed),
    };
    if let Err(error) = platform::run(
        WindowDescriptor {
            title: "Engine Model Viewer - Resource Chain".into(),
            width: 1280,
            height: 720,
        },
        app,
    ) {
        tracing::error!(
            code = "SBX_MODEL_PLATFORM",
            error = %error,
            "platform run failed"
        );
        std::process::exit(1);
    }
    if failed.load(Ordering::Acquire) {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_renderer::{BackendRenderer, MeshUpload};
    use std::sync::Mutex;

    #[derive(Default)]
    struct CapturedUploads {
        meshes: Vec<MeshUpload>,
        textures: Vec<TextureUpload>,
        materials: Vec<MaterialUpload>,
    }

    struct CaptureBackend {
        uploads: Arc<Mutex<CapturedUploads>>,
    }

    impl BackendRenderer for CaptureBackend {
        fn render_frame(
            &mut self,
            _input: &RenderFrameInput,
        ) -> Result<engine_renderer::FrameStats, Vec<Diagnostic>> {
            Ok(engine_renderer::FrameStats::default())
        }

        fn upload_mesh(&mut self, upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
            self.uploads.lock().unwrap().meshes.push(upload);
            Ok(UploadReceipt::new(1))
        }

        fn upload_texture(
            &mut self,
            upload: TextureUpload,
        ) -> Result<UploadReceipt, Vec<Diagnostic>> {
            self.uploads.lock().unwrap().textures.push(upload);
            Ok(UploadReceipt::new(1))
        }

        fn upload_material(
            &mut self,
            upload: MaterialUpload,
        ) -> Result<UploadReceipt, Vec<Diagnostic>> {
            self.uploads.lock().unwrap().materials.push(upload);
            Ok(UploadReceipt::new(1))
        }
    }

    #[test]
    fn fixed_fixture_expands_all_selected_scene_primitive_instances() {
        let scene =
            load_gltf_scene(&default_model_path()).expect("resource-chain fixture should load");
        let expected = scene
            .nodes
            .iter()
            .map(|node| node.primitive_indices.len())
            .sum::<usize>();
        assert_eq!(expected, 4);
        assert_eq!(scene.selected_scene_index, Some(1));

        let resources = instantiate_selected_scene(&scene, &AssetId::new("fallback"));
        assert_eq!(resources.drawables.len(), expected);
        for node in &scene.nodes {
            for &primitive_index in &node.primitive_indices {
                let entity = format!(
                    "gltf.node.{}.primitive.{}",
                    node.source_node_index, primitive_index
                );
                let drawable = resources
                    .drawables
                    .iter()
                    .find(|drawable| drawable.entity.as_deref() == Some(entity.as_str()))
                    .expect("every selected node primitive should be instantiated");
                assert_eq!(drawable.world_transform, node.transform.to_cols_array());
                assert_eq!(drawable.mesh, mesh_asset_id(primitive_index));
            }
        }
    }

    #[test]
    fn srgb_mip_generation_is_complete() {
        let levels = super::super::rgba8_mip_chain(
            4,
            2,
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255,
                0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255, 255,
            ],
            ColorSpace::Srgb,
        );
        assert_eq!(levels.len(), 3);
        assert_eq!((levels[0].width, levels[0].height), (4, 2));
        assert_eq!((levels[1].width, levels[1].height), (2, 1));
        assert_eq!((levels[2].width, levels[2].height), (1, 1));
    }

    #[test]
    fn fixed_fixture_builds_typed_texture_material_and_mesh_uploads() {
        use engine_renderer::{SamplerAddressMode, SamplerFilter};

        let scene =
            load_gltf_scene(&default_model_path()).expect("resource-chain fixture should load");
        let uploads = Arc::new(Mutex::new(CapturedUploads::default()));
        let backend = CaptureBackend {
            uploads: Arc::clone(&uploads),
        };
        let mut renderer = Renderer::new_with_backend(Box::new(backend));
        upload_textures(&mut renderer, &scene);
        upload_materials(&mut renderer, &scene);
        upload_meshes(&mut renderer, &scene);

        let uploads = uploads.lock().unwrap();
        assert_eq!(uploads.textures.len(), 2);
        assert_eq!(uploads.materials.len(), 3);
        assert_eq!(uploads.meshes.len(), 2);
        assert!(uploads
            .textures
            .iter()
            .all(|texture| texture.color_space == ColorSpace::Srgb));
        assert!(uploads
            .textures
            .iter()
            .all(|texture| texture.mip_levels.len() == 2));

        let nearest = &uploads.textures[0].sampler;
        assert_eq!(nearest.min_filter, SamplerFilter::Nearest);
        assert_eq!(nearest.mag_filter, SamplerFilter::Nearest);
        assert_eq!(nearest.mip_filter, SamplerFilter::Nearest);
        assert_eq!(nearest.address_u, SamplerAddressMode::ClampToEdge);
        assert_eq!(nearest.address_v, SamplerAddressMode::MirroredRepeat);

        let linear = &uploads.textures[1].sampler;
        assert_eq!(linear.min_filter, SamplerFilter::Linear);
        assert_eq!(linear.mag_filter, SamplerFilter::Linear);
        assert_eq!(linear.mip_filter, SamplerFilter::Linear);
        assert_eq!(linear.address_u, SamplerAddressMode::Repeat);
        assert_eq!(linear.address_v, SamplerAddressMode::Repeat);

        for material in uploads.materials.iter().skip(1) {
            assert!(matches!(material.transparency, Transparency::Opaque));
            assert!(!material.double_sided);
            assert!(material.base_color_texture.is_some());
        }
        assert!(uploads.meshes.iter().all(|mesh| {
            mesh.vertex_format == engine_renderer::MeshVertexFormat::Pbr32
                && mesh.index_format == engine_renderer::IndexFormat::U32
        }));
    }
}
