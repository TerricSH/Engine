use crate::error::RhiError;
use crate::handles::{
    BufferHandle, DescriptorSetHandle, FramebufferHandle, PipelineHandle, PipelineLayoutHandle,
    RenderPassHandle, ShaderModuleHandle, SurfaceHandle, SwapchainHandle, TextureHandle,
};
use crate::types::{
    AdapterInfo, BackendKind, BufferDescriptor, DeviceDescriptor, FramebufferDescriptor,
    IndexFormat, PipelineDescriptor, PipelineLayoutDescriptor, RenderPassDescriptor,
    RendererStatistics, ShaderModuleDescriptor, SurfaceDescriptor, SwapchainDescriptor,
    TextureDescriptor,
};

// ============================================================================
// CommandEncoder trait – records draw calls into a backend command buffer.
// ============================================================================

pub trait CommandEncoder: Send {
    fn begin_render_pass(
        &mut self,
        render_pass: RenderPassHandle,
        framebuffer: FramebufferHandle,
        area: (u32, u32, u32, u32),
        clear_color: [f32; 4],
        clear_depth: Option<f32>,
    );
    fn bind_pipeline(&mut self, pipeline: PipelineHandle);
    fn bind_vertex_buffers(&mut self, buffers: &[BufferHandle], offsets: &[u64]);
    fn bind_index_buffer(&mut self, buffer: BufferHandle, offset: u64, index_format: IndexFormat);
    fn bind_descriptor_sets(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        first_set: u32,
        sets: &[DescriptorSetHandle],
        dynamic_offsets: &[u32],
    );
    fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, min_depth: f32, max_depth: f32);
    fn set_scissor(&mut self, x: i32, y: i32, w: u32, h: u32);
    fn draw(
        &mut self,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    );
    fn draw_indexed(
        &mut self,
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    );
    fn end_render_pass(&mut self);
    fn push_constants(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        stage_flags: u32,
        offset: u32,
        data: &[u8],
    );
    /// Insert a pipeline barrier for the shadow map (default no-op).
    fn shadow_barrier(&mut self) {}

    // ── Secondary command buffer support ──

    /// Execute a chain of pre-recorded secondary command buffers.
    ///
    /// Secondary command buffers are recorded offline (potentially from
    /// worker threads) and executed inside a render pass.  The default
    /// implementation is a no-op; backends that support secondary buffers
    /// override this method.
    fn execute_commands(&mut self, _secondaries: &[SecondaryCmdBuffer]) {}
}

/// Handle to a pre-recorded secondary command buffer.
///
/// Created by [`CommandPool::record_secondary`](crate::handles::CommandPool::record_secondary)
/// and consumed by [`CommandEncoder::execute_commands`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecondaryCmdBuffer {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

// ============================================================================
// Backend + Device traits (expanded for Gate 3)
// ============================================================================

pub trait Backend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError>;
    fn create_device(&self, descriptor: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError>;
}

pub trait Device: Send + Sync {
    fn adapter_info(&self) -> &AdapterInfo;

    // --- Resource creation (all &mut self for safety) ---

