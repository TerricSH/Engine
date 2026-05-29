use std::collections::BTreeMap;

use engine_serialize::Value;

use crate::Command;
use crate::EditorPanel;
use crate::EditorUi;

// ---------------------------------------------------------------------------
// EditorPluginMeta
// ---------------------------------------------------------------------------

/// Metadata for an editor plugin.
#[derive(Clone, Debug)]
pub struct EditorPluginMeta {
    /// Human-readable plugin name.
    pub name: &'static str,
    /// Plugin version string (semver recommended).
    pub version: &'static str,
    /// Names of plugins this plugin depends on.
    pub dependencies: Vec<&'static str>,
}

// ---------------------------------------------------------------------------
// PanelFactory
// ---------------------------------------------------------------------------

/// A factory that creates editor panels.
pub type PanelFactory = fn() -> Box<dyn EditorPanel>;

// ---------------------------------------------------------------------------
// ComponentInspector trait
// ---------------------------------------------------------------------------

/// Inspector for a specific component type.
///
/// Implementations inspect and mutate the fields of a single component kind
/// using the provided [`EditorUi`] and return zero or more [`Command`]s that
/// should be applied when the user interacts.
pub trait ComponentInspector: Send {
    /// The unique type identifier this inspector handles (e.g.
    /// `"engine.renderable"`).
    fn type_id(&self) -> &'static str;

    /// Draw UI for the component's fields.
    ///
    /// * `ui`      – immediate-mode UI context.
    /// * `fields`  – current field values of the component (mutable so the
    ///   inspector can apply in-place edits that can be committed as commands).
    ///
    /// Returns any [`Command`]s produced by the interaction.
    fn ui(
        &mut self,
        ui: &mut EditorUi,
        fields: &mut BTreeMap<String, Value>,
    ) -> Vec<Box<dyn Command>>;
}

// ---------------------------------------------------------------------------
// EditorPlugin
// ---------------------------------------------------------------------------

/// A registered editor plugin.
///
/// Bundles metadata, panel factories, and component inspectors into a single
/// unit that can be registered with [`EditorPluginRegistry`].
pub struct EditorPlugin {
    /// Descriptive metadata for this plugin.
    pub meta: EditorPluginMeta,
    /// Panel factories keyed by display name.
    pub panels: Vec<(&'static str, PanelFactory)>,
    /// Component inspectors provided by this plugin.
    pub inspectors: Vec<Box<dyn ComponentInspector>>,
}

// ---------------------------------------------------------------------------
// EditorPluginRegistry
// ---------------------------------------------------------------------------

/// Registry for editor plugins.
///
/// Collects [`EditorPlugin`]s and provides indexed access to their panels
/// and component inspectors.  This is the primary integration point for
/// external editor extensions.
pub struct EditorPluginRegistry {
    plugins: Vec<EditorPlugin>,
}

impl EditorPluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register an [`EditorPlugin`].
    ///
    /// The plugin is appended to the internal list.  Duplicate registration
    /// (same plugin by pointer identity) is not prevented – the same plugin
    /// may be registered multiple times.
    pub fn register(&mut self, plugin: EditorPlugin) {
        self.plugins.push(plugin);
    }

    /// Create all registered panel factories into concrete panel objects.
    ///
    /// Returns one panel per registered factory, in registration order.
    pub fn create_panels(&self) -> Vec<Box<dyn EditorPanel>> {
        self.plugins
            .iter()
            .flat_map(|p| &p.panels)
            .map(|(_name, factory)| factory())
            .collect()
    }

    /// Find a [`ComponentInspector`] that handles the given `type_id`.
    ///
    /// Returns the first matching inspector across all registered plugins, or
    /// `None` if no inspector handles this type.
    pub fn inspector_for(&self, type_id: &str) -> Option<&dyn ComponentInspector> {
        for plugin in &self.plugins {
            for inspector in &plugin.inspectors {
                if inspector.type_id() == type_id {
                    return Some(inspector.as_ref());
                }
            }
        }
        None
    }

    /// Iterate over all registered plugins.
    pub fn iter(&self) -> impl Iterator<Item = &EditorPlugin> {
        self.plugins.iter()
    }
}

