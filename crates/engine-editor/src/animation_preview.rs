//! Animation preview panel for the editor.
//!
//! Provides a timeline-based preview of animation clips with transport
//! controls, event markers, and current state-machine blend-state display.
//! All data is driven by the engine's asset registry and animation types.

use engine_animation::assets::{AnimationClip, Skeleton};
use engine_animation::Pose;

use engine_asset::AssetRegistry;
use tracing;

// ---------------------------------------------------------------------------
// AnimEvent
// ---------------------------------------------------------------------------

/// An animation event marker on the preview timeline.
#[derive(Clone, Debug)]
pub struct AnimEvent {
    /// Time (in seconds) within the clip.
    pub time: f32,
    /// Event name / identifier.
    pub name: String,
}

// ---------------------------------------------------------------------------
// AnimClipInfo
// ---------------------------------------------------------------------------

/// Metadata about a loaded animation clip for the preview panel.
#[derive(Clone, Debug)]
pub struct AnimClipInfo {
    /// Total duration of the clip in seconds.
    pub duration: f32,
    /// Number of event markers attached to this clip.
    pub event_count: usize,
    /// If the clip is driven by a state machine, its name.
    pub state_machine: Option<String>,
}

// ---------------------------------------------------------------------------
// AnimationPreviewPanel
// ---------------------------------------------------------------------------

/// Panel state for the animation preview.
pub struct AnimationPreviewPanel {
    // ── Selection ────────────────────────────────────────────────────
    /// Name of the currently selected skeleton asset.
    pub selected_skeleton: Option<String>,
    /// Name of the currently selected clip asset.
    pub selected_clip: Option<String>,
    /// Stable IDs of skeletons available in the asset registry.
    pub available_skeletons: Vec<String>,
    /// All clip names available in the asset registry.
    pub available_clips: Vec<String>,

    // ── Playback ─────────────────────────────────────────────────────
    /// Current playback position in seconds.
    pub playback_time: f32,
    /// Whether playback is running.
    pub playing: bool,
    /// Playback speed multiplier.
    pub speed: f32,
    /// Whether to loop when the clip end is reached.
    pub looping: bool,

    // ── State machine ────────────────────────────────────────────────
    /// Name of the active state machine state, if any.
    pub blend_state: Option<String>,

    // ── Event markers ────────────────────────────────────────────────
    /// Event markers for the selected clip.
    pub events: Vec<AnimEvent>,

    // ── Internal cache ───────────────────────────────────────────────
    /// Cached clip info for the currently selected clip.
    clip_info: Option<AnimClipInfo>,
    /// Cached skeleton asset handle (loaded from registry).
    cached_skeleton: Option<engine_asset::AssetHandle<Skeleton>>,
    cached_skeleton_id: Option<String>,
    /// Cached clip asset handle (loaded from registry).
    cached_clip: Option<engine_asset::AssetHandle<AnimationClip>>,
    cached_clip_name: Option<String>,
    /// Most recently sampled pose (updated each frame when playing).
    pub sampled_pose: Option<Pose>,
}

impl AnimationPreviewPanel {
    /// Create a new animation preview panel with default state.
    pub fn new() -> Self {
        Self {
            selected_skeleton: None,
            selected_clip: None,
            available_skeletons: Vec::new(),
            available_clips: Vec::new(),
            playback_time: 0.0,
            playing: false,
            speed: 1.0,
            looping: true,
            blend_state: None,
            events: Vec::new(),
            clip_info: None,
            cached_skeleton: None,
            cached_skeleton_id: None,
            cached_clip: None,
            cached_clip_name: None,
            sampled_pose: None,
        }
    }

    /// The cached [`AnimClipInfo`] for the selected clip, if loaded.
    pub fn clip_info(&self) -> Option<&AnimClipInfo> {
        self.clip_info.as_ref()
    }
}

impl Default for AnimationPreviewPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// refresh_animation_assets
// ---------------------------------------------------------------------------

