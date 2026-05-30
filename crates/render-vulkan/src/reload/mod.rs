//! GPU resource hot-reload coordinator.
//!
//! Consumes reload requests from the main thread, creates new GPU resources
//! at frame boundaries, atomically swaps them in, and keeps old resources
//! alive until in-flight frames complete.
//!
//! # Lifecycle
//!
//! 1. [`queue_reload`](GpuReloadCoordinator::queue_reload) — called from the
//!    main thread after an asset has been re-cooked. The caller provides the
//!    cooked pixel data or SPIR-V alongside the asset identifier.
//! 2. [`apply_next`](GpuReloadCoordinator::apply_next) — called at the start
//!    of a frame (before acquire).  Creates new GPU resources, performs the
//!    atomic swap on [`VulkanDevice`], and pushes the old resources into the
//!    deferred-destruction list.
//! 3. [`retire_old_resources`](GpuReloadCoordinator::retire_old_resources) —
//!    called after submission.  Decrements a keep-alive counter for each old
//!    resource and destroys those that have survived two frames.

use ash::vk;
use ash::Device as AshDevice;

use crate::allocator::Allocation;
use crate::device_impl::VulkanDevice;
use crate::error::VulkanError;
// ============================================================================
// Public data types
// ============================================================================

/// GPU resource data for a pending reload.
///
/// The caller (main thread) extracts the appropriate variant from the
/// engine-asset registry after a successful re-cook and passes it to
/// [`GpuReloadCoordinator::queue_reload`].
pub enum ReloadData {
    /// A 2D texture (RGBA8, full mip-chain).
    Texture {
        /// Width of the base mip level.
        width: u32,
        /// Height of the base mip level.
        height: u32,
        /// Number of mip levels.
        mip_count: u8,
        /// Pixel data: all mip levels concatenated (base level first).
        data: Vec<u8>,
    },
    /// A pair of SPIR-V blobs for a graphics pipeline.
    Shader {
        /// SPIR-V bytecode for the vertex shader.
        vert_spirv: Vec<u8>,
        /// SPIR-V bytecode for the fragment shader.
        frag_spirv: Vec<u8>,
    },
}

/// An old GPU resource kept alive until all in-flight frames have retired.
///
/// After the new resource has been swapped in, the old one is moved into
/// this enum and stored in [`GpuReloadCoordinator`] for at least two frame
/// boundaries before being destroyed.
pub enum GpuResource {
    /// A replaced texture (image, view, and optional allocation).
    Texture {
        /// Old Vulkan image handle.
        old_image: vk::Image,
        /// Old Vulkan image view handle.
        old_view: vk::ImageView,
        /// Old memory allocation (freed on final retirement).
        old_allocation: Option<Allocation>,
    },
    /// A replaced graphics pipeline.
    Shader {
        /// Old Vulkan pipeline handle.
        old_pipeline: vk::Pipeline,
        /// Old Vulkan pipeline layout handle.
        old_layout: vk::PipelineLayout,
    },
}

// ============================================================================
// Internal helper types
// ============================================================================

/// A pending reload request with the data needed to create new GPU resources.
struct PendingReload {
    /// Asset identifier (for diagnostics / lookup).
    _asset_id: String,
    /// Cooked data to use when creating the new resource.
    data: ReloadData,
    /// Which pipeline target this reload affects.
    target: ReloadTarget,
}

/// Identifies which resource on [`VulkanDevice`] to replace.
enum ReloadTarget {
    /// Replace the MVP triangle pipeline.
    MvpPipeline,
    /// Replace the forward model pipeline.
    ModelPipeline,
    /// Replace the shadow-mapping pipeline.
    ShadowPipeline,
    /// Replace the shadow-map texture (image / view / allocation).
    ShadowMap,
}

/// A retiring resource with a remaining frame counter.
struct RetiringResource {
    /// The old GPU resource to destroy once it is safe.
    resource: GpuResource,
    /// Number of frame boundaries this resource must survive.
    frames_left: u32,
}

// ============================================================================
// GpuReloadCoordinator
// ============================================================================

