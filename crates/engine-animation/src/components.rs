use crate::layers::AnimLayer;
use crate::state_machine::{AnimParamValue, AnimStateMachineInstance};
use engine_scene::Component;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// AnimationPlayer
// ---------------------------------------------------------------------------

/// ECS component for single-clip animation playback.
///
/// Stores playback state: which clip, whether playing/looping, speed,
/// current time position, and render layer.
///
/// ## Gate 11 additions
/// - `state_machine` — optional [`AnimStateMachineInstance`] for state-machine-driven
///   animation.
/// - `layers` — ordered list of [`AnimLayer`]s for layer-based blending.
///
/// Both new fields carry `#[serde(default)]` so that old serialised data without
/// them still deserialises correctly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnimationPlayer {
    /// AssetId string of the animation clip to play.
    pub clip_asset: Option<String>,
    /// Whether playback is actively advancing.
    pub playing: bool,
    /// Whether the clip loops when reaching the end.
    pub looping: bool,
    /// Playback speed multiplier (1.0 = normal speed).
    pub speed: f32,
    /// Current time position in seconds.
    pub current_time: f32,
    /// Render layer for the skinned item.
    pub layer: u32,

    // ── Gate 11 additions ────────────────────────────────────────────────

    /// Optional animation state machine instance.
    #[serde(default)]
    pub state_machine: Option<AnimStateMachineInstance>,
    /// Ordered animation layers for blending.
    #[serde(default)]
    pub layers: Vec<AnimLayer>,
}

impl Component for AnimationPlayer {
    const TYPE_ID: &'static str = "engine.animation_player";
}

impl AnimationPlayer {
    /// Create a new `AnimationPlayer` in the stopped state.
    pub fn new() -> Self {
        Self {
            clip_asset: None,
            playing: false,
            looping: true,
            speed: 1.0,
            current_time: 0.0,
            layer: 0,
            state_machine: None,
            layers: vec![AnimLayer::new("base")],
        }
    }

    /// Create a player that immediately starts playing the given clip.
    pub fn with_clip(clip_asset: impl Into<String>) -> Self {
        Self {
            clip_asset: Some(clip_asset.into()),
            playing: true,
            looping: true,
            speed: 1.0,
            current_time: 0.0,
            layer: 0,
            state_machine: None,
            layers: vec![AnimLayer::new("base")],
        }
    }

    // ── Gate 11 convenience methods ──────────────────────────────────────

    /// Attach a state machine instance to this player.
    pub fn set_state_machine(&mut self, sm: AnimStateMachineInstance) {
        self.state_machine = Some(sm);
    }

    /// Set a parameter on the attached state machine (if any).
    pub fn set_anim_param(&mut self, name: &str, value: AnimParamValue) {
        if let Some(ref mut sm) = self.state_machine {
            sm.set_param(name, value);
        }
    }

    /// Add an animation layer.
    pub fn add_layer(&mut self, layer: AnimLayer) {
        self.layers.push(layer);
    }
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SkeletonComponent
// ---------------------------------------------------------------------------

/// ECS component that attaches a skeleton asset and culling bounds to an entity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkeletonComponent {
    /// AssetId string of the skeleton asset.
    pub skeleton_asset: Option<String>,
    /// AABB half-extents for culling / bounds estimation.
    pub bind_shape: [f32; 3],
}

impl Component for SkeletonComponent {
    const TYPE_ID: &'static str = "engine.skeleton";
}

impl SkeletonComponent {
    pub fn new(skeleton_asset: impl Into<String>) -> Self {
        Self {
            skeleton_asset: Some(skeleton_asset.into()),
            bind_shape: [0.5, 0.5, 0.5],
        }
    }
}
