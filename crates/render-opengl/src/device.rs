use std::sync::Arc;

use glow::HasContext;
use render_core::*;

use crate::encoder::{DeviceRef, OpenGlCommandEncoder};

// ============================================================================
// Format conversion helpers
// ============================================================================

/// Returns (internal_format, format, pixel_type) for tex_image_2d.
fn convert_texture_format(format: TextureFormat) -> (i32, u32, u32) {
    match format {
        TextureFormat::Rgba8Unorm => (glow::RGBA8 as i32, glow::RGBA, glow::UNSIGNED_BYTE),
        TextureFormat::Bgra8Unorm => (glow::RGBA8 as i32, glow::BGRA, glow::UNSIGNED_BYTE),
        TextureFormat::Rgba16Float => (glow::RGBA16F as i32, glow::RGBA, glow::HALF_FLOAT),
        TextureFormat::Depth32Float => (
            glow::DEPTH_COMPONENT32F as i32,
            glow::DEPTH_COMPONENT,
            glow::FLOAT,
        ),
        _ => (glow::RGBA8 as i32, glow::RGBA, glow::UNSIGNED_BYTE),
    }
}

fn _convert_index_format(format: IndexFormat) -> u32 {
    match format {
        IndexFormat::U16 => glow::UNSIGNED_SHORT,
        IndexFormat::U32 => glow::UNSIGNED_INT,
    }
}

/// Returns the GL buffer target for a given usage.
fn buffer_target(usage: BufferUsage) -> u32 {
    if usage.0 & BufferUsage::INDEX.0 != 0 {
        glow::ELEMENT_ARRAY_BUFFER
    } else if usage.0 & BufferUsage::UNIFORM.0 != 0 {
        glow::UNIFORM_BUFFER
    } else {
        glow::ARRAY_BUFFER
    }
}

// ============================================================================
// Resource slabs (generational-index storage)
// ============================================================================

pub(crate) struct Slot<T> {
    pub(crate) generation: u32,
    pub(crate) value: T,
}

/// A simple generational slab. Always appends (never reuses indices) - keeps
/// the code straightforward and generation checking trivial.
pub(crate) struct ResourceSlab<T> {
    slots: Vec<Option<Slot<T>>>,
}

impl<T> ResourceSlab<T> {
    const fn new() -> Self {
        Self { slots: Vec::new() }
    }

    fn alloc(&mut self, value: T) -> (u32, u32) {
        let idx = self.slots.len();
        self.slots.push(Some(Slot {
            generation: 1,
            value,
        }));
        (idx as u32, 1)
    }

    pub(crate) fn get(&self, idx: u32) -> Option<&Slot<T>> {
        self.slots.get(idx as usize).and_then(|s| s.as_ref())
    }

    fn free(&mut self, idx: u32) {
        if let Some(slot) = self.slots.get_mut(idx as usize) {
            *slot = None;
        }
    }
}

// ============================================================================
// Slot types for each resource category
// ============================================================================

pub(crate) struct BufferSlot {
    pub(crate) gl_buffer: glow::Buffer,
    pub(crate) _size_bytes: u64,
    pub(crate) usage: BufferUsage,
}

pub(crate) struct TextureSlot {
    pub(crate) gl_texture: glow::Texture,
    pub(crate) _format: TextureFormat,
    pub(crate) _width: u32,
    pub(crate) _height: u32,
}

pub(crate) struct ShaderModuleSlot {
    pub(crate) _format: ShaderFormat,
    pub(crate) _source_hash: [u8; 32],
}

pub(crate) struct RenderPassSlot {
    pub(crate) _descriptor: RenderPassDescriptor,
}

pub(crate) struct FramebufferSlot {
    pub(crate) gl_framebuffer: glow::Framebuffer,
    pub(crate) _width: u32,
    pub(crate) _height: u32,
}

pub(crate) struct PipelineLayoutSlot {
    pub(crate) _descriptor: PipelineLayoutDescriptor,
}

pub(crate) struct PipelineSlot {
    pub(crate) gl_program: glow::Program,
}

pub(crate) struct SurfaceSlot {}