impl Default for EditorPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Dummy helpers for testing ────────────────────────────────────────

    struct DummyPanel {
        name: String,
        visible: bool,
    }

    impl DummyPanel {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                visible: true,
            }
        }
    }

    impl EditorPanel for DummyPanel {
        fn name(&self) -> &str {
            &self.name
        }
        fn ui(&mut self, _ui: &mut EditorUi) {}
        fn visible(&self) -> bool {
            self.visible
        }
        fn set_visible(&mut self, visible: bool) {
            self.visible = visible;
        }
    }

    fn dummy_panel_factory() -> Box<dyn EditorPanel> {
        Box::new(DummyPanel::new("Dummy Panel"))
    }

    struct DummyInspector;

    impl ComponentInspector for DummyInspector {
        fn type_id(&self) -> &'static str {
            "test.dummy"
        }
        fn ui(
            &mut self,
            _ui: &mut EditorUi,
            _fields: &mut BTreeMap<String, Value>,
        ) -> Vec<Box<dyn Command>> {
            vec![]
        }
    }

    fn dummy_plugin() -> EditorPlugin {
        EditorPlugin {
            meta: EditorPluginMeta {
                name: "test-plugin",
                version: "0.1.0",
                dependencies: vec![],
            },
            panels: vec![("Dummy Panel", dummy_panel_factory as PanelFactory)],
            inspectors: vec![Box::new(DummyInspector)],
        }
    }

    // ── Registry tests ───────────────────────────────────────────────────

    #[test]
    fn registry_new_is_empty() {
        let reg = EditorPluginRegistry::new();
        assert_eq!(reg.iter().count(), 0);
    }

    #[test]
    fn registry_register_increases_count() {
        let mut reg = EditorPluginRegistry::new();
        reg.register(dummy_plugin());
        assert_eq!(reg.iter().count(), 1);
    }

    #[test]
    fn registry_register_multiple_plugins() {
        let mut reg = EditorPluginRegistry::new();
        reg.register(dummy_plugin());
        reg.register(dummy_plugin());
        assert_eq!(reg.iter().count(), 2);
    }

    #[test]
    fn registry_default_is_empty() {
        let reg = EditorPluginRegistry::default();
        assert_eq!(reg.iter().count(), 0);
    }

    #[test]
    fn create_panels_returns_registered_panels() {
        let mut reg = EditorPluginRegistry::new();
        reg.register(dummy_plugin());
        let panels: Vec<Box<dyn EditorPanel>> = reg.create_panels();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].name(), "Dummy Panel");
    }

    #[test]
    fn create_panels_multiple_plugins() {
        let mut reg = EditorPluginRegistry::new();
        reg.register(dummy_plugin());
        reg.register(dummy_plugin());
        assert_eq!(reg.create_panels().len(), 2);
    }

    #[test]
    fn create_panels_empty_registry() {
        let reg = EditorPluginRegistry::new();
        assert!(reg.create_panels().is_empty());
    }

    #[test]
    fn inspector_for_returns_registered_inspector() {
        let mut reg = EditorPluginRegistry::new();
        reg.register(dummy_plugin());
        let inspector = reg.inspector_for("test.dummy");
        assert!(inspector.is_some());
        assert_eq!(inspector.unwrap().type_id(), "test.dummy");
    }

    #[test]
    fn inspector_for_unknown_type_returns_none() {
        let mut reg = EditorPluginRegistry::new();
        reg.register(dummy_plugin());
        assert!(reg.inspector_for("nonexistent.type").is_none());
    }

    #[test]
    fn inspector_for_empty_registry() {
        let reg = EditorPluginRegistry::new();
        assert!(reg.inspector_for("anything").is_none());
    }

    #[test]
    fn iter_returns_all_plugins() {
        let mut reg = EditorPluginRegistry::new();
        reg.register(dummy_plugin());
        let collected: Vec<&EditorPlugin> = reg.iter().collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].meta.name, "test-plugin");
    }

    // ── Dummy inspector tests ────────────────────────────────────────────

    #[test]
    fn dummy_inspector_type_id() {
        let insp = DummyInspector;
        assert_eq!(insp.type_id(), "test.dummy");
    }

    #[test]
    fn dummy_inspector_ui_returns_empty() {
        let mut insp = DummyInspector;
        let mut ui = EditorUi::new();
        let mut fields = BTreeMap::new();
        let cmds = insp.ui(&mut ui, &mut fields);
        assert!(cmds.is_empty());
    }

    // ── Plugin metadata tests ────────────────────────────────────────────

    #[test]
    fn plugin_meta_debug_and_clone() {
        let meta = EditorPluginMeta {
            name: "a",
            version: "0.0.1",
            dependencies: vec!["b"],
        };
        let cloned = meta.clone();
        assert_eq!(format!("{:?}", cloned), format!("{:?}", meta));
    }

    #[test]
    fn dummy_plugin_has_panels_and_inspectors() {
        let plugin = dummy_plugin();
        assert_eq!(plugin.panels.len(), 1);
        assert_eq!(plugin.inspectors.len(), 1);
        assert_eq!(plugin.meta.name, "test-plugin");
        assert_eq!(plugin.meta.version, "0.1.0");
    }
}
