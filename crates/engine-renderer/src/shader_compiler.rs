//! Pluggable shader compiler infrastructure (Phase C — FD-058).
//!
//! Defines the [`ShaderCompiler`] trait and two optional backends:
//!
//! * **ShadercCompiler** (feature `runtime-shader-compilation` + `shaderc`) —
//!   uses Google's `libshaderc` for GLSL→SPIR-V compilation.
//! * **NagaCompiler** (feature `runtime-shader-compilation` + `naga`) —
//!   uses the pure-Rust [`naga`] crate for GLSL→SPIR-V compilation.
//!
//! Both backends are feature-gated so the default build path (pre-compiled
//! SPIR-V via `build.rs`) is not affected.

#![forbid(unsafe_code)]

use render_core::{SpecConstant, SpecValue};

// ============================================================================
// Source format
// ============================================================================

/// Supported shader source formats.
#[derive(Clone, Debug)]
pub enum ShaderSource {
    /// Pre-compiled SPIR-V bytecode.
    SpirV(Vec<u8>),
    /// GLSL source to be compiled at runtime.
    Glsl {
        source: String,
        stage: ShaderStage,
    },
}

/// Shader stage for GLSL source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderStage {
    Vertex,
    Fragment,
    Compute,
}

// ============================================================================
// Compiler trait
// ============================================================================

/// Compiles shader source (GLSL / HLSL) into SPIR-V bytecode.
///
/// Implementations **must** be [`Send`] so they can be used from a
/// [`MaterialResolverV2`](super::MaterialResolverV2) held across threads.
pub trait ShaderCompiler: Send {
    /// Compile `source` into SPIR-V, optionally passing preprocessor
    /// `defines` (name → value).
    fn compile(
        &self,
        source: &ShaderSource,
        defines: &[(String, String)],
    ) -> Result<Vec<u8>, String>;
}

// ============================================================================
// Naga backend  (feature = "runtime-shader-compilation")
// ============================================================================

#[cfg(feature = "runtime-shader-compilation")]
mod naga_backend {
    use super::*;

    /// Pure-Rust GLSL→SPIR-V compiler using the [`naga`] crate.
    pub struct NagaCompiler;

    impl ShaderCompiler for NagaCompiler {
        fn compile(
            &self,
            source: &ShaderSource,
            defines: &[(String, String)],
        ) -> Result<Vec<u8>, String> {
            match source {
                ShaderSource::SpirV(spv) => Ok(spv.clone()),
                ShaderSource::Glsl { source: src, stage } => {
                    let naga_stage = match stage {
                        ShaderStage::Vertex => naga::ShaderStage::Vertex,
                        ShaderStage::Fragment => naga::ShaderStage::Fragment,
                        ShaderStage::Compute => naga::ShaderStage::Compute,
                    };

                    let mut parser = naga::front::glsl::Parser::default();
                    let options = naga::front::glsl::Options {
                        stage: naga_stage,
                        defines: defines
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                        language: naga::front::glsl::Language::V1_50,
                        fake_missing_bindings: false,
                    };

                    let module = parser
                        .parse(&options, src)
                        .map_err(|e| format!("naga GLSL parse error: {e}"))?;

                    let spv = naga::back::spv::write_vec(
                        &module,
                        &naga::back::spv::Options {
                            lang_version: naga::back::spv::LangVersion::V1_6,
                            ..Default::default()
                        },
                        None,
                    )
                    .map_err(|e| format!("naga SPIR-V codegen error: {e}"))?;

                    // Convert Vec<u32> → Vec<u8>
                    let bytes: Vec<u8> = spv
                        .iter()
                        .flat_map(|w| w.to_ne_bytes())
                        .collect();
                    Ok(bytes)
                }
            }
        }
    }
}

#[cfg(feature = "runtime-shader-compilation")]
pub use naga_backend::NagaCompiler;

// ============================================================================
// Shaderc backend  (feature = "runtime-shader-compilation" + shaderc)
// ============================================================================

#[cfg(feature = "runtime-shader-compilation")]
#[cfg(feature = "shaderc")]
mod shaderc_backend {
    use super::*;

    /// Compiler backed by Google's `libshaderc` (requires Vulkan SDK).
    pub struct ShadercCompiler;

