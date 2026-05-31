//! Gate 14 — Gameplay Archetypes (G14-F06).
//!
//! Archetypes are named gameplay categories that map to a specific prefab
//! asset.  They provide a high-level abstraction for spawning common entity
//! types: "player", "enemy", "pickup", "prop", "camera_rig", "trigger".

use serde::{Deserialize, Serialize};

// ── Archetype ──────────────────────────────────────────────────────────────

/// A named gameplay archetype that maps to a prefab asset.
///
/// Archetypes are the primary way for designers to express *what* an entity
/// is in gameplay terms, independently of *how* it is constructed (which is
/// defined by the prefab).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Archetype {
    /// Unique name for this archetype (e.g. `"player"`, `"enemy_grunt"`).
    pub name: String,
    /// Asset identifier of the prefab to instantiate.
    pub prefab_asset: String,
    /// Free-form tags for filtering and categorisation.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Archetype {
    pub fn new(name: impl Into<String>, prefab_asset: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            prefab_asset: prefab_asset.into(),
            tags: Vec::new(),
        }
    }

    /// Add a tag to this archetype (builder pattern).
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
}

// ── ArchetypeRegistry ──────────────────────────────────────────────────────

/// A registry of named gameplay archetypes.
///
/// Archetypes are registered once at startup (or from asset metadata) and
/// are then available for lookup by name or tag.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ArchetypeRegistry {
    entries: Vec<Archetype>,
}

impl ArchetypeRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            entries: Vec::new(),
        };
        reg.register_builtins();
        reg
    }

    /// Register a new archetype.
    ///
    /// If an archetype with the same `name` already exists, it is replaced.
    pub fn register(&mut self, archetype: Archetype) {
        // Replace existing entry with the same name, if any.
        if let Some(existing) = self.entries.iter_mut().find(|e| e.name == archetype.name) {
            *existing = archetype;
        } else {
            self.entries.push(archetype);
        }
    }

    /// Resolve an archetype by name.
    pub fn resolve(&self, name: &str) -> Option<&Archetype> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// List all archetypes that have the given tag.
    pub fn list_by_tag(&self, tag: &str) -> Vec<&Archetype> {
        self.entries
            .iter()
            .filter(|e| e.tags.iter().any(|t| t == tag))
            .collect()
    }

    /// List all registered archetypes.
    pub fn all(&self) -> &[Archetype] {
        &self.entries
    }

    /// Number of registered archetypes.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no archetypes are registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // ── Built-in archetypes ───────────────────────────────────────────

    /// Register the six built-in gameplay archetypes.
    fn register_builtins(&mut self) {
        let builtins = vec![
            Archetype::new("player", "prefabs/player.prefab")
                .with_tag("gameplay")
                .with_tag("controllable"),
            Archetype::new("enemy", "prefabs/enemy.prefab")
                .with_tag("gameplay")
                .with_tag("damageable")
                .with_tag("ai"),
            Archetype::new("pickup", "prefabs/pickup.prefab")
                .with_tag("gameplay")
                .with_tag("collectible"),
            Archetype::new("prop", "prefabs/prop.prefab")
                .with_tag("decor")
                .with_tag("static"),
            Archetype::new("camera_rig", "prefabs/camera_rig.prefab")
                .with_tag("camera")
                .with_tag("gameplay"),
            Archetype::new("trigger", "prefabs/trigger.prefab")
                .with_tag("gameplay")
                .with_tag("interactable"),
        ];

        for archetype in builtins {
            // Builtins use a manual push because they are known to be unique.
            self.entries.push(archetype);
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archetype_new_and_tags() {
        let arch = Archetype::new("custom_boss", "prefabs/boss.prefab").with_tag("boss");
        assert_eq!(arch.name, "custom_boss");
        assert_eq!(arch.prefab_asset, "prefabs/boss.prefab");
        assert!(arch.tags.contains(&"boss".to_string()));
    }

    #[test]
    fn archetype_registry_register_and_resolve() {
        let mut reg = ArchetypeRegistry::new();
        // After new(), builtins are registered.
        assert!(reg.resolve("player").is_some());
        assert!(reg.resolve("enemy").is_some());
        assert!(reg.resolve("pickup").is_some());
        assert!(reg.resolve("prop").is_some());
        assert!(reg.resolve("camera_rig").is_some());
        assert!(reg.resolve("trigger").is_some());

        // Register a custom archetype.
        reg.register(Archetype::new("custom_boss", "prefabs/boss.prefab").with_tag("boss"));
        let resolved = reg.resolve("custom_boss").expect("should resolve");
        assert_eq!(resolved.prefab_asset, "prefabs/boss.prefab");
    }

    #[test]
    fn archetype_registry_resolve_nonexistent() {
        let reg = ArchetypeRegistry::new();
        assert!(reg.resolve("nonexistent").is_none());
    }

    #[test]
    fn archetype_registry_list_by_tag() {
        let reg = ArchetypeRegistry::new();

        let gameplay = reg.list_by_tag("gameplay");
        assert!(!gameplay.is_empty());
        assert!(gameplay.iter().any(|a| a.name == "player"));
        assert!(gameplay.iter().any(|a| a.name == "enemy"));

        let decor = reg.list_by_tag("decor");
        assert_eq!(decor.len(), 1);
        assert_eq!(decor[0].name, "prop");
    }

    #[test]
    fn archetype_registry_replace_existing() {
        let mut reg = ArchetypeRegistry::new();
        reg.register(Archetype::new("player", "prefabs/custom_player.prefab"));
        let resolved = reg.resolve("player").unwrap();
        assert_eq!(resolved.prefab_asset, "prefabs/custom_player.prefab");
    }

    #[test]
    fn archetype_registry_all_and_count() {
        let reg = ArchetypeRegistry::new();
        assert_eq!(reg.len(), 6); // 6 builtins
        assert!(!reg.is_empty());
    }

    #[test]
    fn archetype_no_builtins_when_empty_constructed() {
        // ArchetypeRegistry::new() auto-registers builtins, but we can
        // construct an empty one manually.
        let empty: ArchetypeRegistry = ArchetypeRegistry {
            entries: Vec::new(),
        };
        assert!(empty.is_empty());
    }

    #[test]
    fn archetype_list_by_tag_empty_when_no_match() {
        let reg = ArchetypeRegistry::new();
        let result = reg.list_by_tag("nonexistent_tag");
        assert!(result.is_empty());
    }
}
