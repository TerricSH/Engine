//! HDR equirectangular image to GPU-ready cubemap cooking.

use std::{f32::consts::PI, path::Path};

use engine_serialize::SchemaVersion;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use super::{error::CookError, write_cooked_artifact, AssetType, CookResult, CookedArtifact};

pub const COOKED_ENVIRONMENT_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 1, 0);
const MAX_FACE_SIZE: u32 = 512;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookedEnvironmentMip {
    pub face_size: u32,
    /// +X, -X, +Y, -Y, +Z, -Z; tightly packed linear RGBA16F.
    pub faces: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookedEnvironmentMap {
    pub mip_levels: Vec<CookedEnvironmentMip>,
}

pub fn cook_environment_map(source: &Path, output: &Path) -> Result<CookResult, CookError> {
    let image = image::open(source)
        .map_err(|error| {
            CookError::Parse(format!(
                "failed to load HDR environment {source:?}: {error}"
            ))
        })?
        .to_rgba32f();
    let (width, height) = image.dimensions();
    if width < 2 || height == 0 || width < height {
        return Err(CookError::InvalidAsset(
            "environment source must be a non-empty equirectangular image with width >= height"
                .into(),
        ));
    }

    let face_size = height.min(MAX_FACE_SIZE).next_power_of_two() / 2;
    let face_size = face_size.max(1);
    let base_faces = (0..6)
        .map(|face| {
            (0..face_size)
                .flat_map(|y| {
                    let image = &image;
                    (0..face_size).map(move |x| {
                        sample_equirectangular(image, cube_direction(face, x, y, face_size))
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mip_count = u32::BITS - face_size.leading_zeros();
    let mut mip_levels = Vec::with_capacity(mip_count as usize);
    let mut size = face_size;
    for mip_index in 0..mip_count {
        let roughness = if mip_count > 1 {
            mip_index as f32 / (mip_count - 1) as f32
        } else {
            0.0
        };
        let float_faces = if mip_index == 0 {
            base_faces.clone()
        } else {
            let sample_count = if mip_index + 1 == mip_count { 128 } else { 32 };
            (0..6)
                .map(|face| {
                    (0..size)
                        .flat_map(|y| {
                            let image = &image;
                            (0..size).map(move |x| {
                                prefilter_environment(
                                    image,
                                    cube_direction(face, x, y, size),
                                    roughness,
                                    sample_count,
                                )
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        mip_levels.push(CookedEnvironmentMip {
            face_size: size,
            faces: float_faces
                .iter()
                .map(|face| encode_rgba16f(face))
                .collect(),
        });
        size = (size / 2).max(1);
    }

    let payload = bincode::serialize(&CookedEnvironmentMap { mip_levels })
        .map_err(|error| CookError::InvalidAsset(error.to_string()))?;
    write_cooked_artifact(
        output,
        AssetType::EnvironmentMap.kind_code(),
        &payload,
        COOKED_ENVIRONMENT_SCHEMA_VERSION,
    )
}

pub fn decode_cooked_environment_map(
    artifact: &CookedArtifact,
) -> Result<CookedEnvironmentMap, CookError> {
    if artifact.header.asset_kind != AssetType::EnvironmentMap.kind_code() {
        return Err(CookError::InvalidAsset(
            "artifact is not an environment map".into(),
        ));
    }
    if artifact.header.schema_version != COOKED_ENVIRONMENT_SCHEMA_VERSION {
        return Err(CookError::UnsupportedFormat(format!(
            "unsupported cooked environment schema {:?}",
            artifact.header.schema_version
        )));
    }
    bincode::deserialize(&artifact.payload)
        .map_err(|error| CookError::InvalidAsset(format!("invalid environment payload: {error}")))
}

fn cube_direction(face: usize, x: u32, y: u32, size: u32) -> Vec3 {
    let u = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
    let v = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
    match face {
        0 => Vec3::new(1.0, -v, -u),
        1 => Vec3::new(-1.0, -v, u),
        2 => Vec3::new(u, 1.0, v),
        3 => Vec3::new(u, -1.0, -v),
        4 => Vec3::new(u, -v, 1.0),
        _ => Vec3::new(-u, -v, -1.0),
    }
    .normalize()
}

fn sample_equirectangular(image: &image::Rgba32FImage, direction: Vec3) -> [f32; 4] {
    let (width, height) = image.dimensions();
    let u = (0.5 + direction.z.atan2(direction.x) / (2.0 * PI)).rem_euclid(1.0);
    let v = (0.5 - direction.y.clamp(-1.0, 1.0).asin() / PI).clamp(0.0, 1.0);
    let fx = u * width as f32 - 0.5;
    let fy = v * height as f32 - 0.5;
    let x0 = fx.floor() as i64;
    let y0 = fy.floor() as i64;
    let tx = fx - fx.floor();
    let ty = fy - fy.floor();
    let sample = |x: i64, y: i64| {
        let x = x.rem_euclid(width as i64) as u32;
        let y = y.clamp(0, height as i64 - 1) as u32;
        image.get_pixel(x, y).0
    };
    let p00 = sample(x0, y0);
    let p10 = sample(x0 + 1, y0);
    let p01 = sample(x0, y0 + 1);
    let p11 = sample(x0 + 1, y0 + 1);
    let mut result = [0.0; 4];
    for channel in 0..4 {
        let top = p00[channel] + (p10[channel] - p00[channel]) * tx;
        let bottom = p01[channel] + (p11[channel] - p01[channel]) * tx;
        result[channel] = (top + (bottom - top) * ty).max(0.0);
    }
    result
}

/// Prefilter one radiance direction with the same GGX lobe the runtime PBR
/// shader uses. Sampling the original panorama (instead of each cube face)
/// keeps every mip continuous across face seams.
fn prefilter_environment(
    image: &image::Rgba32FImage,
    normal: Vec3,
    roughness: f32,
    sample_count: u32,
) -> [f32; 4] {
    let normal = normal.normalize();
    let up = if normal.z.abs() < 0.999 {
        Vec3::Z
    } else {
        Vec3::X
    };
    let tangent = up.cross(normal).normalize();
    let bitangent = normal.cross(tangent);
    let view = normal;
    let mut accumulated = [0.0_f32; 4];
    let mut total_weight = 0.0_f32;
    for sample_index in 0..sample_count {
        let xi = [
            sample_index as f32 / sample_count as f32,
            radical_inverse_vdc(sample_index),
        ];
        let half_local = importance_sample_ggx(xi, roughness);
        let half_vector =
            (tangent * half_local.x + bitangent * half_local.y + normal * half_local.z).normalize();
        let light = (2.0 * view.dot(half_vector) * half_vector - view).normalize();
        let weight = normal.dot(light).max(0.0);
        if weight <= 0.0 {
            continue;
        }
        let sample = sample_equirectangular(image, light);
        for channel in 0..4 {
            accumulated[channel] += sample[channel] * weight;
        }
        total_weight += weight;
    }
    if total_weight <= f32::EPSILON {
        return sample_equirectangular(image, normal);
    }
    accumulated.map(|channel| channel / total_weight)
}

fn importance_sample_ggx(xi: [f32; 2], roughness: f32) -> Vec3 {
    let alpha = roughness.max(0.001).powi(2);
    let phi = 2.0 * PI * xi[0];
    let cos_theta = ((1.0 - xi[1]) / (1.0 + (alpha * alpha - 1.0) * xi[1])).sqrt();
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    Vec3::new(phi.cos() * sin_theta, phi.sin() * sin_theta, cos_theta)
}

fn radical_inverse_vdc(bits: u32) -> f32 {
    bits.reverse_bits() as f32 * 2.328_306_4e-10
}

fn encode_rgba16f(pixels: &[[f32; 4]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(pixels.len() * 8);
    for pixel in pixels {
        for value in pixel {
            bytes.extend_from_slice(&f32_to_f16_bits(*value).to_le_bytes());
        }
    }
    bytes
}

fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = bits & 0x7f_ffff;
    if exponent <= 0 {
        if exponent < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x80_0000;
        let shift = 14 - exponent;
        return sign | ((mantissa + (1 << (shift - 1))) >> shift) as u16;
    }
    if exponent >= 31 {
        return sign | 0x7c00;
    }
    sign | ((exponent as u16) << 10) | ((mantissa + 0x1000) >> 13) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::read_cooked_artifact;

    #[test]
    fn cube_directions_are_finite_unit_vectors() {
        for face in 0..6 {
            let direction = cube_direction(face, 0, 0, 1);
            assert!(direction.is_finite());
            assert!((direction.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn half_encoding_covers_common_hdr_values() {
        assert_eq!(f32_to_f16_bits(0.0), 0x0000);
        assert_eq!(f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(f32_to_f16_bits(2.0), 0x4000);
    }

    #[test]
    fn ggx_prefilter_preserves_constant_radiance() {
        let image = image::Rgba32FImage::from_pixel(16, 8, image::Rgba([2.0, 0.5, 4.0, 1.0]));
        for roughness in [0.1, 0.5, 1.0] {
            let filtered = prefilter_environment(&image, Vec3::new(0.3, 0.8, -0.2), roughness, 64);
            for (actual, expected) in filtered.into_iter().zip([2.0, 0.5, 4.0, 1.0]) {
                assert!((actual - expected).abs() < 1.0e-4);
            }
        }
    }

    #[test]
    fn cooker_produces_six_faces_and_a_complete_mip_chain() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("environment.png");
        let output = directory.path().join("environment.cooked");
        let mut image = image::RgbaImage::new(8, 4);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgba([(x * 24) as u8, (y * 48) as u8, 128, 255]);
        }
        image.save(&source).unwrap();

        cook_environment_map(&source, &output).unwrap();
        let artifact = read_cooked_artifact(&output).unwrap();
        let cooked = decode_cooked_environment_map(&artifact).unwrap();

        assert_eq!(
            artifact.header.asset_kind,
            AssetType::EnvironmentMap.kind_code()
        );
        assert_eq!(cooked.mip_levels.len(), 2);
        assert_eq!(cooked.mip_levels[0].face_size, 2);
        assert_eq!(cooked.mip_levels[0].faces.len(), 6);
        assert!(cooked.mip_levels[0]
            .faces
            .iter()
            .all(|face| face.len() == 2 * 2 * 8));
        assert_eq!(cooked.mip_levels[1].face_size, 1);
    }
}
