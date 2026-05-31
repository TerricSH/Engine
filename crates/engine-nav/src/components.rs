//! AI Agent ECS component — bridges pathfinding with [`CharacterController`].

use std::collections::BTreeMap;

use engine_character::{CharacterCommand, CharacterController};
use engine_scene::Component;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::agent::NavAgent;
use crate::navmesh::NavMesh;
use crate::pathfinding::Pathfinder;

// ---------------------------------------------------------------------------
// AiAgent
// ---------------------------------------------------------------------------

/// ECS component that gives an entity autonomous pathfinding behaviour.
///
/// Each frame [`update_ai_agent`] computes a path from the entity's current
/// position to its [`target`](Self::target), feeds the path to an internal
/// [`NavAgent`] for waypoint following, and pushes a [`CharacterCommand`]
/// to the entity's [`CharacterController`].
///
/// # TYPE_ID
///
/// `"engine.nav_agent"`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiAgent {
    /// Optional name or identifier of the navmesh to use.
    pub navmesh_ref: Option<String>,
    /// Agent radius used for navmesh queries.
    pub agent_radius: f32,
    /// Agent height used for navmesh queries.
    pub agent_height: f32,
    /// Movement speed (m/s).
    pub speed: f32,
    /// Distance from the final waypoint at which the agent considers itself
    /// arrived (m).
    pub stopping_distance: f32,
    /// The current abstract target the agent is moving toward.
    pub target: Option<Vec3>,
    /// Entity ID of the [`CharacterController`] this agent drives.
    pub controller_entity_id: u64,

    /// Internal path-following agent (not serialized — rebuilt at runtime).
    #[serde(skip)]
    pub(crate) nav_agent: NavAgent,
}

impl Component for AiAgent {
    const TYPE_ID: &'static str = "engine.nav_agent";
}

impl AiAgent {
    /// Create a new `AiAgent` with default parameters.
    pub fn new() -> Self {
        Self {
            navmesh_ref: None,
            agent_radius: 0.3,
            agent_height: 1.8,
            speed: 5.0,
            stopping_distance: 0.5,
            target: None,
            controller_entity_id: 0,
            nav_agent: NavAgent::new(),
        }
    }

    /// Create a new `AiAgent` that targets a specific entity.
    pub fn with_target(entity_id: u64, target: Vec3) -> Self {
        let mut agent = Self::new();
        agent.controller_entity_id = entity_id;
        agent.target = Some(target);
        agent
    }
}

impl Default for AiAgent {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Serialization hooks (for ComponentRegistry)
// ---------------------------------------------------------------------------

/// Serialize an `AiAgent` component into a field map.
pub fn serialize_ai_agent(component: &dyn std::any::Any) -> BTreeMap<String, engine_serialize::Value>
{
    let agent = component
        .downcast_ref::<AiAgent>()
        .expect("AiAgent expected");
    let mut fields = BTreeMap::new();

    if let Some(ref r) = agent.navmesh_ref {
        fields.insert(
            "navmesh_ref".into(),
            engine_serialize::Value::Str(r.clone()),
        );
    }
    fields.insert(
        "agent_radius".into(),
        engine_serialize::Value::Float32(agent.agent_radius),
    );
    fields.insert(
        "agent_height".into(),
        engine_serialize::Value::Float32(agent.agent_height),
    );
    fields.insert("speed".into(), engine_serialize::Value::Float32(agent.speed));
    fields.insert(
        "stopping_distance".into(),
        engine_serialize::Value::Float32(agent.stopping_distance),
    );
    if let Some(t) = agent.target {
        fields.insert("target".into(), engine_serialize::Value::Vec3(t.into()));
    }
    fields.insert(
        "controller_entity_id".into(),
        engine_serialize::Value::UInt(agent.controller_entity_id),
    );

    fields
}

/// Deserialize an `AiAgent` component from a field map.
pub fn deserialize_ai_agent(
    fields: &BTreeMap<String, engine_serialize::Value>,
) -> Box<dyn std::any::Any> {
    let mut agent = AiAgent::new();

    if let Some(engine_serialize::Value::Str(v)) = fields.get("navmesh_ref") {
        agent.navmesh_ref = Some(v.clone());
    }
    if let Some(engine_serialize::Value::Float32(v)) = fields.get("agent_radius") {
        agent.agent_radius = *v;
    }
    if let Some(engine_serialize::Value::Float32(v)) = fields.get("agent_height") {
        agent.agent_height = *v;
    }
    if let Some(engine_serialize::Value::Float32(v)) = fields.get("speed") {
        agent.speed = *v;
    }
    if let Some(engine_serialize::Value::Float32(v)) = fields.get("stopping_distance") {
        agent.stopping_distance = *v;
    }
    if let Some(engine_serialize::Value::Vec3(v)) = fields.get("target") {
        agent.target = Some((*v).into());
    }
    if let Some(engine_serialize::Value::UInt(v)) = fields.get("controller_entity_id") {
        agent.controller_entity_id = *v;
    }

    Box::new(agent)
}

// ---------------------------------------------------------------------------
// Extension registration
// ---------------------------------------------------------------------------

/// Register AI Agent extensions with the engine's component and debug-draw
/// systems.
///
/// Follows the same pattern as
/// [`engine_character::register_character_extensions`].
pub fn register_nav_extensions(
    component_registry: &mut engine_scene::registry::ComponentRegistry,
    _debug_draw_registry: Option<&mut engine_renderer::DebugDrawRegistry>,
) {
    use engine_scene::registry::{ComponentExtension, ComponentMeta};
    use engine_scene::{ComponentStorageDyn, SparseSet};

    let _ = component_registry.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: AiAgent::TYPE_ID,
            display_name: "AI Agent",
            schema_version: (0, 1, 0),
            has_editor: true,
            has_script_binding: true,
        },
        storage_factory: || -> Box<dyn ComponentStorageDyn> {
            Box::new(SparseSet::<AiAgent>::new())
        },
        serialize: Some(serialize_ai_agent),
        deserialize: Some(deserialize_ai_agent),
    });
}

