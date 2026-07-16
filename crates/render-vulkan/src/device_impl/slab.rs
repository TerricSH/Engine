//! Handle slab allocator types.

use ash::vk;
use ash::Device as AshDevice;

use crate::allocator::{Allocation, SharedAllocator};

// ============================================================================
// Handle slab
// ============================================================================

pub(crate) struct BufEntry {
    pub(crate) buffer: vk::Buffer,
    /// Logical buffer size requested from the RHI.  The backing allocation can
    /// be larger because Vulkan memory requirements include alignment padding,
    /// so write bounds must be checked against this value instead of the
    /// mapped allocation length.
    pub(crate) size: vk::DeviceSize,
    pub(crate) allocator: SharedAllocator,
    pub(crate) allocation: Option<Allocation>,
}

pub(crate) struct TexEntry {
    pub(crate) image: vk::Image,
    pub(crate) view: vk::ImageView,
    pub(crate) sampler: Option<vk::Sampler>,
    pub(crate) format: vk::Format,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) sample_count: u8,
    pub(crate) allocator: SharedAllocator,
    pub(crate) allocation: Option<Allocation>,
}

impl TexEntry {
    fn free_allocation(&mut self) {
        let Some(mut allocation) = self.allocation.take() else {
            return;
        };
        match self.allocator.lock() {
            Ok(mut allocator) => allocator.free(&mut allocation),
            Err(poisoned) => {
                tracing::error!(
                    target: "vulkan::resources",
                    "allocator mutex was poisoned while freeing a texture allocation"
                );
                poisoned.into_inner().free(&mut allocation);
            }
        }
    }

    pub(crate) fn destroy(mut self, device: &AshDevice) {
        unsafe {
            if let Some(sampler) = self.sampler.take() {
                device.destroy_sampler(sampler, None);
            }
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
        }
        self.view = vk::ImageView::null();
        self.image = vk::Image::null();
        self.free_allocation();
    }
}

impl Drop for TexEntry {
    fn drop(&mut self) {
        self.free_allocation();
    }
}

impl BufEntry {
    fn free_allocation(&mut self) {
        let Some(mut allocation) = self.allocation.take() else {
            return;
        };

        match self.allocator.lock() {
            Ok(mut allocator) => allocator.free(&mut allocation),
            Err(poisoned) => {
                tracing::error!(
                    target: "vulkan::resources",
                    "allocator mutex was poisoned while freeing a buffer allocation"
                );
                poisoned.into_inner().free(&mut allocation);
            }
        }
    }

    /// Destroy the Vulkan buffer before releasing its bound memory.
    pub(crate) fn destroy(mut self, device: &AshDevice) {
        // SAFETY: this entry exclusively owns `buffer`; callers remove the
        // entry from the slab before invoking this method.
        unsafe {
            device.destroy_buffer(self.buffer, None);
        }
        self.buffer = vk::Buffer::null();
        self.free_allocation();
    }
}

impl Drop for BufEntry {
    fn drop(&mut self) {
        // The Vulkan buffer itself requires an `AshDevice` and is therefore
        // destroyed by `BufEntry::destroy`.  This fallback still prevents a
        // memory leak if an entry is unwound before reaching that path.
        self.free_allocation();
    }
}

