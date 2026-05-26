//! Physical device selection and queue family discovery.

use ash::khr::{surface, swapchain};
use ash::vk;
use ash::Instance as AshInstance;

use crate::error::{VkResult, VulkanError};

#[derive(Clone, Debug)]
pub struct AdapterSelection {
    pub physical_device: vk::PhysicalDevice,
    pub queue_family_index: u32,
    pub properties: vk::PhysicalDeviceProperties,
}

/// # Safety
/// `instance` / `surface_loader` / `surface` must remain valid.
pub unsafe fn select(
    instance: &AshInstance,
    surface_loader: &surface::Instance,
    surface: vk::SurfaceKHR,
) -> VkResult<AdapterSelection> {
    // SAFETY: instance is valid.
    let devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(|r| VulkanError::vk("enumerate_physical_devices", r))?;

    let mut best: Option<(AdapterSelection, u32)> = None;
    for device in devices {
        let Some(selection) =
            // SAFETY: device handle from the enumeration call above is valid until instance destruction.
            (unsafe { evaluate_device(instance, surface_loader, surface, device) })?
        else {
            continue;
        };
        let score = score_device(&selection.properties);
        match best {
            Some((_, best_score)) if best_score >= score => {}
            _ => best = Some((selection, score)),
        }
    }

    best.map(|(sel, _)| sel)
        .ok_or(VulkanError::NoSuitableAdapter)
}

unsafe fn evaluate_device(
    instance: &AshInstance,
    surface_loader: &surface::Instance,
    surface: vk::SurfaceKHR,
    physical_device: vk::PhysicalDevice,
) -> VkResult<Option<AdapterSelection>> {
    // SAFETY: physical_device is valid per caller.
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };

    // Require VK_KHR_swapchain.
    // SAFETY: physical_device is valid.
    let extensions = unsafe { instance.enumerate_device_extension_properties(physical_device) }
        .map_err(|r| VulkanError::vk("enumerate_device_extension_properties", r))?;
    let has_swapchain = extensions.iter().any(|ext| {
        // SAFETY: extension_name is a null-terminated C string per Vulkan spec.
        let name = unsafe { std::ffi::CStr::from_ptr(ext.extension_name.as_ptr()) };
        name == swapchain::NAME
    });
    if !has_swapchain {
        return Ok(None);
    }

    // SAFETY: physical_device + surface are valid.
    let queue_families =
        unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
    for (index, family) in queue_families.iter().enumerate() {
        let supports_graphics = family.queue_flags.contains(vk::QueueFlags::GRAPHICS);
        // SAFETY: physical_device + surface are valid.
        let supports_present = unsafe {
            surface_loader.get_physical_device_surface_support(
                physical_device,
                index as u32,
                surface,
            )
        }
        .map_err(|r| VulkanError::vk("get_physical_device_surface_support", r))?;
        if supports_graphics && supports_present {
            return Ok(Some(AdapterSelection {
                physical_device,
                queue_family_index: index as u32,
                properties,
            }));
        }
    }
    Ok(None)
}

fn score_device(props: &vk::PhysicalDeviceProperties) -> u32 {
    match props.device_type {
        vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
        vk::PhysicalDeviceType::INTEGRATED_GPU => 500,
        vk::PhysicalDeviceType::VIRTUAL_GPU => 200,
        vk::PhysicalDeviceType::CPU => 50,
        _ => 1,
    }
}
