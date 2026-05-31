//! Performance inspector panel for the editor.
//!
//! Displays real-time frame statistics including rendering, physics,
//! animation, navigation, memory, and asset counts.  Historical frame
//! data is maintained as a rolling 60-frame buffer for graphing.

use tracing;

use engine_animation::components::AnimationPlayer;
use engine_nav::components::AiAgent;
use engine_physics::components::RigidBody;
use engine_renderer::{FrameStats as RendererFrameStats, Renderer};
use engine_scene::World;

use crate::editor_ui::EditorUi;

// ---------------------------------------------------------------------------
// FrameStats
// ---------------------------------------------------------------------------

/// Per-frame performance statistics snapshot.
#[derive(Clone, Debug)]
pub struct FrameStats {
    /// Total frame time in milliseconds.
    pub frame_time_ms: f32,
    /// Number of draw calls submitted to the GPU this frame.
    pub draw_calls: u32,
    /// Number of triangles rasterised this frame.
    pub triangles: u32,
    /// Number of active physics bodies.
    pub physics_bodies: u32,
    /// Number of active animation players.
    pub animation_count: u32,
    /// Number of active navigation agents.
    pub nav_agents: u32,
    /// Process memory usage in megabytes.
    pub memory_mb: f32,
    /// Number of loaded assets.
    pub asset_count: u32,
}

impl FrameStats {
    /// All-zero placeholder.
    pub const ZERO: Self = Self {
        frame_time_ms: 0.0,
        draw_calls: 0,
        triangles: 0,
        physics_bodies: 0,
        animation_count: 0,
        nav_agents: 0,
        memory_mb: 0.0,
        asset_count: 0,
    };

    /// Return a colour-coded label for the frame time.
    ///
    /// Returns `("label", R, G, B)` where:
    /// - green (`(0.0, 1.0, 0.0)`) when `< 8 ms`
    /// - yellow (`(1.0, 1.0, 0.0)`) when `8 – 13 ms`
    /// - red   (`(1.0, 0.3, 0.0)`) when `> 13 ms`
    pub fn frame_time_color(&self) -> (&'static str, [f32; 3]) {
        if self.frame_time_ms <= 8.0 {
            ("good", [0.0, 1.0, 0.0])
        } else if self.frame_time_ms <= 13.0 {
            ("warn", [1.0, 1.0, 0.0])
        } else {
            ("bad", [1.0, 0.3, 0.0])
        }
    }
}

impl Default for FrameStats {
    fn default() -> Self {
        Self::ZERO
    }
}

// ---------------------------------------------------------------------------
// PerformanceSections
// ---------------------------------------------------------------------------

/// Toggleable section visibility for the performance panel.
#[derive(Clone, Debug)]
pub struct PerformanceSections {
    pub rendering: bool,
    pub physics: bool,
    pub animation: bool,
    pub navigation: bool,
    pub memory: bool,
    pub assets: bool,
}

impl PerformanceSections {
    /// All sections collapsed.
    pub fn all_closed() -> Self {
        Self {
            rendering: false,
            physics: false,
            animation: false,
            navigation: false,
            memory: false,
            assets: false,
        }
    }

    /// All sections expanded.
    pub fn all_open() -> Self {
        Self {
            rendering: true,
            physics: true,
            animation: true,
            navigation: true,
            memory: true,
            assets: true,
        }
    }
}

impl Default for PerformanceSections {
    fn default() -> Self {
        // By default only the rendering section is expanded.
        Self {
            rendering: true,
            physics: false,
            animation: false,
            navigation: false,
            memory: false,
            assets: false,
        }
    }
}

// ---------------------------------------------------------------------------
// PerformancePanel
// ---------------------------------------------------------------------------

/// Panel state for the performance inspector.
pub struct PerformancePanel {
    /// Latest frame statistics.
    pub frame_stats: FrameStats,
    /// Rolling 60-frame history.
    pub history: Vec<FrameStats>,
    /// Which sections are expanded.
    pub visible_sections: PerformanceSections,
}

impl PerformancePanel {
    /// Create a new performance panel with default state.
    pub fn new() -> Self {
        Self {
            frame_stats: FrameStats::ZERO,
            history: Vec::with_capacity(60),
            visible_sections: PerformanceSections::default(),
        }
    }

    /// Push the current stats into the rolling history buffer.
    ///
    /// The buffer is capped at 60 entries — older entries are dropped.
    pub fn commit_frame(&mut self) {
        if self.history.len() >= 60 {
            self.history.remove(0);
        }
        self.history.push(self.frame_stats.clone());
    }

