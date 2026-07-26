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
    ) -> Result<(), RhiError>;
    /// Bind one sampled 2D texture through the backend's scene-material path.
    /// Returns `false` when the backend or texture does not expose a sampled
    /// descriptor. General descriptor-set binding remains the preferred API;
    /// this narrow bridge keeps the portable scene renderer fail-closed while
    /// descriptor allocation is added to the RHI contract.
    fn bind_sampled_texture(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _texture: TextureHandle,
    ) -> bool {
        false
    }
    /// Bind a base-color texture and shadow map as one contiguous SRV/sampler
    /// table. Backends return `false` when this portable two-texture scene
    /// binding is unavailable.
    fn bind_sampled_texture_pair(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _base_color: TextureHandle,
        _shadow_map: TextureHandle,
    ) -> bool {
        false
    }
    /// Bind an ordered sampled-texture/sampler table. Scene pipelines use this
    /// for base color, shadow, normal, metallic-roughness, occlusion, and
    /// emissive resources while retaining the pair bridge for older backends.
    fn bind_sampled_texture_set(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _textures: &[TextureHandle],
    ) -> bool {
        false
    }
    /// Bind a uniform buffer through a backend root/descriptor binding used by
    /// the portable skinning path. Returns `false` if the layout does not
    /// declare a compatible binding.
    fn bind_uniform_buffer(
        &mut self,
        _pipeline_layout: PipelineLayoutHandle,
        _buffer: BufferHandle,
    ) -> bool {
        false
    }
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
    /// Issue indirect indexed draws from a GPU buffer.
    ///
    /// `buffer` contains an array of [`VkDrawIndexedIndirectCommand`]-compatible
    /// structs (each 20 bytes: index_count, instance_count, first_index,
    /// vertex_offset, first_instance).
    ///
    /// Backends that do not support indirect draws return a structured
    /// `UnsupportedFeature` error instead of silently dropping the command.
    fn draw_indexed_indirect(
        &mut self,
        _buffer: BufferHandle,
        _offset: u64,
        _draw_count: u32,
        _stride: u32,
    ) -> Result<(), RhiError> {
        Err(RhiError::UnsupportedFeature {
            feature: "indexed indirect draw".to_string(),
        })
    }
    fn end_render_pass(&mut self);
    fn push_constants(
        &mut self,
        pipeline_layout: PipelineLayoutHandle,
        stage_flags: u32,
        offset: u32,
        data: &[u8],
    );
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

    fn destroy_buffer(&mut self, buffer: BufferHandle);
    fn destroy_texture(&mut self, texture: TextureHandle);
    fn destroy_shader_module(&mut self, module: ShaderModuleHandle);
    fn destroy_render_pass(&mut self, pass: RenderPassHandle);
    fn destroy_framebuffer(&mut self, framebuffer: FramebufferHandle);
    fn destroy_pipeline_layout(&mut self, layout: PipelineLayoutHandle);
    fn destroy_pipeline(&mut self, pipeline: PipelineHandle);
    fn destroy_swapchain(&mut self, swapchain: SwapchainHandle);
    fn destroy_surface(&mut self, surface: SurfaceHandle);

    /// Wait for all pending GPU work to complete.
    fn wait_idle(&self);

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
