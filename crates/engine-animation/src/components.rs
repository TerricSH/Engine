use engine_scene::Component;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// AnimationPlayer
// ---------------------------------------------------------------------------

/// ECS component for single-clip animation playback.
///
/// Stores playback state: which clip, whether playing/looping, speed,
/// current time position, and render layer.
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
        }
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
