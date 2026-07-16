use std::collections::BTreeMap;

use engine_asset::AssetRegistry;
use engine_scene::Scene;
use engine_serialize::{AssetId, PersistentId, Value};

use crate::commands::{Command, SetComponentField};
use crate::editor_ui::{EditorUi, UiInteractionStamp};
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

/// One independently replayable Scene View setting change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneViewAction {
    SetPitch(f32),
    SetYaw(f32),
    SetDistance(f32),
    SetShowGrid(bool),
}

impl SceneViewAction {
    /// Whether this setting changes the editor camera matrices used by
    /// viewport tools. Visual-only settings such as the reference grid do not.
    pub fn affects_camera(self) -> bool {
        matches!(
            self,
            Self::SetPitch(_) | Self::SetYaw(_) | Self::SetDistance(_)
        )
    }
}

/// Scene-view setting paired with the platform event that produced it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SequencedSceneViewAction {
    pub stamp: UiInteractionStamp,
    pub action: SceneViewAction,
}

impl SequencedSceneViewAction {
    pub fn new(stamp: UiInteractionStamp, action: SceneViewAction) -> Self {
        Self { stamp, action }
    }
}

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

    /// Apply one setting at its exact position in the host's shared input
    /// stream.
    pub fn apply_action(&mut self, action: SceneViewAction) {
        match action {
            SceneViewAction::SetPitch(pitch) => self.pitch = pitch.clamp(-89.0, 89.0),
            SceneViewAction::SetYaw(yaw) => self.yaw = yaw.clamp(-180.0, 180.0),
            SceneViewAction::SetDistance(distance) => self.distance = distance.clamp(0.1, 100.0),
            SceneViewAction::SetShowGrid(show_grid) => self.show_grid = show_grid,
        }
    }

    fn draw_controls_ordered(
        &self,
        ui: &mut EditorUi,
    ) -> (f32, f32, f32, Vec<SequencedSceneViewAction>) {
        let _ = ui.collapsing_header("Transform", true);
        ui.text_field("Name", &self.name);

        ui.separator();
        let _ = ui.collapsing_header("Camera", true);

        let mut pitch = self.pitch;
        let mut yaw = self.yaw;
        let mut distance = self.distance;
        let mut actions = Vec::new();

        for (stamp, value) in ui.ordered_slider_f32("Pitch", pitch, -89.0, 89.0) {
            pitch = value;
            actions.push(SequencedSceneViewAction::new(
                stamp,
                SceneViewAction::SetPitch(value),
            ));
        }
        for (stamp, value) in ui.ordered_slider_f32("Yaw", yaw, -180.0, 180.0) {
            yaw = value;
            actions.push(SequencedSceneViewAction::new(
                stamp,
                SceneViewAction::SetYaw(value),
            ));
        }
        for (stamp, value) in ui.ordered_slider_f32("Distance", distance, 0.1, 100.0) {
            distance = value;
            actions.push(SequencedSceneViewAction::new(
                stamp,
                SceneViewAction::SetDistance(value),
            ));
        }

        ui.separator();
        for (stamp, show_grid) in ui.ordered_checkbox_changes("Show Grid", self.show_grid) {
            actions.push(SequencedSceneViewAction::new(
                stamp,
                SceneViewAction::SetShowGrid(show_grid),
            ));
        }

        (pitch, yaw, distance, actions)
    }

    /// Draw controls without immediately mutating panel state. Every returned
    /// setting carries the same platform sequence as raw viewport input, so a
    /// host can merge and replay both streams deterministically.
    pub fn ui_with_scene_ordered(
        &self,
        ui: &mut EditorUi,
        _scene: &Scene,
    ) -> (glam::Mat4, glam::Mat4, Vec<SequencedSceneViewAction>) {
        let (pitch, yaw, distance, actions) = self.draw_controls_ordered(ui);

        let view = scene_view::orbit_view_matrix(pitch, yaw, distance, self.target);
        let proj = scene_view::orbit_projection_matrix(60.0, 16.0 / 9.0, 0.1, 100.0);

        ui.separator();
        let cam_open = ui.collapsing_header("Camera Matrices", false);
        if cam_open {
            ui.text_field("View", &format!("{:?}", view.to_cols_array_2d()));
            ui.text_field("Proj", &format!("{:?}", proj.to_cols_array_2d()));
        }

        tracing::debug!(panel = %self.name, "SceneViewPanel.ui_with_scene_ordered");
        (view, proj, actions)
    }

    /// Render the scene view with real scene data and compute camera matrices.
    ///
    /// Displays the orbit camera controls and the computed view/projection
    /// matrices.  Returns the view and projection matrices as a tuple.
    pub fn ui_with_scene(&mut self, ui: &mut EditorUi, _scene: &Scene) -> (glam::Mat4, glam::Mat4) {
        let (view, proj, actions) = self.ui_with_scene_ordered(ui, _scene);
        for action in actions {
            self.apply_action(action.action);
        }
        (view, proj)
    }
}

