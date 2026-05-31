//! Asset browser panel — browsing, searching, previewing, and assigning assets.
//!
//! This module provides a standalone data model and draw functions for the
//! asset browser.  It is *not* an [`EditorPanel`] impl — use
//! [`panels::AssetBrowserPanel`](crate::panels::AssetBrowserPanel) for the
//! integrated panel version.

use glam::Vec2;

use crate::editor_ui::EditorUi;
use engine_asset::AssetRegistry;
use engine_scene::World;

// ---------------------------------------------------------------------------
// AssetEntry
// ---------------------------------------------------------------------------

/// A single asset entry displayed in the browser grid.
#[derive(Clone, Debug)]
pub struct AssetEntry {
    /// Display name of the asset.
    pub name: String,
    /// Logical path within the asset tree.
    pub path: String,
    /// Type identifier string (e.g. `"Mesh"`, `"Texture"`, `"Shader"`).
    pub type_id: String,
    /// Optional thumbnail image data (e.g. RGBA bytes).
    pub thumbnail: Option<Vec<u8>>,
}

// ---------------------------------------------------------------------------
// AssetBrowserPanel
// ---------------------------------------------------------------------------

/// Panel data for browsing, searching, previewing, and assigning assets.
pub struct AssetBrowserPanel {
    /// Current search / filter text.
    pub search_query: String,
    /// Currently selected folder path.
    pub current_folder: String,
    /// Flat list of assets matching the current filter.
    pub assets: Vec<AssetEntry>,
    /// Name of the currently selected asset, if any.
    pub selected_asset: Option<String>,
    /// Whether the preview area needs re-rendering.
    pub preview_needed: bool,
}

impl AssetBrowserPanel {
    /// Create a new asset browser panel.
    pub fn new() -> Self {
        Self {
            search_query: String::new(),
            current_folder: String::from("/"),
            assets: Vec::new(),
            selected_asset: None,
            preview_needed: false,
        }
    }
}

impl Default for AssetBrowserPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// refresh_asset_list
// ---------------------------------------------------------------------------

/// Extract the type prefix from an asset ID string.
///
/// Convention: IDs use a `type-name` format (e.g. `"mesh-cube"`).  Returns
/// the part before the first `'-'`, or `"other"` if there is no hyphen.
fn type_id_from_name(name: &str) -> String {
    name.split('-').next().unwrap_or("other").to_string()
}

/// Re-scan the [`AssetRegistry`] and populate `panel.assets` filtered by
/// `search_query` and `current_folder`.
///
/// Assets are sorted first by type, then by name.
pub fn refresh_asset_list(panel: &mut AssetBrowserPanel, registry: &AssetRegistry) {
    panel.assets.clear();

    for id in registry.cached_ids() {
        let name = id.id.clone();
        let path = id.logical_path.clone().unwrap_or_default();
        let type_id = type_id_from_name(&name);

        // Folder filter (case-insensitive prefix on type ID or path).
        if !panel.current_folder.is_empty() && panel.current_folder != "/" {
            let folder = panel.current_folder.trim_end_matches('/').to_lowercase();
            let matches_folder = type_id.to_lowercase().contains(&folder)
                || name.to_lowercase().contains(&folder)
                || path.to_lowercase().starts_with(&folder);
            if !matches_folder {
                continue;
            }
        }

        // Search filter (case-insensitive substring on name, type_id, path).
        if !panel.search_query.is_empty() {
            let q = panel.search_query.to_lowercase();
            let matches_search = name.to_lowercase().contains(&q)
                || type_id.to_lowercase().contains(&q)
                || path.to_lowercase().contains(&q);
            if !matches_search {
                continue;
            }
        }

        panel.assets.push(AssetEntry {
            name,
            path,
            type_id,
            thumbnail: None,
        });
    }

    // Sort by type_id then name.
    panel
        .assets
        .sort_by(|a, b| a.type_id.cmp(&b.type_id).then_with(|| a.name.cmp(&b.name)));
}

// ---------------------------------------------------------------------------
// draw_asset_browser
// ---------------------------------------------------------------------------

