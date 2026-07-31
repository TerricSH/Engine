use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use engine_renderer::{AssetId, MaterialUpload, TextureMipLevel, TextureUpload};
use engine_serialize::{Diagnostic, DiagnosticSeverity};

use crate::EngineRuntime;

pub(super) fn validate_material_texture_dependencies(
    runtime: &EngineRuntime,
    textures: &[TextureUpload],
    materials: &[(PathBuf, MaterialUpload)],
    replaced_asset_ids: &BTreeSet<AssetId>,
) -> Vec<Diagnostic> {
    let batch_texture_ids = textures
        .iter()
        .map(|upload| upload.texture_id.clone())
        .collect::<BTreeSet<_>>();

    materials
        .iter()
        .flat_map(|(path, upload)| {
            upload
                .texture_references()
                .into_iter()
                .filter_map(|texture_id| {
                    let texture_id = texture_id?;
                    (!material_texture_available(
                        runtime,
                        &batch_texture_ids,
                        replaced_asset_ids,
                        texture_id,
                    ))
                    .then(|| missing_texture_error(path, &upload.material_id, texture_id))
                })
        })
        .collect()
}

pub(crate) fn missing_texture_error(
    path: &Path,
    material_id: &AssetId,
    texture_id: &AssetId,
) -> Diagnostic {
    cooked_error(
        path,
        format!(
            "cooked material '{}' references missing texture '{}'",
            material_id.id, texture_id.id
        ),
    )
}

/// A material's base-color texture resolves when it is decoded in the same
/// batch or already installed as a typed texture that the commit will not
/// unload.
pub(crate) fn material_texture_available(
    runtime: &EngineRuntime,
    batch_texture_ids: &BTreeSet<AssetId>,
    replaced_asset_ids: &BTreeSet<AssetId>,
    texture_id: &AssetId,
) -> bool {
    batch_texture_ids.contains(texture_id)
        || (!replaced_asset_ids.contains(texture_id)
            && runtime
                .asset_registry()
                .get::<TextureUpload>(texture_id)
                .is_some())
}

pub(crate) fn cooked_asset_id(path: &Path) -> Result<AssetId, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| format!("cooked asset has no UTF-8 file stem: {}", path.display()))?;
    Ok(AssetId::new(stem))
}

pub(super) fn split_rgba8_mips(
    width: u32,
    height: u32,
    mip_count: u8,
    data: &[u8],
) -> Result<Vec<TextureMipLevel>, String> {
    if width == 0 || height == 0 || mip_count == 0 {
        return Err("cooked texture dimensions and mip count must be non-zero".into());
    }
    let mut levels = Vec::with_capacity(mip_count as usize);
    let mut offset = 0usize;
    let mut mip_width = width;
    let mut mip_height = height;
    for _ in 0..mip_count {
        let byte_count = (mip_width as usize)
            .checked_mul(mip_height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "cooked texture mip size overflow".to_string())?;
        let end = offset
            .checked_add(byte_count)
            .ok_or_else(|| "cooked texture mip offset overflow".to_string())?;
        let bytes = data
            .get(offset..end)
            .ok_or_else(|| "cooked texture mip chain is truncated".to_string())?;
        levels.push(TextureMipLevel {
            width: mip_width,
            height: mip_height,
            bytes: bytes.to_vec(),
        });
        offset = end;
        mip_width = (mip_width / 2).max(1);
        mip_height = (mip_height / 2).max(1);
    }
    if offset != data.len() {
        return Err(format!(
            "cooked texture contains {} trailing bytes",
            data.len() - offset
        ));
    }
    Ok(levels)
}

pub(super) fn cooked_error(path: &Path, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        "AS0002",
        DiagnosticSeverity::Error,
        "engine-core.cooked-assets",
        message,
    )
    .path(path.to_string_lossy())
}
