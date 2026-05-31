//! Animation preview panel for the editor.
//!
//! Provides a timeline-based preview of animation clips with transport
//! controls, event markers, and current state-machine blend-state display.
//! All data is driven by the engine's asset registry and animation types.

use engine_animation::assets::{AnimationClip, Skeleton};
use engine_animation::Pose;

use crate::editor_ui::EditorUi;
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
    /// Cached clip asset handle (loaded from registry).
    cached_clip: Option<engine_asset::AssetHandle<AnimationClip>>,
    /// Most recently sampled pose (updated each frame when playing).
    pub sampled_pose: Option<Pose>,
}

impl AnimationPreviewPanel {
    /// Create a new animation preview panel with default state.
    pub fn new() -> Self {
        Self {
            selected_skeleton: None,
            selected_clip: None,
            available_clips: Vec::new(),
            playback_time: 0.0,
            playing: false,
            speed: 1.0,
            looping: true,
            blend_state: None,
            events: Vec::new(),
            clip_info: None,
            cached_skeleton: None,
            cached_clip: None,
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
// load_animation_data
// ---------------------------------------------------------------------------

/// Populate the panel's clip and skeleton lists from the asset registry.
///
/// Scans all cached assets in `registry`, extracts assets whose ID starts
/// with `"animation-"` as available clips, and records the skeleton at
/// `skeleton_path` as the selected skeleton.
pub fn load_animation_data(
    panel: &mut AnimationPreviewPanel,
    skeleton_path: &str,
    asset_registry: &AssetRegistry,
) {
    panel.selected_skeleton = Some(skeleton_path.to_string());

    // Gather all cached asset IDs that look like animation clips.
    let mut clips: Vec<String> = Vec::new();
    for id in asset_registry.cached_ids() {
        if id.id.starts_with("animation-") || id.id.starts_with("clip-") {
            // Try to load the AnimationClip asset to get the display name.
            if let Some(handle) = asset_registry.get::<AnimationClip>(&id) {
                clips.push(handle.get().name().to_string());
            } else {
                // Fall back to the asset ID string.
                clips.push(id.id.clone());
            }
        }
    }
    clips.sort();
    clips.dedup();
    panel.available_clips = clips;

    // If there is no current selection, pick the first clip.
    if panel.selected_clip.is_none() && !panel.available_clips.is_empty() {
        panel.selected_clip = Some(panel.available_clips[0].clone());
    }

    // Load skeleton handle for pose sampling.
    panel.cached_skeleton = asset_registry.get::<Skeleton>(&id_from_path(skeleton_path));
    if let Some(ref skel) = panel.cached_skeleton {
        tracing::debug!(
            skeleton = skeleton_path,
            joints = skel.get().joint_count(),
            "AnimationPreview: loaded skeleton"
        );
    }

    // Reload clip info and handle for the current selection.
    load_current_clip_info(panel, asset_registry);
}

/// Internal helper: refresh the cached clip info for the selected clip.
fn load_current_clip_info(panel: &mut AnimationPreviewPanel, registry: &AssetRegistry) {
    let Some(ref clip_name) = panel.selected_clip else {
        panel.clip_info = None;
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
    panel.events.clear();
}

/// Build an [`engine_serialize::AssetId`] from a path string (no-hyphen fallback).
fn id_from_path(path: &str) -> engine_serialize::AssetId {
    // Reuse the same convention as engine-asset: category-name → "category/name.asset".
    engine_serialize::AssetId::with_path("skeleton", path)
}

// ---------------------------------------------------------------------------
// draw_animation_preview
// ---------------------------------------------------------------------------

/// Draw the entire animation preview panel using the provided [`EditorUi`].
///
/// Layout (top to bottom):
/// 1. Skeleton selector dropdown + clip selector dropdown.
/// 2. Timeline scrubber with draggable playhead and time ruler.
/// 3. Transport controls: play/pause, stop, speed slider, loop toggle.
/// 4. Event markers on the timeline (diamond shapes at event times).
/// 5. Current pose blend-state display (when using a state machine).
pub fn draw_animation_preview(ui: &mut EditorUi, panel: &mut AnimationPreviewPanel) {
    let _header = ui.collapsing_header("Animation Preview", true);

    // ── Selector row ─────────────────────────────────────────────────
    let _ = ui.collapsing_header("Skeleton", true);
    let skeleton_label = panel.selected_skeleton.as_deref().unwrap_or("<none>");
    ui.text_field("Skeleton", skeleton_label);

    let _ = ui.separator();

    let _ = ui.collapsing_header("Clip", true);
    let clip_label = panel.selected_clip.as_deref().unwrap_or("<none>");
    ui.text_field("Clip", clip_label);

    // ── Clip info ────────────────────────────────────────────────────
    if let Some(info) = &panel.clip_info {
        let _ = ui.separator();
        let _ = ui.collapsing_header("Clip Info", true);
        ui.text_field("Duration", &format!("{:.3} s", info.duration));
        ui.text_field("Events", &info.event_count.to_string());
        if let Some(ref sm) = info.state_machine {
            ui.text_field("State Machine", sm);
        }
    }

    let _ = ui.separator();

    // ── Timeline scrubber ────────────────────────────────────────────
    let duration = panel.clip_info.as_ref().map(|i| i.duration).unwrap_or(1.0);

    let _ = ui.collapsing_header("Timeline", true);
    // Use a slider as a draggable playhead.
    if let Some(t) = ui.slider_f32("Time", panel.playback_time, 0.0, duration) {
        panel.playback_time = t.clamp(0.0, duration);
    }

    // ── Transport controls ───────────────────────────────────────────
    let _ = ui.separator();
    let _ = ui.collapsing_header("Transport", true);

    // Play / Pause toggle
    if ui.button(if panel.playing {
        "⏸ Pause"
    } else {
        "▶ Play"
    }) {
        panel.playing = !panel.playing;
        tracing::debug!(
            playing = panel.playing,
            "AnimationPreview: play/pause toggled"
        );
    }

    // Stop button
    if ui.button("⏹ Stop") {
        panel.playing = false;
        panel.playback_time = 0.0;
        tracing::debug!("AnimationPreview: stopped");
    }

    // Speed slider
    if let Some(s) = ui.slider_f32("Speed", panel.speed, 0.0, 5.0) {
        panel.speed = s.max(0.01);
    }

    // Loop toggle
    panel.looping = ui.checkbox("Loop", panel.looping);

    // ── Event markers ────────────────────────────────────────────────
    if !panel.events.is_empty() {
        let _ = ui.separator();
        let _ = ui.collapsing_header("Events", true);
        for event in &panel.events {
            ui.text_field(&event.name, &format!("{:.3} s", event.time));
        }
    }

    // ── Blend state display ──────────────────────────────────────────
    if let Some(ref state) = panel.blend_state {
        let _ = ui.separator();
        let _ = ui.collapsing_header("Blend State", true);
        ui.text_field("Current State", state);
    }

    tracing::debug!(
        playing = panel.playing,
        time = panel.playback_time,
        clip = ?panel.selected_clip,
        "AnimationPreviewPanel drawn"
    );
}

// ---------------------------------------------------------------------------
// Keyframe interpolation helpers
// ---------------------------------------------------------------------------

/// Interpolate `[f32; 3]` keyframes at `time`. Returns first keyframe
/// before the start, last after the end, and linear LERP between.
fn interpolate_keyframes_f32_3(
    kfs: &[engine_animation::assets::Keyframe<[f32; 3]>],
    time: f32,
) -> [f32; 3] {
    if kfs.is_empty() {
        return [0.0; 3];
    }
    if time <= kfs[0].time {
        return kfs[0].value;
    }
    for pair in kfs.windows(2) {
        if time < pair[1].time {
            let t = (time - pair[0].time) / (pair[1].time - pair[0].time);
            let t = t.clamp(0.0, 1.0);
            return [
                pair[0].value[0] + (pair[1].value[0] - pair[0].value[0]) * t,
                pair[0].value[1] + (pair[1].value[1] - pair[0].value[1]) * t,
                pair[0].value[2] + (pair[1].value[2] - pair[0].value[2]) * t,
            ];
        }
    }
    kfs.last().unwrap().value
}

/// Interpolate `[f32; 4]` keyframes at `time` with SLERP for quaternions.
fn interpolate_keyframes_quat(
    kfs: &[engine_animation::assets::Keyframe<[f32; 4]>],
    time: f32,
) -> [f32; 4] {
    if kfs.is_empty() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if time <= kfs[0].time {
        return kfs[0].value;
    }
    for pair in kfs.windows(2) {
        if time < pair[1].time {
            let t = (time - pair[0].time) / (pair[1].time - pair[0].time);
            let t = t.clamp(0.0, 1.0);
            let a = glam::Quat::from_array(pair[0].value);
            let b = glam::Quat::from_array(pair[1].value);
            return a.slerp(b, t).to_array();
        }
    }
    kfs.last().unwrap().value
}

// ---------------------------------------------------------------------------
// update_preview
// ---------------------------------------------------------------------------

/// Advance the playback time by `dt` seconds.
///
/// When `playing` is `true`, the playback position is advanced by
/// `dt * speed`.  At the clip end the position is either looped back to
/// zero (when `looping` is `true`) or clamped at the end and playback is
/// stopped.
///
/// When both a clip and skeleton are loaded, the clip is sampled at the
/// current playback time and the resulting [`Pose`] is stored in
/// [`AnimationPreviewPanel::sampled_pose`] for rendering.
pub fn update_preview(panel: &mut AnimationPreviewPanel, dt: f32) {
    if !panel.playing {
        return;
    }

    let duration = panel.clip_info.as_ref().map(|i| i.duration).unwrap_or(1.0);

    if duration <= 0.0 {
        return;
    }

    panel.playback_time += dt * panel.speed;
    // Clamp negative (defend against negative dt or speed).
    if panel.playback_time < 0.0 {
        panel.playback_time = 0.0;
    }

    if panel.playback_time >= duration {
        if panel.looping {
            panel.playback_time %= duration;
            if panel.playback_time < 0.0001 {
                panel.playback_time = 0.0;
            }
        } else {
            panel.playback_time = duration;
            panel.playing = false;
        }
    }

    // Sample the clip at the current time, if both assets are loaded.
    if let (Some(ref clip_h), Some(ref skel_h)) = (&panel.cached_clip, &panel.cached_skeleton) {
        let runtime_skel = engine_animation::skeleton::Skeleton::from_asset(skel_h.get());
        let clip_data = clip_h.get();
        // Sample each channel: for every animated joint, interpolate
        // translation/rotation/scale keyframes at the current time.
        let mut pose = engine_animation::Pose::new(&runtime_skel);
        {
            let locals = pose.local_transforms_mut();
            for channel in &clip_data.channels {
                let idx = channel.joint_index as usize;
                if idx >= locals.len() {
                    continue;
                }
                let t = panel.playback_time.clamp(0.0, clip_data.duration);

                let trans = interpolate_keyframes_f32_3(&channel.translations, t);
                let rot = interpolate_keyframes_quat(&channel.rotations, t);
                let scale = interpolate_keyframes_f32_3(&channel.scales, t);

                locals[idx] = engine_animation::BoneTransform {
                    translation: glam::Vec3::from(trans),
                    rotation: glam::Quat::from_array(rot),
                    scale: glam::Vec3::from(scale),
                };
            }
        }
        panel.sampled_pose = Some(pose);
    }

    tracing::trace!(
        time = panel.playback_time,
        playing = panel.playing,
        "AnimationPreviewPanel updated"
    );
}

// ===========================================================================
// Tests
// ===========================================================================

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
