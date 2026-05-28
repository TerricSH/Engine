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
}
impl<T> Slab<T> {
    pub(crate) fn new() -> Self {
        Self { slots: Vec::new() }
    }
    pub(crate) fn insert(&mut self, v: T) -> (u32, u32) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            if s.is_none() {
                *s = Some((1, v));
                return (i as u32, 1);
            }
        }
        let i = self.slots.len();
        self.slots.push(Some((1, v)));
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
}

// ============================================================================
// Pipeline and pipeline-layout entries
// ============================================================================

pub(crate) struct PipeEntry {
    pub(crate) pipeline: vk::Pipeline,
}

pub(crate) struct PlEntry {
    pub(crate) layout: vk::PipelineLayout,
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
