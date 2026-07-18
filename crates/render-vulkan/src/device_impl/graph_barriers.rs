use ash::vk;
use engine_renderer::render_graph2::{CompiledBarrier, PipeStage, ResourceState};

use super::VulkanDevice;

impl VulkanDevice {
    pub(crate) fn apply_render_graph_barriers(
        &self,
        fi: usize,
        barriers: &[CompiledBarrier],
    ) -> Result<(), String> {
        if barriers.is_empty() {
            return Ok(());
        }

        let mut image_barriers: Vec<vk::ImageMemoryBarrier<'static>> = Vec::new();
        let mut src_stage = vk::PipelineStageFlags::empty();
        let mut dst_stage = vk::PipelineStageFlags::empty();

        for barrier in barriers {
            let Some(image_barrier) = self.image_barrier_from_graph_barrier(barrier)? else {
                continue;
            };

            src_stage |= pipeline_stage(barrier.src_stage);
            dst_stage |= pipeline_stage(barrier.dst_stage);
            image_barriers.push(image_barrier);
        }

        if image_barriers.is_empty() {
            return Ok(());
        }

        let d = &self.logical_device.device;
        let cmd = self
            .frame_sync
            .get(fi)
            .ok_or_else(|| format!("render-graph frame index {fi} is out of range"))?
            .command_buffer;

        // SAFETY: command buffer is in recording state; barriers reference
        // valid images with the declared stage/access masks; device is alive.
        unsafe {
            d.cmd_pipeline_barrier(
                cmd,
                non_empty_stage(src_stage),
                non_empty_stage(dst_stage),
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &image_barriers,
            );
        }
        Ok(())
    }

    fn image_barrier_from_graph_barrier(
        &self,
        barrier: &CompiledBarrier,
    ) -> Result<Option<vk::ImageMemoryBarrier<'static>>, String> {
        // These logical resources are transitioned by the concrete render
        // passes that create/use them. They are still named explicitly here
        // so a typo or an undeclared custom resource fails closed.
        if matches!(
            barrier.resource_name.as_str(),
            "depth" | "shadow_map" | "shadow_depth" | "swapchain"
        ) {
            return Ok(None);
        }

        // Skip transitions FROM Undefined (no layout to preserve) and
        // PresentSrc (swapchain images managed externally).
        if matches!(
            barrier.old_state,
            ResourceState::Undefined | ResourceState::PresentSrc
        ) {
            // Validate the resource name even when no Vulkan command is
            // required, otherwise a custom typo would disappear silently.
            self.graph_resource_image(&barrier.resource_name)?;
            return Ok(None);
        }

        let (image, aspect_mask, layer_count) =
            self.graph_resource_image(&barrier.resource_name)?;
        Ok(Some(
            vk::ImageMemoryBarrier::default()
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count,
                })
                .src_access_mask(access_mask(barrier.old_state))
                .dst_access_mask(access_mask(barrier.new_state))
                .old_layout(image_layout(barrier.old_state))
                .new_layout(image_layout(barrier.new_state)),
        ))
    }

    fn graph_resource_image(
        &self,
        resource_name: &str,
    ) -> Result<(vk::Image, vk::ImageAspectFlags, u32), String> {
        let (image, aspect_mask, layer_count) = match resource_name {
            "hdr_color" => (
                self.hdr_color_image
                    .ok_or_else(|| "HDR graph resource is not initialized".to_owned())?,
                vk::ImageAspectFlags::COLOR,
                1,
            ),
            "depth_stencil" => (
                self.depth_image
                    .ok_or_else(|| "depth graph resource is not initialized".to_owned())?,
                vk::ImageAspectFlags::DEPTH,
                1,
            ),
            _ => {
                return Err(format!(
                "Vulkan backend has no image binding for render-graph resource '{resource_name}'"
            ))
            }
        };

        if image == vk::Image::null() {
            return Err(format!(
                "render-graph resource '{resource_name}' resolved to a null Vulkan image"
            ));
        }

        Ok((image, aspect_mask, layer_count))
    }
}

fn pipeline_stage(stage: PipeStage) -> vk::PipelineStageFlags {
    match stage {
        PipeStage::TopOfPipe => vk::PipelineStageFlags::TOP_OF_PIPE,
        PipeStage::ColorAttachmentOutput => vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
        PipeStage::EarlyFragmentTests => vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS,
        PipeStage::LateFragmentTests => vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
        PipeStage::FragmentShader => vk::PipelineStageFlags::FRAGMENT_SHADER,
        PipeStage::ComputeShader => vk::PipelineStageFlags::COMPUTE_SHADER,
        PipeStage::Transfer => vk::PipelineStageFlags::TRANSFER,
        PipeStage::BottomOfPipe => vk::PipelineStageFlags::BOTTOM_OF_PIPE,
    }
}

fn non_empty_stage(stage: vk::PipelineStageFlags) -> vk::PipelineStageFlags {
    if stage.is_empty() {
        vk::PipelineStageFlags::TOP_OF_PIPE
    } else {
        stage
    }
}

fn access_mask(state: ResourceState) -> vk::AccessFlags {
    match state {
        ResourceState::Undefined | ResourceState::PresentSrc => vk::AccessFlags::empty(),
        ResourceState::ColorAttachmentOptimal => vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        ResourceState::DepthStencilAttachmentOptimal => {
            vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE
        }
        ResourceState::DepthStencilReadOnlyOptimal | ResourceState::ShaderReadOnlyOptimal => {
            vk::AccessFlags::SHADER_READ
        }
        ResourceState::TransferSrcOptimal => vk::AccessFlags::TRANSFER_READ,
        ResourceState::TransferDstOptimal => vk::AccessFlags::TRANSFER_WRITE,
        ResourceState::General => vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
    }
}

fn image_layout(state: ResourceState) -> vk::ImageLayout {
    match state {
        ResourceState::Undefined => vk::ImageLayout::UNDEFINED,
        ResourceState::ColorAttachmentOptimal => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        ResourceState::DepthStencilAttachmentOptimal => {
            vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
        }
        ResourceState::DepthStencilReadOnlyOptimal => {
            vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL
        }
        ResourceState::ShaderReadOnlyOptimal => vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        ResourceState::TransferSrcOptimal => vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        ResourceState::TransferDstOptimal => vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        ResourceState::PresentSrc => vk::ImageLayout::PRESENT_SRC_KHR,
        ResourceState::General => vk::ImageLayout::GENERAL,
    }
}