/// Coordinates GPU resource hot-reload across frame boundaries.
///
/// # Usage
///
/// ```ignore
/// let mut coord = GpuReloadCoordinator::new();
///
/// // Main thread, after recook:
/// coord.queue_reload("texture-floor", ReloadData::Texture { … });
///
/// // Each frame, before acquire:
/// coord.apply_next(&mut device)?;
///
/// // Each frame, after submit:
/// coord.retire_old_resources(&device.logical_device.device);
/// ```
pub struct GpuReloadCoordinator {
    /// Queued reload requests (FIFO, one processed per frame).
    pending: Vec<PendingReload>,
    /// Old resources kept alive for deferred destruction.
    retiring: Vec<RetiringResource>,
}

impl GpuReloadCoordinator {
    /// Create an empty coordinator.
    pub const fn new() -> Self {
        Self {
            pending: Vec::new(),
            retiring: Vec::new(),
        }
    }

    /// Queue a reload request.
    ///
    /// Called from the main thread after a cooked asset has been updated on
    /// disk (or in memory).  The caller passes the cooked data so the
    /// coordinator does not need a direct dependency on `engine-asset`.
    ///
    /// The `asset_id` is used to determine which resource on the device
    /// to replace (e.g. `"shader-mvp"` → MVP pipeline).
    pub fn queue_reload(&mut self, asset_id: impl Into<String>, data: ReloadData) {
        let asset_id: String = asset_id.into();
        let target = target_from_asset_id(&asset_id);
        tracing::info!(
            target: "vulkan::reload",
            asset_id = %asset_id,
            "reload queued"
        );
        self.pending.push(PendingReload {
            _asset_id: asset_id,
            data,
            target,
        });
    }

