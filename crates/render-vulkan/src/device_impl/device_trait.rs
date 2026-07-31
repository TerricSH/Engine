//! `render_core::Device` trait implementation for VulkanDevice.
//!
//! This file implements the render-core abstraction layer, covering buffer
//! creation, render-pass / framebuffer / pipeline creation, frame lifecycle
//! (begin/end), and pixel readback.

use ash::vk;
use std::ops::Range;

use render_core::{
    self, AdapterInfo, BindGroupLayoutDescriptor, BufferDescriptor, BufferHandle, BufferUsage,
    CommandEncoder as CmdEncoderTrait, FramebufferDescriptor, FramebufferHandle,
    PipelineDescriptor, PipelineHandle, PipelineLayoutDescriptor, PipelineLayoutHandle,
    RenderPassDescriptor, RenderPassHandle, RendererStatistics, ShaderModuleDescriptor,
    ShaderModuleHandle, ShaderStage, SurfaceDescriptor, SurfaceHandle, SwapchainDescriptor,
    SwapchainHandle, TextureDescriptor, TextureFormat, TextureHandle, TextureUsage,
};

use crate::allocator::{AllocationCreateDesc, MemoryLocation};

use super::{
    blend_attachment_from_mode, compare_op, default_dep,
    encoder::VkCmdEncoder,
    mk_sm, parse_polygon_mode, parse_sample_count, parse_topology,
    resource_kind_to_descriptor_type,
    slab::{BufEntry, FbEntry, PipeEntry, PlEntry, TexEntry},
    vfmt, VulkanDevice,
};

fn fallback_pipeline_set_layouts(
    frame: Option<vk::DescriptorSetLayout>,
    shadow: Option<vk::DescriptorSetLayout>,
    material: Option<vk::DescriptorSetLayout>,
) -> Result<Vec<vk::DescriptorSetLayout>, render_core::RhiError> {
    let slots = [frame, shadow, material];
    let Some(last_used) = slots.iter().rposition(Option::is_some) else {
        return Ok(Vec::new());
    };

    slots
        .into_iter()
        .take(last_used + 1)
        .enumerate()
        .map(|(set_index, layout)| match layout {
            Some(layout) if layout != vk::DescriptorSetLayout::null() => Ok(layout),
            _ => Err(render_core::RhiError::InvalidDescriptor {
                field: "bind_group_layouts".into(),
                reason: format!(
                    "descriptor set layout {set_index} must be initialized before later sets"
                ),
            }),
        })
        .collect()
}

fn validate_contiguous_bind_group_layouts(
    layouts: &[BindGroupLayoutDescriptor],
) -> Result<(), render_core::RhiError> {
    let Some(max_set) = layouts.iter().map(|layout| layout.set_index).max() else {
        return Ok(());
    };
    let mut seen = vec![false; max_set as usize + 1];
    for layout in layouts {
        let slot = &mut seen[layout.set_index as usize];
        if *slot {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "bind_group_layouts".into(),
                reason: format!("duplicate descriptor set layout {}", layout.set_index),
            });
        }
        *slot = true;
    }
    if let Some(missing) = seen.iter().position(|present| !present) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "bind_group_layouts".into(),
            reason: format!("missing descriptor set layout {missing}"),
        });
    }
    Ok(())
}

fn ordered_bind_group_layouts(
    layouts: &[BindGroupLayoutDescriptor],
) -> Result<Vec<&BindGroupLayoutDescriptor>, render_core::RhiError> {
    validate_contiguous_bind_group_layouts(layouts)?;
    let mut ordered: Vec<_> = layouts.iter().collect();
    ordered.sort_by_key(|layout| layout.set_index);
    Ok(ordered)
}