pub(crate) struct SwapchainSlot {
    pub(crate) _width: u32,
    pub(crate) _height: u32,
}

// ============================================================================
// OpenGlBackend
// ============================================================================

pub struct OpenGlBackend {
    gl: Arc<glow::Context>,
}

impl OpenGlBackend {
    pub fn new(gl: glow::Context) -> Self {
        Self { gl: Arc::new(gl) }
    }
}

impl Backend for OpenGlBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::OpenGl
    }

    fn enumerate_adapters(&self) -> Result<Vec<AdapterInfo>, RhiError> {
        // OpenGL does not have physical-device enumeration like Vulkan.
        // Return a single generic adapter built from the driver string.
        let name = unsafe { self.gl.get_parameter_string(glow::RENDERER) };
        Ok(vec![AdapterInfo {
            backend: BackendKind::OpenGl,
            name,
            vendor_id: None,
            device_id: None,
            driver_version: None,
            capabilities: BackendCapabilities {
                max_texture_dimension_2d: unsafe {
                    self.gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32
                },
                max_color_attachments: unsafe {
                    self.gl.get_parameter_i32(glow::MAX_COLOR_ATTACHMENTS) as u8
                },
                supports_swapchain: false,
                supports_timestamps: false,
                supports_debug_markers: false,
                supported_shader_formats: vec![ShaderFormat::Glsl],
                supported_surface_formats: vec![
                    TextureFormat::Rgba8Unorm,
                    TextureFormat::Bgra8Unorm,
                ],
                limits: ResourceLimits::default(),
            },
        }])
    }

    fn create_device(&self, descriptor: &DeviceDescriptor) -> Result<Box<dyn Device>, RhiError> {
        Ok(Box::new(OpenGlDevice::new(self.gl.clone(), descriptor)))
    }
}

// ============================================================================
// OpenGlDevice
// ============================================================================

pub struct OpenGlDevice {
    adapter: AdapterInfo,
    gl: Arc<glow::Context>,

    // Resource slabs
    pub(crate) buffers: ResourceSlab<BufferSlot>,
    pub(crate) textures: ResourceSlab<TextureSlot>,
    pub(crate) shader_modules: ResourceSlab<ShaderModuleSlot>,
    pub(crate) render_passes: ResourceSlab<RenderPassSlot>,
    pub(crate) framebuffers: ResourceSlab<FramebufferSlot>,
    pub(crate) pipeline_layouts: ResourceSlab<PipelineLayoutSlot>,
    pub(crate) pipelines: ResourceSlab<PipelineSlot>,
    pub(crate) surfaces: ResourceSlab<SurfaceSlot>,
    pub(crate) swapchains: ResourceSlab<SwapchainSlot>,
}

impl OpenGlDevice {
    fn new(gl: Arc<glow::Context>, descriptor: &DeviceDescriptor) -> Self {
        let adapter = descriptor.adapter.clone();
        Self {
            adapter,
            gl,
            buffers: ResourceSlab::new(),
            textures: ResourceSlab::new(),
            shader_modules: ResourceSlab::new(),
            render_passes: ResourceSlab::new(),
            framebuffers: ResourceSlab::new(),
            pipeline_layouts: ResourceSlab::new(),
            pipelines: ResourceSlab::new(),
            surfaces: ResourceSlab::new(),
            swapchains: ResourceSlab::new(),
        }
    }
}

impl Device for OpenGlDevice {
    fn adapter_info(&self) -> &AdapterInfo {
        &self.adapter
    }

    // ██ surfaces / swapchain (dummy ― handled by platform) ████████████████████████████████

    fn create_surface(
        &mut self,
        _descriptor: &SurfaceDescriptor,
    ) -> Result<SurfaceHandle, RhiError> {
        let (idx, gen) = self.surfaces.alloc(SurfaceSlot {});
        Ok(ResourceHandle::new(idx, gen))
    }

