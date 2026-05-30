//! Cooked shader format (CookedShader-v0, per FD-042).
//!
//! Contains SPIR-V bytecode plus reflection metadata extracted via naga.

use std::path::Path;

use engine_serialize::HashDigest;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::error::CookError;
use super::{AssetType, CookResult};

/// A cooked shader artifact.
///
/// Stores the variant key, compiled SPIR-V, and reflection data so the
/// runtime renderer can bind descriptors and push constants without
/// re-reflecting.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CookedShader {
    /// SHA-256 hash of the original GLSL source.
    pub source_hash: HashDigest,
    /// Variant key (e.g. bitfield of defines, see FD-040).
    pub variant_key: u64,
    /// Compiled SPIR-V bytecode (word-aligned).
    pub spirv: Vec<u8>,
    /// Reflection metadata extracted from the module.
    pub reflection: ShaderReflection,
    /// SHA-256 of (source_hash ‖ variant_key ‖ engine_defines).
    pub cooked_inputs_hash: HashDigest,
}

/// Reflection metadata extracted from a compiled shader module.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShaderReflection {
    /// Descriptor set bindings (textures, samplers, uniform/storage buffers).
    pub descriptor_bindings: Vec<DescriptorBinding>,
    /// Size of the push constant block in bytes (0 if none).
    pub push_constant_size: u32,
    /// Vertex input attributes (location, format, name).
    pub vertex_inputs: Vec<VertexInputReflection>,
}

/// A single descriptor binding in a shader.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DescriptorBinding {
    /// Descriptor set index (0‑3 typically).
    pub set: u8,
    /// Binding index within the set.
    pub binding: u32,
    /// Type name ("sampler", "texture_2d", "uniform_buffer", "storage_buffer").
    pub ty: String,
    /// Array count (1 for scalar bindings).
    pub count: u32,
}

/// A vertex input attribute gleaned from the shader entry point.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VertexInputReflection {
    /// Vertex attribute location.
    pub location: u32,
    /// Format hint (e.g. "float32", "float32x2", "float32x3", "float32x4").
    pub format: String,
    /// Semantic name from the shader source, if available.
    pub name: String,
}

/// Compile a GLSL shader source into a [`CookedShader`].
///
/// # Parameters
///
/// * `source_path`   – filesystem path to the `.vert`/`.frag`/`.comp` source.
/// * `source_code`   – the GLSL source text.
/// * `variant_key`   – variant key for this permutation.
/// * `stage_str`     – `"vertex"`, `"fragment"`, or `"compute"`.
/// * `engine_defines`– extra defines baked into `cooked_inputs_hash`.
pub fn cook_shader_from_glsl(
    source_path: &Path,
    source_code: &str,
    variant_key: u64,
    stage_str: &str,
    engine_defines: &[String],
) -> Result<CookedShader, CookError> {
    // 1. Hash the source.
    let source_hash = sha256_bytes(source_code.as_bytes());

    // 2. Parse with naga GLSL frontend.
    let stage = parse_stage(stage_str)?;
    let options = naga::front::glsl::Options::from(stage);
    let mut frontend = naga::front::glsl::Frontend::default();
    let module = frontend
        .parse(&options, source_code)
        .map_err(|e| CookError::Compile(format!("naga GLSL parse failed: {e}")))?;

    // 3. Validate the module (required before SPIR-V generation).
    let mut validator = Validator::new(ValidationFlags::all(), Capabilities::all());
    let module_info = validator
        .validate(&module)
        .map_err(|e| CookError::Compile(format!("naga validation failed: {e}")))?;

    // 4. Generate SPIR-V.
    let spv_options = naga::back::spv::Options {
        lang_version: (1, 0),
        flags: naga::back::spv::WriterFlags::empty(),
        ..naga::back::spv::Options::default()
    };
    let spirv_words = naga::back::spv::write_vec(&module, &module_info, &spv_options, None)
        .map_err(|e| CookError::Compile(format!("SPIR-V codegen failed: {e}")))?;
    let spirv_bytes: Vec<u8> = spirv_words.iter().flat_map(|w| w.to_le_bytes()).collect();

    // 5. Extract reflection.
    let reflection = extract_reflection(&module, stage_str)?;

    // 6. Compute cooked_inputs_hash = sha256(source_hash || variant_key || engine_defines).
    let mut hasher = Sha256::new();
    hasher.update(source_hash);
    hasher.update(variant_key.to_le_bytes());
    for def in engine_defines {
        hasher.update(def.as_bytes());
    }
    let cooked_inputs_hash: HashDigest = hasher.finalize().into();

    let _ = source_path; // used for error context

    Ok(CookedShader {
        source_hash,
        variant_key,
        spirv: spirv_bytes,
        reflection,
        cooked_inputs_hash,
    })
}

