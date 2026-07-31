use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::commands::SetComponentField;
use engine_asset::cook::AssetType;
use engine_serialize::{AssetId, PersistentId, SchemaVersion};
use thiserror::Error;

use super::assignment_command;

/// Number of entries shown on every asset-browser page.
pub const ASSET_BROWSER_PAGE_SIZE: usize = 12;

/// Concrete project asset kinds supported by the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssetKind {
    Mesh,
    Texture,
    Shader,
    Scene,
    Material,
    Pipeline,
    Script,
    Audio,
    Font,
    Animation,
    Skeleton,
    NavMesh,
    Logic,
    Prefab,
    EnvironmentMap,
    MorphTargetSet,
    Unknown,
}

impl AssetKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mesh => "Mesh",
            Self::Texture => "Texture",
            Self::Shader => "Shader",
            Self::Scene => "Scene",
            Self::Material => "Material",
            Self::Pipeline => "Pipeline",
            Self::Script => "Script",
            Self::Audio => "Audio",
            Self::Font => "Font",
            Self::Animation => "Animation",
            Self::Skeleton => "Skeleton",
            Self::NavMesh => "NavMesh",
            Self::Logic => "Logic",
            Self::Prefab => "Prefab",
            Self::EnvironmentMap => "Environment Map",
            Self::MorphTargetSet => "Morph Targets",
            Self::Unknown => "Unknown",
        }
    }
}

impl From<&AssetType> for AssetKind {
    fn from(asset_type: &AssetType) -> Self {
        match asset_type {
            AssetType::Mesh => Self::Mesh,
            AssetType::Texture => Self::Texture,
            AssetType::Shader => Self::Shader,
            AssetType::Scene => Self::Scene,
            AssetType::Material => Self::Material,
            AssetType::Pipeline => Self::Pipeline,
            AssetType::Script => Self::Script,
            AssetType::Audio => Self::Audio,
            AssetType::Font => Self::Font,
            AssetType::Animation => Self::Animation,
            AssetType::Skeleton => Self::Skeleton,
            AssetType::NavMesh => Self::NavMesh,
            AssetType::Logic => Self::Logic,
            AssetType::Prefab => Self::Prefab,
            AssetType::EnvironmentMap => Self::EnvironmentMap,
            AssetType::MorphTargetSet => Self::MorphTargetSet,
            AssetType::Unknown => Self::Unknown,
        }
    }
}

/// Type filter applied to the browser's current result set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssetKindFilter {
    #[default]
    All,
    Mesh,
    Texture,
    Shader,
    Scene,
    Material,
    Pipeline,
    Script,
    Audio,
    Font,
    Animation,
    Skeleton,
    NavMesh,
    Logic,
    Prefab,
    EnvironmentMap,
    MorphTargetSet,
    Unknown,
}

impl AssetKindFilter {
    /// Every concrete filter in stable editor-menu order.
    pub const ALL_KINDS: [Self; 17] = [
        Self::Mesh,
        Self::Texture,
        Self::Shader,
        Self::Scene,
        Self::Material,
        Self::Pipeline,
        Self::Script,
        Self::Audio,
        Self::Font,
        Self::Animation,
        Self::Skeleton,
        Self::NavMesh,
        Self::Logic,
        Self::Prefab,
        Self::EnvironmentMap,
        Self::MorphTargetSet,
        Self::Unknown,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Mesh => "Mesh",
            Self::Texture => "Texture",
            Self::Shader => "Shader",
            Self::Scene => "Scene",
            Self::Material => "Material",
            Self::Pipeline => "Pipeline",
            Self::Script => "Script",
            Self::Audio => "Audio",
            Self::Font => "Font",
            Self::Animation => "Animation",
            Self::Skeleton => "Skeleton",
            Self::NavMesh => "NavMesh",
            Self::Logic => "Logic",
            Self::Prefab => "Prefab",
            Self::EnvironmentMap => "Environment Map",
            Self::MorphTargetSet => "Morph Targets",
            Self::Unknown => "Unknown",
        }
    }

    const fn matches(self, kind: AssetKind) -> bool {
        match self {
            Self::All => true,
            Self::Mesh => matches!(kind, AssetKind::Mesh),
            Self::Texture => matches!(kind, AssetKind::Texture),
            Self::Shader => matches!(kind, AssetKind::Shader),
            Self::Scene => matches!(kind, AssetKind::Scene),
            Self::Material => matches!(kind, AssetKind::Material),
            Self::Pipeline => matches!(kind, AssetKind::Pipeline),
            Self::Script => matches!(kind, AssetKind::Script),
            Self::Audio => matches!(kind, AssetKind::Audio),
            Self::Font => matches!(kind, AssetKind::Font),
            Self::Animation => matches!(kind, AssetKind::Animation),
            Self::Skeleton => matches!(kind, AssetKind::Skeleton),
            Self::NavMesh => matches!(kind, AssetKind::NavMesh),
            Self::Logic => matches!(kind, AssetKind::Logic),
            Self::Prefab => matches!(kind, AssetKind::Prefab),
            Self::EnvironmentMap => matches!(kind, AssetKind::EnvironmentMap),
            Self::MorphTargetSet => matches!(kind, AssetKind::MorphTargetSet),
            Self::Unknown => matches!(kind, AssetKind::Unknown),
        }
    }
}