    /// Return a slice of the full history buffer.
    pub fn history(&self) -> &[FrameStats] {
        &self.history
    }
}

impl Default for PerformancePanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// record_frame
// ---------------------------------------------------------------------------

/// Populate `stats` with a snapshot of the current engine performance data.
///
/// Queries the ECS `World` for component counts (physics bodies, animation
/// players, nav agents) and — when `renderer_stats` is provided — merges
/// GPU-side statistics (draw calls, triangles, frame time) into `stats`.
///
/// Fields that require platform-specific measurement (`memory_mb`) are
/// left at their default value; callers should set them externally when
/// the platform support is available.
pub fn record_frame(
    stats: &mut FrameStats,
    world: &World,
    renderer_stats: Option<&RendererFrameStats>,
) {
    // ── Physics ──────────────────────────────────────────────────────
    stats.physics_bodies = world.query::<RigidBody>().count() as u32;

    // ── Animation ────────────────────────────────────────────────────
    stats.animation_count = world.query::<AnimationPlayer>().count() as u32;

    // ── Navigation ───────────────────────────────────────────────────
    stats.nav_agents = world.query::<AiAgent>().count() as u32;

    // ── GPU stats (from renderer) ────────────────────────────────────
    // Reset renderer-origin fields to zero so stale data from a
    // previous frame doesn't persist when renderer_stats is None.
    stats.draw_calls = 0;
    stats.triangles = 0;
    stats.frame_time_ms = 0.0;

    if let Some(rs) = renderer_stats {
        stats.draw_calls = rs.draw_calls;
        stats.triangles = u32::try_from(rs.triangles).unwrap_or(u32::MAX);
        stats.frame_time_ms = rs.gpu_frame_ms;
    }

    // ── Memory & assets ──────────────────────────────────────────────
    // `memory_mb` requires platform-specific querying (e.g.
    // GetProcessMemoryInfo on Windows, /proc/self/status on Linux) and
    // is left at its default value (0.0) here.
    // `asset_count` should be set externally from the AssetRegistry.

    tracing::trace!(
        physics = stats.physics_bodies,
        anim = stats.animation_count,
        nav = stats.nav_agents,
        draw_calls = stats.draw_calls,
        gpu_ms = stats.frame_time_ms,
        "PerformancePanel: frame recorded"
    );
}

// ---------------------------------------------------------------------------
// draw_performance
// ---------------------------------------------------------------------------

/// Draw the entire performance inspector panel.
///
/// Sections (collapsible):
/// - **Rendering** — frame-time bar graph, draw call count, triangle count
/// - **Physics**   — active rigid body count
/// - **Animation** — active animation player count
/// - **Navigation** — active AI agent count
/// - **Memory**    — process memory usage
/// - **Assets**    — loaded asset count (type breakdown placeholder)
///
/// Frame time values are colour-coded:
/// - green  `< 8 ms`   — smooth
/// - yellow `8–13 ms`  — marginal
/// - red    `> 13 ms`  — expensive
pub fn draw_performance(ui: &mut EditorUi, panel: &mut PerformancePanel) {
    let _ = ui.collapsing_header("Performance Inspector", true);

    let stats = &panel.frame_stats;
    let (_tag, color) = stats.frame_time_color();
    let color_str = format!("({:.2}, {:.2}, {:.2})", color[0], color[1], color[2]);

    // ── Summary line ─────────────────────────────────────────────────
    ui.text_field(
        "Frame Time",
        &format!("{:.2} ms  [{}]", stats.frame_time_ms, color_str),
    );
    ui.text_field("Draw Calls", &stats.draw_calls.to_string());
    ui.text_field("Triangles", &stats.triangles.to_string());

    let _ = ui.separator();

    // ── Rendering section ────────────────────────────────────────────
    panel.visible_sections.rendering =
        ui.collapsing_header("Rendering", panel.visible_sections.rendering);
    if panel.visible_sections.rendering {
        draw_frame_time_graph(ui, &panel.history);
        ui.text_field("Draw Calls", &stats.draw_calls.to_string());
        ui.text_field("Triangles", &stats.triangles.to_string());
        ui.text_field("GPU Frame", &format!("{:.2} ms", stats.frame_time_ms));
    }

    // ── Physics section ──────────────────────────────────────────────
    panel.visible_sections.physics =
        ui.collapsing_header("Physics", panel.visible_sections.physics);
    if panel.visible_sections.physics {
        ui.text_field("Rigid Bodies", &stats.physics_bodies.to_string());
    }

    // ── Animation section ────────────────────────────────────────────
    panel.visible_sections.animation =
        ui.collapsing_header("Animation", panel.visible_sections.animation);
    if panel.visible_sections.animation {
        ui.text_field("Anim Players", &stats.animation_count.to_string());
    }

    // ── Navigation section ───────────────────────────────────────────
    panel.visible_sections.navigation =
        ui.collapsing_header("Navigation", panel.visible_sections.navigation);
    if panel.visible_sections.navigation {
        ui.text_field("AI Agents", &stats.nav_agents.to_string());
    }

    // ── Memory section ───────────────────────────────────────────────
    panel.visible_sections.memory = ui.collapsing_header("Memory", panel.visible_sections.memory);
    if panel.visible_sections.memory {
        ui.text_field(
            "Process",
            &format!("{:.1} MB (requires platform hook)", stats.memory_mb),
        );
    }

    // ── Assets section ───────────────────────────────────────────────
    panel.visible_sections.assets = ui.collapsing_header("Assets", panel.visible_sections.assets);
    if panel.visible_sections.assets {
        ui.text_field("Loaded Assets", &stats.asset_count.to_string());
        // Type breakdown — wired when AssetRegistry provides per-type counts.
        ui.text_field("Meshes", "0");
        ui.text_field("Textures", "0");
        ui.text_field("Materials", "0");
        ui.text_field("Animations", "0");
        ui.text_field("Audio", "0");
    }

    tracing::debug!(
        frame_ms = stats.frame_time_ms,
        sections = ?panel.visible_sections,
        "PerformancePanel drawn"
    );
}

