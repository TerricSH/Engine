//! Asset-browser data model and lightweight immediate-mode UI.
//!
//! Asset kinds are derived from the concrete value cached in
//! [`AssetRegistry`]. An asset's name is deliberately never used as a type
//! discriminator: IDs are project data and are not a trustworthy schema.

use glam::Vec2;

use crate::commands::SetComponentField;
use crate::editor_ui::EditorUi;
use engine_asset::AssetRegistry;
use engine_renderer::{MaterialUpload, MeshUpload, TextureUpload};
use engine_scene::World;
use engine_serialize::{AssetId, PersistentId, Value};

/// Number of entries shown on every asset-browser page.
pub const ASSET_BROWSER_PAGE_SIZE: usize = 12;

/// Concrete renderer asset kinds supported by the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssetKind {
    Mesh,
    Material,
    Texture,
}

impl AssetKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mesh => "Mesh",
            Self::Material => "Material",
            Self::Texture => "Texture",
        }
    }
}

/// Type filter applied to the browser's current result set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AssetKindFilter {
    #[default]
    All,
    Mesh,
    Material,
    Texture,
}

impl AssetKindFilter {
    const fn matches(self, kind: AssetKind) -> bool {
        match self {
            Self::All => true,
            Self::Mesh => matches!(kind, AssetKind::Mesh),
            Self::Material => matches!(kind, AssetKind::Material),
            Self::Texture => matches!(kind, AssetKind::Texture),
        }
    }
}

/// A single typed asset displayed in the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetEntry {
    /// The complete registry key, including its optional logical path.
    pub id: AssetId,
    /// Kind proven by a successful typed lookup in the registry.
    pub kind: AssetKind,
    /// Optional thumbnail image data (for example, RGBA bytes).
    pub thumbnail: Option<Vec<u8>>,
}

impl AssetEntry {
    pub fn new(id: AssetId, kind: AssetKind) -> Self {
        Self {
            id,
            kind,
            thumbnail: None,
        }
    }

    /// User-facing label which retains enough path information to distinguish
    /// registry keys that share the same short ID.
    pub fn display_name(&self) -> String {
        match self.id.logical_path.as_deref() {
            Some(path) if !path.is_empty() => format!("{} ({path})", self.id.id),
            _ => self.id.id.clone(),
        }
    }
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
    assets: Vec<AssetEntry>,
    selected_asset: Option<AssetId>,
    page: usize,
    refresh_requested: bool,
    /// Whether the preview area needs re-rendering.
    pub preview_needed: bool,
}

