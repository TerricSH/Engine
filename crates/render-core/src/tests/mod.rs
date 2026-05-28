use crate::{
    BackendKind, BufferHandle, BufferUsage, ResourceHandle, ShaderFormat, TextureFormat,
    TextureUsage,
};
use crate::RhiError;

#[test]
fn rhi_error_codes_match_registry() {
    let cases = [
        (RhiError::UnsupportedBackend, "rhi.unsupported_backend"),
        (
            RhiError::UnsupportedFeature {
                feature: "timeline-semaphore".to_string(),
            },
            "rhi.unsupported_feature",
        ),
        (
            RhiError::UnsupportedLimit {
                limit: "max_bind_groups".to_string(),
                requested: 4,
                available: 2,
            },
            "rhi.unsupported_limit",
        ),
        (RhiError::InvalidHandle, "rhi.invalid_handle"),
        (RhiError::DeviceLost, "rhi.device_lost"),
        (RhiError::SurfaceLost, "rhi.surface_lost"),
        (RhiError::OutOfMemory, "rhi.out_of_memory"),
        (
            RhiError::AllocationFailed { bytes: 64 },
            "rhi.allocation_failed",
        ),
        (
            RhiError::ValidationFailed {
                detail: "bad layout".to_string(),
            },
            "rhi.validation_failed",
        ),
        (
            RhiError::IncompatibleBindLayout {
                reason: "set count".to_string(),
            },
            "rhi.incompatible_bind_layout",
        ),
        (
            RhiError::Backend {
                detail: "driver".to_string(),
            },
            "rhi.backend",
        ),
    ];

    for (error, code) in cases {
        assert_eq!(error.code(), code);
    }
}

// ── ResourceHandle tests ─────────────────────────────────────────────────

#[test]
fn resource_handle_new_creates_handle() {
    let handle = ResourceHandle::<()>::new(42, 1);
    assert_eq!(handle.index, 42);
    assert_eq!(handle.generation, 1);
}

#[test]
fn resource_handle_equality() {
    let a = ResourceHandle::<()>::new(1, 2);
    let b = ResourceHandle::<()>::new(1, 2);
    let c = ResourceHandle::<()>::new(1, 3);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn resource_handle_copy_clone() {
    let a = BufferHandle::new(7, 1);
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn resource_handle_debug_format() {
    let handle = ResourceHandle::<()>::new(10, 2);
    let debug = format!("{:?}", handle);
    assert!(debug.contains("ResourceHandle"));
    assert!(debug.contains("index: 10"));
    assert!(debug.contains("generation: 2"));
}

#[test]
fn resource_handle_different_kinds() {
    let buf = BufferHandle::new(0, 1);
    let tex = crate::TextureHandle::new(5, 1);
    assert_ne!(buf.index, tex.index);
    assert_eq!(buf.generation, tex.generation);
}

// ── BufferUsage tests ────────────────────────────────────────────────────

#[test]
fn buffer_usage_constants_have_distinct_bits() {
    let all = BufferUsage::VERTEX.0
        | BufferUsage::INDEX.0
        | BufferUsage::UNIFORM.0
        | BufferUsage::STORAGE.0
        | BufferUsage::COPY_SRC.0
        | BufferUsage::COPY_DST.0;
    assert_eq!(all, 0b0011_1111);
}

#[test]
fn buffer_usage_bit_operations() {
    let combined = BufferUsage::VERTEX.0 | BufferUsage::INDEX.0;
    assert!(combined & BufferUsage::VERTEX.0 != 0);
    assert!(combined & BufferUsage::INDEX.0 != 0);
    assert!(combined & BufferUsage::UNIFORM.0 == 0);
}

#[test]
fn buffer_usage_default_is_zero() {
    let default = BufferUsage::default();
    assert_eq!(default.0, 0);
}

// ── TextureUsage tests ───────────────────────────────────────────────────

#[test]
fn texture_usage_constants_have_distinct_bits() {
    let all = TextureUsage::SAMPLED.0
        | TextureUsage::COLOR_ATTACHMENT.0
        | TextureUsage::DEPTH_ATTACHMENT.0
        | TextureUsage::COPY_SRC.0
        | TextureUsage::COPY_DST.0;
    assert_eq!(all, 0b0001_1111);
}

#[test]
fn texture_usage_combination() {
    let rt = TextureUsage::COLOR_ATTACHMENT.0 | TextureUsage::COPY_DST.0;
    assert!(rt & TextureUsage::COLOR_ATTACHMENT.0 != 0);
    assert!(rt & TextureUsage::COPY_DST.0 != 0);
    assert!(rt & TextureUsage::SAMPLED.0 == 0);
}

#[test]
fn texture_usage_default_is_zero() {
    assert_eq!(TextureUsage::default().0, 0);
}

// ── BackendKind tests ────────────────────────────────────────────────────

#[test]
fn backend_kind_debug() {
    assert_eq!(format!("{:?}", BackendKind::Vulkan), "Vulkan");
    assert_eq!(format!("{:?}", BackendKind::OpenGl), "OpenGl");
    assert_eq!(format!("{:?}", BackendKind::DirectX12), "DirectX12");
}

#[test]
fn backend_kind_equality() {
    assert_eq!(BackendKind::Vulkan, BackendKind::Vulkan);
    assert_ne!(BackendKind::Vulkan, BackendKind::OpenGl);
}

// ── TextureFormat tests ──────────────────────────────────────────────────

#[test]
fn texture_format_debug() {
    assert_eq!(format!("{:?}", TextureFormat::Rgba8Unorm), "Rgba8Unorm");
    assert_eq!(format!("{:?}", TextureFormat::Depth32Float), "Depth32Float");
}

#[test]
fn texture_format_equality() {
    assert_eq!(TextureFormat::Rgba8Unorm, TextureFormat::Rgba8Unorm);
    assert_ne!(TextureFormat::Rgba8Unorm, TextureFormat::Bgra8Unorm);
}

// ── ShaderFormat tests ───────────────────────────────────────────────────

#[test]
fn shader_format_debug() {
    assert_eq!(format!("{:?}", ShaderFormat::SpirV), "SpirV");
    assert_eq!(format!("{:?}", ShaderFormat::Glsl), "Glsl");
    assert_eq!(format!("{:?}", ShaderFormat::Dxil), "Dxil");
}

#[test]
fn shader_format_equality() {
    assert_eq!(ShaderFormat::SpirV, ShaderFormat::SpirV);
    assert_ne!(ShaderFormat::SpirV, ShaderFormat::Glsl);
}

// ── RhiError severity tests ──────────────────────────────────────────────

#[test]
fn rhi_error_severity_classification() {
    assert_eq!(RhiError::UnsupportedBackend.severity(), "fatal");
    assert_eq!(RhiError::SurfaceLost.severity(), "error-recoverable");
    assert_eq!(RhiError::InvalidHandle.severity(), "error");
    assert_eq!(RhiError::DeviceLost.severity(), "fatal");
    assert_eq!(
        RhiError::Backend {
            detail: "test".into()
        }
        .severity(),
        "error"
    );
}
