//! Concrete render pass implementations for the Vulkan backend.
//!
//! Each pass implements the [`RenderPass`](engine_renderer::RenderPass) trait
//! and is registered in the [`PassRegistry`](engine_renderer::PassRegistry)
//! during [`SceneRenderer`](crate::scene_renderer::SceneRenderer) initialisation.
//!
//! # Passes
//!
//! | Module                        | Kind string                   | Purpose                |
//! |-------------------------------|-------------------------------|------------------------|
//! | [`hdr_forward`]               | `"opaque_pbr_forward_pass"`   | Main forward shading   |
//! | [`shadow`]                    | `"directional_shadow_pass"`   | CSM directional shadow |
//! | [`tonemap`]                   | `"tone_map_pass"`             | HDR→LDR tone-mapping   |


pub mod hdr_forward;
pub mod shadow;
pub mod tonemap;