impl AssetBrowserPanel {
    pub fn new() -> Self {
        Self {
            search_query: String::new(),
            current_folder: "/".to_string(),
            kind_filter: AssetKindFilter::All,
            all_assets: Vec::new(),
            assets: Vec::new(),
            selected_asset: None,
            page: 0,
            refresh_requested: false,
            preview_needed: false,
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
        self.current_folder = folder.into();
        self.recompute_visible_assets();
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
        let changed = self.selected_asset != id;
        self.selected_asset = id;
        self.preview_needed |= changed;
        true
    }

    /// Queue a project asset recook/reload request for the editor host.
    pub fn request_refresh(&mut self) {
        self.refresh_requested = true;
    }

    /// Consume the pending project asset recook/reload request.
    pub fn take_refresh_request(&mut self) -> bool {
        std::mem::take(&mut self.refresh_requested)
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

    fn replace_registry_snapshot(&mut self, entries: Vec<AssetEntry>) {
        self.all_assets = entries;
        if self
            .selected_asset
            .as_ref()
            .is_some_and(|selected| !self.all_assets.iter().any(|entry| &entry.id == selected))
        {
            self.selected_asset = None;
            self.preview_needed = true;
        }
        self.recompute_visible_assets();
    }

    fn recompute_visible_assets(&mut self) {
        let query = self.search_query.trim().to_lowercase();
        let folder = self.current_folder.trim().trim_matches('/').to_lowercase();

        self.assets = self
            .all_assets
            .iter()
            .filter(|entry| self.kind_filter.matches(entry.kind))
            .filter(|entry| {
                if folder.is_empty() {
                    return true;
                }
                entry.id.logical_path.as_deref().is_some_and(|path| {
                    path.trim_start_matches('/')
                        .to_lowercase()
                        .starts_with(&folder)
                })
            })
            .filter(|entry| {
                query.is_empty()
                    || entry.id.id.to_lowercase().contains(&query)
                    || entry.kind.label().to_lowercase().contains(&query)
                    || entry
                        .id
                        .logical_path
                        .as_deref()
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

impl Default for AssetBrowserPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Re-scan the registry, retaining only renderer assets whose concrete cached
/// type is known. Unknown or raw-only entries are intentionally omitted.
pub fn refresh_asset_list(panel: &mut AssetBrowserPanel, registry: &AssetRegistry) {
    let mut entries = Vec::new();
    for id in registry.cached_ids() {
        let kind = if registry.get::<MeshUpload>(&id).is_some() {
            Some(AssetKind::Mesh)
        } else if registry.get::<MaterialUpload>(&id).is_some() {
            Some(AssetKind::Material)
        } else if registry.get::<TextureUpload>(&id).is_some() {
            Some(AssetKind::Texture)
        } else {
            None
        };
        if let Some(kind) = kind {
            entries.push(AssetEntry::new(id, kind));
        }
    }
    panel.replace_registry_snapshot(entries);
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
        AssetKind::Texture => return None,
    };
    Some(SetComponentField::new(
        target_entity,
        "engine.renderable".to_string(),
        field.to_string(),
        Value::Asset(asset.id.clone()),
    ))
}

/// Draw the browser using its already-refreshed registry snapshot.
pub fn draw_asset_browser(ui: &mut EditorUi, panel: &mut AssetBrowserPanel) {
    if ui.button("Reimport Project Assets") {
        panel.request_refresh();
    }
    if ui.collapsing_header("Filters", true) {
        if let Some(query) = ui.text_field("Search", panel.search_query()) {
            panel.set_search_query(query);
        }
        if ui.button("All") {
            panel.set_kind_filter(AssetKindFilter::All);
        }
        if ui.button("Meshes") {
            panel.set_kind_filter(AssetKindFilter::Mesh);
        }
        if ui.button("Materials") {
            panel.set_kind_filter(AssetKindFilter::Material);
        }
        if ui.button("Textures") {
            panel.set_kind_filter(AssetKindFilter::Texture);
        }
    }

    ui.separator();
    ui.label_value("Assets", &panel.assets().len().to_string());
    if panel.assets().is_empty() {
        ui.label_value("Info", "No typed renderer assets match the filters.");
        return;
    }

    let selected = panel.selected_asset().cloned();
    let page_entries: Vec<(AssetId, AssetKind, String)> = panel
        .visible_assets()
        .iter()
        .map(|entry| (entry.id.clone(), entry.kind, entry.display_name()))
        .collect();
    let mut clicked = None;
    for (id, kind, name) in page_entries {
        let marker = if selected.as_ref() == Some(&id) {
            "*"
        } else {
            " "
        };
        if ui.button(&format!("{marker} [{}] {name}", kind.label())) {
            clicked = Some(id);
        }
    }
    if let Some(id) = clicked {
        let _ = panel.select_asset(Some(id));
    }

    ui.separator();
    if ui.button("Prev") {
        panel.previous_page();
    }
    if ui.button("Next") {
        panel.next_page();
    }
    let current_page = if panel.page_count() == 0 {
        0
    } else {
        panel.page() + 1
    };
    ui.label_value("Page", &format!("{current_page}/{}", panel.page_count()));

    if let Some(entry) = panel.selected_entry() {
        ui.separator();
        ui.label_value("Selected", &entry.id.id);
        ui.label_value("Type", entry.kind.label());
        if let Some(path) = entry.id.logical_path.as_deref() {
            ui.label_value("Path", path);
        }
    }
}

/// Legacy direct-World assignment retained for source compatibility.
///
/// New editor integrations must use [`assignment_command`] so changes flow
/// through scene history. Textures are never treated as materials.
#[deprecated(note = "use assignment_command or AssetBrowserPanel::selected_assignment_command")]
pub fn drag_assign_asset(
    panel: &AssetBrowserPanel,
    _pointer_pos: Vec2,
    target_entity: Option<engine_scene::Entity>,
    world: &mut World,
    _prefab_load: Option<&dyn engine_scene::prefab_instance::PrefabLoad>,
) -> bool {
    use engine_scene::components::Renderable;

    let Some(entry) = panel.selected_entry() else {
        return false;
    };
    let Some(entity) = target_entity else {
        return false;
    };
    let Some(renderable) = world.get_mut::<Renderable>(entity) else {
        return false;
    };

    match entry.kind {
        AssetKind::Mesh => renderable.mesh_asset.clone_from(&entry.id.id),
        AssetKind::Material => renderable.material_asset.clone_from(&entry.id.id),
        AssetKind::Texture => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::commands::Command;
    use engine_renderer::{
        AxisAlignedBox, ColorSpace, IndexFormat, MeshVertexFormat, SamplerDescriptor,
        TextureMipLevel, TextureUploadFormat, Transparency,
    };
    use engine_scene::{ComponentRecord, DiagnosticsPolicy, EntityRecord, Scene, SceneSettings};
    use engine_serialize::SchemaVersion;

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
            base_color_texture: None,
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
        let registry = registry_with_typed_assets();
        let mut panel = AssetBrowserPanel::new();
        refresh_asset_list(&mut panel, &registry);

        assert_eq!(panel.assets().len(), 3);
        assert_eq!(
            panel
                .assets()
                .iter()
                .find(|entry| entry.id.id == "plain-name")
                .map(|entry| entry.kind),
            Some(AssetKind::Mesh)
        );
        assert_eq!(
            panel
                .assets()
                .iter()
                .find(|entry| entry.id.id == "not-a-prefix")
                .map(|entry| entry.kind),
            Some(AssetKind::Texture)
        );
        assert!(!panel.assets().iter().any(|entry| entry.id.id == "mesh-lie"));
    }

    #[test]
    fn search_and_kind_filters_recompute_immediately_and_clamp_page() {
        let registry = registry_with_typed_assets();
        let mut panel = AssetBrowserPanel::new();
        refresh_asset_list(&mut panel, &registry);

        panel.set_search_query("ALBEDO");
        assert_eq!(panel.assets().len(), 1);
        assert_eq!(panel.assets()[0].kind, AssetKind::Texture);

        panel.set_kind_filter(AssetKindFilter::Mesh);
        assert!(panel.assets().is_empty());
        assert_eq!(panel.page(), 0);

        panel.set_search_query("");
        assert_eq!(panel.assets().len(), 1);
        assert_eq!(panel.assets()[0].kind, AssetKind::Mesh);
    }

    #[test]
    fn pagination_uses_fixed_size_and_clamps_after_filtering() {
        let mut registry = AssetRegistry::new();
        for index in 0..(ASSET_BROWSER_PAGE_SIZE + 2) {
            let id = AssetId::with_path(format!("asset-{index:02}"), "models/");
            registry.insert_typed(id.clone(), mesh(id));
        }
        let mut panel = AssetBrowserPanel::new();
        refresh_asset_list(&mut panel, &registry);

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
        let registry = registry_with_typed_assets();
        let mut panel = AssetBrowserPanel::new();
        refresh_asset_list(&mut panel, &registry);
        let id = AssetId::with_path("plain-name", "models/plain.mesh");

        assert!(panel.select_asset(Some(id.clone())));
        panel.set_kind_filter(AssetKindFilter::Texture);
        assert_eq!(panel.selected_asset(), Some(&id));
        assert_eq!(
            panel.selected_entry().map(|entry| entry.kind),
            Some(AssetKind::Mesh)
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
    fn texture_never_builds_a_renderable_material_command() {
        let texture = AssetEntry::new(AssetId::new("albedo"), AssetKind::Texture);
        assert!(assignment_command("target".to_string(), &texture).is_none());
    }

    #[test]
    fn refresh_removes_selection_only_when_registry_entry_disappears() {
        let registry = registry_with_typed_assets();
        let mut panel = AssetBrowserPanel::new();
        refresh_asset_list(&mut panel, &registry);
        let selected = AssetId::with_path("plain-name", "models/plain.mesh");
        assert!(panel.select_asset(Some(selected)));

        refresh_asset_list(&mut panel, &AssetRegistry::new());
        assert!(panel.selected_asset().is_none());
        assert!(panel.preview_needed);
    }

    #[test]
    fn project_refresh_request_is_edge_triggered() {
        let mut panel = AssetBrowserPanel::new();
        assert!(!panel.take_refresh_request());
        panel.request_refresh();
        assert!(panel.take_refresh_request());
        assert!(!panel.take_refresh_request());
    }

    #[test]
    fn draw_empty_browser_does_not_panic() {
        let mut panel = AssetBrowserPanel::new();
        let mut ui = EditorUi::new();
        draw_asset_browser(&mut ui, &mut panel);
    }
}
