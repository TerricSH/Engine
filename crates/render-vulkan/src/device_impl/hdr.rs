//! HDR offscreen rendering + tone-mapping resources for VulkanDevice.
//!
//! Phase 2.1: Creates an RGBA16F color texture, a forward HDR render pass /
//! pipeline, a tone-mapping render pass / pipeline, and per-swapchain tone
//! framebuffers.

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{mk_sm, mrt_blend_attachments, VulkanDevice};

mod cleanup;
mod forward;
mod targets;
mod tone_mapping;

use targets::free_hdr_target_allocation;
