#![forbid(unsafe_code)]

use render_core::{AdapterInfo, Backend, BackendKind, Device, DeviceDescriptor, RhiError};

#[derive(Clone, Copy, Debug, Default)]
pub struct VulkanBackend;

impl VulkanBackend {
    pub const fn new() -> Self {
        Self
    }
}

impl Backend for VulkanBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Vulkan
    }

    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError> {
        Ok(Vec::new())
    }

    fn create_device(&self, _descriptor: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError> {
        Err(RhiError::UnsupportedBackend)
    }
}

pub fn backend() -> VulkanBackend {
    VulkanBackend::new()
}