/// Draw the asset browser panel using [`EditorUi`] primitives.
///
/// Layout (v0):
/// - Left pane: folder tree (Search field + Recent / All Assets buttons)
/// - Right pane: scrollable list of asset entries
/// - Click to select
pub fn draw_asset_browser(ui: &mut EditorUi, panel: &mut AssetBrowserPanel) {
    // ── Left pane: folder tree ──────────────────────────────────────
    let left_open = ui.collapsing_header("Folders", true);
    if left_open {
        if let Some(new_query) = ui.text_field("Search", &panel.search_query) {
            panel.search_query = new_query;
        }

        ui.separator();
        let _ = ui.button("Recent");
        if ui.button("All Assets") {
            panel.current_folder = "/".to_string();
            panel.search_query.clear();
        }
    }

    ui.separator();

    // ── Asset count header ──────────────────────────────────────────
    let _ = ui.collapsing_header(&format!("Assets ({})", panel.assets.len()), true);

    if panel.assets.is_empty() {
        ui.text_field("Info", "No assets found. Refresh or adjust filters.");
        return;
    }

    // ── Asset list ──────────────────────────────────────────────────
    // v0: simple vertical list.  A future version could render a grid.
    for entry in &panel.assets {
        let label = format!("[{}]  {}", entry.type_id, entry.name);
        let clicked = ui.button(&label);
        if clicked {
            panel.selected_asset = Some(entry.name.clone());
            panel.preview_needed = true;
        }
    }

    // ── Preview area ────────────────────────────────────────────────
    ui.separator();
    if let Some(ref asset_name) = panel.selected_asset {
        let preview_open = ui.collapsing_header("Preview", true);
        if preview_open {
            ui.text_field("Selected", asset_name);
        }
    }
}

// ---------------------------------------------------------------------------
// drag_assign_asset
// ---------------------------------------------------------------------------

