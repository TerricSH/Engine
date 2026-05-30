use ash::vk;
use engine_renderer::render_graph::{CompiledBarrier, PipeStage, ResourceState};

use super::VulkanDevice;

impl VulkanDevice {
    pub(crate) fn apply_render_graph_barriers(&self, fi: usize, barriers: &[CompiledBarrier]) {
        if barriers.is_empty() {
            return;
        }

        let mut image_barriers: Vec<vk::ImageMemoryBarrier<'static>> = Vec::new();
        let mut src_stage = vk::PipelineStageFlags::empty();
        let mut dst_stage = vk::PipelineStageFlags::empty();

        for barrier in barriers {
            let Some(image_barrier) = self.image_barrier_from_graph_barrier(barrier) else {
                continue;
            };

            src_stage |= pipeline_stage(barrier.src_stage);
            dst_stage |= pipeline_stage(barrier.dst_stage);
            image_barriers.push(image_barrier);
        }

        if image_barriers.is_empty() {
            return;
        }

        let d = &self.logical_device.device;
        let cmd = self.frame_sync[fi].command_buffer;

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
    }

    fn image_barrier_from_graph_barrier(
        &self,
        barrier: &CompiledBarrier,
    ) -> Option<vk::ImageMemoryBarrier<'static>> {
        if matches!(
            barrier.new_state,
            ResourceState::ColorAttachmentOptimal
                | ResourceState::DepthStencilAttachmentOptimal
                | ResourceState::PresentSrc
        ) {
            return None;
        }

        let (image, aspect_mask, layer_count) =
            self.graph_resource_image(&barrier.resource_name)?;
        Some(
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
        )
    }

    fn graph_resource_image(
        &self,
        resource_name: &str,
    ) -> Option<(vk::Image, vk::ImageAspectFlags, u32)> {
        let (image, aspect_mask, layer_count) = match resource_name {
            "hdr_color" => (self.hdr_color_image?, vk::ImageAspectFlags::COLOR, 1),
            "depth_stencil" => (self.depth_image?, vk::ImageAspectFlags::DEPTH, 1),
            "ssao_output" => (self.ssao_output_image?, vk::ImageAspectFlags::COLOR, 1),
            _ => return None,
        };

        if image == vk::Image::null() {
            return None;
        }

        Some((image, aspect_mask, layer_count))
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
