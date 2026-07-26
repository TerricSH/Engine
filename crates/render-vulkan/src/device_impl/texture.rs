//! Material texture upload and descriptor binding (Phase 3.1).
//!
//! Provides methods to:
//! - Upload CPU pixel data to a GPU texture and cache it.
//! - Write a COMBINED_IMAGE_SAMPLER descriptor to bind a cached texture
//!   at set=2, binding=1 (binding=0 is the MaterialUBO).

use ash::vk;

use crate::error::VkResult;

use super::{reload::SampledTextureDescriptor, VulkanDevice};

pub(crate) const FALLBACK_MATERIAL_TEXTURE_ID: &str = "__engine_fallback_white";

impl VulkanDevice {
    // ------------------------------------------------------------------
    // Texture upload
    // ------------------------------------------------------------------

    // ------------------------------------------------------------------
    // Descriptor binding
    // ------------------------------------------------------------------

    /// Ensure every material can bind a valid texture descriptor even when
    /// the source material has no base-color texture.
    pub(crate) fn create_fallback_material_texture(&mut self) -> VkResult<()> {
        if self.textures.contains_key(FALLBACK_MATERIAL_TEXTURE_ID) {
            return Ok(());
        }

        let texture = self.create_sampled_texture_resource(
            SampledTextureDescriptor::rgba8_unorm(1, 1, 1, &[255; 4]),
        )?;

        self.textures
            .insert(FALLBACK_MATERIAL_TEXTURE_ID.to_owned(), texture);
        Ok(())
    }

    /// Write the COMBINED_IMAGE_SAMPLER descriptor for `asset_id` into
    /// one declared material binding in the given descriptor set.
    ///
    /// Returns `Ok(true)` when the descriptor was written, `Ok(false)` if
    /// the texture is not in the cache, or `Err` on device error.
    pub(crate) fn bind_material_texture_at(
        &self,
        asset_id: &str,
        binding: u32,
        desc_set: vk::DescriptorSet,
    ) -> VkResult<bool> {
        let Some(gpu_tex) = self.textures.get(asset_id) else {
            return Ok(false);
        };
        let d = &self.logical_device.device;

        let image_info = [vk::DescriptorImageInfo::default()
            .sampler(gpu_tex.sampler)
            .image_view(gpu_tex.view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(desc_set)
            .dst_binding(binding)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&image_info)];
        // SAFETY: `d` is a valid AshDevice; `desc_set` is a valid descriptor
        // set allocated from `material_desc_pool`; `gpu_tex` resources are
        // alive and valid.
        unsafe {
            d.update_descriptor_sets(&writes, &[]);
        }
        Ok(true)
    }

    // ------------------------------------------------------------------
    // Cleanup
    // ------------------------------------------------------------------

    /// Destroy a single `GpuTexture` (image, view, allocation, sampler).
    pub(crate) fn destroy_gpu_texture(&self, tex: super::GpuTexture) {
        let d = &self.logical_device.device;
        // SAFETY: all handles were created by this device and are still alive.
        unsafe {
            d.destroy_sampler(tex.sampler, None);
            d.destroy_image_view(tex.view, None);
            d.destroy_image(tex.image, None);
        }
        let mut allocation = tex.allocation;
        match self.logical_device.allocator().lock() {
            Ok(mut guard) => guard.free(&mut allocation),
            Err(poisoned) => {
                tracing::error!(
                    target: "vulkan::resources",
                    "allocator mutex was poisoned while destroying a sampled texture"
                );
                poisoned.into_inner().free(&mut allocation);
            }
        }
    }

    /// Destroy all cached GPU textures.
    ///
    /// Does not destroy the material descriptor pool or layout; those are
    /// handled by the `Drop` impl.
    pub(crate) fn destroy_material_textures(&mut self) {
        // Drain into a local vec to avoid simultaneous &self borrow.
        let entries: Vec<super::GpuTexture> = self.textures.drain().map(|(_, t)| t).collect();
        for tex in entries {
            self.destroy_gpu_texture(tex);
        }
    }
}