/// Refresh the available animation assets and resolve the current selection.
/// Existing playback state is retained unless the selected clip changes.
pub fn refresh_animation_assets(panel: &mut AnimationPreviewPanel, asset_registry: &AssetRegistry) {
    let mut clips: Vec<String> = Vec::new();
    let mut skeletons: Vec<String> = Vec::new();
    for id in asset_registry.cached_ids() {
        if let Some(handle) = asset_registry.get::<AnimationClip>(&id) {
            clips.push(handle.get().name().to_string());
        }
        if asset_registry.get::<Skeleton>(&id).is_some() {
            skeletons.push(id.id.clone());
        }
    }
    clips.sort();
    clips.dedup();
    skeletons.sort();
    skeletons.dedup();
    panel.available_clips = clips;
    panel.available_skeletons = skeletons;

    if panel
        .selected_clip
        .as_ref()
        .is_none_or(|selected| !panel.available_clips.contains(selected))
    {
        panel.selected_clip = panel.available_clips.first().cloned();
    }
    if panel
        .selected_skeleton
        .as_ref()
        .is_none_or(|selected| !panel.available_skeletons.contains(selected))
    {
        panel.selected_skeleton = panel.available_skeletons.first().cloned();
    }

    if panel.cached_skeleton_id != panel.selected_skeleton {
        panel.cached_skeleton = panel.selected_skeleton.as_ref().and_then(|selected| {
            asset_registry.cached_ids().into_iter().find_map(|id| {
                (id.id == *selected || id.logical_path.as_deref() == Some(selected.as_str()))
                    .then(|| asset_registry.get::<Skeleton>(&id))
                    .flatten()
            })
        });
        panel.cached_skeleton_id = panel.selected_skeleton.clone();
    }
    if panel.cached_clip_name != panel.selected_clip {
        load_current_clip_info(panel, asset_registry);
        panel.cached_clip_name = panel.selected_clip.clone();
    }
    sample_preview_pose(panel);
}

/// Internal helper: refresh the cached clip info for the selected clip.
fn load_current_clip_info(panel: &mut AnimationPreviewPanel, registry: &AssetRegistry) {
    let Some(ref clip_name) = panel.selected_clip else {
        panel.clip_info = None;
        panel.cached_clip = None;
        panel.events.clear();
        return;
    };

    // Scan cached assets for a clip matching the selected name.
    for id in registry.cached_ids() {
        if let Some(handle) = registry.get::<AnimationClip>(&id) {
            if handle.get().name() == clip_name {
                let clip = handle.get();
                let duration = clip.duration();
                panel.clip_info = Some(AnimClipInfo {
                    duration,
                    event_count: 0, // events are not exposed on the asset type yet
                    state_machine: None,
                });
                // Store clip handle for pose sampling.
                panel.cached_clip = Some(handle.clone());
                // Reset playback when changing clips.
                panel.playback_time = 0.0;
                panel.playing = false;
                panel.sampled_pose = None;
                tracing::debug!(
                    clip = clip_name,
                    duration,
                    "AnimationPreview: loaded clip info"
                );
                return;
            }
        }
    }

    // Clip not found in registry – reset info.
    panel.clip_info = None;
    panel.cached_clip = None;
    panel.events.clear();
}

/// Advance preview playback independently from the editor UI.
pub fn update_preview(panel: &mut AnimationPreviewPanel, delta_seconds: f32) {
    if panel.playing && delta_seconds.is_finite() && delta_seconds > 0.0 {
        if let Some(duration) = panel.clip_info.as_ref().map(|clip| clip.duration) {
            if duration.is_finite() && duration > 0.0 {
                panel.playback_time += delta_seconds * panel.speed.max(0.0);
                if panel.playback_time >= duration {
                    if panel.looping {
                        panel.playback_time %= duration;
                    } else {
                        panel.playback_time = duration;
                        panel.playing = false;
                    }
                }
            }
        }
    }
    sample_preview_pose(panel);
}

