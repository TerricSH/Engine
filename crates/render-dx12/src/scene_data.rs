//! Portable render-contract data adapters for the DX12 scene backend.
//!
//! This module owns CPU-side packing and selection policies. Keeping these
//! pure functions separate leaves `Dx12SceneRenderer` responsible for resource
//! lifetime and pass orchestration instead of material/post-process policy.

use engine_renderer::{
    AdvancedMaterialParameters, Diagnostic, DiagnosticSeverity, EnvironmentSettings,
    MaterialUpload, RenderFrameInput, ToneMapping, Transparency, TransparencyMode,
};
use engine_serialize::AssetId;
use glam::{Mat4, Vec3};

use crate::scene_renderer::Dx12ShadowFrameData;

pub(crate) fn default_material_constants() -> [u8; 32] {
    material_constants([0.8, 0.6, 0.4, 1.0], 0.0, 1.0, 1.0)
}

pub(crate) fn default_emissive_constants() -> [u8; 16] {
    [0; 16]
}

pub(crate) fn default_advanced_constants() -> [u8; 16] {
    advanced_constants(AdvancedMaterialParameters::default())
}

pub(crate) fn matrix_bytes(matrix: Mat4) -> [u8; 64] {
    let mut bytes = [0_u8; 64];
    for (destination, value) in bytes
        .chunks_exact_mut(4)
        .zip(matrix.to_cols_array().into_iter())
    {
        destination.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

pub(crate) fn float4_bytes(values: [f32; 4]) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    for (destination, value) in bytes.chunks_exact_mut(4).zip(values) {
        destination.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

pub(crate) fn shadow_scene_constants(
    shadow: Option<Dx12ShadowFrameData>,
    world: Mat4,
) -> ([u8; 64], [u8; 16], [u8; 16]) {
    let fallback_direction = Vec3::new(0.5, 0.8, 0.3).normalize();
    let (light_matrix, parameters, direction) = match shadow {
        Some(shadow) => (
            shadow.light_view_projection * world,
            [
                1.0,
                if shadow.soft { 1.0 } else { 0.0 },
                1.0 / 2048.0,
                0.0015,
            ],
            shadow.light_direction_to_surface,
        ),
        None => (
            Mat4::IDENTITY,
            [0.0, 0.0, 1.0 / 2048.0, 0.0015],
            fallback_direction,
        ),
    };
    (
        matrix_bytes(light_matrix),
        float4_bytes(parameters),
        float4_bytes([direction.x, direction.y, direction.z, 0.0]),
    )
}

pub(crate) fn select_environment_map(
    settings: &EnvironmentSettings,
    camera_position: Vec3,
) -> Option<&AssetId> {
    settings
        .reflection_probes
        .iter()
        .filter_map(|probe| {
            let position = Vec3::from_array(probe.position);
            let extents =
                Vec3::from_array(probe.half_extents) + Vec3::splat(probe.blend_distance.max(0.0));
            let offset = (camera_position - position).abs();
            (offset.cmple(extents).all()).then(|| {
                let normalized_distance =
                    (offset / extents.max(Vec3::splat(0.0001))).length_squared();
                (probe, normalized_distance)
            })
        })
        .max_by(|(left, left_distance), (right, right_distance)| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| right_distance.total_cmp(left_distance))
        })
        .map(|(probe, _)| &probe.environment_map)
        .or(settings.environment_map.as_ref())
}

pub(crate) fn material_constants_from_upload(upload: &MaterialUpload) -> [u8; 32] {
    let mut constants = material_constants(
        upload.base_color,
        upload.metallic,
        upload.roughness,
        upload.ambient_occlusion,
    );
    constants[28..32].copy_from_slice(
        &material_surface_flags(upload.base_color_texture.is_some(), &upload.transparency)
            .to_ne_bytes(),
    );
    constants
}

pub(crate) fn material_constants_from_bytes(
    bytes: &[u8],
    uses_texture: bool,
    transparency: &Transparency,
    weighted_oit: bool,
) -> [u8; 32] {
    let mut constants = default_material_constants();
    let copy_len = bytes.len().min(constants.len());
    constants[..copy_len].copy_from_slice(&bytes[..copy_len]);
    set_material_surface_flags(&mut constants, uses_texture, transparency, weighted_oit);
    constants
}

pub(crate) fn set_material_surface_flags(
    constants: &mut [u8; 32],
    uses_texture: bool,
    transparency: &Transparency,
    weighted_oit: bool,
) {
    let mut flags = material_surface_flags(uses_texture, transparency);
    if weighted_oit && matches!(transparency, Transparency::Blend) {
        flags += 8.0;
    }
    constants[28..32].copy_from_slice(&flags.to_ne_bytes());
}

pub(crate) fn emissive_constants_from_bytes(bytes: &[u8], texture_flags: u32) -> [u8; 16] {
    let mut constants = default_emissive_constants();
    if bytes.len() > 32 {
        let copy_len = (bytes.len() - 32).min(12);
        constants[..copy_len].copy_from_slice(&bytes[32..32 + copy_len]);
    }
    constants[12..16].copy_from_slice(&(texture_flags as f32).to_ne_bytes());
    constants
}

pub(crate) fn emissive_constants(emissive: [f32; 3], texture_flags: u32) -> [u8; 16] {
    float4_bytes([emissive[0], emissive[1], emissive[2], texture_flags as f32])
}

pub(crate) fn advanced_constants_from_upload(upload: &MaterialUpload) -> [u8; 16] {
    advanced_constants(upload.advanced)
}

pub(crate) fn advanced_constants_from_bytes(bytes: &[u8]) -> [u8; 16] {
    if bytes.len() < 112 {
        return default_advanced_constants();
    }
    let read = |offset: usize| {
        f32::from_ne_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("validated material byte offset"),
        )
    };
    advanced_constants(AdvancedMaterialParameters {
        clearcoat: read(48),
        clearcoat_roughness: read(52),
        subsurface: read(56),
        anisotropy: read(60),
        subsurface_color: [read(64), read(68), read(72)],
        sheen_color: [read(80), read(84), read(88)],
        rim_color: [read(96), read(100), read(104)],
        rim_power: read(108),
    })
}