    impl ShaderCompiler for ShadercCompiler {
        fn compile(
            &self,
            source: &ShaderSource,
            defines: &[(String, String)],
        ) -> Result<Vec<u8>, String> {
            match source {
                ShaderSource::SpirV(spv) => Ok(spv.clone()),
                ShaderSource::Glsl { source: src, stage } => {
                    let shaderc_kind = match stage {
                        ShaderStage::Vertex => shaderc::ShaderKind::Vertex,
                        ShaderStage::Fragment => shaderc::ShaderKind::Fragment,
                        ShaderStage::Compute => shaderc::ShaderKind::Compute,
                    };

                    let mut compiler = shaderc::Compiler::new()
                        .ok_or_else(|| "shaderc: failed to create compiler".to_string())?;

                    let mut opts = shaderc::CompileOptions::new()
                        .ok_or_else(|| "shaderc: failed to create options".to_string())?;

                    opts.set_target_env(
                        shaderc::TargetEnv::Vulkan,
                        shaderc::EnvVersion::Vulkan1_3 as u32,
                    );
                    opts.set_target_spirv(shaderc::SpirvVersion::V1_6);
                    opts.set_optimization_level(shaderc::OptimizationLevel::Performance);

                    for (k, v) in defines {
                        opts.add_macro_definition(k, Some(v));
                    }

                    let artifact = compiler
                        .compile_into_spirv(src, shaderc_kind, "shader", "main", Some(&opts))
                        .map_err(|e| format!("shaderc error: {e}"))?;

                    Ok(artifact.as_binary_u8().to_vec())
                }
            }
        }
    }
}

#[cfg(feature = "runtime-shader-compilation")]
#[cfg(feature = "shaderc")]
pub use shaderc_backend::ShadercCompiler;

// ============================================================================
// Helper: build Vulkan specialization info from a slice of SpecConstant
// ============================================================================

/// Build a serialised data blob and `VkSpecializationMapEntry` array from
/// a list of [`SpecConstant`]s.
///
/// The Vulkan runtime calls this when creating a pipeline so that
/// `layout(constant_id = N)` declarations in GLSL get their values.
pub fn build_specialization_data(
    constants: &[SpecConstant],
) -> (Vec<u8>, Vec<u32>) {
    // Each spec constant is stored as 4 bytes in the data block.
    let mut data: Vec<u8> = Vec::with_capacity(constants.len() * 4);
    // Each map entry: (constant_id, offset, size) — stored as flat u32s.
    let mut entries: Vec<u32> = Vec::with_capacity(constants.len() * 3);

    for sc in constants {
        let offset = data.len() as u32;
        let (val_bytes, size): ([u8; 4], u32) = match sc.value {
            SpecValue::Bool(b) => ([(b as u8), 0, 0, 0], 4),
            SpecValue::U32(v) => (v.to_ne_bytes(), 4),
            SpecValue::F32(v) => (v.to_ne_bytes(), 4),
        };
        data.extend_from_slice(&val_bytes);
        entries.push(sc.id);        // constantID
        entries.push(offset);       // offset
        entries.push(size);         // size
    }

    (data, entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_specialization_data_produces_valid_entries() {
        let constants = vec![
            SpecConstant {
                id: 0,
                value: SpecValue::Bool(true),
            },
            SpecConstant {
                id: 1,
                value: SpecValue::U32(42),
            },
            SpecConstant {
                id: 2,
                value: SpecValue::F32(3.14),
            },
        ];

        let (data, entries) = build_specialization_data(&constants);

        // 3 constants × 4 bytes each
        assert_eq!(data.len(), 12);
        // 3 entries × 3 u32s each (id, offset, size)
        assert_eq!(entries.len(), 9);

        // entry 0: id=0, offset=0, size=1
        assert_eq!(entries[0], 0);
        assert_eq!(entries[1], 0);
        assert_eq!(entries[2], 1);

        // entry 1: id=1, offset=4, size=4
        assert_eq!(entries[3], 1);
        assert_eq!(entries[4], 4);
        assert_eq!(entries[5], 4);

        // entry 2: id=2, offset=8, size=4
        assert_eq!(entries[6], 2);
        assert_eq!(entries[7], 8);
        assert_eq!(entries[8], 4);

        // Data values
        assert_eq!(data[0], 1); // bool true = 1
        assert_eq!(&data[4..8], &42u32.to_ne_bytes());
        assert_eq!(&data[8..12], &3.14f32.to_ne_bytes());
    }

    #[test]
    fn empty_specialization() {
        let (data, entries) = build_specialization_data(&[]);
        assert!(data.is_empty());
        assert!(entries.is_empty());
    }
}