fn vulkan_descriptor_bindings(
    layout: &BindGroupLayoutDescriptor,
) -> Result<Vec<vk::DescriptorSetLayoutBinding<'static>>, render_core::RhiError> {
    let mut binding_indices = std::collections::BTreeSet::new();
    layout
        .bindings
        .iter()
        .map(|binding| {
            if !binding_indices.insert(binding.binding) {
                return Err(render_core::RhiError::InvalidDescriptor {
                    field: "bind_group_layouts.bindings".into(),
                    reason: format!(
                        "descriptor set {} contains duplicate binding {}",
                        layout.set_index, binding.binding
                    ),
                });
            }
            Ok(vk::DescriptorSetLayoutBinding::default()
                .binding(binding.binding)
                .descriptor_type(resource_kind_to_descriptor_type(&binding.resource_kind)?)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT))
        })
        .collect()
}

fn color_attachment_format(
    requested: Option<&TextureFormat>,
    swapchain_format: Option<vk::Format>,
) -> vk::Format {
    match requested {
        Some(TextureFormat::Bgra8Unorm) => swapchain_format.unwrap_or(vk::Format::B8G8R8A8_UNORM),
        Some(TextureFormat::Rgba8Unorm) => vk::Format::R8G8B8A8_UNORM,
        Some(TextureFormat::Rgba16Float) => vk::Format::R16G16B16A16_SFLOAT,
        _ => swapchain_format.unwrap_or(vk::Format::B8G8R8A8_UNORM),
    }
}

fn texture_format(format: TextureFormat) -> vk::Format {
    match format {
        TextureFormat::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
        TextureFormat::Bgra8Unorm => vk::Format::B8G8R8A8_UNORM,
        TextureFormat::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
        TextureFormat::Depth32Float => vk::Format::D32_SFLOAT,
        _ => vk::Format::UNDEFINED,
    }
}

fn checked_buffer_write_range(
    buffer_size: u64,
    offset: u64,
    data_len: usize,
) -> Result<Range<usize>, render_core::RhiError> {
    let start = usize::try_from(offset).map_err(|_| render_core::RhiError::InvalidDescriptor {
        field: "write_buffer.range".into(),
        reason: format!("offset {offset} cannot be represented on this platform"),
    })?;
    let logical_size =
        usize::try_from(buffer_size).map_err(|_| render_core::RhiError::Backend {
            detail: format!("buffer size {buffer_size} cannot be represented on this platform"),
        })?;
    let end =
        start
            .checked_add(data_len)
            .ok_or_else(|| render_core::RhiError::InvalidDescriptor {
                field: "write_buffer.range".into(),
                reason: format!("offset {offset} plus {data_len} bytes overflows address space"),
            })?;
    if end > logical_size {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "write_buffer.range".into(),
            reason: format!(
                "write range {offset}..{} exceeds buffer size {buffer_size}",
                offset.saturating_add(data_len as u64)
            ),
        });
    }
    Ok(start..end)
}

fn vertex_format_size(format: &str) -> Option<u32> {
    match format {
        "float32x2" => Some(8),
        "float32x3" => Some(12),
        "float32x4" | "uint32x4" => Some(16),
        _ => None,
    }
}