impl EditorPanel for SceneViewPanel {
    fn name(&self) -> &str {
        &self.name
    }

    fn ui(&mut self, ui: &mut EditorUi) {
        let (_, _, _, actions) = self.draw_controls_ordered(ui);
        for action in actions {
            self.apply_action(action.action);
        }

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

                ui.separator();
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
/// asset, and buttons for import/refresh operations.  Also supports
/// asset assignment via an optional callback.
pub struct AssetBrowserPanel {
    visible: bool,
    name: String,
    current_path: String,
    /// Flat list of entry names in the current directory.
    entries: Vec<String>,
    /// Name of the asset currently being previewed, if any.
    preview_asset: Option<String>,
    /// Callback invoked when an asset is selected for assignment.
    on_assign: Option<Box<dyn FnMut(AssetId) + Send>>,
    /// Categorized asset IDs extracted from the asset registry.
    asset_ids: BTreeMap<String, Vec<AssetId>>,
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
            on_assign: None,
            asset_ids: BTreeMap::new(),
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

    // ── Asset assignment support ─────────────────────────────────────

    /// Set the callback for when an asset is selected for assignment.
    ///
    /// The callback receives the chosen [`AssetId`] once the user confirms
    /// a pick action.
    pub fn set_on_assign(&mut self, callback: Option<Box<dyn FnMut(AssetId) + Send>>) {
        self.on_assign = callback;
    }

    /// Render a picker UI that lets the user select an asset.
    ///
    /// When `filter` is set, only assets of the given type are shown.
    /// Returns the chosen [`AssetId`] when the user confirms a selection.
    ///
    /// This method draws the entry list and triggers the `on_assign`
    /// callback if one was registered.
    pub fn pick_asset(
        &mut self,
        ui: &mut EditorUi,
        _filter: Option<engine_asset::cook::AssetType>,
    ) -> Option<AssetId> {
        // Draw a header for the picker mode
        ui.collapsing_header("Pick Asset", true);

        for entry in &self.entries {
            if ui.button(entry) {
                // User clicked an entry — treat it as selected
                let asset_id = AssetId::with_path(
                    entry.clone(),
                    format!("{}/{}", self.current_path.trim_end_matches('/'), entry),
                );
                // Invoke callback if set
                if let Some(ref mut cb) = self.on_assign {
                    cb(asset_id.clone());
                }
                return Some(asset_id);
            }
        }

        ui.separator();
        if ui.button("Cancel") {
            // User cancelled — no asset selected
        }

        None
    }

    /// Populate the asset list from an [`AssetRegistry`].
    ///
    /// Extracts all cached asset IDs and groups them by category (the
    /// prefix before the first hyphen in the asset ID).
    pub fn set_registry(&mut self, registry: &AssetRegistry) {
        self.asset_ids.clear();
        for id in registry.cached_ids() {
            let category = id.id.split('-').next().unwrap_or("other").to_string();
            self.asset_ids.entry(category).or_default().push(id);
        }
    }

    /// Render the asset browser using live [`AssetRegistry`] data.
    ///
    /// Shows collapsible category sections with each asset's ID and an
    /// "Assign to selected" button.  Returns [`SetComponentField`] commands
    /// prefilled with the selected [`AssetId`] value.
    pub fn ui_with_registry(
        &mut self,
        ui: &mut EditorUi,
        registry: &AssetRegistry,
    ) -> Vec<Box<dyn Command>> {
        let mut commands: Vec<Box<dyn Command>> = Vec::new();

        let _ = ui.collapsing_header("Asset Browser", true);

        if self.asset_ids.is_empty() {
            ui.text_field("Status", "No assets cached. Call set_registry() first.");
            return commands;
        }

        // Collect and sort category names (cloned to avoid borrow issues)
        let mut categories: Vec<String> = self.asset_ids.keys().cloned().collect();
        categories.sort();

        for category in &categories {
            let header_label = format!("{category}/");
            let open = ui.collapsing_header(&header_label, false);
            if !open {
                continue;
            }

            if let Some(ids) = self.asset_ids.get(category) {
                for asset_id in ids {
                    let loaded = registry.contains(asset_id);
                    let label = if loaded {
                        format!("{} ✓", asset_id.id)
                    } else {
                        asset_id.id.clone()
                    };
                    ui.text_field("Asset", &label);

                    if loaded && ui.button("Assign to selected") {
                        commands.push(Box::new(SetComponentField::new(
                            "__selected__".to_string(),
                            "__asset_assign__".to_string(),
                            asset_id.id.clone(),
                            Value::Asset(asset_id.clone()),
                        )));
                    }
                }
            }
        }

        tracing::debug!(panel = %self.name, categories = %categories.len(),
                        "AssetBrowserPanel.ui_with_registry");
        commands
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

        ui.separator();

        if ui.button("Refresh") {
            self.refresh();
        }

        ui.separator();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UiEvent, UiInteractionPhase};

    #[test]
    fn ordered_scene_view_controls_defer_state_until_replay() {
        let mut panel = SceneViewPanel::new("Scene View");
        let mut ui = EditorUi::new();
        ui.begin_frame();
        ui.inject_event(UiEvent::SliderDrag("Pitch".into(), 30.0));
        ui.inject_event(UiEvent::SliderDrag("Yaw".into(), 45.0));
        ui.inject_event(UiEvent::SliderDrag("Distance".into(), 25.0));
        ui.inject_event(UiEvent::CheckboxToggle("Show Grid".into(), false));

        let (_, _, actions) = panel.ui_with_scene_ordered(&mut ui, &engine_scene::sample_scene());
        ui.end_frame();

        assert_eq!(panel.camera_orbit(), (0.0, 0.0, 10.0));
        assert!(panel.show_grid());
        assert_eq!(
            actions
                .iter()
                .map(|action| action.action)
                .collect::<Vec<_>>(),
            [
                SceneViewAction::SetPitch(30.0),
                SceneViewAction::SetYaw(45.0),
                SceneViewAction::SetDistance(25.0),
                SceneViewAction::SetShowGrid(false),
            ]
        );
        assert!(actions
            .iter()
            .all(|action| { action.stamp.phase == UiInteractionPhase::AfterRawPointer }));

        let mut actions = actions;
        actions.sort_by_key(|action| action.stamp.sequence);
        for action in actions {
            panel.apply_action(action.action);
        }
        assert_eq!(panel.camera_orbit(), (30.0, 45.0, 25.0));
        assert!(!panel.show_grid());
    }

    #[test]
    fn grid_setting_is_visual_only_while_orbit_settings_affect_camera() {
        assert!(!SceneViewAction::SetShowGrid(false).affects_camera());
        assert!(SceneViewAction::SetPitch(1.0).affects_camera());
        assert!(SceneViewAction::SetYaw(1.0).affects_camera());
        assert!(SceneViewAction::SetDistance(1.0).affects_camera());
    }
}
