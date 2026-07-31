mod decode;
mod decoded;
mod runtime;
mod types;
mod validation;

pub use types::{
    decode_cooked_batch, CookedAssetLoadReport, CookedCommitMode, DecodedBatch, ValidatedBatch,
};

pub(crate) use decoded::{additive_conflict_error, DecodedCookedAsset};
pub(crate) use types::InstallPlan;
pub(crate) use validation::{cooked_asset_id, material_texture_available, missing_texture_error};

#[cfg(test)]
#[path = "cooked_assets/tests.rs"]
pub(crate) mod tests;
