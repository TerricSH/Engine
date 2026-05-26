//! `vk::Result` -> internal [`VulkanError`] -> public [`RhiError`] mapping.

use ash::vk;
use render_core::RhiError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VulkanError {
    #[error("vulkan loader failed: {0}")]
    Loader(String),
    #[error("vulkan call `{op}` failed: {result:?}")]
    Vk {
        op: &'static str,
        result: vk::Result,
    },
    #[error("gpu allocation failed: {0}")]
    Allocation(String),
    #[error("allocation for `{0}` is not CPU mapped")]
    MemoryNotMapped(&'static str),
    #[error("no physical device satisfies Gate 2 requirements")]
    NoSuitableAdapter,
    #[error("no queue family supports graphics + present on the chosen surface")]
    NoSuitableQueue,
    #[error("required vulkan extension missing: {0}")]
    MissingExtension(&'static str),
    #[error(
        "shader artifact `{0}` is empty; compile `shaders/{0}` to SPIR-V (see shaders/README.md)"
    )]
    MissingShader(&'static str),
    #[error("swapchain extent is zero (window minimized); rendering paused")]
    SurfaceMinimized,
    #[error("swapchain is out of date; recreation required")]
    SwapchainOutOfDate,
    #[error("raw window handle is not supported on this platform: {0}")]
    UnsupportedWindow(&'static str),
}

impl VulkanError {
    pub const fn vk(op: &'static str, result: vk::Result) -> Self {
        Self::Vk { op, result }
    }

    pub fn into_rhi(self) -> RhiError {
        match self {
            Self::Loader(detail) => RhiError::Backend {
                detail: format!("vulkan loader: {detail}"),
            },
            Self::Vk { op, result } => RhiError::Backend {
                detail: format!("{op}: {result:?}"),
            },
            Self::Allocation(detail) => RhiError::Backend {
                detail: format!("gpu allocation: {detail}"),
            },
            Self::MemoryNotMapped(name) => RhiError::Backend {
                detail: format!("allocation is not CPU mapped: {name}"),
            },
            Self::NoSuitableAdapter | Self::NoSuitableQueue => RhiError::UnsupportedBackend,
            Self::MissingExtension(name) => RhiError::UnsupportedFeature {
                feature: name.to_string(),
            },
            Self::MissingShader(name) => RhiError::Backend {
                detail: format!("missing shader artifact: {name}"),
            },
            Self::SurfaceMinimized | Self::SwapchainOutOfDate => RhiError::SurfaceLost,
            Self::UnsupportedWindow(detail) => RhiError::Backend {
                detail: format!("unsupported window: {detail}"),
            },
        }
    }
}

pub type VkResult<T> = Result<T, VulkanError>;