fn pack_unorm_rgba8(values: [f32; 4]) -> u32 {
    let quantize = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    quantize(values[0])
        | (quantize(values[1]) << 8)
        | (quantize(values[2]) << 16)
        | (quantize(values[3]) << 24)
}

/// Compact the portable advanced-material tail to four DX12 root constants.
/// This leaves another float4 for environment settings while keeping root
/// constants plus descriptor tables inside D3D12's 64-DWORD signature budget.
pub(crate) fn advanced_constants(parameters: AdvancedMaterialParameters) -> [u8; 16] {
    let words = [
        pack_unorm_rgba8([
            parameters.clearcoat,
            parameters.clearcoat_roughness,
            parameters.subsurface,
            parameters.anisotropy * 0.5 + 0.5,
        ]),
        pack_unorm_rgba8([
            parameters.subsurface_color[0],
            parameters.subsurface_color[1],
            parameters.subsurface_color[2],
            0.0,
        ]),
        pack_unorm_rgba8([
            parameters.sheen_color[0],
            parameters.sheen_color[1],
            parameters.sheen_color[2],
            0.0,
        ]),
        pack_unorm_rgba8([
            parameters.rim_color[0],
            parameters.rim_color[1],
            parameters.rim_color[2],
            (parameters.rim_power - 0.01) / (32.0 - 0.01),
        ]),
    ];
    let mut bytes = [0_u8; 16];
    for (destination, word) in bytes.chunks_exact_mut(4).zip(words) {
        destination.copy_from_slice(&word.to_ne_bytes());
    }
    bytes
}