    fn create_swapchain(
        &mut self,
        descriptor: &SwapchainDescriptor,
    ) -> Result<SwapchainHandle, RhiError> {
        let (idx, gen) = self.swapchains.alloc(SwapchainSlot {
            _width: descriptor.width,
            _height: descriptor.height,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_surface(&mut self, handle: SurfaceHandle) {
        self.surfaces.free(handle.index);
    }

    fn destroy_swapchain(&mut self, handle: SwapchainHandle) {
        self.swapchains.free(handle.index);
    }

    // ██ buffers ████████████████████████████████████████████████████████████████████████████████

    fn create_buffer(&mut self, descriptor: &BufferDescriptor) -> Result<BufferHandle, RhiError> {
        // SAFETY: glow buffer creation.
        let gl_buffer = unsafe {
            self.gl
                .create_buffer()
                .map_err(|e| RhiError::Backend { detail: e })?
        };
        let target = buffer_target(descriptor.usage_flags);
        unsafe {
            self.gl.bind_buffer(target, Some(gl_buffer));
            self.gl
                .buffer_data_size(target, descriptor.size_bytes as i32, glow::STATIC_DRAW);
        }

        let (idx, gen) = self.buffers.alloc(BufferSlot {
            gl_buffer,
            _size_bytes: descriptor.size_bytes,
            usage: descriptor.usage_flags,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn write_buffer(
        &mut self,
        buffer: BufferHandle,
        data: &[u8],
        offset: u64,
    ) -> Result<(), RhiError> {
        let slot = self
            .buffers
            .get(buffer.index)
            .filter(|s| s.generation == buffer.generation)
            .ok_or(RhiError::InvalidHandle)?;
        let target = buffer_target(slot.value.usage);
        unsafe {
            self.gl.bind_buffer(target, Some(slot.value.gl_buffer));
            self.gl
                .buffer_sub_data_u8_slice(target, offset as i32, data);
        }
        Ok(())
    }

    fn destroy_buffer(&mut self, handle: BufferHandle) {
        let slot = self.buffers.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                unsafe { self.gl.delete_buffer(slot.value.gl_buffer) };
            }
        }
        self.buffers.free(handle.index);
    }

    // ██ textures ███████████████████████████████████████████████████████████████████████████████

    fn create_texture(
        &mut self,
        descriptor: &TextureDescriptor,
    ) -> Result<TextureHandle, RhiError> {
        // SAFETY: glow texture creation.
        let gl_texture = unsafe {
            self.gl
                .create_texture()
                .map_err(|e| RhiError::Backend { detail: e })?
        };
        let (internal_fmt, fmt, pixel_type) = convert_texture_format(descriptor.format);

        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(gl_texture));
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal_fmt,
                descriptor.width as i32,
                descriptor.height as i32,
                0,
                fmt,
                pixel_type,
                glow::PixelUnpackData::Slice(None),
            );
        }

        let (idx, gen) = self.textures.alloc(TextureSlot {
            gl_texture,
            _format: descriptor.format,
            _width: descriptor.width,
            _height: descriptor.height,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_texture(&mut self, handle: TextureHandle) {
        let slot = self.textures.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                unsafe { self.gl.delete_texture(slot.value.gl_texture) };
            }
        }
        self.textures.free(handle.index);
    }

    // ██ shader modules ██████████████████████████████████████████████████████████████████████

    fn create_shader_module(
        &mut self,
        descriptor: &ShaderModuleDescriptor,
    ) -> Result<ShaderModuleHandle, RhiError> {
        let (idx, gen) = self.shader_modules.alloc(ShaderModuleSlot {
            _format: descriptor.format,
            _source_hash: descriptor.source_hash,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_shader_module(&mut self, handle: ShaderModuleHandle) {
        self.shader_modules.free(handle.index);
    }

    // ██ render passes ██████████████████████████████████████████████████████████████████████

    fn create_render_pass(
        &mut self,
        descriptor: &RenderPassDescriptor,
    ) -> Result<RenderPassHandle, RhiError> {
        let (idx, gen) = self.render_passes.alloc(RenderPassSlot {
            _descriptor: descriptor.clone(),
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_render_pass(&mut self, handle: RenderPassHandle) {
        self.render_passes.free(handle.index);
    }

    // ██ framebuffers ███████████████████████████████████████████████████████████████████████

    fn create_framebuffer(
        &mut self,
        descriptor: &FramebufferDescriptor,
    ) -> Result<FramebufferHandle, RhiError> {
        let gl_framebuffer = unsafe {
            self.gl
                .create_framebuffer()
                .map_err(|e| RhiError::Backend { detail: e })?
        };

        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, Some(gl_framebuffer));
        }

        // Attach color textures.
        for (i, &color_handle) in descriptor.color_attachments.iter().enumerate() {
            if let Some(tex_slot) = self
                .textures
                .get(color_handle.index)
                .filter(|s| s.generation == color_handle.generation)
            {
                unsafe {
                    self.gl.framebuffer_texture_2d(
                        glow::FRAMEBUFFER,
                        glow::COLOR_ATTACHMENT0 + i as u32,
                        glow::TEXTURE_2D,
                        Some(tex_slot.value.gl_texture),
                        0,
                    );
                }
            }
        }

        // Attach depth-stencil if present.
        if let Some(depth_handle) = descriptor.depth_stencil_attachment {
            if let Some(tex_slot) = self
                .textures
                .get(depth_handle.index)
                .filter(|s| s.generation == depth_handle.generation)
            {
                unsafe {
                    self.gl.framebuffer_texture_2d(
                        glow::FRAMEBUFFER,
                        glow::DEPTH_ATTACHMENT,
                        glow::TEXTURE_2D,
                        Some(tex_slot.value.gl_texture),
                        0,
                    );
                }
            }
        }

        let (idx, gen) = self.framebuffers.alloc(FramebufferSlot {
            gl_framebuffer,
            _width: descriptor.width,
            _height: descriptor.height,
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_framebuffer(&mut self, handle: FramebufferHandle) {
        let slot = self.framebuffers.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                unsafe { self.gl.delete_framebuffer(slot.value.gl_framebuffer) };
            }
        }
        self.framebuffers.free(handle.index);
    }

    // ██ pipeline layouts ██████████████████████████████████████████████████████████████████

    fn create_pipeline_layout(
        &mut self,
        descriptor: &PipelineLayoutDescriptor,
    ) -> Result<PipelineLayoutHandle, RhiError> {
        let (idx, gen) = self.pipeline_layouts.alloc(PipelineLayoutSlot {
            _descriptor: descriptor.clone(),
        });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_pipeline_layout(&mut self, handle: PipelineLayoutHandle) {
        self.pipeline_layouts.free(handle.index);
    }

    // ██ pipelines █████████████████████████████████████████████████████████████████████████

    fn create_pipeline(
        &mut self,
        descriptor: &PipelineDescriptor,
    ) -> Result<PipelineHandle, RhiError> {
        let gl_program = unsafe {
            self.gl
                .create_program()
                .map_err(|e| RhiError::Backend { detail: e })?
        };

        struct Attached {
            shader: glow::Shader,
        }
        let mut attached: Vec<Attached> = Vec::new();

        for (i, &mod_handle) in descriptor.shader_modules.iter().enumerate() {
            // Validate handle but we don't need the slot contents for this stub.
            if self
                .shader_modules
                .get(mod_handle.index)
                .filter(|s| s.generation == mod_handle.generation)
                .is_none()
            {
                // Clean up and bail on invalid handle.
                for a in &attached {
                    unsafe {
                        self.gl.detach_shader(gl_program, a.shader);
                        self.gl.delete_shader(a.shader);
                    }
                }
                unsafe { self.gl.delete_program(gl_program) };
                return Err(RhiError::InvalidHandle);
            }

            let shader_type = if i == 0 {
                glow::VERTEX_SHADER
            } else {
                glow::FRAGMENT_SHADER
            };

            let gl_shader = unsafe {
                self.gl
                    .create_shader(shader_type)
                    .map_err(|e| RhiError::Backend { detail: e })?
            };

            // Set empty source for now. Real source loading would look up
            // source_hash in an external cache and call shader_source() here.
            unsafe {
                self.gl.shader_source(gl_shader, "");
                self.gl.compile_shader(gl_shader);
                if !self.gl.get_shader_compile_status(gl_shader) {
                    let log = self.gl.get_shader_info_log(gl_shader);
                    tracing::warn!(target: "opengl", "shader[{}] compile: {}", i, log);
                }
                self.gl.attach_shader(gl_program, gl_shader);
            }

            attached.push(Attached { shader: gl_shader });
        }

        unsafe {
            self.gl.link_program(gl_program);
            if !self.gl.get_program_link_status(gl_program) {
                let log = self.gl.get_program_info_log(gl_program);
                for a in &attached {
                    self.gl.detach_shader(gl_program, a.shader);
                    self.gl.delete_shader(a.shader);
                }
                self.gl.delete_program(gl_program);
                return Err(RhiError::ValidationFailed {
                    detail: format!("pipeline link: {log}"),
                });
            }
            // Detach and delete shaders after successful link.
            for a in &attached {
                self.gl.detach_shader(gl_program, a.shader);
                self.gl.delete_shader(a.shader);
            }
        }

        let (idx, gen) = self.pipelines.alloc(PipelineSlot { gl_program });
        Ok(ResourceHandle::new(idx, gen))
    }

    fn destroy_pipeline(&mut self, handle: PipelineHandle) {
        let slot = self.pipelines.get(handle.index);
        if let Some(slot) = slot {
            if slot.generation == handle.generation {
                unsafe { self.gl.delete_program(slot.value.gl_program) };
            }
        }
        self.pipelines.free(handle.index);
    }

    // ██ frame lifecycle █████████████████████████████████████████████████████████████████

    fn begin_frame(
        &mut self,
        _swapchain: SwapchainHandle,
    ) -> Result<(u32, Box<dyn CommandEncoder>), RhiError> {
        let encoder = OpenGlCommandEncoder {
            gl: self.gl.clone(),
            device_ptr: DeviceRef(std::ptr::from_ref::<OpenGlDevice>(self)),
            current_program: None,
            current_framebuffer: None,
        };
        Ok((0, Box::new(encoder)))
    }

    fn end_frame(
        &mut self,
        _swapchain: SwapchainHandle,
        _encoder: Box<dyn CommandEncoder>,
        _image_index: u32,
    ) -> Result<RendererStatistics, RhiError> {
        unsafe {
            self.gl.finish();
        }
        Ok(RendererStatistics::default())
    }

    fn recreate_swapchain(
        &mut self,
        _swapchain: SwapchainHandle,
        _width: u32,
        _height: u32,
    ) -> Result<(), RhiError> {
        Ok(())
    }

    fn wait_idle(&self) {
        unsafe {
            self.gl.finish();
        }
    }

    // ██ framebuffer readback ███████████████████████████████████████████████████████████████

    fn read_pixels(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, RhiError> {
        if width == 0 || height == 0 {
            return Ok(Vec::new());
        }

        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|v| v.checked_mul(4))
            .ok_or(RhiError::Backend {
                detail: "read_pixels: integer overflow in buffer size".to_string(),
            })?;

        let mut pixels = vec![0u8; size];

        // SAFETY: glow's read_pixels writes RGBA data into the pixel buffer.
        // The buffer is sized exactly to hold (width × height × 4) bytes.
        unsafe {
            self.gl.read_pixels(
                x as i32,
                y as i32,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }

        // OpenGL reads rows bottom-to-top; the trait contract specifies
        // top-to-bottom rows. Flip the rows in a second buffer.
        let row_size = (width as usize) * 4;
        let mut flipped = vec![0u8; size];
        for row in 0..height as usize {
            let src_start = (height as usize - 1 - row) * row_size;
            let dst_start = row * row_size;
            flipped[dst_start..dst_start + row_size]
                .copy_from_slice(&pixels[src_start..src_start + row_size]);
        }

        Ok(flipped)
    }
}

// ============================================================================
// Public constructor
// ============================================================================

pub fn backend(gl: glow::Context) -> OpenGlBackend {
    OpenGlBackend::new(gl)
}