    fn create_surface(
        &mut self,
        _descriptor: &SurfaceDescriptor,
    ) -> Result<SurfaceHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "surface creation is not implemented by this device".to_string(),
        })
    }

    fn create_swapchain(
        &mut self,
        _descriptor: &SwapchainDescriptor,
    ) -> Result<SwapchainHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "swapchain creation is not implemented by this device".to_string(),
        })
    }

    fn create_buffer(&mut self, _descriptor: &BufferDescriptor) -> Result<BufferHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "buffer creation is not implemented by this device".to_string(),
        })
    }

    fn write_buffer(
        &mut self,
        _buffer: BufferHandle,
        _data: &[u8],
        _offset: u64,
    ) -> Result<(), RhiError> {
        Err(RhiError::Backend {
            detail: "buffer write is not implemented by this device".to_string(),
        })
    }

    fn create_texture(
        &mut self,
        _descriptor: &TextureDescriptor,
    ) -> Result<TextureHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "texture creation is not implemented by this device".to_string(),
        })
    }

    fn create_shader_module(
        &mut self,
        _descriptor: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "shader module creation is not implemented by this device".to_string(),
        })
    }

    fn create_render_pass(
        &mut self,
        _descriptor: &RenderPassDescriptor,
    ) -> Result<RenderPassHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "render pass creation is not implemented by this device".to_string(),
        })
    }

    fn create_framebuffer(
        &mut self,
        _descriptor: &FramebufferDescriptor,
    ) -> Result<FramebufferHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "framebuffer creation is not implemented by this device".to_string(),
        })
    }

    fn create_pipeline_layout(
        &mut self,
        _descriptor: &PipelineLayoutDescriptor,
    ) -> Result<PipelineLayoutHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "pipeline layout creation is not implemented by this device".to_string(),
        })
    }

    fn create_pipeline(
        &mut self,
        _descriptor: &PipelineDescriptor,
    ) -> Result<PipelineHandle, RhiError> {
        Err(RhiError::Backend {
            detail: "pipeline creation is not implemented by this device".to_string(),
        })
    }

    // --- Frame lifecycle ---

    /// Begin a new frame. Returns the swapchain image index and a command encoder
    /// that the caller uses to record commands for this frame.
    fn begin_frame(
        &mut self,
        _swapchain: SwapchainHandle,
    ) -> Result<(u32, Box<dyn CommandEncoder>), RhiError> {
        Err(RhiError::Backend {
            detail: "begin_frame is not implemented by this device".to_string(),
        })
    }

    /// End the current frame: submit recorded commands and present.
    fn end_frame(
        &mut self,
        _swapchain: SwapchainHandle,
        _encoder: Box<dyn CommandEncoder>,
        _image_index: u32,
    ) -> Result<RendererStatistics, RhiError> {
        Err(RhiError::Backend {
            detail: "end_frame is not implemented by this device".to_string(),
        })
    }

    /// Recreate a swapchain (typically after a resize).
    fn recreate_swapchain(
        &mut self,
        _swapchain: SwapchainHandle,
        _width: u32,
        _height: u32,
    ) -> Result<(), RhiError> {
        Err(RhiError::Backend {
            detail: "recreate_swapchain is not implemented by this device".to_string(),
        })
    }

    // --- Resource destruction ---

    fn destroy_buffer(&mut self, _buffer: BufferHandle) {}
    fn destroy_texture(&mut self, _texture: TextureHandle) {}
    fn destroy_shader_module(&mut self, _module: ShaderModuleHandle) {}
    fn destroy_render_pass(&mut self, _pass: RenderPassHandle) {}
    fn destroy_framebuffer(&mut self, _fb: FramebufferHandle) {}
    fn destroy_pipeline_layout(&mut self, _layout: PipelineLayoutHandle) {}
    fn destroy_pipeline(&mut self, _pipeline: PipelineHandle) {}
    fn destroy_swapchain(&mut self, _swapchain: SwapchainHandle) {}
    fn destroy_surface(&mut self, _surface: SurfaceHandle) {}

    /// Wait for all pending GPU work to complete.
    fn wait_idle(&self) {}

    // --- Screenshot ---

    /// Read a region of the current framebuffer into a RGBA byte buffer.
    ///
    /// `(x, y, width, height)` specifies the region in pixel coordinates.
    /// Returns a `Vec<u8>` of RGBA pixels (4 bytes per pixel, row-major,
    /// top-to-bottom), or an error if the backend does not support
    /// framebuffer readback or if the device is in an invalid state for
    /// reading.
    ///
    /// The default implementation returns `Err(RhiError::UnsupportedFeature)`.
    fn read_pixels(
        &mut self,
        _x: u32,
        _y: u32,
        _width: u32,
        _height: u32,
    ) -> Result<Vec<u8>, RhiError> {
        Err(RhiError::UnsupportedFeature {
            feature: "framebuffer readback".to_string(),
        })
    }
}