pub(crate) fn material_texture_flags_from_ids(texture_ids: &[Option<String>; 5]) -> u32 {
    texture_ids
        .iter()
        .enumerate()
        .fold(0_u32, |flags, (index, texture)| {
            flags | u32::from(texture.is_some()) << index
        })
}

pub(crate) fn material_surface_flags(uses_texture: bool, transparency: &Transparency) -> f32 {
    match transparency {
        Transparency::Masked { cutoff } => (if uses_texture { 3.0 } else { 2.0 }) + cutoff * 0.5,
        Transparency::Opaque | Transparency::Blend | Transparency::Additive => {
            if uses_texture {
                1.0
            } else {
                0.0
            }
        }
    }
}

pub(crate) fn material_constants(
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    ambient_occlusion: f32,
) -> [u8; 32] {
    let mut bytes = [0_u8; 32];
    for (destination, value) in bytes.chunks_exact_mut(4).zip(base_color.into_iter().chain([
        metallic,
        roughness,
        ambient_occlusion,
        0.0,
    ])) {
        destination.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

pub(crate) fn tone_map_constants(input: &RenderFrameInput) -> Result<[u8; 128], Vec<Diagnostic>> {
    let mode = match input.render_options.tone_mapping {
        ToneMapping::Aces => 0_u32,
        ToneMapping::Reinhard => 1,
        ToneMapping::None => 2,
    };
    let exposure = match input.render_options.exposure_ev100 {
        None => 1.0,
        Some(ev100) if ev100.is_finite() => (-ev100).exp2(),
        Some(ev100) => {
            return Err(vec![Diagnostic::new(
                "DX1258",
                DiagnosticSeverity::Error,
                "scene_renderer",
                format!("exposure_ev100 must be finite, received {ev100}"),
            )]);
        }
    };
    if !exposure.is_finite() {
        return Err(vec![Diagnostic::new(
            "DX1258",
            DiagnosticSeverity::Error,
            "scene_renderer",
            "exposure_ev100 produced a non-finite multiplier",
        )]);
    }
    let post = input.render_options.post_process;
    let effect_flags = u32::from(post.enabled && post.bloom.enabled)
        | (u32::from(post.enabled && post.color_grading.enabled) << 1)
        | (u32::from(post.enabled && post.vignette.enabled) << 2)
        | (u32::from(
            input.render_options.transparency_mode == TransparencyMode::WeightedBlendedOit,
        ) << 3);
    let mut bytes = Vec::with_capacity(128);
    bytes.extend_from_slice(&mode.to_ne_bytes());
    bytes.extend_from_slice(&exposure.to_ne_bytes());
    // The DX12 swapchain target is BGRA8_UNORM, not an sRGB view.
    bytes.extend_from_slice(&0_u32.to_ne_bytes());
    bytes.extend_from_slice(&effect_flags.to_ne_bytes());
    for vector in [
        [
            post.bloom.threshold,
            post.bloom.intensity,
            post.bloom.radius,
            0.0,
        ],
        [
            post.color_grading.color_filter[0],
            post.color_grading.color_filter[1],
            post.color_grading.color_filter[2],
            post.color_grading.saturation,
        ],
        [post.color_grading.contrast, 0.0, 0.0, 0.0],
        [
            post.color_grading.lift[0],
            post.color_grading.lift[1],
            post.color_grading.lift[2],
            0.0,
        ],
        [
            post.color_grading.gamma[0],
            post.color_grading.gamma[1],
            post.color_grading.gamma[2],
            0.0,
        ],
        [
            post.color_grading.gain[0],
            post.color_grading.gain[1],
            post.color_grading.gain[2],
            0.0,
        ],
        [
            post.vignette.intensity,
            post.vignette.smoothness,
            post.vignette.roundness,
            0.0,
        ],
    ] {
        for value in vector {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    Ok(bytes
        .try_into()
        .expect("DX12 tone-map constant layout is exactly 128 bytes"))
}
