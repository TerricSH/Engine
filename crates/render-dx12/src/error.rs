use thiserror::Error;

/// Errors specific to the DirectX 12 backend.
#[derive(Debug, Error)]
pub enum Dx12Error {
    /// No suitable DX12 adapter was found on the system.
    #[error("no suitable DirectX 12 adapter found")]
    AdapterNotFound,

    /// Device creation failed with a driver-level error.
    #[error("device creation failed: {0}")]
    DeviceCreationFailed(String),

    /// The requested texture format is not supported by DX12.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
}