// ---------------------------------------------------------------------------
// draw_frame_time_graph
// ---------------------------------------------------------------------------

/// Draw a simple 60-frame bar chart of recent frame times.
///
/// Each bar represents one frame in the history buffer.  The bar height
/// is proportional to the frame time, and the colour follows the same
/// green / yellow / red scheme used elsewhere.
pub fn draw_frame_time_graph(ui: &mut EditorUi, history: &[FrameStats]) {
    let _ = ui.collapsing_header("Frame Time Graph (last 60)", true);

    if history.is_empty() {
        ui.text_field("Graph", "(no data)");
        return;
    }

    // Find the maximum frame time in the buffer for normalisation.
    let max_ms = history
        .iter()
        .map(|s| s.frame_time_ms)
        .fold(0.0f32, f32::max)
        .max(0.001);

    // Draw each bar as a text line.  In a full UI backend this would be
    // a proper bar chart; here we use a simple character-based approach.
    let bar_width = 20usize; // max bar width in characters.
    for (i, stats) in history.iter().enumerate() {
        let ratio = (stats.frame_time_ms / max_ms).clamp(0.0, 1.0);
        let filled = (ratio * bar_width as f32).round() as usize;
        let filled = filled.min(bar_width);
        let empty = bar_width - filled;

        let (_tag, _color) = stats.frame_time_color();
        let bar: String = std::iter::repeat('#').take(filled).collect();
        let space: String = std::iter::repeat('.').take(empty).collect();
        let label = format!(
            "[{:>3}] ▕{}{}▏ {:.1} ms",
            i + 1,
            bar,
            space,
            stats.frame_time_ms
        );
        ui.text_field(&format!("#{:03}", i + 1), &label);
    }

    tracing::trace!(samples = history.len(), max_ms, "Frame-time graph drawn");
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── FrameStats defaults ──────────────────────────────────────────────

    #[test]
    fn frame_stats_default_is_zero() {
        let s = FrameStats::default();
        assert_eq!(s.frame_time_ms, 0.0);
        assert_eq!(s.draw_calls, 0);
        assert_eq!(s.triangles, 0);
        assert_eq!(s.physics_bodies, 0);
        assert_eq!(s.animation_count, 0);
        assert_eq!(s.nav_agents, 0);
        assert_eq!(s.memory_mb, 0.0);
        assert_eq!(s.asset_count, 0);
    }

    #[test]
    fn frame_stats_zero_constant() {
        let s = FrameStats::ZERO;
        assert_eq!(s.frame_time_ms, 0.0);
        assert_eq!(s.draw_calls, 0);
    }

    // ── Frame time colour coding ────────────────────────────────────────

    #[test]
    fn frame_time_green_below_8() {
        let s = FrameStats {
            frame_time_ms: 5.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "good");
    }

    #[test]
    fn frame_time_yellow_8_to_13() {
        let s = FrameStats {
            frame_time_ms: 10.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "warn");
    }

    #[test]
    fn frame_time_red_above_13() {
        let s = FrameStats {
            frame_time_ms: 20.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "bad");
    }

    #[test]
    fn frame_time_boundary_8_is_green() {
        let s = FrameStats {
            frame_time_ms: 8.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "good");
    }

    #[test]
    fn frame_time_boundary_13_is_yellow() {
        let s = FrameStats {
            frame_time_ms: 13.0,
            ..FrameStats::ZERO
        };
        let (tag, _) = s.frame_time_color();
        assert_eq!(tag, "warn");
    }

    // ── Recording frame pushes to history ───────────────────────────────

    #[test]
    fn commit_frame_pushes_to_history() {
        let mut panel = PerformancePanel::new();
        assert!(panel.history.is_empty());

        panel.frame_stats.frame_time_ms = 16.5;
        panel.commit_frame();

        assert_eq!(panel.history.len(), 1);
        assert!((panel.history[0].frame_time_ms - 16.5).abs() < 0.001);
    }

    #[test]
    fn commit_frame_caps_at_60() {
        let mut panel = PerformancePanel::new();

        // Push 65 frames.
        for i in 0..65 {
            panel.frame_stats.frame_time_ms = i as f32;
            panel.commit_frame();
        }

        assert_eq!(panel.history.len(), 60);
        // The oldest frame should be frame 5 (0..4 were evicted).
        assert!((panel.history[0].frame_time_ms - 5.0).abs() < 0.001);
        // The newest should be frame 64.
        assert!((panel.history[59].frame_time_ms - 64.0).abs() < 0.001);
    }

    #[test]
    fn multiple_commits_maintain_order() {
        let mut panel = PerformancePanel::new();
        panel.frame_stats.frame_time_ms = 1.0;
        panel.commit_frame();
        panel.frame_stats.frame_time_ms = 2.0;
        panel.commit_frame();
        panel.frame_stats.frame_time_ms = 3.0;
        panel.commit_frame();

        assert_eq!(panel.history.len(), 3);
        assert!((panel.history[0].frame_time_ms - 1.0).abs() < 0.001);
        assert!((panel.history[1].frame_time_ms - 2.0).abs() < 0.001);
        assert!((panel.history[2].frame_time_ms - 3.0).abs() < 0.001);
    }

    // ── PerformanceSections ─────────────────────────────────────────────

    #[test]
    fn sections_default_rendering_only() {
        let s = PerformanceSections::default();
        assert!(s.rendering);
        assert!(!s.physics);
        assert!(!s.animation);
        assert!(!s.navigation);
        assert!(!s.memory);
        assert!(!s.assets);
    }

    #[test]
    fn sections_all_open() {
        let s = PerformanceSections::all_open();
        assert!(s.rendering);
        assert!(s.physics);
        assert!(s.animation);
        assert!(s.navigation);
        assert!(s.memory);
        assert!(s.assets);
    }

    #[test]
    fn sections_all_closed() {
        let s = PerformanceSections::all_closed();
        assert!(!s.rendering);
        assert!(!s.physics);
        assert!(!s.animation);
        assert!(!s.navigation);
        assert!(!s.memory);
        assert!(!s.assets);
    }

    // ── renderer parameter accepted (no-op) ─────────────────────────────

    #[test]
    fn record_frame_accepts_renderer() {
        let renderer = Renderer::new();
        let world = World::new();
        let mut stats = FrameStats::ZERO;

        // Should not panic.  Pass None for renderer stats (no GPU data in test).
        record_frame(&mut stats, &world, None);

        // World is empty → all counts stay zero.
        assert_eq!(stats.physics_bodies, 0);
        assert_eq!(stats.animation_count, 0);
        assert_eq!(stats.nav_agents, 0);
    }

    #[test]
    fn record_frame_counts_world_components() {
        use engine_scene::Component;

        let mut world = World::new();

        // Add a few rigid bodies.
        let e1 = world.create_entity();
        world.add_component(e1, RigidBody::default());
        let e2 = world.create_entity();
        world.add_component(e2, RigidBody::default());

        // Add one animation player.
        let e3 = world.create_entity();
        world.add_component(e3, AnimationPlayer::new());

        let mut stats = FrameStats::ZERO;
        record_frame(&mut stats, &world, None);

        assert_eq!(stats.physics_bodies, 2);
        assert_eq!(stats.animation_count, 1);
        assert_eq!(stats.nav_agents, 0);
    }

    // ── history() accessor ──────────────────────────────────────────────

    #[test]
    fn history_accessor_returns_slice() {
        let mut panel = PerformancePanel::new();
        assert!(panel.history().is_empty());

        panel.frame_stats.frame_time_ms = 7.0;
        panel.commit_frame();

        assert_eq!(panel.history().len(), 1);
        assert!((panel.history()[0].frame_time_ms - 7.0).abs() < 0.001);
    }
}
