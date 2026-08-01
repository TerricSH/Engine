use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[cfg(not(feature = "subsystem-audio"))]
use std::any::Any;

#[cfg(all(
    feature = "subsystem-animation",
    feature = "subsystem-audio",
    feature = "subsystem-navigation"
))]
use engine_asset::cook::registered_asset_type_id;
use engine_asset::cook::AssetType;
use engine_renderer::{
    AssetId, ColorSpace, MaterialUpload, MeshUpload, MeshVertexFormat, SamplerDescriptor,
    TextureMipLevel, TextureUpload, TextureUploadFormat, Transparency,
};

use crate::EngineRuntime;

use super::validation::{split_rgba8_mips, validate_material_texture_dependencies};
use super::*;

include!("tests/helpers.rs");
include!("tests/decode_and_dependencies.rs");
include!("tests/staged_and_additive.rs");
include!("tests/streaming.rs");
include!("tests/extension_assets.rs");
include!("tests/subsystem_assets.rs");