/// A single project asset displayed in the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetEntry {
    /// The complete registry key, including its optional logical path.
    pub id: AssetId,
    /// Kind declared by a source manifest, or proven by a concrete typed
    /// registry lookup for registry-only assets.
    pub kind: AssetKind,
    /// Source path exactly as declared by the authoritative source manifest.
    /// Registry-only and built-in assets do not invent one.
    pub source_path: Option<String>,
    /// Whether this exact [`AssetId`] currently exists in [`engine_asset::AssetRegistry`].
    pub loaded: bool,
    /// Whether the conventional cooked artifact currently exists on disk.
    pub cooked: bool,
    /// `true` for source-manifest assets, `false` for registry-only/built-in
    /// assets merged into the project view.
    pub manifest_declared: bool,
}

impl AssetEntry {
    pub fn new(id: AssetId, kind: AssetKind) -> Self {
        Self {
            id,
            kind,
            source_path: None,
            loaded: false,
            cooked: false,
            manifest_declared: false,
        }
    }

    /// User-facing label which retains enough path information to distinguish
    /// registry keys that share the same short ID.
    pub fn display_name(&self) -> String {
        match self.browser_path() {
            Some(path) if !path.is_empty() => format!("{} ({path})", self.id.id),
            _ => self.id.id.clone(),
        }
    }

    pub fn browser_path(&self) -> Option<&str> {
        self.source_path
            .as_deref()
            .or(self.id.logical_path.as_deref())
    }

    /// Canonical project folder containing this asset.
    pub fn folder_path(&self) -> String {
        asset_folder_for_browser_path(self.browser_path())
    }
}

/// One concrete folder derived from authoritative asset source/logical paths.
///
/// Folder entries are not invented assets and never enter the registry. They
/// are a deterministic view over the current catalog snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetFolder {
    /// Canonical slash-separated path. Root is always `/`.
    pub path: String,
    /// Last path component, or `Assets` for the root.
    pub name: String,
    /// Number of parent folders below the root.
    pub depth: usize,
    /// Assets stored directly in this folder (not recursive descendants).
    pub direct_asset_count: usize,
}

/// Summary returned by a successful project asset-catalog refresh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetRefreshSummary {
    pub manifest_count: usize,
    pub declared_asset_count: usize,
    pub registry_only_asset_count: usize,
}

/// A source catalog is replaced only after every manifest passes validation.
#[derive(Debug, Error)]
pub enum AssetBrowserRefreshError {
    #[error("could not read asset source root '{}': {source}", path.display())]
    SourceRootRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not enumerate asset source root '{}': {source}", path.display())]
    SourceEntryRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not read source manifest '{}': {source}", path.display())]
    ManifestRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse source manifest '{}': {source}", path.display())]
    ManifestParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "source manifest '{}' uses schema {found:?}; expected {expected:?}",
        path.display()
    )]
    UnsupportedSchema {
        path: PathBuf,
        found: SchemaVersion,
        expected: SchemaVersion,
    },
    #[error("source manifest '{}' has invalid asset id '{}': {detail}", path.display(), id.id)]
    InvalidAssetId {
        path: PathBuf,
        id: AssetId,
        detail: String,
    },
    #[error(
        "asset id '{id}' is duplicated or differs only by case in '{}' and '{}'",
        first_manifest.display(),
        duplicate_manifest.display()
    )]
    DuplicateAssetId {
        id: String,
        first_manifest: PathBuf,
        duplicate_manifest: PathBuf,
    },
}

/// Stateful asset-browser model.
///
/// `assets` is always the immediately recomputed, filtered result. The full
/// typed registry snapshot is kept separately so changing search or type
/// filters never requires another registry scan.
pub struct AssetBrowserPanel {
    search_query: String,
    current_folder: String,
    kind_filter: AssetKindFilter,
    all_assets: Vec<AssetEntry>,
    folders: Vec<AssetFolder>,
    assets: Vec<AssetEntry>,
    selected_asset: Option<AssetId>,
    page: usize,
}

