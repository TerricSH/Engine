use core::fmt;
use core::marker::PhantomData;

/// Opaque typed handle for GPU resources.
pub struct ResourceHandle<KIND> {
    pub index: u32,
    pub generation: u32,
    marker: PhantomData<fn() -> KIND>,
}

impl<KIND> ResourceHandle<KIND> {
    pub const fn new(index: u32, generation: u32) -> Self {
        Self {
            index,
            generation,
            marker: PhantomData,
        }
    }
}

impl<KIND> Clone for ResourceHandle<KIND> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<KIND> Copy for ResourceHandle<KIND> {}

impl<KIND> fmt::Debug for ResourceHandle<KIND> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceHandle")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<KIND> PartialEq for ResourceHandle<KIND> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<KIND> Eq for ResourceHandle<KIND> {}

impl<KIND> core::hash::Hash for ResourceHandle<KIND> {
    fn hash<HASHER: core::hash::Hasher>(&self, state: &mut HASHER) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

// --- Phantom marker types ---

pub enum BufferKind {}
pub enum TextureKind {}
pub enum ShaderModuleKind {}
pub enum PipelineKind {}
pub enum BindGroupKind {}
pub enum RenderPassKind {}
pub enum SurfaceKind {}
pub enum SwapchainKind {}
pub enum FramebufferKind {}
pub enum PipelineLayoutKind {}
pub enum DescriptorSetLayoutKind {}
pub enum DescriptorPoolKind {}
pub enum DescriptorSetKind {}
pub enum CommandBufferKind {}

// --- Typed handle aliases ---

pub type BufferHandle = ResourceHandle<BufferKind>;
pub type TextureHandle = ResourceHandle<TextureKind>;
pub type ShaderModuleHandle = ResourceHandle<ShaderModuleKind>;
pub type PipelineHandle = ResourceHandle<PipelineKind>;
pub type BindGroupHandle = ResourceHandle<BindGroupKind>;
pub type RenderPassHandle = ResourceHandle<RenderPassKind>;
pub type SurfaceHandle = ResourceHandle<SurfaceKind>;
pub type SwapchainHandle = ResourceHandle<SwapchainKind>;
pub type FramebufferHandle = ResourceHandle<FramebufferKind>;
pub type PipelineLayoutHandle = ResourceHandle<PipelineLayoutKind>;
pub type DescriptorSetLayoutHandle = ResourceHandle<DescriptorSetLayoutKind>;
pub type DescriptorPoolHandle = ResourceHandle<DescriptorPoolKind>;
pub type DescriptorSetHandle = ResourceHandle<DescriptorSetKind>;
pub type CommandBufferHandle = ResourceHandle<CommandBufferKind>;