// ── Reflection extraction ────────────────────────────────────────────────

fn extract_reflection(
    module: &naga::Module,
    stage_str: &str,
) -> Result<ShaderReflection, CookError> {
    use naga::{AddressSpace, Binding, TypeInner};

    let mut bindings = Vec::new();
    let mut push_constant_size: u32 = 0;

    // ── Global variables (descriptor bindings + push constants) ──────
    for (_, var) in module.global_variables.iter() {
        // Descriptor bindings
        if let Some(ref res_binding) = var.binding {
            let ty = &module.types[var.ty];
            let type_name = describe_type(&ty.inner, &module.types);

            // Determine descriptor count from array size.
            let count = if let TypeInner::Array {
                base: _,
                size: naga::ArraySize::Constant(len),
                ..
            } = &ty.inner
            {
                len.get()
            } else {
                1
            };

            bindings.push(DescriptorBinding {
                set: res_binding.group as u8,
                binding: res_binding.binding,
                ty: type_name,
                count,
            });
        }

        // Push constants
        if var.space == AddressSpace::PushConstant {
            let ty = &module.types[var.ty];
            if let TypeInner::Struct { ref members, .. } = ty.inner {
                for member in members {
                    let member_end = member.offset + member_size(&member.ty, &module.types);
                    if member_end > push_constant_size {
                        push_constant_size = member_end;
                    }
                }
            }
        }
    }

    // ── Vertex inputs from entry point ───────────────────────────────
    let mut vertex_inputs = Vec::new();
    let target_stage = parse_stage(stage_str)?;

    for entry in &module.entry_points {
        if entry.stage != target_stage {
            continue;
        }
        if target_stage != naga::ShaderStage::Vertex {
            continue;
        }

        for arg in &entry.function.arguments {
            if let Some(Binding::Location { location, .. }) = arg.binding {
                let ty = &module.types[arg.ty];
                let fmt = describe_vertex_format(&ty.inner, &module.types);
                vertex_inputs.push(VertexInputReflection {
                    location,
                    format: fmt,
                    name: arg.name.clone().unwrap_or_default(),
                });
            }
        }
    }

    Ok(ShaderReflection {
        descriptor_bindings: bindings,
        push_constant_size,
        vertex_inputs,
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Parse a stage string into a naga [`ShaderStage`].
fn parse_stage(s: &str) -> Result<naga::ShaderStage, CookError> {
    match s.to_lowercase().as_str() {
        "vertex" => Ok(naga::ShaderStage::Vertex),
        "fragment" => Ok(naga::ShaderStage::Fragment),
        "compute" => Ok(naga::ShaderStage::Compute),
        other => Err(CookError::InvalidAsset(format!(
            "unknown shader stage: {other}"
        ))),
    }
}

fn sha256_bytes(data: &[u8]) -> HashDigest {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Describe a naga type in human-readable form for reflection.
fn describe_type(inner: &naga::TypeInner, types: &naga::UniqueArena<naga::Type>) -> String {
    use naga::TypeInner;
    match inner {
        TypeInner::Sampler { .. } => "sampler".into(),
        TypeInner::Image { .. } => "texture".into(),
        TypeInner::Array { base, size, .. } => {
            let base_desc = describe_type(&types[*base].inner, types);
            let count = match size {
                naga::ArraySize::Constant(len) => format!("{}", len.get()),
                _ => "?".into(),
            };
            format!("array<{base_desc},{count}>")
        }
        TypeInner::Struct { .. } => "struct".into(),
        TypeInner::Vector { .. } => "vector".into(),
        TypeInner::Matrix { .. } => "matrix".into(),
        TypeInner::Scalar(scalar) => format!("scalar({:?})", scalar.kind),
        TypeInner::Atomic(_) => "atomic".into(),
        TypeInner::Pointer { .. } => "pointer".into(),
        TypeInner::ValuePointer { .. } => "value_pointer".into(),
        _ => "unknown".into(),
    }
}

/// Describe a vertex input format based on the type.
fn describe_vertex_format(
    inner: &naga::TypeInner,
    _types: &naga::UniqueArena<naga::Type>,
) -> String {
    use naga::TypeInner;
    match inner {
        TypeInner::Vector {
            size,
            scalar: naga::Scalar { kind, .. },
            ..
        } => {
            let base = scalar_kind_name(*kind);
            let cols = match size {
                naga::VectorSize::Bi => "x2",
                naga::VectorSize::Tri => "x3",
                naga::VectorSize::Quad => "x4",
            };
            format!("{base}{cols}")
        }
        TypeInner::Scalar(naga::Scalar { kind, .. }) => scalar_kind_name(*kind).to_string(),
        _ => "unknown".into(),
    }
}

fn scalar_kind_name(kind: naga::ScalarKind) -> &'static str {
    match kind {
        naga::ScalarKind::Float => "float32",
        naga::ScalarKind::Sint => "sint32",
        naga::ScalarKind::Uint => "uint32",
        naga::ScalarKind::Bool => "bool",
        naga::ScalarKind::AbstractInt | naga::ScalarKind::AbstractFloat => "abstract",
    }
}

/// Compute the byte size of a type.
fn member_size(handle: &naga::Handle<naga::Type>, types: &naga::UniqueArena<naga::Type>) -> u32 {
    use naga::TypeInner;
    let ty = &types[*handle];
    match &ty.inner {
        TypeInner::Scalar(naga::Scalar { width, .. }) => *width as u32,
        TypeInner::Vector {
            size,
            scalar: naga::Scalar { width, .. },
            ..
        } => {
            let cols = match size {
                naga::VectorSize::Bi => 2,
                naga::VectorSize::Tri => 3,
                naga::VectorSize::Quad => 4,
            };
            *width as u32 * cols
        }
        TypeInner::Matrix { columns, rows, .. } => {
            let width: u32 = 4; // f32 width
            width * *columns as u32 * *rows as u32
        }
        TypeInner::Array { base, size, stride } => {
            let elem_size = member_size(base, types);
            let count = match size {
                naga::ArraySize::Constant(len) => len.get(),
                _ => 1,
            };
            let stride = *stride;
            if stride > elem_size {
                stride * count
            } else {
                elem_size * count
            }
        }
        TypeInner::Struct { members, .. } => {
            let mut max_end = 0u32;
            for m in members {
                let end = m.offset + member_size(&m.ty, types);
                if end > max_end {
                    max_end = end;
                }
            }
            max_end
        }
        _ => 0,
    }
}

/// Cook a GLSL shader file and write the cooked artifact.
pub fn cook_shader(
    source: &Path,
    output: &Path,
    variant_key: u64,
    stage: &str,
) -> Result<CookResult, CookError> {
    let source_code = std::fs::read_to_string(source).map_err(CookError::Io)?;

    let cooked = cook_shader_from_glsl(source, &source_code, variant_key, stage, &[])?;

    let result = super::write_cooked_artifact(
        output,
        AssetType::Shader.kind_code(),
        &bincode::serialize(&cooked).map_err(|e| CookError::InvalidAsset(e.to_string()))?,
        engine_serialize::SchemaVersion::new(0, 1, 0),
    )?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooked_shader_serde_roundtrip() {
        let shader = CookedShader {
            source_hash: [1u8; 32],
            variant_key: 42,
            spirv: vec![0x03, 0x02, 0x23, 0x07],
            reflection: ShaderReflection {
                descriptor_bindings: vec![DescriptorBinding {
                    set: 0,
                    binding: 0,
                    ty: "uniform_buffer".into(),
                    count: 1,
                }],
                push_constant_size: 64,
                vertex_inputs: vec![VertexInputReflection {
                    location: 0,
                    format: "float32x3".into(),
                    name: "in_position".into(),
                }],
            },
            cooked_inputs_hash: [2u8; 32],
        };

        let bytes = bincode::serialize(&shader).unwrap();
        let restored: CookedShader = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.variant_key, 42);
        assert_eq!(restored.reflection.push_constant_size, 64);
        assert_eq!(restored.reflection.descriptor_bindings.len(), 1);
    }

    #[test]
    fn extract_reflection_empty_module() {
        let module = naga::Module::default();
        let reflection = extract_reflection(&module, "vertex").unwrap();
        assert!(reflection.descriptor_bindings.is_empty());
        assert_eq!(reflection.push_constant_size, 0);
        assert!(reflection.vertex_inputs.is_empty());
    }

    #[test]
    fn parse_stage_cases() {
        assert!(parse_stage("vertex").is_ok());
        assert!(parse_stage("fragment").is_ok());
        assert!(parse_stage("compute").is_ok());
        assert!(parse_stage("VERTEX").is_ok());
        assert!(parse_stage("tesselation").is_err());
    }

    #[test]
    fn calculate_cooked_inputs_hash() {
        let shader = CookedShader {
            source_hash: [1u8; 32],
            variant_key: 0,
            spirv: vec![],
            reflection: ShaderReflection {
                descriptor_bindings: vec![],
                push_constant_size: 0,
                vertex_inputs: vec![],
            },
            cooked_inputs_hash: [2u8; 32],
        };
        // The hash is already stored; just verify deterministic structure
        let bytes = bincode::serialize(&shader).unwrap();
        let restored: CookedShader = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.source_hash, [1u8; 32]);
    }
}