impl AssetBrowserPanel {
    pub fn new() -> Self {
        Self {
            search_query: String::new(),
            current_folder: "/".to_string(),
            kind_filter: AssetKindFilter::All,
            all_assets: Vec::new(),
            folders: vec![AssetFolder {
                path: "/".to_string(),
                name: "Assets".to_string(),
                depth: 0,
                direct_asset_count: 0,
            }],
            assets: Vec::new(),
            selected_asset: None,
            page: 0,
        }
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Change the case-insensitive search and immediately recompute results.
    pub fn set_search_query(&mut self, query: impl Into<String>) {
        self.search_query = query.into();
        self.recompute_visible_assets();
    }

    pub fn current_folder(&self) -> &str {
        &self.current_folder
    }

    /// Change the logical-path folder and immediately recompute results.
    pub fn set_current_folder(&mut self, folder: impl Into<String>) {
        let folder = normalize_asset_folder(&folder.into());
        self.current_folder = self
            .folders
            .iter()
            .find(|entry| entry.path.eq_ignore_ascii_case(&folder))
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| "/".to_string());
        self.recompute_visible_assets();
    }

    /// Complete deterministic folder tree, root first and parents before
    /// children. UI code can render this directly without scanning assets.
    pub fn folders(&self) -> &[AssetFolder] {
        &self.folders
    }

    /// Canonical breadcrumb paths from the root to the current folder.
    pub fn breadcrumbs(&self) -> Vec<String> {
        if self.current_folder == "/" {
            return vec!["/".to_string()];
        }
        let mut breadcrumbs = vec!["/".to_string()];
        let mut current = String::new();
        for part in self.current_folder.trim_start_matches('/').split('/') {
            current.push('/');
            current.push_str(part);
            breadcrumbs.push(current.clone());
        }
        breadcrumbs
    }

    pub const fn kind_filter(&self) -> AssetKindFilter {
        self.kind_filter
    }

    /// Change the concrete asset-type filter and immediately recompute results.
    pub fn set_kind_filter(&mut self, filter: AssetKindFilter) {
        self.kind_filter = filter;
        self.recompute_visible_assets();
    }

    /// All entries matching the current search, folder and type filters.
    pub fn assets(&self) -> &[AssetEntry] {
        &self.assets
    }

    /// Complete unfiltered authoritative catalog snapshot.
    pub fn catalog_assets(&self) -> &[AssetEntry] {
        &self.all_assets
    }

    /// Entries on the current fixed-size page.
    pub fn visible_assets(&self) -> &[AssetEntry] {
        let start = self.page.saturating_mul(ASSET_BROWSER_PAGE_SIZE);
        let end = start
            .saturating_add(ASSET_BROWSER_PAGE_SIZE)
            .min(self.assets.len());
        self.assets.get(start..end).unwrap_or(&[])
    }

    pub const fn page(&self) -> usize {
        self.page
    }

    pub const fn page_size(&self) -> usize {
        ASSET_BROWSER_PAGE_SIZE
    }

    pub fn page_count(&self) -> usize {
        self.assets.len().div_ceil(ASSET_BROWSER_PAGE_SIZE)
    }

    pub fn previous_page(&mut self) -> bool {
        if self.page == 0 {
            return false;
        }
        self.page -= 1;
        true
    }

    pub fn next_page(&mut self) -> bool {
        if self.page + 1 >= self.page_count() {
            return false;
        }
        self.page += 1;
        true
    }

    /// The exact registry key selected by the user.
    pub fn selected_asset(&self) -> Option<&AssetId> {
        self.selected_asset.as_ref()
    }

    pub fn selected_entry(&self) -> Option<&AssetEntry> {
        let selected = self.selected_asset.as_ref()?;
        self.all_assets.iter().find(|entry| &entry.id == selected)
    }

    pub fn select_asset(&mut self, id: Option<AssetId>) -> bool {
        if let Some(candidate) = id.as_ref() {
            if !self.all_assets.iter().any(|entry| &entry.id == candidate) {
                return false;
            }
        }
        self.selected_asset = id;
        true
    }