pub(crate) struct Slab<T> {
    pub(crate) slots: Vec<Option<(u32, T)>>,
    free_generations: Vec<u32>,
}
impl<T> Slab<T> {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_generations: Vec::new(),
        }
    }
    pub(crate) fn insert(&mut self, v: T) -> (u32, u32) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            if s.is_none() {
                let generation = self.free_generations[i].max(1);
                *s = Some((generation, v));
                return (i as u32, generation);
            }
        }
        let i = self.slots.len();
        self.slots.push(Some((1, v)));
        self.free_generations.push(1);
        (i as u32, 1)
    }
    pub(crate) fn get(&self, idx: u32, gen: u32) -> Option<&T> {
        self.slots
            .get(idx as usize)
            .and_then(|s| s.as_ref().filter(|(g, _)| *g == gen).map(|(_, v)| v))
    }
    pub(crate) fn get_mut(&mut self, idx: u32, gen: u32) -> Option<&mut T> {
        self.slots
            .get_mut(idx as usize)
            .and_then(|s| s.as_mut().filter(|(g, _)| *g == gen).map(|(_, v)| v))
    }

    pub(crate) fn remove(&mut self, idx: u32, gen: u32) -> Option<T> {
        let slot = self.slots.get_mut(idx as usize)?;
        let (stored_generation, _) = slot.as_ref()?;
        if *stored_generation != gen {
            return None;
        }

        let (_, value) = slot.take()?;
        let next_generation = gen.wrapping_add(1).max(1);
        if let Some(stored) = self.free_generations.get_mut(idx as usize) {
            *stored = next_generation;
        }
        Some(value)
    }

    /// Remove every live value and invalidate all outstanding handles.
    pub(crate) fn drain_values(&mut self) -> Vec<T> {
        let mut values = Vec::new();
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some((generation, value)) = slot.take() else {
                continue;
            };
            self.free_generations[index] = generation.wrapping_add(1).max(1);
            values.push(value);
        }
        values
    }
}

// ============================================================================
// Pipeline and pipeline-layout entries
// ============================================================================

pub(crate) struct PipeEntry {
    pub(crate) pipeline: vk::Pipeline,
}

pub(crate) struct FbEntry {
    pub(crate) framebuffer: vk::Framebuffer,
    pub(crate) color_attachment_count: u32,
    pub(crate) has_depth: bool,
}

pub(crate) struct PlEntry {
    pub(crate) layout: vk::PipelineLayout,
    /// Descriptor set layouts owned by this pipeline layout (created from
    /// PipelineLayoutDescriptor::bind_group_layouts).  Destroyed when the
    /// pipeline layout is destroyed.
    pub(crate) set_layouts: Vec<vk::DescriptorSetLayout>,
    pub(crate) _device: AshDevice,
}

// ============================================================================
// Frame sync
// ============================================================================

pub(crate) struct FrameSync {
    /// Binary semaphore signalled by `vkAcquireNextImageKHR`.
    pub(crate) image_available: vk::Semaphore,
    /// Binary semaphore signalled by queue submission and waited by present.
    pub(crate) render_finished: vk::Semaphore,
    pub(crate) in_flight_fence: vk::Fence,
    pub(crate) command_pool: vk::CommandPool,
    pub(crate) command_buffer: vk::CommandBuffer,
}

#[cfg(test)]
mod tests {
    use super::Slab;

    #[test]
    fn remove_invalidates_old_generation_and_reuses_slot() {
        let mut slab = Slab::new();

        let (index, generation) = slab.insert(10u32);
        assert_eq!(slab.remove(index, generation), Some(10));
        assert!(slab.get(index, generation).is_none());

        let (reused_index, new_generation) = slab.insert(20u32);
        assert_eq!(reused_index, index);
        assert_ne!(new_generation, generation);
        assert_eq!(slab.get(reused_index, new_generation), Some(&20));
    }

    #[test]
    fn stale_or_repeated_remove_is_a_safe_noop() {
        let mut slab = Slab::new();
        let (index, generation) = slab.insert(10u32);

        assert!(slab.remove(index, generation.wrapping_add(1)).is_none());
        assert_eq!(slab.get(index, generation), Some(&10));
        assert_eq!(slab.remove(index, generation), Some(10));
        assert!(slab.remove(index, generation).is_none());
        assert!(slab.remove(u32::MAX, generation).is_none());
    }

    #[test]
    fn drain_invalidates_live_handles_and_preserves_reuse() {
        let mut slab = Slab::new();
        let (first_index, first_generation) = slab.insert(10u32);
        let (second_index, second_generation) = slab.insert(20u32);

        let mut drained = slab.drain_values();
        drained.sort_unstable();
        assert_eq!(drained, vec![10, 20]);
        assert!(slab.get(first_index, first_generation).is_none());
        assert!(slab.get(second_index, second_generation).is_none());

        let (reused_index, reused_generation) = slab.insert(30u32);
        assert_eq!(reused_index, first_index);
        assert_ne!(reused_generation, first_generation);
        assert_eq!(slab.get(reused_index, reused_generation), Some(&30));
    }
}
