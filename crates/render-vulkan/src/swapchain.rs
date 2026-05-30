//! Swapchain + image views with resize-friendly recreation.

use ash::khr::{surface, swapchain};
use ash::vk;
use ash::{Device as AshDevice, Instance as AshInstance};

use crate::error::{VkResult, VulkanError};

pub struct Swapchain {
    pub loader: swapchain::Device,
    pub swapchain: vk::SwapchainKHR,
    pub format: vk::Format,
    pub _color_space: vk::ColorSpaceKHR,
    pub extent: vk::Extent2D,
    pub images: Vec<vk::Image>,
    pub image_views: Vec<vk::ImageView>,
    device: AshDevice,
}

impl Swapchain {
    /// Build a swapchain sized for the requested `width`/`height`.
    ///
    /// Returns `Err(VulkanError::SurfaceMinimized)` if the surface's
    /// current extent is `0x0` (e.g. window minimized) so the renderer
    /// can pause without leaking a zero-sized swapchain.
    ///
    /// # Safety
    /// All handles in scope must remain valid.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn new(
        instance: &AshInstance,
        device: AshDevice,
        physical_device: vk::PhysicalDevice,
        queue_family_index: u32,
        surface_loader: &surface::Instance,
        surface: vk::SurfaceKHR,
        width: u32,
        height: u32,
    ) -> VkResult<Self> {
        // SAFETY: surface + physical device are valid.
        let capabilities = unsafe {
            surface_loader.get_physical_device_surface_capabilities(physical_device, surface)
        }
        .map_err(|r| VulkanError::vk("get_physical_device_surface_capabilities", r))?;

        let extent = compute_extent(&capabilities, width, height);
        if extent.width == 0 || extent.height == 0 {
            return Err(VulkanError::SurfaceMinimized);
        }

        // SAFETY: surface + physical device are valid.
        let formats =
            unsafe { surface_loader.get_physical_device_surface_formats(physical_device, surface) }
                .map_err(|r| VulkanError::vk("get_physical_device_surface_formats", r))?;
        let surface_format = choose_format(&formats);

        // SAFETY: surface + physical device are valid.
        let present_modes = unsafe {
            surface_loader.get_physical_device_surface_present_modes(physical_device, surface)
        }
        .map_err(|r| VulkanError::vk("get_physical_device_surface_present_modes", r))?;
        let present_mode = choose_present_mode(&present_modes);

        let mut image_count = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 && image_count > capabilities.max_image_count {
            image_count = capabilities.max_image_count;
        }

        let pre_transform = if capabilities
            .supported_transforms
            .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
        {
            vk::SurfaceTransformFlagsKHR::IDENTITY
        } else {
            capabilities.current_transform
        };

        let _queue_family_index = queue_family_index;
        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(pre_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        let loader = swapchain::Device::new(instance, &device);
        // SAFETY: all referenced slices outlive this call.
        let swapchain = unsafe { loader.create_swapchain(&create_info, None) }
            .map_err(|r| VulkanError::vk("create_swapchain", r))?;
        // SAFETY: swapchain is valid.
        let images = unsafe { loader.get_swapchain_images(swapchain) }
            .map_err(|r| VulkanError::vk("get_swapchain_images", r))?;

        let mut image_views = Vec::with_capacity(images.len());
        for &image in &images {
            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(surface_format.format)
                .components(vk::ComponentMapping {
                    r: vk::ComponentSwizzle::IDENTITY,
                    g: vk::ComponentSwizzle::IDENTITY,
                    b: vk::ComponentSwizzle::IDENTITY,
                    a: vk::ComponentSwizzle::IDENTITY,
                })
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            // SAFETY: device + view_info valid.
            let view = unsafe { device.create_image_view(&view_info, None) }
                .map_err(|r| VulkanError::vk("create_image_view", r))?;
            image_views.push(view);
        }

        tracing::info!(
            target: "vulkan",
            width = extent.width,
            height = extent.height,
            format = ?surface_format.format,
            present_mode = ?present_mode,
            images = images.len(),
            "swapchain created"
        );

        Ok(Self {
            loader,
            swapchain,
            format: surface_format.format,
            _color_space: surface_format.color_space,
            extent,
            images,
            image_views,
            device,
        })
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        // SAFETY: caller has already waited for in-flight frames to finish
        // (VulkanRenderer drop path / resize path call device_wait_idle).
        unsafe {
            for &view in &self.image_views {
                self.device.destroy_image_view(view, None);
            }
            self.loader.destroy_swapchain(self.swapchain, None);
        }
    }
}

fn compute_extent(caps: &vk::SurfaceCapabilitiesKHR, width: u32, height: u32) -> vk::Extent2D {
    if caps.current_extent.width != u32::MAX {
        return caps.current_extent;
    }
    vk::Extent2D {
        width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
        height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
    }
}

fn choose_format(formats: &[vk::SurfaceFormatKHR]) -> vk::SurfaceFormatKHR {
    if let Some(&fmt) = formats.iter().find(|f| {
        f.format == vk::Format::B8G8R8A8_SRGB && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
    }) {
        return fmt;
    }
    formats.first().copied().unwrap_or(vk::SurfaceFormatKHR {
        format: vk::Format::B8G8R8A8_UNORM,
        color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
    })
}

fn choose_present_mode(modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
    if modes.contains(&vk::PresentModeKHR::MAILBOX) {
        vk::PresentModeKHR::MAILBOX
    } else {
        vk::PresentModeKHR::FIFO
    }
}