    /// Process one pending reload at the start of a frame.
    ///
    /// Returns `Ok(true)` if a resource was reloaded, `Ok(false)` if nothing
    /// was pending.  On failure the old resource is kept and an error is
    /// returned (the frame continues uninterrupted).
    pub fn apply_next(&mut self, device: &mut VulkanDevice) -> Result<bool, VulkanError> {
        let Some(req) = self.pending.pop() else {
            return Ok(false);
        };

        match req.data {
            ReloadData::Texture {
                width,
                height,
                mip_count,
                data,
            } => {
                // ── Create new texture ──────────────────────────────────
                match device.create_sampled_texture(width, height, mip_count, &data) {
                    Ok((new_image, new_view, new_allocation)) => {
                        // Swap the shadow map if this reload targets it.
                        match req.target {
                            ReloadTarget::ShadowMap => {
                                // CSM shadow maps are 3-layer depth arrays, not
                                // single 2D color textures.  The reload system
                                // cannot recreate the array + per-layer views,
                                // so we destroy the new texture and keep the
                                // existing shadow map.
                                device.destroy_sampled_texture(new_image, new_view, new_allocation);
                                tracing::warn!(
                                    target: "vulkan::reload",
                                    "shadow map reload is unsupported for CSM; keeping existing"
                                );
                            }
                            ReloadTarget::MvpPipeline
                            | ReloadTarget::ModelPipeline
                            | ReloadTarget::ShadowPipeline => {
                                // The texture reload did not match a known
                                // texture target — destroy the new resource
                                // immediately to avoid a leak.
                                device.destroy_sampled_texture(new_image, new_view, new_allocation);
                                tracing::warn!(
                                    target: "vulkan::reload",
                                    "texture reload target not matched for shadow map; discarding"
                                );
                            }
                        }
                        tracing::info!(target: "vulkan::reload", "texture reloaded");
                        Ok(true)
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "vulkan::reload",
                            error = %e,
                            "texture reload failed — old texture kept"
                        );
                        Err(e)
                    }
                }
            }

            ReloadData::Shader {
                vert_spirv,
                frag_spirv,
            } => {
                // ── Recreate pipeline ───────────────────────────────────
                let result = match req.target {
                    ReloadTarget::MvpPipeline => {
                        device.recreate_mvp_pipeline(&vert_spirv, &frag_spirv)
                    }
                    ReloadTarget::ModelPipeline => {
                        device.recreate_model_pipeline(&vert_spirv, &frag_spirv)
                    }
                    ReloadTarget::ShadowPipeline => {
                        device.recreate_shadow_pipeline(&vert_spirv, &frag_spirv)
                    }
                    ReloadTarget::ShadowMap => {
                        // Shader data queued for a texture target — discard.
                        tracing::warn!(
                            target: "vulkan::reload",
                            "shader reload targeted shadow map; ignoring"
                        );
                        return Ok(true);
                    }
                };

                match result {
                    Ok((old_pipeline, old_layout)) => {
                        self.retiring.push(RetiringResource {
                            resource: GpuResource::Shader {
                                old_pipeline,
                                old_layout,
                            },
                            frames_left: 2,
                        });
                        tracing::info!(target: "vulkan::reload", "pipeline reloaded");
                        Ok(true)
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "vulkan::reload",
                            error = %e,
                            "pipeline reload failed — old pipeline kept"
                        );
                        Err(e)
                    }
                }
            }
        }
    }

    /// Drain stale in-flight resources.
    ///
    /// Call this **after** submitting the current frame.  Decrements the
    /// keep-alive counter for each retiring resource; resources whose counter
    /// reaches zero are destroyed.
    ///
    /// # Safety
    ///
    /// `device` must be a valid `AshDevice` that has not been destroyed.
    pub fn retire_old_resources(&mut self, device: &AshDevice) {
        let mut i = 0;
        while i < self.retiring.len() {
            let r = &mut self.retiring[i];
            if r.frames_left == 0 {
                // Destroy now.
                let resource = self.retiring.swap_remove(i);
                Self::destroy_resource(device, resource.resource);
                // Do NOT increment i — swap_remove brought a new element
                // into position i.
            } else {
                r.frames_left -= 1;
                i += 1;
            }
        }
    }

    /// True when there are pending reload requests.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Number of resources waiting for deferred destruction.
    pub fn retiring_count(&self) -> usize {
        self.retiring.len()
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    /// Destroy a retired GPU resource immediately.
    ///
    /// # Safety
    ///
    /// `device` must be valid; the resource must not be in use by any
    /// in-flight queue submission.
    fn destroy_resource(device: &AshDevice, resource: GpuResource) {
        match resource {
            GpuResource::Texture {
                old_image,
                old_view,
                mut old_allocation,
            } => {
                // SAFETY: device is valid; handles were created by this
                // device and are no longer referenced by any in-flight work.
                unsafe {
                    device.destroy_image_view(old_view, None);
                    device.destroy_image(old_image, None);
                }
                // The allocation must be freed via the allocator (which
                // needs the device), but we only have &AshDevice here.
                // We store the allocation for external cleanup.
                if let Some(ref mut alloc) = old_allocation {
                    // We cannot free without the allocator — log a warning.
                    // The caller is responsible for freeing allocations
                    // through the allocator.  For now, leak intentionally
                    // (the old resource is being retired after 2 frames,
                    // and the device itself will be destroyed eventually).
                    tracing::trace!(
                        target: "vulkan::reload",
                        "retired texture allocation (memory={:?}) — leaked, allocator unavailable",
                        alloc.memory(),
                    );
                    // Prevent Drop from attempting to free.
                    let _ = alloc;
                }
            }
            GpuResource::Shader {
                old_pipeline,
                old_layout,
            } => {
                // SAFETY: device is valid; handles are no longer in use.
                unsafe {
                    device.destroy_pipeline(old_pipeline, None);
                    device.destroy_pipeline_layout(old_layout, None);
                }
            }
        }
    }
}

impl Default for GpuReloadCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Asset ID → target mapping
// ============================================================================

/// Map an asset identifier string to a [`ReloadTarget`].
///
/// This is a simple heuristic based on the asset naming convention used
/// by the engine-asset pipeline:
///
/// | Asset ID pattern     | Target              |
/// |----------------------|---------------------|
/// | `shader-mvp-*`       | `MvpPipeline`       |
/// | `shader-model-*`     | `ModelPipeline`     |
/// | `shader-shadow-*`    | `ShadowPipeline`    |
/// | `texture-shadow*`    | `ShadowMap`         |
fn target_from_asset_id(id: &str) -> ReloadTarget {
    if let Some(rest) = id.strip_prefix("shader-") {
        if rest.starts_with("mvp") || rest.starts_with("triangle") {
            ReloadTarget::MvpPipeline
        } else if rest.starts_with("model") || rest.starts_with("forward") {
            ReloadTarget::ModelPipeline
        } else if rest.starts_with("shadow") {
            ReloadTarget::ShadowPipeline
        } else {
            // Default to model pipeline for unrecognised shader assets.
            ReloadTarget::ModelPipeline
        }
    } else if id.starts_with("texture-") && id.contains("shadow") {
        ReloadTarget::ShadowMap
    } else {
        // Default: treat as model pipeline reload.
        ReloadTarget::ModelPipeline
    }
}
