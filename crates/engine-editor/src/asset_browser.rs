//! Asset-browser data model used by the cross-platform editor shell.
//!
//! Source manifests are the authority for project asset kinds. Registry-only
//! and built-in assets are merged afterwards; their kinds are derived from
//! concrete cached values and never guessed from an ID string.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::commands::SetComponentField;
use engine_asset::cook::manifest::CURRENT_MANIFEST_VERSION;
use engine_asset::cook::{AssetType, SourceManifest};
use engine_asset::{validate_asset_id, AssetRegistry};
use engine_renderer::{MaterialUpload, MeshUpload, TextureUpload};
use engine_serialize::{AssetId, PersistentId, SchemaVersion, Value};
use thiserror::Error;

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
    Unknown,
}

impl AssetKindFilter {
    /// Every concrete filter in stable editor-menu order.
    pub const ALL_KINDS: [Self; 15] = [
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
    /// Whether this exact [`AssetId`] currently exists in [`AssetRegistry`].
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

    fn replace_registry_snapshot(&mut self, entries: Vec<AssetEntry>, source_folders: &[String]) {
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

/// Refresh the complete project asset catalog from authoritative source
/// manifests and the live registry.
///
/// This uses the cooker's manifest discovery rules: only direct children of
/// `source_root` whose extension is `.manifest` (case-insensitive) are read,
/// and files are processed in a deterministic case-insensitive name order.
/// Every manifest is parsed and validated before the panel snapshot changes,
/// so a malformed manifest never leaves a partial catalog behind.
///
/// Manifest entries are authoritative for kind and source path. Cached assets
/// that are not declared by any manifest are then merged as registry-only
/// entries, including unknown/non-rendering types. Tool-owned `editor/*`
/// cache entries remain private and are not project content.
pub fn refresh_project_asset_list(
    panel: &mut AssetBrowserPanel,
    registry: &AssetRegistry,
    source_root: &Path,
) -> Result<AssetRefreshSummary, AssetBrowserRefreshError> {
    let source_folders = collect_source_folders(source_root)?;
    let directory = std::fs::read_dir(source_root).map_err(|source| {
        AssetBrowserRefreshError::SourceRootRead {
            path: source_root.to_path_buf(),
            source,
        }
    })?;

    let mut manifest_paths = Vec::new();
    for directory_entry in directory {
        let directory_entry =
            directory_entry.map_err(|source| AssetBrowserRefreshError::SourceEntryRead {
                path: source_root.to_path_buf(),
                source,
            })?;
        let path = directory_entry.path();
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("manifest"))
        {
            manifest_paths.push(path);
        }
    }
    manifest_paths.sort_by(|left, right| {
        let left_name = left.file_name().unwrap_or_default().to_string_lossy();
        let right_name = right.file_name().unwrap_or_default().to_string_lossy();
        left_name
            .to_ascii_lowercase()
            .cmp(&right_name.to_ascii_lowercase())
            .then_with(|| left_name.cmp(&right_name))
    });

    let cooked_root = source_root
        .parent()
        .map(|assets_root| assets_root.join("cooked"))
        .unwrap_or_else(|| source_root.join("cooked"));
    let mut entries = Vec::new();
    let mut portable_ids: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();

    for manifest_path in &manifest_paths {
        let content = std::fs::read_to_string(manifest_path).map_err(|source| {
            AssetBrowserRefreshError::ManifestRead {
                path: manifest_path.clone(),
                source,
            }
        })?;
        let manifest: SourceManifest = serde_json::from_str(&content).map_err(|source| {
            AssetBrowserRefreshError::ManifestParse {
                path: manifest_path.clone(),
                source,
            }
        })?;
        if manifest.schema_version != CURRENT_MANIFEST_VERSION {
            return Err(AssetBrowserRefreshError::UnsupportedSchema {
                path: manifest_path.clone(),
                found: manifest.schema_version,
                expected: CURRENT_MANIFEST_VERSION,
            });
        }

        let mut manifest_assets = manifest.assets;
        manifest_assets.sort_by(|left, right| {
            left.id
                .id
                .cmp(&right.id.id)
                .then_with(|| left.source_path.cmp(&right.source_path))
        });
        for source_asset in manifest_assets {
            validate_manifest_asset_id(&source_asset.id).map_err(|detail| {
                AssetBrowserRefreshError::InvalidAssetId {
                    path: manifest_path.clone(),
                    id: source_asset.id.clone(),
                    detail,
                }
            })?;

            let portable_id = source_asset.id.id.to_ascii_lowercase();
            if let Some((first_id, first_manifest)) = portable_ids.get(&portable_id) {
                return Err(AssetBrowserRefreshError::DuplicateAssetId {
                    id: first_id.clone(),
                    first_manifest: first_manifest.clone(),
                    duplicate_manifest: manifest_path.clone(),
                });
            }
            portable_ids.insert(
                portable_id,
                (source_asset.id.id.clone(), manifest_path.clone()),
            );

            let mut entry = AssetEntry::new(
                source_asset.id.clone(),
                AssetKind::from(&source_asset.asset_type),
            );
            entry.source_path = Some(source_asset.source_path);
            entry.loaded = registry.contains(&entry.id);
            entry.cooked = cooked_artifact_path(&cooked_root, &entry.id).is_file();
            entry.manifest_declared = true;
            entries.push(entry);
        }
    }

    let declared_asset_count = entries.len();
    let mut registry_only_asset_count = 0;
    for id in registry.cached_ids() {
        if id.id.starts_with("editor/") || portable_ids.contains_key(&id.id.to_ascii_lowercase()) {
            continue;
        }
        let kind = registry_asset_kind(registry, &id);
        let mut entry = AssetEntry::new(id, kind);
        entry.loaded = true;
        entry.cooked = cooked_artifact_path(&cooked_root, &entry.id).is_file();
        entries.push(entry);
        registry_only_asset_count += 1;
    }

    panel.replace_registry_snapshot(entries, &source_folders);
    Ok(AssetRefreshSummary {
        manifest_count: manifest_paths.len(),
        declared_asset_count,
        registry_only_asset_count,
    })
}

fn collect_source_folders(source_root: &Path) -> Result<Vec<String>, AssetBrowserRefreshError> {
    fn visit(
        source_root: &Path,
        directory: &Path,
        folders: &mut Vec<String>,
    ) -> Result<(), AssetBrowserRefreshError> {
        let entries = std::fs::read_dir(directory).map_err(|source| {
            AssetBrowserRefreshError::SourceEntryRead {
                path: directory.to_path_buf(),
                source,
            }
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| AssetBrowserRefreshError::SourceEntryRead {
                path: directory.to_path_buf(),
                source,
            })?;
            let file_type =
                entry
                    .file_type()
                    .map_err(|source| AssetBrowserRefreshError::SourceEntryRead {
                        path: entry.path(),
                        source,
                    })?;
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            let relative = path.strip_prefix(source_root).map_err(|source| {
                AssetBrowserRefreshError::SourceEntryRead {
                    path: path.clone(),
                    source: std::io::Error::other(source.to_string()),
                }
            })?;
            let folder = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            folders.push(format!("/{folder}"));
            visit(source_root, &path, folders)?;
        }
        Ok(())
    }

    let mut folders = Vec::new();
    visit(source_root, source_root, &mut folders)?;
    Ok(folders)
}

fn registry_asset_kind(registry: &AssetRegistry, id: &AssetId) -> AssetKind {
    if registry.get::<MeshUpload>(id).is_some() {
        AssetKind::Mesh
    } else if registry.get::<MaterialUpload>(id).is_some() {
        AssetKind::Material
    } else if registry.get::<TextureUpload>(id).is_some() {
        AssetKind::Texture
    } else {
        AssetKind::Unknown
    }
}

fn cooked_artifact_path(cooked_root: &Path, id: &AssetId) -> PathBuf {
    cooked_root.join(format!("{}.cooked", id.id))
}

fn validate_manifest_asset_id(id: &AssetId) -> Result<(), String> {
    if id.id.is_empty() || id.id.len() > 128 {
        return Err("asset id must contain between 1 and 128 ASCII characters".into());
    }
    if !id
        .id
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        return Err("asset id must start with an ASCII letter or digit".into());
    }
    if !id
        .id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(
            "asset id may contain only ASCII letters, digits, hyphens, underscores, and dots"
                .into(),
        );
    }
    validate_asset_id(id).map_err(|error| error.to_string())
}