/// Evaluate the selected cooked clip against the selected cooked skeleton.
///
/// This is the single sampling path used by transport playback and manual
/// timeline scrubbing. A missing or incompatible selection clears the last
/// pose instead of leaving stale preview data visible.
pub fn sample_preview_pose(panel: &mut AnimationPreviewPanel) -> bool {
    let (Some(clip), Some(skeleton)) = (&panel.cached_clip, &panel.cached_skeleton) else {
        panel.sampled_pose = None;
        return false;
    };
    let runtime_skeleton = engine_animation::skeleton::Skeleton::from_asset(skeleton.get());
    panel.sampled_pose = Some(engine_animation::AnimationEvaluator::evaluate_pose(
        clip.get(),
        panel.playback_time,
        &runtime_skeleton,
    ));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Defaults ─────────────────────────────────────────────────────────

    #[test]
    fn default_panel_has_no_selection() {
        let panel = AnimationPreviewPanel::new();
        assert!(panel.selected_skeleton.is_none());
        assert!(panel.selected_clip.is_none());
        assert!(panel.available_clips.is_empty());
        assert_eq!(panel.playback_time, 0.0);
        assert!(!panel.playing);
        assert_eq!(panel.speed, 1.0);
        assert!(panel.looping);
    }

    #[test]
    fn refresh_empty_registry_clears_stale_asset_selection() {
        let mut panel = AnimationPreviewPanel::new();
        panel.selected_skeleton = Some("missing-skeleton".into());
        panel.selected_clip = Some("missing-clip".into());

        refresh_animation_assets(&mut panel, &AssetRegistry::new());

        assert!(panel.available_skeletons.is_empty());
        assert!(panel.available_clips.is_empty());
        assert!(panel.selected_skeleton.is_none());
        assert!(panel.selected_clip.is_none());
        assert!(panel.clip_info().is_none());
    }

    // ── Play / pause ─────────────────────────────────────────────────────

    #[test]
    fn play_pause_toggles_playing() {
        let mut panel = AnimationPreviewPanel::new();
        assert!(!panel.playing);

        // Simulate the play button.
        panel.playing = !panel.playing;
        assert!(panel.playing);

        // Simulate the pause button.
        panel.playing = !panel.playing;
        assert!(!panel.playing);
    }

    #[test]
    fn stop_resets_time_and_playing() {
        let mut panel = AnimationPreviewPanel::new();
        panel.playing = true;
        panel.playback_time = 0.5;
        // Simulate stop.
        panel.playing = false;
        panel.playback_time = 0.0;
        assert!(!panel.playing);
        assert_eq!(panel.playback_time, 0.0);
    }

    // ── update_preview ───────────────────────────────────────────────────

    #[test]
    fn update_preview_when_not_playing_does_nothing() {
        let mut panel = AnimationPreviewPanel::new();
        panel.clip_info = Some(AnimClipInfo {
            duration: 2.0,
            event_count: 0,
            state_machine: None,
        });
        panel.playing = false;
        panel.playback_time = 0.5;

        update_preview(&mut panel, 1.0);

        // Should not advance.
        assert_eq!(panel.playback_time, 0.5);
    }

    #[test]
    fn update_preview_advances_time() {
        let mut panel = AnimationPreviewPanel::new();
        panel.clip_info = Some(AnimClipInfo {
            duration: 10.0,
            event_count: 0,
            state_machine: None,
        });
        panel.playing = true;
        panel.speed = 1.0;
        panel.playback_time = 0.0;

        update_preview(&mut panel, 2.0);

        assert!((panel.playback_time - 2.0).abs() < 0.001);
    }

    #[test]
    fn update_preview_respects_speed() {
        let mut panel = AnimationPreviewPanel::new();
        panel.clip_info = Some(AnimClipInfo {
            duration: 10.0,
            event_count: 0,
            state_machine: None,
        });
        panel.playing = true;
        panel.speed = 2.0;
        panel.playback_time = 0.0;

        update_preview(&mut panel, 1.0);

        // 1 s * 2x speed = 2 s advance.
        assert!((panel.playback_time - 2.0).abs() < 0.001);
    }

    #[test]
    fn update_preview_loops_at_end() {
        let mut panel = AnimationPreviewPanel::new();
        panel.clip_info = Some(AnimClipInfo {
            duration: 3.0,
            event_count: 0,
            state_machine: None,
        });
        panel.playing = true;
        panel.speed = 1.0;
        panel.looping = true;
        panel.playback_time = 2.5;

        update_preview(&mut panel, 1.0); // advances to 3.5 → loops to 0.5

        assert!((panel.playback_time - 0.5).abs() < 0.001);
        assert!(panel.playing);
    }

    #[test]
    fn update_preview_stops_at_end_when_not_looping() {
        let mut panel = AnimationPreviewPanel::new();
        panel.clip_info = Some(AnimClipInfo {
            duration: 5.0,
            event_count: 0,
            state_machine: None,
        });
        panel.playing = true;
        panel.speed = 1.0;
        panel.looping = false;
        panel.playback_time = 4.0;

        update_preview(&mut panel, 2.0); // advances to 6.0 → clamped at 5.0

        assert!((panel.playback_time - 5.0).abs() < 0.001);
        assert!(!panel.playing);
    }

    #[test]
    fn update_preview_zero_duration() {
        let mut panel = AnimationPreviewPanel::new();
        panel.clip_info = Some(AnimClipInfo {
            duration: 0.0,
            event_count: 0,
            state_machine: None,
        });
        panel.playing = true;
        panel.playback_time = 0.0;

        update_preview(&mut panel, 1.0);

        // Should not change anything.
        assert_eq!(panel.playback_time, 0.0);
    }

    // ── AnimClipInfo ─────────────────────────────────────────────────────

    #[test]
    fn clip_info_roundtrip() {
        let info = AnimClipInfo {
            duration: 3.5,
            event_count: 2,
            state_machine: Some("walk".to_string()),
        };
        assert!((info.duration - 3.5).abs() < 0.001);
        assert_eq!(info.event_count, 2);
        assert_eq!(info.state_machine.as_deref(), Some("walk"));
    }

    // ── AnimEvent ────────────────────────────────────────────────────────

    #[test]
    fn anim_event_fields() {
        let ev = AnimEvent {
            time: 1.5,
            name: "footstep".to_string(),
        };
        assert!((ev.time - 1.5).abs() < 0.001);
        assert_eq!(ev.name, "footstep");
    }
}
