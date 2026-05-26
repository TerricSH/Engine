#![forbid(unsafe_code)]

use render_core::{AdapterInfo, Backend, BackendKind, Device, DeviceDescriptor, RhiError};

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectX12Backend;

impl DirectX12Backend {
    pub const fn new() -> Self {
        Self
    }
}

impl Backend for DirectX12Backend {
    fn kind(&self) -> BackendKind {
        BackendKind::DirectX12
    }

    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError> {
        Ok(Vec::new())
    }

    fn create_device(&self, _descriptor: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError> {
        Err(RhiError::UnsupportedBackend)
    }
}

pub fn backend() -> DirectX12Backend {
    DirectX12Backend::new()
}
