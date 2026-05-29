//! Custom Vulkan memory allocator.
//!
//! Replaces `gpu-allocator` to avoid an `ash` version conflict.
//!
//! This is a simple direct allocator — every allocation gets its own
//! `VkDeviceMemory` block.  No sub-allocation, no pooling.

use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Public aliases & re-exports
// ---------------------------------------------------------------------------

pub type SharedAllocator = Arc<Mutex<VulkanAllocator>>;
pub use self::inner::{
    Allocation, AllocationCreateDesc, AllocationScheme, MemoryLocation, VulkanAllocator,
};

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

mod inner {
    use ash::vk;
    use ash::Device as AshDevice;

    /// Location hint that guides memory-type selection.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum MemoryLocation {
        GpuOnly,
        CpuToGpu,
        GpuToCpu,
        #[allow(dead_code)]
        CpuOnly,
    }

    /// Descriptor passed to [`VulkanAllocator::allocate`].
    pub struct AllocationCreateDesc {
        pub name: &'static str,
        pub requirements: vk::MemoryRequirements,
        pub location: MemoryLocation,
        #[allow(dead_code)]
        pub linear: bool,
        #[allow(dead_code)]
        pub allocation_scheme: AllocationScheme,
    }

    /// Placeholder — unused.
    pub enum AllocationScheme {
        GpuAllocatorManaged,
    }

    /// A single device-memory allocation (owns one `VkDeviceMemory`).
    pub struct Allocation {
        memory: vk::DeviceMemory,
        size: vk::DeviceSize,
        offset: vk::DeviceSize,
        mapped_ptr: Option<*mut u8>,
        _name: &'static str,
    }

    impl Allocation {
        pub fn memory(&self) -> vk::DeviceMemory {
            self.memory
        }
        pub fn offset(&self) -> u64 {
            self.offset
        }
        pub fn mapped_slice_mut(&mut self) -> Option<&mut [u8]> {
            let ptr = self.mapped_ptr?;
            Some(unsafe { std::slice::from_raw_parts_mut(ptr, self.size as usize) })
        }
    }

    /// Simple direct device-memory allocator.
    pub struct VulkanAllocator {
        device: AshDevice,
        memory_properties: vk::PhysicalDeviceMemoryProperties,
    }

    impl VulkanAllocator {
        /// # Safety
        /// `device` must be valid; `memory_properties` from `get_physical_device_memory_properties`.
        pub unsafe fn new(
            device: AshDevice,
            memory_properties: vk::PhysicalDeviceMemoryProperties,
        ) -> Self {
            Self {
                device,
                memory_properties,
            }
        }

        /// Allocate device memory.
        pub fn allocate(&mut self, desc: &AllocationCreateDesc) -> Result<Allocation, String> {
            let mt = self
                .find_memory_type(desc.requirements.memory_type_bits, desc.location)
                .ok_or_else(|| format!("no suitable memory type for {}", desc.name))?;

            let size = desc.requirements.size;
            let info = vk::MemoryAllocateInfo::default()
                .allocation_size(size)
                .memory_type_index(mt);

            let memory = unsafe {
                self.device
                    .allocate_memory(&info, None)
                    .map_err(|e| format!("vkAllocateMemory({}): {:?}", desc.name, e))?
            };

            let mapped_ptr = match desc.location {
                MemoryLocation::GpuOnly => None,
                _ => {
                    let p = unsafe {
                        self.device
                            .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                            .map_err(|e| format!("vkMapMemory({}): {:?}", desc.name, e))?
                    };
                    Some(p as *mut u8)
                }
            };

            Ok(Allocation {
                memory,
                size,
                offset: 0,
                mapped_ptr,
                _name: desc.name,
            })
        }

        /// Free memory.
        pub fn free(&mut self, alloc: &mut Allocation) {
            if alloc.memory != vk::DeviceMemory::null() {
                unsafe {
                    self.device.free_memory(alloc.memory, None);
                }
                alloc.memory = vk::DeviceMemory::null();
                alloc.mapped_ptr = None;
                alloc.size = 0;
            }
        }

        fn find_memory_type(&self, bits: u32, location: MemoryLocation) -> Option<u32> {
            for i in 0..self.memory_properties.memory_type_count {
                if bits & (1 << i) == 0 {
                    continue;
                }
                let flags = self.memory_properties.memory_types[i as usize].property_flags;
                let need = match location {
                    MemoryLocation::GpuOnly => vk::MemoryPropertyFlags::DEVICE_LOCAL,
                    MemoryLocation::CpuToGpu => {
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT
                    }
                    MemoryLocation::GpuToCpu => {
                        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_CACHED
                    }
                    MemoryLocation::CpuOnly => vk::MemoryPropertyFlags::HOST_VISIBLE,
                };
                if flags.contains(need) {
                    return Some(i);
                }
            }
            None
        }
    }
}
