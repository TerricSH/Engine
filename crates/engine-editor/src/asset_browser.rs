//! Asset-browser data model used by the cross-platform editor shell.
//!
//! Source manifests are the authority for project asset kinds. Registry-only
//! and built-in assets are merged afterwards; their kinds are derived from
//! concrete cached values and never guessed from an ID string.

mod assignment;
mod catalog;
mod model;

pub use assignment::assignment_command;
pub use catalog::refresh_project_asset_list;
pub use model::{
    AssetBrowserPanel, AssetBrowserRefreshError, AssetEntry, AssetFolder, AssetKind,
    AssetKindFilter, AssetRefreshSummary, ASSET_BROWSER_PAGE_SIZE,
};

#[cfg(test)]
mod tests;