fn validate_graphics_pipeline_descriptor(
    desc: &PipelineDescriptor,
) -> Result<(), render_core::RhiError> {
    let sample_count = desc.sample_count.unwrap_or(1);
    if !matches!(sample_count, 1 | 2 | 4 | 8) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.sample_count".into(),
            reason: format!("unsupported sample count {sample_count}"),
        });
    }
    if !matches!(
        desc.topology.as_deref(),
        None | Some("point_list")
            | Some("line_list")
            | Some("line_strip")
            | Some("triangle_list")
            | Some("triangle_strip")
            | Some("triangle_fan")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.topology".into(),
            reason: format!("unsupported topology {:?}", desc.topology),
        });
    }
    if !matches!(
        desc.polygon_mode.as_deref(),
        None | Some("fill") | Some("line") | Some("point")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.polygon_mode".into(),
            reason: format!("unsupported polygon mode {:?}", desc.polygon_mode),
        });
    }
    if !matches!(
        desc.raster_state.cull_mode.as_deref(),
        None | Some("none") | Some("front") | Some("back")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.raster_state.cull_mode".into(),
            reason: format!("unsupported cull mode {:?}", desc.raster_state.cull_mode),
        });
    }
    if !matches!(
        desc.raster_state.front_face.as_deref(),
        None | Some("clockwise") | Some("cw") | Some("counter_clockwise") | Some("ccw")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.raster_state.front_face".into(),
            reason: format!("unsupported front face {:?}", desc.raster_state.front_face),
        });
    }
    if !matches!(
        desc.depth_state.compare.as_deref(),
        None | Some("less") | Some("equal") | Some("lequal") | Some("greater") | Some("always")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.depth_state.compare".into(),
            reason: format!(
                "unsupported depth comparison {:?}",
                desc.depth_state.compare
            ),
        });
    }
    if !matches!(
        desc.blend_state.mode.as_deref(),
        None | Some("Opaque") | Some("Alpha") | Some("Additive") | Some("Multiply")
    ) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.blend_state.mode".into(),
            reason: format!("unsupported blend mode {:?}", desc.blend_state.mode),
        });
    }
    if desc.render_targets.contains(&TextureFormat::Depth32Float) {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.render_targets".into(),
            reason: "Depth32Float is not a color render target".into(),
        });
    }
    if let Some(format) = desc.depth_state.format {
        if format != TextureFormat::Depth32Float {
            return Err(render_core::RhiError::UnsupportedFeature {
                feature: format!("Vulkan pipeline depth format {format:?}"),
            });
        }
    }
    if !desc.vertex_layout.attributes.is_empty() && desc.vertex_layout.stride_bytes == 0 {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.vertex_layout.stride_bytes".into(),
            reason: "a non-empty vertex layout requires a non-zero stride".into(),
        });
    }
    for attribute in &desc.vertex_layout.attributes {
        let size = vertex_format_size(&attribute.format).ok_or_else(|| {
            render_core::RhiError::InvalidDescriptor {
                field: "pipeline.vertex_layout.attributes.format".into(),
                reason: format!("unsupported vertex format '{}'", attribute.format),
            }
        })?;
        if attribute
            .offset_bytes
            .checked_add(size)
            .is_none_or(|end| end > desc.vertex_layout.stride_bytes)
        {
            return Err(render_core::RhiError::InvalidDescriptor {
                field: "pipeline.vertex_layout.attributes.offset_bytes".into(),
                reason: format!(
                    "attribute '{}' exceeds vertex stride {}",
                    attribute.semantic, desc.vertex_layout.stride_bytes
                ),
            });
        }
    }
    let mut specialization_ids = std::collections::BTreeSet::new();
    if let Some(duplicate) = desc
        .specialization
        .iter()
        .find(|constant| !specialization_ids.insert(constant.id))
    {
        return Err(render_core::RhiError::InvalidDescriptor {
            field: "pipeline.specialization".into(),
            reason: format!("duplicate specialization constant id {}", duplicate.id),
        });
    }
    Ok(())
}

fn vulkan_specialization_data(
    constants: &[render_core::SpecConstant],
) -> (Vec<u8>, Vec<vk::SpecializationMapEntry>) {
    let mut data = Vec::with_capacity(constants.len() * std::mem::size_of::<u32>());
    let mut entries = Vec::with_capacity(constants.len());
    for constant in constants {
        let offset = data.len() as u32;
        let bytes = match constant.value {
            render_core::SpecValue::Bool(value) => u32::from(value).to_ne_bytes(),
            render_core::SpecValue::U32(value) => value.to_ne_bytes(),
            render_core::SpecValue::F32(value) => value.to_ne_bytes(),
        };
        data.extend_from_slice(&bytes);
        entries.push(vk::SpecializationMapEntry {
            constant_id: constant.id,
            offset,
            size: bytes.len(),
        });
    }
    (data, entries)
}

mod frame;
mod pipeline;
mod render_targets;
mod resources;

use frame::vulkan_device_frame_methods;
use pipeline::vulkan_device_pipeline_methods;
use render_targets::vulkan_device_render_target_methods;
use resources::vulkan_device_resource_methods;

impl render_core::Device for VulkanDevice {
    vulkan_device_resource_methods!();
    vulkan_device_render_target_methods!();
    vulkan_device_pipeline_methods!();
    vulkan_device_frame_methods!();
}

#[cfg(test)]
mod tests {
    include!("device_trait/tests.rs");
}
