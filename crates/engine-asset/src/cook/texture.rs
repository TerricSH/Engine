//! Texture cooking pipeline.
//!
//! Loads image files via the `image` crate, converts to RGBA8, generates
//! a mip-chain, and serializes as [`CookedTexture`] + [`CookedAssetHeader`].

use std::path::Path;

use engine_serialize::SchemaVersion;
use serde::{Deserialize, Serialize};

use super::error::CookError;
use super::{write_cooked_artifact, AssetType, CookResult};

/// Supported texture pixel formats for cooked output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextureFormat {
    Rgba8Unorm,
    R8Unorm,
    Rg8Unorm,
    Bgra8Unorm,
}

/// A cooked texture asset with mip-chain.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CookedTexture {
    /// Width of the base mip level in pixels.
    pub width: u32,
    /// Height of the base mip level in pixels.
    pub height: u32,
    /// Number of mip levels.
    pub mip_count: u8,
    /// Pixel format.
    pub format: TextureFormat,
    /// Pixel data: all mip levels concatenated (base level first).
    pub data: Vec<u8>,
}

/// Cook a texture from a source image file.
///
/// Supported input formats: anything the `image` crate can decode (PNG, JPEG,
/// BMP, TIFF, WEBP, etc.).
///
/// The output is an RGBA8 texture with a full mip-chain, written as a
/// bincode-serialized [`CookedTexture`] prefixed by a [`CookedAssetHeader`].
pub fn cook_texture(source: &Path, output: &Path) -> Result<CookResult, CookError> {
    // 1. Load the image.
    let img = image::open(source)
        .map_err(|e| CookError::Parse(format!("failed to load image {:?}: {e}", source)))?;

    // 2. Convert to RGBA8.
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    // 3. Generate mip chain.
    let (mip_count, mip_data) = generate_mip_chain(&rgba);

    // 4. Build cooked texture.
    let cooked = CookedTexture {
        width,
        height,
        mip_count,
        format: TextureFormat::Rgba8Unorm,
        data: mip_data,
    };

    // 5. Serialize and write.
    let payload =
        bincode::serialize(&cooked).map_err(|e| CookError::InvalidAsset(e.to_string()))?;

    let result = write_cooked_artifact(
        output,
        AssetType::Texture.kind_code(),
        &payload,
        SchemaVersion::new(0, 1, 0),
    )?;

    Ok(result)
}

/// Generate a full mip-chain for an RGBA8 image using simple box-filter
/// downsampling.
///
/// Returns `(mip_count, flattened_data)` where all mip levels are
/// concatenated, base level first.
fn generate_mip_chain(rgba: &image::RgbaImage) -> (u8, Vec<u8>) {
    let (w, h) = rgba.dimensions();
    let max_dim = w.max(h);
    let mip_count = (max_dim.ilog2() + 1) as u8;

    let mut data = rgba.to_vec(); // base level
    let mut prev_w = w;
    let mut prev_h = h;

    for level in 1..mip_count as u32 {
        let new_w = (prev_w / 2).max(1);
        let new_h = (prev_h / 2).max(1);

        let mut mip = Vec::with_capacity((new_w * new_h * 4) as usize);

        for y in 0..new_h {
            for x in 0..new_w {
                // Box-filter: average a 2×2 block from the previous level.
                let src_x = x * 2;
                let src_y = y * 2;

                let mut r = 0u32;
                let mut g = 0u32;
                let mut b = 0u32;
                let mut a = 0u32;
                let mut count = 0u32;

                for dy in 0..2 {
                    for dx in 0..2 {
                        let px_x = src_x + dx;
                        let px_y = src_y + dy;
                        if px_x < prev_w && px_y < prev_h {
                            let idx = (px_y * prev_w + px_x) as usize * 4;

                            // For mip level 1, read from the base level (first w*h*4 bytes).
                            // For mip level 2+, read from the previous level we just added.
                            let read_idx = if level == 1 {
                                idx
                            } else {
                                let prev_start = mip_level_offset(level - 1, &data, w, h);
                                prev_start + idx
                            };

                            if read_idx + 3 < data.len() {
                                r += data[read_idx] as u32;
                                g += data[read_idx + 1] as u32;
                                b += data[read_idx + 2] as u32;
                                a += data[read_idx + 3] as u32;
                                count += 1;
                            }
                        }
                    }
                }

                if count > 0 {
                    mip.push((r / count) as u8);
                    mip.push((g / count) as u8);
                    mip.push((b / count) as u8);
                    mip.push((a / count) as u8);
                } else {
                    mip.extend_from_slice(&[0, 0, 0, 0]);
                }
            }
        }

        data.extend_from_slice(&mip);
        prev_w = new_w;
        prev_h = new_h;
    }

    (mip_count, data)
}

/// Compute the byte offset in `data` where mip level `level` starts.
fn mip_level_offset(level: u32, _data: &[u8], base_w: u32, base_h: u32) -> usize {
    let mut offset = 0u64;
    let mut w = base_w as u64;
    let mut h = base_h as u64;

    for _ in 0..level {
        offset += w * h * 4;
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }

    offset as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooked_texture_serde_roundtrip() {
        let tex = CookedTexture {
            width: 64,
            height: 64,
            mip_count: 7,
            format: TextureFormat::Rgba8Unorm,
            data: vec![128u8; 64 * 64 * 4],
        };

        let bytes = bincode::serialize(&tex).unwrap();
        let restored: CookedTexture = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.width, 64);
        assert_eq!(restored.height, 64);
        assert_eq!(restored.mip_count, 7);
        assert_eq!(restored.format, TextureFormat::Rgba8Unorm);
    }

    #[test]
    fn mip_level_offset_base() {
        // Level 0 starts at 0.
        let data = vec![0u8; 256];
        assert_eq!(mip_level_offset(0, &data, 8, 8), 0);
    }

    #[test]
    fn mip_level_offset_level1() {
        // 8x8 RGBA = 256 bytes, level 1 starts at 256.
        let data = vec![0u8; 256 + 64];
        assert_eq!(mip_level_offset(1, &data, 8, 8), 256);
    }

    #[test]
    fn generate_mip_chain_4x4() {
        let mut img = image::RgbaImage::new(4, 4);
        // Fill with a simple pattern
        for y in 0..4 {
            for x in 0..4 {
                let pixel = if (x + y) % 2 == 0 {
                    image::Rgba([255, 0, 0, 255])
                } else {
                    image::Rgba([0, 255, 0, 255])
                };
                img.put_pixel(x, y, pixel);
            }
        }

        let (mip_count, data) = generate_mip_chain(&img);
        // 4x4 → max_dim=4, ilog2=2 → mips = 3 (4, 2, 1)
        assert_eq!(mip_count, 3);

        let expected_size = (4 * 4 + 2 * 2 + 1) * 4;
        assert_eq!(data.len(), expected_size);
    }
}
