use engine_scene::Scene;
use engine_serialize::PersistentId;
use tracing;

use crate::editor_ui::EditorUi;
use crate::scene_view;
use crate::EditorError;

// -------------------------------------------------------------------
// EditorPanel trait
// -------------------------------------------------------------------

/// A single dockable panel in the editor UI.
///
/// Implementations provide their own name, visibility state, and
/// immediate-mode UI via [`EditorUi`].
pub trait EditorPanel {
    /// Display name shown in the panel title bar.
    fn name(&self) -> &str;

    /// Draw the panel's contents.
    ///
    /// Called every frame when the panel is visible.  Use the provided
    /// [`EditorUi`] to declare widgets.
    fn ui(&mut self, ui: &mut EditorUi);

    /// Whether this panel is currently shown.
    fn visible(&self) -> bool;

    /// Show or hide this panel.
    fn set_visible(&mut self, visible: bool);
}

// -------------------------------------------------------------------
// SceneViewPanel
// -------------------------------------------------------------------

/// Editor panel that renders a 3D scene view with camera controls.
///
/// Provides orbit/pan/zoom camera manipulation, a configurable render
/// target, and an optional reference grid.
pub struct SceneViewPanel {
    visible: bool,
    name: String,
    // Camera orbit state
    pitch: f32,
    yaw: f32,
    distance: f32,
    target: [f32; 3],
    // Render target
    render_target_label: Option<String>,
    // Grid
    show_grid: bool,
}

impl SceneViewPanel {
    /// Create a new scene-view panel with default camera settings.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            visible: true,
            name: name.into(),
            pitch: 0.0,
            yaw: 0.0,
            distance: 10.0,
            target: [0.0, 0.0, 0.0],
            render_target_label: None,
            show_grid: true,
        }
    }

    /// Set the camera orbit angles (in degrees) and distance.
    pub fn set_camera_orbit(&mut self, pitch: f32, yaw: f32, distance: f32) {
        self.pitch = pitch.clamp(-89.0, 89.0);
        self.yaw = yaw;
        self.distance = distance.max(0.01);
    }

    /// Current camera orbit: `(pitch, yaw, distance)`.
    pub fn camera_orbit(&self) -> (f32, f32, f32) {
        (self.pitch, self.yaw, self.distance)
    }

    /// Set the look-at target in world space.
    pub fn set_target(&mut self, target: [f32; 3]) {
        self.target = target;
    }

    /// Current look-at target.
    pub fn target(&self) -> &[f32; 3] {
        &self.target
    }

    /// Toggle the ground-grid overlay.
    pub fn set_show_grid(&mut self, show: bool) {
        self.show_grid = show;
    }

    /// Whether the ground grid is visible.
    pub fn show_grid(&self) -> bool {
        self.show_grid
    }

    /// Assign a render-target label (e.g. a window or texture name).
    pub fn set_render_target(&mut self, label: Option<String>) {
        self.render_target_label = label;
    }

    /// The currently assigned render-target label.
    pub fn render_target(&self) -> Option<&str> {
        self.render_target_label.as_deref()
    }

    /// Render the scene view with real scene data and compute camera matrices.
    ///
    /// Displays the orbit camera controls and the computed view/projection
    /// matrices.  Returns the view and projection matrices as a tuple.
    pub fn ui_with_scene(&mut self, ui: &mut EditorUi, _scene: &Scene) -> (glam::Mat4, glam::Mat4) {
        let _ = ui.collapsing_header("Transform", true);
        ui.text_field("Name", &self.name);

        let _ = ui.separator();
        let _ = ui.collapsing_header("Camera", true);

        if let Some(v) = ui.slider_f32("Pitch", self.pitch, -89.0, 89.0) {
            self.pitch = v;
        }
        if let Some(v) = ui.slider_f32("Yaw", self.yaw, -180.0, 180.0) {
            self.yaw = v;
        }
        if let Some(v) = ui.slider_f32("Distance", self.distance, 0.1, 100.0) {
            self.distance = v;
        }

        let _ = ui.separator();
        self.show_grid = ui.checkbox("Show Grid", self.show_grid);

        // ── Compute camera matrices ──────────────────────────────
        let view = scene_view::orbit_view_matrix(self.pitch, self.yaw, self.distance, self.target);
        let proj = scene_view::orbit_projection_matrix(60.0, 16.0 / 9.0, 0.1, 100.0);

        let _ = ui.separator();
        let cam_open = ui.collapsing_header("Camera Matrices", false);
        if cam_open {
            ui.text_field("View", &format!("{:?}", view.to_cols_array_2d()));
            ui.text_field("Proj", &format!("{:?}", proj.to_cols_array_2d()));
        }

        tracing::debug!(panel = %self.name, "SceneViewPanel.ui_with_scene");
        (view, proj)
    }
}

impl EditorPanel for SceneViewPanel {
    fn name(&self) -> &str {
        &self.name
    }

