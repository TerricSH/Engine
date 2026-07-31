use super::*;

pub(super) fn decode_gltf_image(
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