/// Construct the real serialized-scene edit for a renderer asset.
///
/// This never mutates an ECS [`World`]. Callers execute the returned command
/// through `EditorScene`/`CommandHistory`, preserving undo and dirty tracking.
pub fn assignment_command(
    target_entity: PersistentId,
    asset: &AssetEntry,
) -> Option<SetComponentField> {
    let field = match asset.kind {
        AssetKind::Mesh => "mesh",
        AssetKind::Material => "material",
        AssetKind::Texture
        | AssetKind::Shader
        | AssetKind::Scene
        | AssetKind::Pipeline
        | AssetKind::Script
        | AssetKind::Audio
        | AssetKind::Font
        | AssetKind::Animation
        | AssetKind::Skeleton
        | AssetKind::NavMesh
        | AssetKind::Logic
        | AssetKind::Prefab
        | AssetKind::Unknown => return None,
    };
    Some(SetComponentField::new(
        target_entity,
        "engine.renderable".to_string(),
        field.to_string(),
        Value::Asset(asset.id.clone()),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::commands::Command;
    use engine_asset::cook::{CookRules, SourceAssetEntry};
    use engine_renderer::{
        AxisAlignedBox, ColorSpace, IndexFormat, MeshVertexFormat, SamplerDescriptor,
        TextureMipLevel, TextureUploadFormat, Transparency,
    };
    use engine_scene::{ComponentRecord, DiagnosticsPolicy, EntityRecord, Scene, SceneSettings};
    use engine_serialize::SchemaVersion;

    static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

    struct AssetCatalogFixture {
        root: PathBuf,
        source_root: PathBuf,
        cooked_root: PathBuf,
    }

    impl AssetCatalogFixture {
        fn new(name: &str) -> Self {
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "engine_editor_asset_browser_{name}_{}_{sequence}",
                std::process::id()
            ));
            let source_root = root.join("assets/source");
            let cooked_root = root.join("assets/cooked");
            std::fs::create_dir_all(&source_root).expect("create source root");
            std::fs::create_dir_all(&cooked_root).expect("create cooked root");
            Self {
                root,
                source_root,
                cooked_root,
            }
        }

        fn write_manifest(&self, name: &str, manifest: &SourceManifest) {
            let bytes = serde_json::to_vec_pretty(manifest).expect("serialize source manifest");
            std::fs::write(self.source_root.join(name), bytes).expect("write source manifest");
        }

        fn write_cooked_marker(&self, id: &str) {
            std::fs::write(self.cooked_root.join(format!("{id}.cooked")), b"cooked")
                .expect("write cooked marker");
        }
    }

    impl Drop for AssetCatalogFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn source_entry(id: &str, asset_type: AssetType, source_path: &str) -> SourceAssetEntry {
        SourceAssetEntry {
            id: AssetId::new(id),
            asset_type,
            source_path: source_path.to_string(),
            cook_rules: CookRules::default(),
        }
    }

    fn empty_catalog_refresh(
        panel: &mut AssetBrowserPanel,
        registry: &AssetRegistry,
        fixture: &AssetCatalogFixture,
    ) {
        refresh_project_asset_list(panel, registry, &fixture.source_root)
            .expect("refresh project asset catalog");
    }

    fn mesh(id: AssetId) -> MeshUpload {
        MeshUpload {
            mesh_id: id,
            vertex_format: MeshVertexFormat::Pbr32,
            vertex_count: 3,
            vertex_bytes: vec![0; 96],
            index_format: IndexFormat::U16,
            index_count: 3,
            index_bytes: vec![0, 0, 1, 0, 2, 0],
            bounds: AxisAlignedBox::UNIT,
            content_hash: [1; 32],
        }
    }

    fn texture(id: AssetId) -> TextureUpload {
        TextureUpload {
            texture_id: id,
            width: 1,
            height: 1,
            format: TextureUploadFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            mip_levels: vec![TextureMipLevel {
                width: 1,
                height: 1,
                bytes: vec![255; 4],
            }],
            sampler: SamplerDescriptor::default(),
            content_hash: [2; 32],
        }
    }

    fn material(id: AssetId) -> MaterialUpload {
        MaterialUpload {
            material_id: id,
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 1.0,
            ambient_occlusion: 1.0,
            emissive: [0.0; 3],
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            transparency: Transparency::Opaque,
            double_sided: false,
            content_hash: [3; 32],
        }
    }

    fn registry_with_typed_assets() -> AssetRegistry {
        let mut registry = AssetRegistry::new();
        let mesh_id = AssetId::with_path("plain-name", "models/plain.mesh");
        let texture_id = AssetId::with_path("not-a-prefix", "textures/albedo.png");
        let material_id = AssetId::with_path("also-plain", "materials/default.mat");
        registry.insert_typed(mesh_id.clone(), mesh(mesh_id));
        registry.insert_typed(texture_id.clone(), texture(texture_id));
        registry.insert_typed(material_id.clone(), material(material_id));
        registry.insert_typed(AssetId::new("mesh-lie"), 42_u32);
        registry
    }

    #[test]
    fn registry_concrete_types_drive_classification_not_id_prefixes() {
        let fixture = AssetCatalogFixture::new("registry_types");
        let registry = registry_with_typed_assets();
        let mut panel = AssetBrowserPanel::new();
        empty_catalog_refresh(&mut panel, &registry, &fixture);

        assert_eq!(panel.catalog_assets().len(), 4);
        assert_eq!(
            panel
                .catalog_assets()
                .iter()
                .find(|entry| entry.id.id == "plain-name")
                .map(|entry| entry.kind),
            Some(AssetKind::Mesh)
        );
        assert_eq!(
            panel
                .catalog_assets()
                .iter()
                .find(|entry| entry.id.id == "not-a-prefix")
                .map(|entry| entry.kind),
            Some(AssetKind::Texture)
        );
        let unknown = panel
            .catalog_assets()
            .iter()
            .find(|entry| entry.id.id == "mesh-lie")
            .expect("raw/extension cache entry remains visible");
        assert_eq!(unknown.kind, AssetKind::Unknown);
        assert!(unknown.loaded);
        assert!(!unknown.manifest_declared);
    }

    #[test]
    fn authoritative_manifests_expose_every_asset_type_and_filter() {
        let fixture = AssetCatalogFixture::new("all_manifest_types");
        let cases = vec![
            (AssetType::Mesh, AssetKind::Mesh, AssetKindFilter::Mesh),
            (
                AssetType::Texture,
                AssetKind::Texture,
                AssetKindFilter::Texture,
            ),
            (
                AssetType::Shader,
                AssetKind::Shader,
                AssetKindFilter::Shader,
            ),
            (AssetType::Scene, AssetKind::Scene, AssetKindFilter::Scene),
            (
                AssetType::Material,
                AssetKind::Material,
                AssetKindFilter::Material,
            ),
            (
                AssetType::Pipeline,
                AssetKind::Pipeline,
                AssetKindFilter::Pipeline,
            ),
            (
                AssetType::Script,
                AssetKind::Script,
                AssetKindFilter::Script,
            ),
            (AssetType::Audio, AssetKind::Audio, AssetKindFilter::Audio),
            (AssetType::Font, AssetKind::Font, AssetKindFilter::Font),
            (
                AssetType::Animation,
                AssetKind::Animation,
                AssetKindFilter::Animation,
            ),
            (
                AssetType::Skeleton,
                AssetKind::Skeleton,
                AssetKindFilter::Skeleton,
            ),
            (
                AssetType::NavMesh,
                AssetKind::NavMesh,
                AssetKindFilter::NavMesh,
            ),
            (AssetType::Logic, AssetKind::Logic, AssetKindFilter::Logic),
            (
                AssetType::Prefab,
                AssetKind::Prefab,
                AssetKindFilter::Prefab,
            ),
            (
                AssetType::Unknown,
                AssetKind::Unknown,
                AssetKindFilter::Unknown,
            ),
        ];
        let assets = cases
            .iter()
            .enumerate()
            .map(|(index, (asset_type, _, _))| {
                source_entry(
                    &format!("asset-{index:02}"),
                    asset_type.clone(),
                    &format!("types/{index:02}.source"),
                )
            })
            .collect();
        fixture.write_manifest(
            "catalog.MANIFEST",
            &SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets,
            },
        );
        fixture.write_cooked_marker("asset-01");

        let mut registry = AssetRegistry::new();
        let loaded_mesh = AssetId::new("asset-00");
        registry.insert_typed(loaded_mesh.clone(), mesh(loaded_mesh));
        let mut panel = AssetBrowserPanel::new();
        let summary =
            refresh_project_asset_list(&mut panel, &registry, &fixture.source_root).unwrap();

        assert_eq!(summary.manifest_count, 1);
        assert_eq!(summary.declared_asset_count, cases.len());
        assert_eq!(summary.registry_only_asset_count, 0);
        panel.set_current_folder("/types");
        assert_eq!(panel.assets().len(), cases.len());
        for (index, (_, expected_kind, filter)) in cases.iter().enumerate() {
            panel.set_kind_filter(*filter);
            assert_eq!(panel.assets().len(), 1, "{} filter", filter.label());
            let id = format!("asset-{index:02}");
            let entry = panel
                .assets()
                .iter()
                .find(|entry| entry.id.id == id)
                .expect("manifest asset appears in catalog");
            assert_eq!(entry.kind, *expected_kind);
            assert_eq!(
                entry.source_path.as_deref(),
                Some(format!("types/{index:02}.source").as_str())
            );
            assert!(entry.manifest_declared);
            assert_eq!(entry.loaded, index == 0);
            assert_eq!(entry.cooked, index == 1);
            assert_eq!(panel.assets()[0].kind, *expected_kind);
        }

        assert_eq!(AssetKindFilter::ALL_KINDS.len(), cases.len());
        panel.set_kind_filter(AssetKindFilter::All);
        panel.set_search_query("07.source");
        assert_eq!(panel.assets().len(), 1);
        assert_eq!(panel.assets()[0].kind, AssetKind::Audio);
        panel.set_current_folder("/");
        panel.set_search_query("14.SOURCE");
        assert_eq!(panel.assets().len(), 1);
        assert_eq!(panel.assets()[0].kind, AssetKind::Unknown);
    }

    #[test]
    fn manifest_kind_is_authoritative_and_registry_only_assets_are_merged() {
        let fixture = AssetCatalogFixture::new("authority_and_merge");
        fixture.write_manifest(
            "game.manifest",
            &SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: vec![source_entry(
                    "declared-audio",
                    AssetType::Audio,
                    "audio/theme.ogg",
                )],
            },
        );
        fixture.write_cooked_marker("declared-audio");

        let mut registry = AssetRegistry::new();
        let declared = AssetId::new("declared-audio");
        registry.insert_typed(declared.clone(), mesh(declared));
        let declared_alias =
            AssetId::with_path("declared-audio", "runtime/duplicate-cache-key.mesh");
        registry.insert_typed(declared_alias.clone(), mesh(declared_alias));
        let builtin = AssetId::with_path("builtin-cube", "builtin/cube.mesh");
        registry.insert_typed(builtin.clone(), mesh(builtin.clone()));
        registry.insert_typed(AssetId::new("extension-data"), 7_u32);

        let mut panel = AssetBrowserPanel::new();
        let summary =
            refresh_project_asset_list(&mut panel, &registry, &fixture.source_root).unwrap();

        assert_eq!(summary.declared_asset_count, 1);
        assert_eq!(summary.registry_only_asset_count, 2);
        let declared = panel
            .catalog_assets()
            .iter()
            .find(|entry| entry.id.id == "declared-audio")
            .unwrap();
        assert_eq!(declared.kind, AssetKind::Audio);
        assert_eq!(declared.source_path.as_deref(), Some("audio/theme.ogg"));
        assert!(declared.loaded);
        assert!(declared.cooked);
        assert!(declared.manifest_declared);

        let builtin = panel
            .catalog_assets()
            .iter()
            .find(|entry| entry.id.id == "builtin-cube")
            .unwrap();
        assert_eq!(builtin.kind, AssetKind::Mesh);
        assert!(builtin.loaded);
        assert!(!builtin.manifest_declared);
        assert!(builtin.source_path.is_none());

        let extension = panel
            .catalog_assets()
            .iter()
            .find(|entry| entry.id.id == "extension-data")
            .unwrap();
        assert_eq!(extension.kind, AssetKind::Unknown);
    }

    #[test]
    fn invalid_manifest_is_reported_without_replacing_previous_snapshot() {
        let fixture = AssetCatalogFixture::new("invalid_manifest_transaction");
        let registry = registry_with_typed_assets();
        let mut panel = AssetBrowserPanel::new();
        empty_catalog_refresh(&mut panel, &registry, &fixture);
        let previous = panel.catalog_assets().to_vec();
        std::fs::write(fixture.source_root.join("broken.manifest"), b"{")
            .expect("write invalid manifest");

        let error = refresh_project_asset_list(&mut panel, &registry, &fixture.source_root)
            .expect_err("invalid manifest must fail refresh");
        assert!(matches!(
            error,
            AssetBrowserRefreshError::ManifestParse { .. }
        ));
        assert_eq!(panel.catalog_assets(), previous);
    }

    #[test]
    fn unsupported_manifest_schema_is_rejected() {
        let fixture = AssetCatalogFixture::new("unsupported_schema");
        fixture.write_manifest(
            "future.manifest",
            &SourceManifest {
                schema_version: SchemaVersion::new(99, 0, 0),
                assets: Vec::new(),
            },
        );
        let error = refresh_project_asset_list(
            &mut AssetBrowserPanel::new(),
            &AssetRegistry::new(),
            &fixture.source_root,
        )
        .expect_err("unsupported schema must fail refresh");
        assert!(matches!(
            error,
            AssetBrowserRefreshError::UnsupportedSchema { .. }
        ));
    }

    #[test]
    fn duplicate_manifest_ids_are_rejected_case_insensitively() {
        let fixture = AssetCatalogFixture::new("duplicate_ids");
        for (manifest_name, id) in [
            ("a.manifest", "Shared.Asset"),
            ("b.manifest", "shared.asset"),
        ] {
            fixture.write_manifest(
                manifest_name,
                &SourceManifest {
                    schema_version: CURRENT_MANIFEST_VERSION,
                    assets: vec![source_entry(
                        id,
                        AssetType::Scene,
                        "scenes/shared.scene.ron",
                    )],
                },
            );
        }
        let error = refresh_project_asset_list(
            &mut AssetBrowserPanel::new(),
            &AssetRegistry::new(),
            &fixture.source_root,
        )
        .expect_err("duplicate IDs must fail refresh");
        assert!(matches!(
            error,
            AssetBrowserRefreshError::DuplicateAssetId { .. }
        ));
    }

    #[test]
    fn manifest_asset_ids_use_cook_validation_rules() {
        let fixture = AssetCatalogFixture::new("invalid_asset_id");
        fixture.write_manifest(
            "bad-id.manifest",
            &SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: vec![source_entry("not/portable", AssetType::Mesh, "mesh.gltf")],
            },
        );
        let error = refresh_project_asset_list(
            &mut AssetBrowserPanel::new(),
            &AssetRegistry::new(),
            &fixture.source_root,
        )
        .expect_err("invalid ID must fail refresh");
        assert!(matches!(
            error,
            AssetBrowserRefreshError::InvalidAssetId { .. }
        ));
    }

    #[test]
    fn tool_owned_temporary_textures_do_not_appear_as_project_assets() {
        let fixture = AssetCatalogFixture::new("private_editor_assets");
        let mut registry = registry_with_typed_assets();
        let id = AssetId::new("editor/preview/temporary/0");
        registry.insert_typed(id.clone(), texture(id));
        let mut panel = AssetBrowserPanel::new();
        empty_catalog_refresh(&mut panel, &registry, &fixture);

        assert!(!panel
            .catalog_assets()
            .iter()
            .any(|entry| entry.id.id.starts_with("editor/")));
    }

    #[test]
    fn search_and_kind_filters_recompute_immediately_and_clamp_page() {
        let fixture = AssetCatalogFixture::new("search_filters");
        let registry = registry_with_typed_assets();
        let mut panel = AssetBrowserPanel::new();
        empty_catalog_refresh(&mut panel, &registry, &fixture);

        panel.set_search_query("ALBEDO");
        assert_eq!(panel.assets().len(), 1);
        assert_eq!(panel.assets()[0].kind, AssetKind::Texture);

        panel.set_kind_filter(AssetKindFilter::Mesh);
        assert!(panel.assets().is_empty());
        assert_eq!(panel.page(), 0);

        panel.set_current_folder("/models");
        panel.set_search_query("");
        assert_eq!(panel.assets().len(), 1);
        assert_eq!(panel.assets()[0].kind, AssetKind::Mesh);
    }

    #[test]
    fn folders_breadcrumbs_and_direct_contents_come_from_catalog_paths() {
        let fixture = AssetCatalogFixture::new("folders");
        std::fs::create_dir_all(fixture.source_root.join("empty/nested")).unwrap();
        fixture.write_manifest(
            "game.manifest",
            &SourceManifest {
                schema_version: CURRENT_MANIFEST_VERSION,
                assets: vec![
                    source_entry("root", AssetType::Logic, "root.ron"),
                    source_entry("shared", AssetType::Mesh, "models/shared.gltf"),
                    source_entry("hero", AssetType::Mesh, "models/hero/body.gltf"),
                    source_entry("albedo", AssetType::Texture, "textures/albedo.png"),
                ],
            },
        );
        let mut panel = AssetBrowserPanel::new();
        empty_catalog_refresh(&mut panel, &AssetRegistry::new(), &fixture);

        assert_eq!(
            panel
                .folders()
                .iter()
                .map(|folder| folder.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "/",
                "/empty",
                "/empty/nested",
                "/models",
                "/models/hero",
                "/textures"
            ]
        );
        assert_eq!(panel.assets().len(), 1);
        assert_eq!(panel.assets()[0].id.id, "root");

        panel.set_current_folder("MODELS");
        assert_eq!(panel.current_folder(), "/models");
        assert_eq!(panel.assets().len(), 1);
        assert_eq!(panel.assets()[0].id.id, "shared");

        panel.set_current_folder("/models/hero");
        assert_eq!(panel.breadcrumbs(), vec!["/", "/models", "/models/hero"]);
        assert_eq!(panel.assets()[0].id.id, "hero");

        panel.set_current_folder("/empty/nested");
        assert!(panel.assets().is_empty());

        panel.set_current_folder("/models");
        panel.set_search_query("hero");
        assert_eq!(panel.assets().len(), 1);
        assert_eq!(panel.assets()[0].id.id, "hero");

        panel.set_current_folder("/missing");
        assert_eq!(panel.current_folder(), "/");
    }

    #[test]
    fn pagination_uses_fixed_size_and_clamps_after_filtering() {
        let fixture = AssetCatalogFixture::new("pagination");
        let mut registry = AssetRegistry::new();
        for index in 0..(ASSET_BROWSER_PAGE_SIZE + 2) {
            let id = AssetId::with_path(format!("asset-{index:02}"), "models/");
            registry.insert_typed(id.clone(), mesh(id));
        }
        let mut panel = AssetBrowserPanel::new();
        empty_catalog_refresh(&mut panel, &registry, &fixture);
        panel.set_current_folder("/models");

        assert_eq!(panel.page_size(), ASSET_BROWSER_PAGE_SIZE);
        assert_eq!(panel.page_count(), 2);
        assert_eq!(panel.visible_assets().len(), ASSET_BROWSER_PAGE_SIZE);
        assert!(panel.next_page());
        assert_eq!(panel.visible_assets().len(), 2);
        assert!(!panel.next_page());

        panel.set_search_query("asset-00");
        assert_eq!(panel.page(), 0);
        assert_eq!(panel.page_count(), 1);
        assert!(!panel.previous_page());
    }

    #[test]
    fn selection_keeps_complete_asset_id_across_filters() {
        let fixture = AssetCatalogFixture::new("selection");
        let registry = registry_with_typed_assets();
        let mut panel = AssetBrowserPanel::new();
        empty_catalog_refresh(&mut panel, &registry, &fixture);
        let id = AssetId::with_path("plain-name", "models/plain.mesh");

        assert!(panel.select_asset(Some(id.clone())));
        panel.set_kind_filter(AssetKindFilter::Texture);
        assert_eq!(panel.selected_asset(), Some(&id));
        assert_eq!(
            panel.selected_entry().map(|entry| entry.kind),
            Some(AssetKind::Mesh)
        );
    }

    #[test]
    fn reveal_asset_clears_filters_opens_folder_and_selects_exact_identity() {
        let fixture = AssetCatalogFixture::new("reveal_asset");
        let mut registry = AssetRegistry::new();
        for index in 0..(ASSET_BROWSER_PAGE_SIZE + 2) {
            let id = AssetId::with_path(
                format!("asset-{index:02}"),
                format!("models/asset-{index:02}.mesh"),
            );
            registry.insert_typed(id.clone(), mesh(id));
        }
        let mut panel = AssetBrowserPanel::new();
        empty_catalog_refresh(&mut panel, &registry, &fixture);
        panel.set_search_query("does-not-match");
        panel.set_kind_filter(AssetKindFilter::Texture);
        let target_index = ASSET_BROWSER_PAGE_SIZE + 1;
        let target_id = format!("asset-{target_index:02}");
        let target_path = format!("models/asset-{target_index:02}.mesh");

        assert!(panel.reveal_asset(&target_id));
        assert_eq!(panel.search_query(), "");
        assert_eq!(panel.kind_filter(), AssetKindFilter::All);
        assert_eq!(panel.current_folder(), "/models");
        assert_eq!(panel.page(), 1);
        assert_eq!(
            panel.selected_asset(),
            Some(&AssetId::with_path(target_id, target_path))
        );
    }

    fn scene_with_renderable() -> Scene {
        let mut fields = BTreeMap::new();
        fields.insert("mesh".to_string(), Value::Asset(AssetId::new("old-mesh")));
        fields.insert(
            "material".to_string(),
            Value::Asset(AssetId::new("old-material")),
        );
        let mut components = BTreeMap::new();
        components.insert(
            "engine.renderable".to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields,
            },
        );
        Scene {
            schema_version: SchemaVersion::new(0, 1, 0),
            engine_version: "test".to_string(),
            scene_id: "test-scene".to_string(),
            name: "Test".to_string(),
            entities: vec![EntityRecord {
                persistent_id: "target".to_string(),
                parent: None,
                name: None,
                enabled: true,
                components,
            }],
            scene_settings: SceneSettings::default(),
            dependencies: Vec::new(),
            diagnostics_policy: DiagnosticsPolicy::Strict,
        }
    }

    #[test]
    fn mesh_and_material_create_real_undoable_scene_commands() {
        for (kind, field) in [(AssetKind::Mesh, "mesh"), (AssetKind::Material, "material")] {
            let id = AssetId::with_path(format!("new-{field}"), format!("assets/{field}"));
            let entry = AssetEntry::new(id.clone(), kind);
            let mut command = assignment_command("target".to_string(), &entry).unwrap();
            let mut scene = scene_with_renderable();
            command.execute(&mut scene).unwrap();

            assert_eq!(
                scene.entities[0].components["engine.renderable"].fields[field],
                Value::Asset(id)
            );
            command.undo(&mut scene).unwrap();
            assert_eq!(
                scene.entities[0].components["engine.renderable"].fields[field],
                Value::Asset(AssetId::new(format!("old-{field}")))
            );
        }
    }

    #[test]
    fn only_mesh_and_material_are_assignable_to_renderables() {
        for kind in [
            AssetKind::Texture,
            AssetKind::Shader,
            AssetKind::Scene,
            AssetKind::Pipeline,
            AssetKind::Script,
            AssetKind::Audio,
            AssetKind::Font,
            AssetKind::Animation,
            AssetKind::Skeleton,
            AssetKind::NavMesh,
            AssetKind::Logic,
            AssetKind::Unknown,
        ] {
            let asset = AssetEntry::new(AssetId::new(kind.label().to_lowercase()), kind);
            assert!(
                assignment_command("target".to_string(), &asset).is_none(),
                "{} must not be assignable to Renderable",
                kind.label()
            );
        }
    }

    #[test]
    fn refresh_removes_selection_only_when_registry_entry_disappears() {
        let fixture = AssetCatalogFixture::new("selection_removal");
        let registry = registry_with_typed_assets();
        let mut panel = AssetBrowserPanel::new();
        empty_catalog_refresh(&mut panel, &registry, &fixture);
        let selected = AssetId::with_path("plain-name", "models/plain.mesh");
        assert!(panel.select_asset(Some(selected)));

        empty_catalog_refresh(&mut panel, &AssetRegistry::new(), &fixture);
        assert!(panel.selected_asset().is_none());
    }
}
