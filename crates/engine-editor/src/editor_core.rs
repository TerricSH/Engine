use std::collections::BTreeMap;
use std::path::PathBuf;

use engine_serialize::PersistentId;
use serde::{Deserialize, Serialize};
use tracing;

use crate::editor_ui::EditorUi;
use crate::panels::EditorPanel;
use crate::plugin::EditorPluginRegistry;

// -------------------------------------------------------------------
// Editor – main editor orchestrator
// -------------------------------------------------------------------

/// Serialisable layout state for a single panel.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PanelLayoutState {
    pub name: String,
    pub visible: bool,
    pub dock_zone: String,
    pub order: usize,
}

/// Main editor orchestrator.
///
/// Owns a list of dockable [`EditorPanel`]s, tracks the currently
/// selected entity, and drives per-frame update and rendering.
pub struct Editor {
    panels: Vec<Box<dyn EditorPanel>>,
    selected_entity: Option<PersistentId>,
    /// Persisted layout state keyed by panel name.
    layout_state: BTreeMap<String, PanelLayoutState>,
    /// Optional path for saving/loading layout.
    layout_path: Option<PathBuf>,
}

impl Editor {
    /// Create an empty editor with no panels registered.
    pub fn new() -> Self {
        Self {
            panels: Vec::new(),
            selected_entity: None,
            layout_state: BTreeMap::new(),
            layout_path: None,
        }
    }

    /// Register a panel.
    pub fn add_panel(&mut self, panel: Box<dyn EditorPanel>) {
        tracing::info!(name = %panel.name(), "Editor: panel added");
        self.panels.push(panel);
    }

    /// Look up a registered panel by name.
    pub fn panel_mut(&mut self, name: &str) -> Option<&mut dyn EditorPanel> {
        for panel in &mut self.panels {
            if panel.name() == name {
                return Some(panel.as_mut());
            }
        }
        None
    }

    /// Advance all panels by `dt` seconds.
    ///
    /// Called once per frame before [`Editor::render`].
    pub fn update(&mut self, _dt: f32) {
        // Per-frame update hook for panels that need time-driven logic.
        for panel in &mut self.panels {
            let _ = panel.name();
        }
    }

    /// Render all visible panels.
    ///
    /// Each panel receives a fresh [`EditorUi`] context.
    pub fn render(&mut self, ui: &mut EditorUi) {
        for panel in &mut self.panels {
            if panel.visible() {
                ui.reset();
                panel.ui(ui);
            }
        }
    }

    /// The currently selected entity ID, if any.
    pub fn selected_entity(&self) -> Option<PersistentId> {
        self.selected_entity.clone()
    }

    /// Set or clear the selected entity.
    pub fn set_selected_entity(&mut self, entity: Option<PersistentId>) {
        tracing::debug!(
            old = ?self.selected_entity,
            new = ?entity,
            "Editor: selection changed"
        );
        self.selected_entity = entity;
    }

    /// Persist current panel layout to a JSON file at the given path.
    pub fn save_layout(&self, path: impl Into<PathBuf>) -> Result<(), String> {
        let path: PathBuf = path.into();
        let json = serde_json::to_string_pretty(&self.layout_state)
            .map_err(|e| format!("failed to serialize layout: {e}"))?;
        std::fs::write(&path, &json)
            .map_err(|e| format!("failed to write layout to {}: {e}", path.display()))?;
        tracing::info!(path = %path.display(), "Editor layout saved");
        Ok(())
    }

    /// Load panel layout from a JSON file.
    pub fn restore_layout(&mut self, path: impl Into<PathBuf>) -> Result<(), String> {
        let path: PathBuf = path.into();
        if !path.exists() {
            return Err(format!("layout file not found: {}", path.display()));
        }
        let json =
            std::fs::read_to_string(&path).map_err(|e| format!("failed to read layout: {e}"))?;
        let state: BTreeMap<String, PanelLayoutState> =
            serde_json::from_str(&json).map_err(|e| format!("failed to parse layout: {e}"))?;
        self.layout_state = state;
        tracing::info!(path = %path.display(), "Editor layout restored");
        Ok(())
    }

    /// Load panels and inspectors from an [`EditorPluginRegistry`].
    ///
    /// This is the primary integration path for external editor extensions.
    /// Call this during editor initialisation after the engine subsystems
    /// have been set up but before entering the main loop.
    pub fn load_plugins(&mut self, registry: &EditorPluginRegistry) {
        for panel in registry.create_panels() {
            self.add_panel(panel);
        }
        tracing::info!(count = registry.iter().count(), "Editor: plugins loaded");
    }

    /// Update `layout_state` from current panel visibility/order.
    /// Call this after panels are registered.
    pub fn sync_layout_from_panels(&mut self) {
        for (i, panel) in self.panels.iter().enumerate() {
            let name = panel.name().to_string();
            self.layout_state.insert(
                name.clone(),
                PanelLayoutState {
                    name,
                    visible: panel.visible(),
                    dock_zone: "main".into(),
                    order: i,
                },
            );
        }
    }

    /// Apply the persisted layout state to registered panels.
    pub fn apply_layout(&mut self) {
        for panel in &mut self.panels {
            if let Some(state) = self.layout_state.get(panel.name()) {
                panel.set_visible(state.visible);
            }
        }
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
