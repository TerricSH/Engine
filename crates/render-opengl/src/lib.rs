#![forbid(unsafe_code)]

use render_core::{AdapterInfo, Backend, BackendKind, Device, DeviceDescriptor, RhiError};

#[derive(Clone, Copy, Debug, Default)]
pub struct OpenGlBackend;

impl OpenGlBackend {
    pub const fn new() -> Self {
        Self
    }
}

impl Backend for OpenGlBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::OpenGl
    }

    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError> {
        Ok(Vec::new())
    }

    fn create_device(&self, _descriptor: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError> {
        Err(RhiError::UnsupportedBackend)
    }
}

pub fn backend() -> OpenGlBackend {
    OpenGlBackend::new()
}
