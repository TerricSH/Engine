//! DirectX 12 backend implementation.
//!
//! This crate provides a full D3D12 rendering backend via the `windows` crate.
//! On non-Windows platforms, the backend returns stubs with descriptive errors.
//!
//! # Feature flags
//!
//! * `backend-dx12` (default) — enables the `windows` dependency and full
//!   D3D12 device creation, swapchain, and pipeline management.
//!
//! # Enabling
//!
//! The workspace requires the `windows` crate with D3D12 and DXGI features:
//!
//! ```toml
//! windows = { version = "0.58", features = [
//!     "Win32_Graphics_Direct3D12",
//!     "Win32_Graphics_Dxgi_Common",
//!     "Win32_Graphics_Dxgi",
//!     "Win32_Graphics_Direct3D",
//!     "Win32_Foundation",
//!     "Win32_System_Threading",
//!     "Win32_UI_WindowsAndMessaging",
//! ] }
//! ```

pub mod backend;
pub mod device;
mod encoder;
pub mod error;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod handle;
mod pipeline;
mod resources;
#[cfg(all(target_os = "windows", feature = "backend-dx12"))]
mod scene_data;
pub mod scene_renderer;
mod swapchain;

pub use backend::{backend, is_available, DirectX12Backend, Dx12Adapter};
pub use device::Dx12Device;
pub use error::Dx12Error;

#[cfg(test)]
mod tests {
    use super::*;
    use render_core::Backend;

    #[cfg(all(target_os = "windows", feature = "backend-dx12"))]
    use render_core::{
        BindGroupLayoutBinding, BindGroupLayoutDescriptor, Device, DeviceDescriptor,
        FramebufferDescriptor, PipelineLayoutDescriptor, PushConstantRange, RenderPassDescriptor,
        TextureDescriptor, TextureFormat, TextureUsage, ValidationMode,
    };

    #[test]
    fn dx12_backend_kind() {
        let backend = DirectX12Backend::new();
        assert_eq!(backend.kind(), render_core::BackendKind::DirectX12);
    }

    #[test]
    fn dx12_backend_enumerate_adapters() {
        let backend = DirectX12Backend::new();
        let result = backend.enumerate_adapters();
        // Should succeed (stub on non-Windows, real on Windows)
        assert!(result.is_ok());
    }

    #[test]
    fn dx12_backend_helper_creates_backend() {
        let b = backend();
        assert_eq!(b.kind(), render_core::BackendKind::DirectX12);
    }

    #[test]
    fn dx12_error_adapter_not_found_display() {
        let err = Dx12Error::AdapterNotFound;
        assert_eq!(err.to_string(), "no suitable DirectX 12 adapter found");
    }

    #[test]
    fn dx12_error_device_creation_failed_display() {
        let err = Dx12Error::DeviceCreationFailed("driver not found".to_string());
        assert_eq!(err.to_string(), "device creation failed: driver not found");
    }

    #[test]
    fn dx12_is_available_on_current_platform() {
        // is_available returns true only on Windows with the feature enabled
        #[cfg(all(target_os = "windows", feature = "backend-dx12"))]
        assert!(is_available());
        #[cfg(not(all(target_os = "windows", feature = "backend-dx12")))]
        assert!(!is_available());
    }

