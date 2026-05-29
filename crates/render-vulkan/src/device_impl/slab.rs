//! Handle slab allocator types.

use ash::vk;
use ash::Device as AshDevice;

use crate::allocator::{Allocation, SharedAllocator};

// ============================================================================
// Handle slab
// ============================================================================

pub(crate) struct BufEntry {
    pub(crate) buffer: vk::Buffer,
    pub(crate) allocator: SharedAllocator,
    pub(crate) allocation: Option<Allocation>,
}

impl Drop for BufEntry {
    fn drop(&mut self) {
        if let Some(mut a) = self.allocation.take() {
            let _ = self.allocator.lock().unwrap().free(&mut a);
        }
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
}

// ============================================================================
// Pipeline and pipeline-layout entries
// ============================================================================

pub(crate) struct PipeEntry {
    pub(crate) pipeline: vk::Pipeline,
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
    pub(crate) image_available: vk::Semaphore,
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
}
