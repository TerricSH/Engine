//! `VK_KHR_surface` wrapper over an `ash_window` surface handle.

use ash::khr::surface;
use ash::vk;
use ash::{Entry, Instance as AshInstance};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::error::{VkResult, VulkanError};

pub struct Surface {
    pub loader: surface::Instance,
    pub surface: vk::SurfaceKHR,
}

impl Surface {
    /// # Safety
    /// Window/display handles must remain valid until the [`Surface`] is dropped.
    pub unsafe fn new(
        entry: &Entry,
        instance: &AshInstance,
        display_handle: RawDisplayHandle,
        window_handle: RawWindowHandle,
    ) -> VkResult<Self> {
        // SAFETY: `display_handle` / `window_handle` are valid per caller contract.
        let surface = unsafe {
            ash_window::create_surface(entry, instance, display_handle, window_handle, None)
        }
        .map_err(|r| VulkanError::vk("ash_window::create_surface", r))?;
        let loader = surface::Instance::new(entry, instance);
        Ok(Self { loader, surface })
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: invariant maintained by VulkanRenderer drop order.
        unsafe { self.loader.destroy_surface(self.surface, None) }
    }
}