// ---------------------------------------------------------------------------
// Per-frame update
// ---------------------------------------------------------------------------

/// Run one frame of AI agent pathfinding and push movement commands to the
/// associated [`CharacterController`].
///
/// This function:
/// 1. Syncs the internal [`NavAgent`] position with the character's position.
/// 2. If a [`target`](AiAgent::target) is set, uses [`Pathfinder`] to compute
///    a path on the given [`NavMesh`] and feeds it to the NavAgent.
/// 3. Calls [`NavAgent::update`] to get a [`MovementIntent`].
/// 4. Builds a [`CharacterCommand`] from the intent and pushes it to the
///    controller via [`CharacterController::push_command`].
///
/// Call this once per frame from the game loop for each active AI agent.
pub fn update_ai_agent(
    agent: &mut AiAgent,
    character: &mut CharacterController,
    navmesh: &NavMesh,
    dt: f32,
) {
    // Sync the NavAgent's position with the character's actual position.
    agent.nav_agent.set_position(character.position());
    agent.nav_agent.set_speed(agent.speed);

    // If a target is set and the agent has finished its current path (or
    // has a different target), compute a new path.
    if let Some(target) = agent.target {
        if agent.nav_agent.is_path_finished() {
            let pathfinder = Pathfinder::new();
            match pathfinder.find_path(navmesh, character.position(), target) {
                Ok(path) => {
                    agent.nav_agent.set_path(path);
                }
                Err(_) => {
                    // No path found — stop moving.
                    return;
                }
            }
        }
    } else {
        // No target — ensure the agent stops.
        agent.nav_agent.stop();
    }

    // Advance the NavAgent along its path.
    let (_update, intent) = agent.nav_agent.update(dt);

    // Convert the MovementIntent into a CharacterCommand.
    if let Some(intent) = intent {
        let cmd = CharacterCommand {
            direction: intent.direction,
            desired_speed: intent.desired_speed,
            jump_requested: intent.jump_requested,
        };
        character.push_command(cmd);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::Component;

    // ── Component creation ────────────────────────────────────────────────

    #[test]
    fn ai_agent_creation() {
        let agent = AiAgent::new();
        assert!(agent.navmesh_ref.is_none());
        assert!((agent.agent_radius - 0.3).abs() < 1e-6);
        assert!((agent.agent_height - 1.8).abs() < 1e-6);
        assert!((agent.speed - 5.0).abs() < 1e-6);
        assert!((agent.stopping_distance - 0.5).abs() < 1e-6);
        assert!(agent.target.is_none());
        assert_eq!(agent.controller_entity_id, 0);
    }

    #[test]
    fn ai_agent_type_id() {
        assert_eq!(AiAgent::TYPE_ID, "engine.nav_agent");
    }

    #[test]
    fn ai_agent_with_target() {
        let agent = AiAgent::with_target(42, Vec3::new(10.0, 0.0, 20.0));
        assert_eq!(agent.controller_entity_id, 42);
        assert_eq!(agent.target, Some(Vec3::new(10.0, 0.0, 20.0)));
    }

    #[test]
    fn ai_agent_default_impl() {
        let agent = AiAgent::default();
        assert!((agent.speed - 5.0).abs() < 1e-6);
    }

    // ── Serialization roundtrip ───────────────────────────────────────────

    #[test]
    fn ai_agent_serde_roundtrip() {
        let mut agent = AiAgent::new();
        agent.navmesh_ref = Some("navmesh_main".into());
        agent.agent_radius = 0.5;
        agent.agent_height = 2.0;
        agent.speed = 3.0;
        agent.stopping_distance = 1.0;
        agent.target = Some(Vec3::new(100.0, 0.0, 200.0));
        agent.controller_entity_id = 7;

        let serialized = serialize_ai_agent(&agent);
        let deserialized = deserialize_ai_agent(&serialized);
        let restored: &AiAgent = deserialized.downcast_ref().unwrap();

        assert_eq!(restored.navmesh_ref, Some("navmesh_main".into()));
        assert!((restored.agent_radius - 0.5).abs() < 1e-6);
        assert!((restored.agent_height - 2.0).abs() < 1e-6);
        assert!((restored.speed - 3.0).abs() < 1e-6);
        assert!((restored.stopping_distance - 1.0).abs() < 1e-6);
        assert_eq!(restored.target, Some(Vec3::new(100.0, 0.0, 200.0)));
        assert_eq!(restored.controller_entity_id, 7);
    }

    #[test]
    fn ai_agent_serde_defaults_on_empty() {
        let fields = BTreeMap::new();
        let deserialized = deserialize_ai_agent(&fields);
        let restored: &AiAgent = deserialized.downcast_ref().unwrap();

        assert!(restored.navmesh_ref.is_none());
        assert!((restored.agent_radius - 0.3).abs() < 1e-6);
        assert!((restored.agent_height - 1.8).abs() < 1e-6);
        assert!((restored.speed - 5.0).abs() < 1e-6);
        assert!((restored.stopping_distance - 0.5).abs() < 1e-6);
        assert!(restored.target.is_none());
        assert_eq!(restored.controller_entity_id, 0);
    }
}
