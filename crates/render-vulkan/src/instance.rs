//! Vulkan entry, instance, validation layers, and debug messenger.

use std::ffi::{c_void, CStr};

use ash::ext::debug_utils;
use ash::vk;
use ash::{Entry, Instance as AshInstance};
use raw_window_handle::RawDisplayHandle;

use crate::error::{VkResult, VulkanError};

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

pub struct Instance {
    pub entry: Entry,
    pub instance: AshInstance,
    debug_utils_loader: Option<debug_utils::Instance>,
    debug_messenger: Option<vk::DebugUtilsMessengerEXT>,
}

impl Instance {
    /// Create a Vulkan instance suitable for desktop Gate 2 rendering.
    ///
    /// # Safety
    /// Caller must hold the raw display handle valid for the duration of
    /// this call. The returned [`Instance`] owns all underlying resources
    /// and destroys them on drop.
    pub unsafe fn new(display_handle: RawDisplayHandle, enable_validation: bool) -> VkResult<Self> {
        // SAFETY: Entry::load dlopens the platform Vulkan loader.
        let entry = unsafe { Entry::load() }.map_err(|e| VulkanError::Loader(e.to_string()))?;

        let app_name = c"engine-sandbox";
        let engine_name = c"engine";
        let app_info = vk::ApplicationInfo::default()
            .application_name(app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(engine_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_2);

        // Required surface extensions per platform.
        let mut extension_names: Vec<*const i8> =
            ash_window::enumerate_required_extensions(display_handle)
                .map_err(|r| VulkanError::vk("enumerate_required_extensions", r))?
                .to_vec();

        // Probe optional validation-related extensions and layers.
        let mut layer_names: Vec<*const i8> = Vec::new();
        let validation_layer_available = enable_validation && validation_layer_present(&entry)?;
        if enable_validation && !validation_layer_available {
            tracing::warn!(
                target: "vulkan",
                "validation requested but VK_LAYER_KHRONOS_validation is not installed; \
                 install the LunarG Vulkan SDK to enable validation"
            );
        }
        if validation_layer_available {
            layer_names.push(VALIDATION_LAYER.as_ptr());
            extension_names.push(debug_utils::NAME.as_ptr());
        }

        let create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extension_names)
            .enabled_layer_names(&layer_names);

        // SAFETY: the slices above outlive this call.
        let instance = unsafe { entry.create_instance(&create_info, None) }
            .map_err(|r| VulkanError::vk("create_instance", r))?;

        let (debug_utils_loader, debug_messenger) = if validation_layer_available {
            let loader = debug_utils::Instance::new(&entry, &instance);
            let messenger_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                        | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                        | vk::DebugUtilsMessageSeverityFlagsEXT::INFO,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(debug_callback));
            // SAFETY: messenger_info outlives this call.
            let messenger = unsafe { loader.create_debug_utils_messenger(&messenger_info, None) }
                .map_err(|r| VulkanError::vk("create_debug_utils_messenger", r))?;
            (Some(loader), Some(messenger))
        } else {
            (None, None)
        };

        tracing::info!(
            target: "vulkan",
            validation = validation_layer_available,
            "vulkan instance created"
        );

        Ok(Self {
            entry,
            instance,
            debug_utils_loader,
            debug_messenger,
        })
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: resources outlived all dependent objects (renderer drops
        // pipeline/swapchain/device/surface in the correct order before
        // Instance is dropped because field order in VulkanRenderer puts
        // Instance last).
        unsafe {
            if let (Some(loader), Some(messenger)) =
                (self.debug_utils_loader.as_ref(), self.debug_messenger)
            {
                loader.destroy_debug_utils_messenger(messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

fn validation_layer_present(entry: &Entry) -> VkResult<bool> {
    // SAFETY: entry is valid; the returned vector is freshly allocated.
    let layers = unsafe { entry.enumerate_instance_layer_properties() }
        .map_err(|r| VulkanError::vk("enumerate_instance_layer_properties", r))?;
    for layer in &layers {
        // SAFETY: layer.layer_name is a null-terminated C string per Vulkan spec.
        let name = unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) };
        if name == VALIDATION_LAYER {
            return Ok(true);
        }
    }
    Ok(false)
}

unsafe extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _msg_type: vk::DebugUtilsMessageTypeFlagsEXT,
    callback_data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user_data: *mut c_void,
) -> vk::Bool32 {
    if callback_data.is_null() {
        return vk::FALSE;
    }
    // SAFETY: Vulkan guarantees `p_message` is a valid C string for the
    // duration of this callback.
    let message = unsafe {
        let ptr = (*callback_data).p_message;
        if ptr.is_null() {
            return vk::FALSE;
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    };
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        tracing::error!(target: "vulkan", "{}", message);
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        tracing::warn!(target: "vulkan", "{}", message);
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::INFO) {
        tracing::info!(target: "vulkan", "{}", message);
    } else {
        tracing::debug!(target: "vulkan", "{}", message);
    }
    vk::FALSE
}
