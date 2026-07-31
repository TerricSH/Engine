//! Shadow mapping for VulkanDevice (directional light CSM, 2048 x 2048, 3 cascades).

use ash::vk;

use crate::error::{VkResult, VulkanError};

use super::{mk_sm, VulkanDevice};

/// Number of CSM cascades.
pub(crate) const CSM_CASCADE_COUNT: usize = 3;

/// Validation failures produced while deriving camera or directional-light
/// data for cascaded shadow maps.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CascadeDataError {
    #[error("projection matrix contains non-finite values")]
    NonFiniteProjection,
    #[error("projection matrix is not a supported right-handed Vulkan zero-to-one projection")]
    UnsupportedProjection,
    #[error("projection matrix does not encode finite positive near/far planes")]
    InvalidClipPlanes,
    #[error("view matrix contains non-finite values")]
    NonFiniteView,
    #[error("view matrix is not invertible")]
    NonInvertibleView,
    #[error("directional shadow light direction must be finite and non-zero")]
    InvalidLightDirection,
    #[error("camera frustum cannot be converted into finite cascade bounds")]
    DegenerateFrustum,
}

mod cascade_math;
mod resources;

#[cfg(test)]
mod tests {
    include!("shadow/tests.rs");
}