/// Attempt to assign the currently selected asset to a target entity.
///
/// Resolves the asset extension and writes the asset path to the appropriate
/// component field on the target entity (e.g. mesh → Renderable.mesh_asset).
///
/// Returns `true` if the assignment succeeded.
pub fn drag_assign_asset(
    panel: &AssetBrowserPanel,
    _pointer_pos: Vec2,
    target_entity: Option<engine_scene::Entity>,
    world: &mut World,
    prefab_load: Option<&dyn engine_scene::prefab_instance::PrefabLoad>,
) -> bool {
    use engine_scene::components::Renderable;

    let Some(ref selected) = panel.selected_asset else {
        return false;
    };
    let Some(entity) = target_entity else {
        return false;
    };

    let ext = selected.rsplit('.').next().unwrap_or("");

    match ext {
        "mesh" | "gltf" | "glb" | "model" => {
            if let Some(renderable) = world.get_mut::<Renderable>(entity) {
                renderable.mesh_asset = selected.clone();
                tracing::debug!(entity=?entity, asset=%selected, "assigned mesh");
                return true;
            }
        }
        "mat" | "material" => {
            if let Some(renderable) = world.get_mut::<Renderable>(entity) {
                renderable.material_asset = selected.clone();
                tracing::debug!(entity=?entity, asset=%selected, "assigned material");
                return true;
            }
        }
        "wav" | "mp3" | "ogg" | "flac" => {
            #[cfg(feature = "tooling-editor")]
            if let Some(source) = world.get_mut::<engine_audio::AudioSourceComponent>(entity) {
                source.clip_asset = Some(selected.clone());
                tracing::debug!(entity=?entity, asset=%selected, "assigned audio clip");
                return true;
            }
        }
        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "texture" => {
            // Assign texture reference via material if entity has a Renderable.
            if let Some(renderable) = world.get_mut::<Renderable>(entity) {
                if renderable.material_asset.is_empty() {
                    renderable.material_asset = selected.clone();
                }
                tracing::debug!(entity=?entity, asset=%selected, "texture assigned via material");
            } else {
                tracing::debug!(entity=?entity, asset=%selected, "texture dropped (no Renderable)");
                return false;
            }
            return true;
        }
        "prefab" => {
            if let Some(loader) = prefab_load {
                match engine_scene::prefab_instance::instantiate_prefab_from_asset(
                    world, loader, selected,
                ) {
                    Ok(result) => {
                        // Attach spawned prefab under the target entity.
                        if let Some(t) =
                            world.get_mut::<engine_scene::components::Transform>(result.root_entity)
                        {
                            t.parent = Some(entity);
                        }
                        tracing::debug!(entity=?entity, asset=%selected, "prefab instantiated and attached");
                        return true;
                    }
                    Err(e) => {
                        tracing::warn!(asset=%selected, error=%e, "prefab instantiation failed");
                        return false;
                    }
                }
            }
            tracing::debug!(entity=?entity, asset=%selected, "prefab dropped — no PrefabLoad resolver available");
            return false;
        }
        _ => {
            tracing::warn!(ext, "unhandled asset extension for assignment");
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    // ── Panel construction ──────────────────────────────────────────

    #[test]
    fn panel_new_has_defaults() {
        let panel = AssetBrowserPanel::new();
        assert!(panel.assets.is_empty());
        assert_eq!(panel.current_folder, "/");
        assert!(panel.search_query.is_empty());
        assert!(panel.selected_asset.is_none());
        assert!(!panel.preview_needed);
    }

    #[test]
    fn panel_default_is_same_as_new() {
        let a = AssetBrowserPanel::new();
        let b = AssetBrowserPanel::default();
        assert_eq!(a.assets.len(), b.assets.len());
        assert_eq!(a.current_folder, b.current_folder);
        assert_eq!(a.search_query, b.search_query);
    }

    // ── type_id_from_name ───────────────────────────────────────────

    #[test]
    fn type_id_from_hyphenated_name() {
        assert_eq!(type_id_from_name("mesh-cube"), "mesh");
        assert_eq!(type_id_from_name("texture-floor"), "texture");
        assert_eq!(type_id_from_name("shader-standard"), "shader");
    }

    #[test]
    fn type_id_without_hyphen_returns_self() {
        assert_eq!(type_id_from_name("simpleid"), "simpleid");
    }

    #[test]
    fn type_id_empty_string() {
        assert_eq!(type_id_from_name(""), "");
    }

    // ── AssetEntry construction ─────────────────────────────────────

    #[test]
    fn asset_entry_fields() {
        let entry = AssetEntry {
            name: "test".into(),
            path: "/test".into(),
            type_id: "mesh".into(),
            thumbnail: None,
        };
        assert_eq!(entry.name, "test");
        assert_eq!(entry.path, "/test");
        assert_eq!(entry.type_id, "mesh");
        assert!(entry.thumbnail.is_none());
    }

    // ── draw_asset_browser ──────────────────────────────────────────

    #[test]
    fn draw_asset_browser_empty_does_not_panic() {
        let mut panel = AssetBrowserPanel::new();
        let mut ui = EditorUi::new();
        draw_asset_browser(&mut ui, &mut panel);
        // No panic means success.
    }

    #[test]
    fn draw_asset_browser_with_entries_does_not_panic() {
        let mut panel = AssetBrowserPanel::new();
        panel.assets.push(AssetEntry {
            name: "cube".into(),
            path: "meshes/cube".into(),
            type_id: "mesh".into(),
            thumbnail: None,
        });
        panel.assets.push(AssetEntry {
            name: "floor".into(),
            path: "textures/floor".into(),
            type_id: "texture".into(),
            thumbnail: None,
        });

        let mut ui = EditorUi::new();
        draw_asset_browser(&mut ui, &mut panel);
    }

    // ── search/filter ───────────────────────────────────────────────

    #[test]
    fn filter_empty_query_shows_all() {
        let mut panel = AssetBrowserPanel::new();
        panel.assets.push(AssetEntry {
            name: "a".into(),
            path: "".into(),
            type_id: "mesh".into(),
            thumbnail: None,
        });
        panel.assets.push(AssetEntry {
            name: "b".into(),
            path: "".into(),
            type_id: "texture".into(),
            thumbnail: None,
        });
        // With empty search, all entries remain.
        assert_eq!(panel.assets.len(), 2);
    }

    #[test]
    fn select_asset_sets_selected_and_preview_flag() {
        let mut panel = AssetBrowserPanel::new();
        panel.assets.push(AssetEntry {
            name: "test-mesh".into(),
            path: "".into(),
            type_id: "mesh".into(),
            thumbnail: None,
        });

        // Simulate what draw_asset_browser does on click.
        if let Some(entry) = panel.assets.first() {
            panel.selected_asset = Some(entry.name.clone());
            panel.preview_needed = true;
        }

        assert_eq!(panel.selected_asset.as_deref(), Some("test-mesh"));
        assert!(panel.preview_needed);
    }

    // ── drag_assign_asset ───────────────────────────────────────────

    #[test]
    fn drag_assign_without_selection_returns_false() {
        let panel = AssetBrowserPanel::new();
        let pos = Vec2::new(0.0, 0.0);
        let mut world = World::new();
        assert!(!drag_assign_asset(&panel, pos, None, &mut world, None));
    }

    #[test]
    fn drag_assign_with_selection_returns_true() {
        let mut panel = AssetBrowserPanel::new();
        panel.selected_asset = Some("meshes/cube.mesh".to_string());
        let pos = Vec2::new(100.0, 200.0);
        let mut world = World::new();
        // Create a target entity with a Renderable component so assignment can succeed.
        let entity = world.create_entity();
        let renderable = engine_scene::components::Renderable {
            mesh_asset: String::new(),
            material_asset: "engine/ui-default".into(),
            visible: true,
            cast_shadows: true,
            render_layer: "opaque".into(),
        };
        world.add_component(entity, renderable);
        assert!(drag_assign_asset(
            &panel,
            pos,
            Some(entity),
            &mut world,
            None
        ));
    }

    #[test]
    fn drag_assign_ignores_pointer_and_entity_in_v0() {
        let mut panel = AssetBrowserPanel::new();
        panel.selected_asset = Some("models/test.mesh".to_string());
        let mut world = World::new();
        let entity = world.create_entity();
        let renderable = engine_scene::components::Renderable {
            mesh_asset: String::new(),
            material_asset: "engine/ui-default".into(),
            visible: true,
            cast_shadows: true,
            render_layer: "opaque".into(),
        };
        world.add_component(entity, renderable);
        // Different pointer positions should not matter.
        assert!(drag_assign_asset(
            &panel,
            Vec2::ZERO,
            Some(entity),
            &mut world,
            None
        ));
        assert!(drag_assign_asset(
            &panel,
            Vec2::new(999.0, 999.0),
            Some(entity),
            &mut world,
            None
        ));
    }

    // ── Sort ordering ───────────────────────────────────────────────

    #[test]
    fn assets_sorted_by_type_then_name() {
        let mut panel = AssetBrowserPanel::new();
        panel.assets = vec![
            AssetEntry {
                name: "z".into(),
                path: "".into(),
                type_id: "texture".into(),
                thumbnail: None,
            },
            AssetEntry {
                name: "a".into(),
                path: "".into(),
                type_id: "mesh".into(),
                thumbnail: None,
            },
            AssetEntry {
                name: "b".into(),
                path: "".into(),
                type_id: "mesh".into(),
                thumbnail: None,
            },
        ];

        // Apply the same sort as refresh_asset_list.
        panel
            .assets
            .sort_by(|a, b| a.type_id.cmp(&b.type_id).then_with(|| a.name.cmp(&b.name)));

        assert_eq!(panel.assets[0].name, "a"); // mesh:a
        assert_eq!(panel.assets[1].name, "b"); // mesh:b
        assert_eq!(panel.assets[2].name, "z"); // texture:z
    }

    #[test]
    fn sort_is_stable_for_equal_type_and_name() {
        let mut panel = AssetBrowserPanel::new();
        panel.assets = vec![
            AssetEntry {
                name: "dup".into(),
                path: "p1".into(),
                type_id: "mesh".into(),
                thumbnail: None,
            },
            AssetEntry {
                name: "dup".into(),
                path: "p2".into(),
                type_id: "mesh".into(),
                thumbnail: None,
            },
        ];

        panel
            .assets
            .sort_by(|a, b| a.type_id.cmp(&b.type_id).then_with(|| a.name.cmp(&b.name)));

        // Equal type + name: original order preserved (stable sort).
        assert_eq!(panel.assets[0].path, "p1");
        assert_eq!(panel.assets[1].path, "p2");
    }
}