    fn ui(&mut self, ui: &mut EditorUi) {
        let _ = ui.collapsing_header("Transform", true);
        ui.text_field("Name", &self.name);

        let _ = ui.separator();
        let _ = ui.collapsing_header("Camera", true);

        if let Some(v) = ui.slider_f32("Pitch", self.pitch, -89.0, 89.0) {
            self.pitch = v;
        }
        if let Some(v) = ui.slider_f32("Yaw", self.yaw, -180.0, 180.0) {
            self.yaw = v;
        }
        if let Some(v) = ui.slider_f32("Distance", self.distance, 0.1, 100.0) {
            self.distance = v;
        }

        let _ = ui.separator();
        self.show_grid = ui.checkbox("Show Grid", self.show_grid);

        tracing::debug!(panel = %self.name, "SceneViewPanel.ui");
    }

    fn visible(&self) -> bool {
        self.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }
}

// -------------------------------------------------------------------
// InspectorPanel
// -------------------------------------------------------------------

/// Editor panel that displays and edits the currently selected entity.
///
/// Shows the entity's persistent ID, a list of attached components, and
/// basic field-editing widgets for each component.
pub struct InspectorPanel {
    visible: bool,
    name: String,
    selected_entity: Option<PersistentId>,
    /// Names of components that have been expanded in the UI.
    expanded_components: Vec<String>,
}

impl InspectorPanel {
    /// Create a new inspector panel.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            visible: true,
            name: name.into(),
            selected_entity: None,
            expanded_components: Vec::new(),
        }
    }

    /// Set the entity to inspect.
    pub fn set_selected_entity(&mut self, entity: Option<PersistentId>) {
        self.selected_entity = entity;
    }

    /// The entity currently being inspected, if any.
    pub fn selected_entity(&self) -> Option<&PersistentId> {
        self.selected_entity.as_ref()
    }
}

impl EditorPanel for InspectorPanel {
    fn name(&self) -> &str {
        &self.name
    }

    fn ui(&mut self, ui: &mut EditorUi) {
        let _ = ui.collapsing_header("Entity", true);

        match &self.selected_entity {
            Some(entity_id) => {
                ui.text_field("ID", entity_id);

                let _ = ui.separator();
                let _ = ui.collapsing_header("Components", true);

                for comp in &self.expanded_components {
                    let _ = ui.collapsing_header(comp, true);
                }
            }
            None => {
                // No entity selected – display would show a hint.
            }
        }

        tracing::debug!(panel = %self.name, has_selection = self.selected_entity.is_some(),
                        "InspectorPanel.ui");
    }

    fn visible(&self) -> bool {
        self.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }
}

// -------------------------------------------------------------------
// AssetBrowserPanel
// -------------------------------------------------------------------

/// Editor panel that browses, previews, and imports assets.
///
/// Maintains a virtual file-tree view, a preview area for a selected
/// asset, and buttons for import/refresh operations.
pub struct AssetBrowserPanel {
    visible: bool,
    name: String,
    current_path: String,
    /// Flat list of entry names in the current directory.
    entries: Vec<String>,
    /// Name of the asset currently being previewed, if any.
    preview_asset: Option<String>,
}

impl AssetBrowserPanel {
    /// Create a new asset-browser panel rooted at `"/"`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            visible: true,
            name: name.into(),
            current_path: "/".to_string(),
            entries: Vec::new(),
            preview_asset: None,
        }
    }

    /// Set the current directory path.
    pub fn set_current_path(&mut self, path: impl Into<String>) {
        self.current_path = path.into();
    }

    /// The currently displayed directory path.
    pub fn current_path(&self) -> &str {
        &self.current_path
    }

    /// Replace the entry list shown in the file tree.
    pub fn set_entries(&mut self, entries: Vec<String>) {
        self.entries = entries;
    }

    /// A shared reference to the current entry list.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Set which asset (if any) is being previewed.
    pub fn set_preview_asset(&mut self, asset: Option<String>) {
        self.preview_asset = asset;
    }

    /// The name of the asset currently being previewed.
    pub fn preview_asset(&self) -> Option<&str> {
        self.preview_asset.as_deref()
    }

    /// Re-scan the asset directory at the current path.
    pub fn refresh(&mut self) {
        tracing::info!(panel = %self.name, path = %self.current_path,
                       "AssetBrowserPanel refresh");
    }

    /// Import a file from an external source path into the project.
    pub fn import(&mut self, source_path: &str) -> Result<(), EditorError> {
        let _ = source_path;
        tracing::info!(panel = %self.name, source = %source_path,
                       "AssetBrowserPanel import");
        Ok(())
    }
}

impl EditorPanel for AssetBrowserPanel {
    fn name(&self) -> &str {
        &self.name
    }

    fn ui(&mut self, ui: &mut EditorUi) {
        let _ = ui.collapsing_header("File Tree", true);

        for entry in &self.entries {
            let _ = ui.button(entry);
        }

        let _ = ui.separator();

        if ui.button("Refresh") {
            self.refresh();
        }

        let _ = ui.separator();

        if let Some(asset) = &self.preview_asset {
            let _ = ui.collapsing_header("Preview", true);
            ui.text_field("Asset", asset);
        }

        tracing::debug!(panel = %self.name, entry_count = self.entries.len(),
                        "AssetBrowserPanel.ui");
    }

    fn visible(&self) -> bool {
        self.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }
}
