use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RhiError {
    #[error("unsupported backend")]
    UnsupportedBackend,
    #[error("unsupported feature: {feature}")]
    UnsupportedFeature { feature: String },
    #[error("unsupported limit {limit}: requested {requested}, available {available}")]
    UnsupportedLimit {
        limit: String,
        requested: u64,
        available: u64,
    },
    #[error("invalid descriptor field {field}: {reason}")]
    InvalidDescriptor { field: String, reason: String },
    #[error("invalid resource handle")]
    InvalidHandle,
    #[error("device lost")]
    DeviceLost,
    #[error("surface lost")]
    SurfaceLost,
    #[error("out of memory")]
    OutOfMemory,
    #[error("allocation failed for {bytes} bytes")]
    AllocationFailed { bytes: u64 },
    #[error("validation failed: {detail}")]
    ValidationFailed { detail: String },
    #[error("incompatible bind layout: {reason}")]
    IncompatibleBindLayout { reason: String },
    #[error("backend error: {detail}")]
    Backend { detail: String },
}

impl RhiError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedBackend => "rhi.unsupported_backend",
            Self::UnsupportedFeature { .. } => "rhi.unsupported_feature",
            Self::UnsupportedLimit { .. } => "rhi.unsupported_limit",
            Self::InvalidDescriptor { .. } => "rhi.invalid_descriptor",
            Self::InvalidHandle => "rhi.invalid_handle",
            Self::DeviceLost => "rhi.device_lost",
            Self::SurfaceLost => "rhi.surface_lost",
            Self::OutOfMemory => "rhi.out_of_memory",
            Self::AllocationFailed { .. } => "rhi.allocation_failed",
            Self::ValidationFailed { .. } => "rhi.validation_failed",
            Self::IncompatibleBindLayout { .. } => "rhi.incompatible_bind_layout",
            Self::Backend { .. } => "rhi.backend",
        }
    }

    pub const fn severity(&self) -> &'static str {
        match self {
            Self::UnsupportedBackend
            | Self::UnsupportedFeature { .. }
            | Self::UnsupportedLimit { .. }
            | Self::DeviceLost
            | Self::OutOfMemory => "fatal",
            Self::SurfaceLost => "error-recoverable",
            Self::InvalidDescriptor { .. }
            | Self::InvalidHandle
            | Self::AllocationFailed { .. }
            | Self::ValidationFailed { .. }
            | Self::IncompatibleBindLayout { .. }
            | Self::Backend { .. } => "error",
        }
    }
}