    /// Reveal a catalog asset the way Unity's Project window does after a
    /// create, import, duplicate, or move operation. Search/type filters are
    /// cleared, the containing folder and page are opened, and the exact
    /// registry identity (including its logical path) becomes selected.
    pub fn reveal_asset(&mut self, stable_id: &str) -> bool {
        let Some(entry) = self
            .all_assets
            .iter()
            .find(|entry| entry.id.id == stable_id)
            .cloned()
        else {
            return false;
        };
        self.search_query.clear();
        self.kind_filter = AssetKindFilter::All;
        self.set_current_folder(entry.folder_path());
        let Some(index) = self
            .assets
            .iter()
            .position(|candidate| candidate.id == entry.id)
        else {
            return false;
        };
        self.page = index / ASSET_BROWSER_PAGE_SIZE;
        self.selected_asset = Some(entry.id);
        true
    }

    /// Construct an undoable scene command for the selected mesh or material.
    /// Textures intentionally return `None`: assigning a texture requires
    /// editing a material, not replacing a renderable's material asset.
    pub fn selected_assignment_command(
        &self,
        target_entity: PersistentId,
    ) -> Option<SetComponentField> {
        assignment_command(target_entity, self.selected_entry()?)
    }

    pub(super) fn replace_registry_snapshot(
        &mut self,
        entries: Vec<AssetEntry>,
        source_folders: &[String],
    ) {
        self.all_assets = entries;
        self.rebuild_folders(source_folders);
        if self
            .selected_asset
            .as_ref()
            .is_some_and(|selected| !self.all_assets.iter().any(|entry| &entry.id == selected))
        {
            self.selected_asset = None;
        }
        self.recompute_visible_assets();
    }

    fn rebuild_folders(&mut self, source_folders: &[String]) {
        let mut direct_counts = BTreeMap::<String, usize>::new();
        direct_counts.insert("/".to_string(), 0);
        for folder in source_folders {
            direct_counts
                .entry(normalize_asset_folder(folder))
                .or_default();
        }
        for asset in &self.all_assets {
            let folder = asset.folder_path();
            *direct_counts.entry(folder.clone()).or_default() += 1;
            let mut ancestor = String::new();
            for part in folder.trim_start_matches('/').split('/') {
                if part.is_empty() {
                    continue;
                }
                ancestor.push('/');
                ancestor.push_str(part);
                direct_counts.entry(ancestor.clone()).or_default();
            }
        }
        self.folders = direct_counts
            .into_iter()
            .map(|(path, direct_asset_count)| AssetFolder {
                name: if path == "/" {
                    "Assets".to_string()
                } else {
                    path.rsplit('/').next().unwrap_or("Assets").to_string()
                },
                depth: path
                    .trim_matches('/')
                    .split('/')
                    .filter(|part| !part.is_empty())
                    .count(),
                path,
                direct_asset_count,
            })
            .collect();
        self.folders.sort_by(|left, right| {
            left.path
                .to_ascii_lowercase()
                .cmp(&right.path.to_ascii_lowercase())
                .then_with(|| left.path.cmp(&right.path))
        });
        if !self
            .folders
            .iter()
            .any(|folder| folder.path == self.current_folder)
        {
            self.current_folder = "/".to_string();
        }
    }

    fn recompute_visible_assets(&mut self) {
        let query = self.search_query.trim().to_lowercase();
        let folder = self.current_folder.to_ascii_lowercase();

        self.assets = self
            .all_assets
            .iter()
            .filter(|entry| self.kind_filter.matches(entry.kind))
            .filter(|entry| {
                let asset_folder = entry.folder_path().to_ascii_lowercase();
                if query.is_empty() {
                    asset_folder == folder
                } else {
                    folder == "/"
                        || asset_folder == folder
                        || asset_folder.starts_with(&format!("{folder}/"))
                }
            })
            .filter(|entry| {
                query.is_empty()
                    || entry.id.id.to_lowercase().contains(&query)
                    || entry.kind.label().to_lowercase().contains(&query)
                    || entry
                        .browser_path()
                        .is_some_and(|path| path.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();

        self.assets.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.id.id.cmp(&right.id.id))
                .then_with(|| left.id.logical_path.cmp(&right.id.logical_path))
        });
        self.clamp_page();
    }

    fn clamp_page(&mut self) {
        self.page = self.page.min(self.page_count().saturating_sub(1));
    }
}

fn normalize_asset_folder(folder: &str) -> String {
    let mut parts = Vec::new();
    let normalized = folder.replace('\\', "/");
    for part in normalized.split('/') {
        match part.trim() {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn asset_folder_for_browser_path(path: Option<&str>) -> String {
    let Some(path) = path else {
        return "/".to_string();
    };
    let normalized = path.replace('\\', "/");
    if normalized.ends_with('/') {
        return normalize_asset_folder(&normalized);
    }
    let parent = normalized
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    normalize_asset_folder(parent)
}

impl Default for AssetBrowserPanel {
    fn default() -> Self {
        Self::new()
    }
}