    #[cfg(all(target_os = "windows", feature = "backend-dx12"))]
    #[test]
    fn dx12_root_signature_accepts_scene_constants() {
        let adapter = DirectX12Backend::new()
            .enumerate_adapters()
            .expect("adapter enumeration")
            .into_iter()
            .next()
            .expect("DX12 adapter");
        let descriptor = DeviceDescriptor {
            required_limits: adapter.capabilities.limits.clone(),
            adapter,
            required_features: Vec::new(),
            debug_label: Some("root-signature-smoke".into()),
            validation_mode: ValidationMode::Standard,
        };
        let mut device = Dx12Device::create(&descriptor).expect("DX12 device");
        let layout = device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                bind_group_layouts: vec![BindGroupLayoutDescriptor {
                    set_index: 0,
                    bindings: vec![
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampled_texture_set".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 0,
                            resource_kind: "sampler_set".into(),
                        },
                        BindGroupLayoutBinding {
                            binding: 1,
                            resource_kind: "uniform_buffer".into(),
                        },
                    ],
                }],
                push_constant_ranges: vec![PushConstantRange {
                    stage_flags: 3,
                    offset: 0,
                    size: 208,
                }],
                debug_label: Some("root-constant-smoke".into()),
            })
            .expect("208-byte root-constant signature with paired textures");
        device.destroy_pipeline_layout(layout);
    }

    #[cfg(all(target_os = "windows", feature = "backend-dx12"))]
    #[test]
    fn dx12_creates_sampled_depth_framebuffer() {
        let adapter = DirectX12Backend::new()
            .enumerate_adapters()
            .expect("adapter enumeration")
            .into_iter()
            .next()
            .expect("DX12 adapter");
        let descriptor = DeviceDescriptor {
            required_limits: adapter.capabilities.limits.clone(),
            adapter,
            required_features: Vec::new(),
            debug_label: Some("sampled-depth-smoke".into()),
            validation_mode: ValidationMode::Standard,
        };
        let mut device = Dx12Device::create(&descriptor).expect("DX12 device");
        let texture = device
            .create_texture(&TextureDescriptor {
                width: 64,
                height: 64,
                depth_or_layers: 1,
                mip_levels: 1,
                format: TextureFormat::Depth32Float,
                usage_flags: TextureUsage(
                    TextureUsage::DEPTH_ATTACHMENT.0 | TextureUsage::SAMPLED.0,
                ),
                sample_count: 1,
                debug_label: Some("sampled-depth".into()),
            })
            .expect("sampled depth texture");
        let pass = device
            .create_render_pass(&RenderPassDescriptor {
                color_attachments: Vec::new(),
                depth_stencil_format: Some(TextureFormat::Depth32Float),
                sample_count: 1,
                present_after: false,
                debug_label: Some("depth-only-pass".into()),
            })
            .expect("depth-only pass");
        let framebuffer = device
            .create_framebuffer(&FramebufferDescriptor {
                render_pass: pass,
                color_attachments: Vec::new(),
                depth_stencil_attachment: Some(texture),
                width: 64,
                height: 64,
                debug_label: Some("sampled-depth-framebuffer".into()),
            })
            .expect("sampled depth framebuffer");
        device.destroy_framebuffer(framebuffer);
        device.destroy_render_pass(pass);
        device.destroy_texture(texture);
    }

    #[cfg(all(target_os = "windows", feature = "backend-dx12"))]
    #[test]
    fn dx12_uploads_rgba8_texture_and_creates_sampled_descriptors() {
        let adapter = DirectX12Backend::new()
            .enumerate_adapters()
            .expect("adapter enumeration")
            .into_iter()
            .next()
            .expect("DX12 adapter");
        let descriptor = DeviceDescriptor {
            required_limits: adapter.capabilities.limits.clone(),
            adapter,
            required_features: Vec::new(),
            debug_label: Some("texture-upload-smoke".into()),
            validation_mode: ValidationMode::Standard,
        };
        let mut device = Dx12Device::create(&descriptor).expect("DX12 device");
        let texture = device
            .upload_sampled_rgba8(
                2,
                2,
                engine_renderer::ColorSpace::Srgb,
                &[engine_renderer::TextureMipLevel {
                    width: 2,
                    height: 2,
                    bytes: vec![255; 16],
                }],
                engine_renderer::SamplerDescriptor::default(),
            )
            .expect("sampled RGBA8 upload");
        let (_, index) = Dx12Device::decode_handle(texture.index);
        let inner = device.textures.get(index).expect("uploaded texture");
        assert!(inner.sampled_srv_heap.is_some());
        assert!(inner.sampled_sampler_heap.is_some());
        device.destroy_texture(texture);
    }
}
