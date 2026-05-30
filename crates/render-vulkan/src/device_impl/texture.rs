//! Material texture upload and descriptor binding (Phase 3.1).
//!
//! Provides methods to:
//! - Upload CPU pixel data to a GPU texture and cache it.
//! - Write a COMBINED_IMAGE_SAMPLER descriptor to bind a cached texture
//!   at set=2, binding=1 (binding=0 is the MaterialUBO).

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::VulkanDevice;

impl VulkanDevice {
    // ------------------------------------------------------------------
    // Texture upload
    // ------------------------------------------------------------------

    /// Upload a 2D texture to the GPU and cache it under `id`.
    ///
    /// `data` must be `width × height × 4` bytes (R8G8B8A8_UNORM).
    /// Creates the image, image view, and a sampler with linear filtering and
    /// repeat addressing mode.
    ///
    /// If a texture with the same `id` already exists, it is replaced (old
    /// resources are freed immediately).
    pub(crate) fn upload_texture(
        &mut self,
        id: &str,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> VkResult<()> {
        // Remove existing entry if present (drop old resources).
        if let Some(old) = self.textures.remove(id) {
            self.destroy_gpu_texture(old);
        }

        let d = &self.logical_device.device;
        let _allocator = self.logical_device.allocator();

        // ---- 1. Create image + upload via staging ----
        let (image, image_view, allocation) =
            self.create_sampled_texture(width, height, 1, data)?;

        // ---- 2. Create sampler (linear filtering, repeat) ----
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .min_lod(0.0)
            .max_lod(1.0)
            .mip_lod_bias(0.0)
            .anisotropy_enable(false);
        // SAFETY: `d` is a valid AshDevice; `sampler_info` describes a valid
        // sampler; `None` means no custom allocator.
        let sampler = unsafe { d.create_sampler(&sampler_info, None) }
            .map_err(|r| VulkanError::vk("create_material_sampler", r))?;

        let gpu_tex = super::GpuTexture {
            image,
            view: image_view,
            allocation,
            sampler,
        };
        self.textures.insert(id.to_string(), gpu_tex);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Descriptor binding
    // ------------------------------------------------------------------

    /// Write the COMBINED_IMAGE_SAMPLER descriptor for `asset_id` into
    /// the given `desc_set` at binding=1 (set=2 — binding=0 is the MaterialUBO).
    ///
    /// Returns `Ok(true)` when the descriptor was written, `Ok(false)` if
    /// the texture is not in the cache, or `Err` on device error.
    pub(crate) fn bind_material_texture(
        &self,
        asset_id: &str,
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
            .dst_binding(1)
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
    fn destroy_gpu_texture(&self, tex: super::GpuTexture) {
        let d = &self.logical_device.device;
        // SAFETY: all handles were created by this device and are still alive.
        unsafe {
            d.destroy_sampler(tex.sampler, None);
            d.destroy_image_view(tex.view, None);
            d.destroy_image(tex.image, None);
        }
        if let Ok(mut guard) = self.logical_device.allocator().lock() {
            let mut a = tex.allocation;
            guard.free(&mut a);
        }
    }

    /// Destroy all cached GPU textures.
    ///
    /// Does NOT destroy the material descriptor pool or layout 鈥?those are
    /// handled by the `Drop` impl.
    pub(crate) fn destroy_material_textures(&mut self) {
        // Drain into a local vec to avoid simultaneous &self borrow.
        let entries: Vec<super::GpuTexture> = self.textures.drain().map(|(_, t)| t).collect();
        for tex in entries {
            self.destroy_gpu_texture(tex);
        }
    }
}
