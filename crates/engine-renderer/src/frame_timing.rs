//! Backend-neutral per-frame CPU/GPU pass timing contract (ENG-04).
//!
//! The types here are the cross-backend profiling contract:
//!
//! - [`PassTiming`] / [`FrameTimings`] describe one frame's per-pass and
//!   per-stage timings. CPU stages (`update`, `script_tick`, `extraction`,
//!   `sync_render_assets`, `render_submit`) are recorded by the engine
//!   runtime; GPU pass times are reported by the active backend through
//!   [`FrameStats`](crate::FrameStats).
//! - [`FrameTimingTracker`] owns the per-frame CPU stage recorder and the
//!   rolling window used to compute avg/p95/max aggregates.
//!
//! GPU timestamps are asynchronous: a backend typically reads query results
//! `frames-in-flight` frames after the work was recorded, so the GPU samples
//! attached to a frame may belong to an earlier frame (see
//! [`FrameTimings::gpu_frame_index`]). Backends that cannot provide GPU
//! timestamps report [`GpuTimingStatus::Unavailable`] instead of failing.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Default number of frames retained in the rolling statistics window.
pub const DEFAULT_TIMING_WINDOW: usize = 120;

/// Availability of GPU pass timestamps for a frame or backend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuTimingStatus {
    /// GPU timestamps were recorded and read back.
    Available,
    /// The backend records GPU timestamps but the first asynchronous
    /// read-back has not landed yet (frames-in-flight delay).
    Pending,
    /// The backend or device cannot provide GPU timestamps.
    #[default]
    Unavailable,
    /// GPU timestamps were disabled by configuration.
    Disabled,
}

/// One GPU pass timing sample reported by a backend.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuPassTime {
    pub name: String,
    pub ms: f32,
}

/// Timing for a single named pass or stage.
///
/// `cpu_ms` is present for CPU stages recorded by the engine runtime;
/// `gpu_ms` is present only when the backend reported a GPU timestamp for
/// this pass on the frame the sample describes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PassTiming {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_ms: Option<f32>,
}

/// Full timing breakdown for one frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameTimings {
    pub frame_index: u64,
    pub passes: Vec<PassTiming>,
    /// Sum of all CPU stage times recorded for this frame.
    pub total_cpu_ms: f32,
    /// Sum of all GPU pass times, present when at least one GPU sample was
    /// reported for this frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_gpu_ms: Option<f32>,
    /// GPU timestamp availability for this frame.
    pub gpu_status: GpuTimingStatus,
    /// Frame the attached GPU samples were recorded on. GPU read-back is
    /// asynchronous (frames-in-flight delay), so this can be earlier than
    /// `frame_index`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_frame_index: Option<u64>,
}

/// Aggregate statistics over a rolling window of samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimingAggregate {
    pub samples: u32,
    pub avg_ms: f32,
    pub p95_ms: f32,
    pub max_ms: f32,
}

impl TimingAggregate {
    /// Compute avg/p95/max over `values`. `p95` uses the nearest-rank method.
    fn from_values(values: &[f32]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let samples = sorted.len();
        let sum: f64 = sorted.iter().map(|value| f64::from(*value)).sum();
        let p95_index = (samples * 95).div_ceil(100).max(1) - 1;
        Some(Self {
            samples: samples as u32,
            avg_ms: (sum / samples as f64) as f32,
            p95_ms: sorted[p95_index],
            max_ms: sorted[samples - 1],
        })
    }
}

/// Rolling aggregate for one named pass.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PassTimingStats {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<TimingAggregate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<TimingAggregate>,
}

/// Rolling-window summary of frame timings, consumed by tooling (editor
/// diagnostics, headless run reports).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FrameTimingSummary {
    /// Number of frames currently in the rolling window.
    pub window_frames: u32,
    /// Maximum frames retained in the rolling window.
    pub window_capacity: u32,
    /// GPU timestamp availability reported by the most recent frame.
    pub gpu_status: GpuTimingStatus,
    pub passes: Vec<PassTimingStats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cpu: Option<TimingAggregate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_gpu: Option<TimingAggregate>,
}

/// A CPU stage currently being recorded. Nested stages pause their parent so
/// each stage's time excludes the time spent in its children; the sum of all
/// stage times therefore approximates the total measured frame path.
struct ActiveStage {
    name: String,
    /// Start of the current un-paused segment.
    segment_start: Instant,
    /// Accumulated own time (excluding nested child stages).
    own: Duration,
    /// First-seen order for stable output.
    order: usize,
}

struct CompletedStage {
    name: String,
    own: Duration,
    order: usize,
}

/// Per-frame CPU stage recorder plus rolling-window statistics.
///
/// Recording contract:
/// - `begin_stage` / `end_stage` must nest strictly. Ending a stage that is
///   not the innermost active stage closes the innermost stage under its own
///   name instead of corrupting the stack.
/// - `finish_frame` closes any stages left open and publishes a
///   [`FrameTimings`] into the rolling window.
/// - `discard_frame` drops a partially recorded frame (render failure)
///   without polluting the statistics.
///
/// Clock reads happen only at stage boundaries; the recorder performs no
/// allocation beyond the bounded pass list and history window.
pub struct FrameTimingTracker {
    window: usize,
    active: Vec<ActiveStage>,
    completed: Vec<CompletedStage>,
    next_order: usize,
    history: VecDeque<FrameTimings>,
    last: Option<FrameTimings>,
}

impl FrameTimingTracker {
    /// Create a tracker with the default rolling window (120 frames).
    pub fn new() -> Self {
        Self::with_window(DEFAULT_TIMING_WINDOW)
    }

    /// Create a tracker retaining at most `window.max(1)` frames.
    pub fn with_window(window: usize) -> Self {
        Self {
            window: window.max(1),
            active: Vec::new(),
            completed: Vec::new(),
            next_order: 0,
            history: VecDeque::new(),
            last: None,
        }
    }

    /// Begin a CPU stage, pausing the enclosing stage (if any).
    pub fn begin_stage(&mut self, name: &str) {
        let now = Instant::now();
        if let Some(parent) = self.active.last_mut() {
            parent.own += now.saturating_duration_since(parent.segment_start);
        }
        let order = self.next_order;
        self.next_order += 1;
        self.active.push(ActiveStage {
            name: name.to_string(),
            segment_start: now,
            own: Duration::ZERO,
            order,
        });
    }

    /// End a CPU stage, resuming the enclosing stage (if any).
    ///
    /// When `name` does not match the innermost active stage the innermost
    /// stage is closed under its actual name; this keeps a mismatched call
    /// site from discarding measurements or panicking.
    pub fn end_stage(&mut self, name: &str) {
        let now = Instant::now();
        loop {
            let Some(stage) = self.active.last() else {
                return;
            };
            let matches = stage.name == name;
            let stage = self.active.pop().expect("checked non-empty above");
            let own = stage.own + now.saturating_duration_since(stage.segment_start);
            self.completed.push(CompletedStage {
                name: stage.name,
                own,
                order: stage.order,
            });
            if let Some(parent) = self.active.last_mut() {
                parent.segment_start = now;
            }
            if matches {
                return;
            }
        }
    }

    /// Close the frame and publish its [`FrameTimings`].
    ///
    /// `gpu_status` / `gpu_frame_index` / `gpu_passes` come from the backend's
    /// frame statistics; pass `None`/empty when the backend reports no GPU
    /// timestamps.
    pub fn finish_frame(
        &mut self,
        frame_index: u64,
        gpu_status: GpuTimingStatus,
        gpu_frame_index: Option<u64>,
        gpu_passes: Vec<GpuPassTime>,
    ) -> FrameTimings {
        // Defensively close any stages a caller forgot to end so their time
        // is still attributed instead of silently dropped.
        while let Some(stage) = self.active.pop() {
            let own = stage.own + stage.segment_start.elapsed();
            self.completed.push(CompletedStage {
                name: stage.name,
                own,
                order: stage.order,
            });
        }

        // Merge CPU stages by name (summing repeats), preserving first-seen
        // order.
        let mut passes: Vec<PassTiming> = Vec::new();
        let mut completed = std::mem::take(&mut self.completed);
        completed.sort_by_key(|stage| stage.order);
        for stage in completed {
            let ms = stage.own.as_secs_f32() * 1_000.0;
            if let Some(existing) = passes.iter_mut().find(|pass| pass.name == stage.name) {
                existing.cpu_ms = Some(existing.cpu_ms.unwrap_or(0.0) + ms);
            } else {
                passes.push(PassTiming {
                    name: stage.name,
                    cpu_ms: Some(ms),
                    gpu_ms: None,
                });
            }
        }

        // Merge GPU samples: attach to a CPU pass of the same name when one
        // exists (GPU pass names live in the render-graph namespace, so they
        // usually append as their own passes).
        let mut total_gpu = 0.0f32;
        let mut any_gpu = false;
        for sample in gpu_passes {
            any_gpu = true;
            total_gpu += sample.ms;
            if let Some(existing) = passes.iter_mut().find(|pass| pass.name == sample.name) {
                existing.gpu_ms = Some(existing.gpu_ms.unwrap_or(0.0) + sample.ms);
            } else {
                passes.push(PassTiming {
                    name: sample.name,
                    cpu_ms: None,
                    gpu_ms: Some(sample.ms),
                });
            }
        }

        let total_cpu_ms = passes.iter().filter_map(|pass| pass.cpu_ms).sum::<f32>();
        let timings = FrameTimings {
            frame_index,
            passes,
            total_cpu_ms,
            total_gpu_ms: any_gpu.then_some(total_gpu),
            gpu_status,
            gpu_frame_index,
        };

        if self.history.len() >= self.window {
            self.history.pop_front();
        }
        self.history.push_back(timings.clone());
        self.last = Some(timings.clone());
        timings
    }

    /// Drop all partially recorded stage state without publishing a frame.
    pub fn discard_frame(&mut self) {
        self.active.clear();
        self.completed.clear();
    }

    /// The most recently published frame timings.
    pub fn last_frame(&self) -> Option<&FrameTimings> {
        self.last.as_ref()
    }

    /// Rolling-window aggregates over every published frame.
    pub fn summary(&self) -> FrameTimingSummary {
        let mut pass_order: Vec<String> = Vec::new();
        for frame in &self.history {
            for pass in &frame.passes {
                if !pass_order.contains(&pass.name) {
                    pass_order.push(pass.name.clone());
                }
            }
        }
        let passes = pass_order
            .into_iter()
            .map(|name| {
                let cpu_values: Vec<f32> = self
                    .history
                    .iter()
                    .filter_map(|frame| {
                        frame
                            .passes
                            .iter()
                            .find(|pass| pass.name == name)
                            .and_then(|pass| pass.cpu_ms)
                    })
                    .collect();
                let gpu_values: Vec<f32> = self
                    .history
                    .iter()
                    .filter_map(|frame| {
                        frame
                            .passes
                            .iter()
                            .find(|pass| pass.name == name)
                            .and_then(|pass| pass.gpu_ms)
                    })
                    .collect();
                PassTimingStats {
                    name,
                    cpu: TimingAggregate::from_values(&cpu_values),
                    gpu: TimingAggregate::from_values(&gpu_values),
                }
            })
            .collect();

        let total_cpu_values: Vec<f32> = self
            .history
            .iter()
            .map(|frame| frame.total_cpu_ms)
            .collect();
        let total_gpu_values: Vec<f32> = self
            .history
            .iter()
            .filter_map(|frame| frame.total_gpu_ms)
            .collect();

        FrameTimingSummary {
            window_frames: self.history.len() as u32,
            window_capacity: self.window as u32,
            gpu_status: self
                .last
                .as_ref()
                .map(|frame| frame.gpu_status)
                .unwrap_or_default(),
            passes,
            total_cpu: TimingAggregate::from_values(&total_cpu_values),
            total_gpu: TimingAggregate::from_values(&total_gpu_values),
        }
    }
}

impl Default for FrameTimingTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn gpu(name: &str, ms: f32) -> GpuPassTime {
        GpuPassTime {
            name: name.to_string(),
            ms,
        }
    }

    #[test]
    fn nested_stages_attribute_own_time_and_sum_to_total() {
        let mut tracker = FrameTimingTracker::new();
        let frame_start = Instant::now();
        tracker.begin_stage("update");
        sleep(Duration::from_millis(2));
        tracker.begin_stage("script_tick");
        sleep(Duration::from_millis(2));
        tracker.end_stage("script_tick");
        sleep(Duration::from_millis(2));
        tracker.end_stage("update");
        let wall_ms = frame_start.elapsed().as_secs_f32() * 1_000.0;
        let frame = tracker.finish_frame(7, GpuTimingStatus::Unavailable, None, Vec::new());

        let update = frame
            .passes
            .iter()
            .find(|pass| pass.name == "update")
            .unwrap()
            .cpu_ms
            .unwrap();
        let script = frame
            .passes
            .iter()
            .find(|pass| pass.name == "script_tick")
            .unwrap()
            .cpu_ms
            .unwrap();

        // Nested accounting: `update` excludes the time spent in
        // `script_tick`, so the stage sum must approximate the measured wall
        // time rather than double-count the child.
        assert!(
            script >= 1.0,
            "script_tick should measure ~2ms, got {script}"
        );
        assert!(
            update >= 3.0,
            "update should measure its own ~4ms, got {update}"
        );
        let stage_sum: f32 = frame.passes.iter().filter_map(|pass| pass.cpu_ms).sum();
        assert!(
            (stage_sum - frame.total_cpu_ms).abs() < f32::EPSILON,
            "total_cpu_ms must equal the sum of stage times"
        );
        assert!(
            stage_sum <= wall_ms + 1.0,
            "stage sum {stage_sum} must not exceed wall time {wall_ms}"
        );
        assert!(
            stage_sum >= wall_ms - 3.0,
            "stage sum {stage_sum} should cover wall time {wall_ms}"
        );
    }

    #[test]
    fn repeated_stage_names_are_summed() {
        let mut tracker = FrameTimingTracker::new();
        for _ in 0..2 {
            tracker.begin_stage("sync_render_assets");
            sleep(Duration::from_millis(1));
            tracker.end_stage("sync_render_assets");
        }
        let frame = tracker.finish_frame(0, GpuTimingStatus::Unavailable, None, Vec::new());
        assert_eq!(frame.passes.len(), 1);
        let first: f32 = frame.passes[0].cpu_ms.unwrap();
        assert!(first >= 1.5, "repeated stages must sum, got {first}");
    }

    #[test]
    fn mismatched_end_stage_closes_inner_stage_without_panicking() {
        let mut tracker = FrameTimingTracker::new();
        tracker.begin_stage("outer");
        tracker.begin_stage("inner");
        tracker.end_stage("outer");
        let frame = tracker.finish_frame(0, GpuTimingStatus::Unavailable, None, Vec::new());
        // Both stages were closed (inner under its own name, then outer).
        assert!(frame.passes.iter().any(|pass| pass.name == "inner"));
        assert!(frame.passes.iter().any(|pass| pass.name == "outer"));
        // finish_frame closes everything; nothing leaks into the next frame.
        let next = tracker.finish_frame(1, GpuTimingStatus::Unavailable, None, Vec::new());
        assert!(next.passes.is_empty());
    }

    #[test]
    fn discard_frame_drops_partial_recording() {
        let mut tracker = FrameTimingTracker::new();
        tracker.begin_stage("update");
        tracker.end_stage("update");
        tracker.begin_stage("extraction");
        tracker.discard_frame();
        let frame = tracker.finish_frame(0, GpuTimingStatus::Unavailable, None, Vec::new());
        assert!(frame.passes.is_empty());
        assert_eq!(frame.total_cpu_ms, 0.0);
    }

    #[test]
    fn gpu_samples_merge_by_name_and_drive_totals() {
        let mut tracker = FrameTimingTracker::new();
        tracker.begin_stage("render_submit");
        tracker.end_stage("render_submit");
        let frame = tracker.finish_frame(
            4,
            GpuTimingStatus::Available,
            Some(2),
            vec![
                gpu("opaque_pbr_forward_pass", 1.5),
                gpu("tone_map_pass", 0.25),
            ],
        );
        assert_eq!(frame.gpu_status, GpuTimingStatus::Available);
        assert_eq!(frame.gpu_frame_index, Some(2));
        assert_eq!(frame.total_gpu_ms, Some(1.75));
        let forward = frame
            .passes
            .iter()
            .find(|pass| pass.name == "opaque_pbr_forward_pass")
            .unwrap();
        assert_eq!(forward.cpu_ms, None);
        assert_eq!(forward.gpu_ms, Some(1.5));
    }

    #[test]
    fn rolling_window_caps_at_capacity_and_aggregates() {
        let mut tracker = FrameTimingTracker::with_window(4);
        for frame_index in 0..6u64 {
            tracker.begin_stage("update");
            tracker.end_stage("update");
            tracker.finish_frame(frame_index, GpuTimingStatus::Unavailable, None, Vec::new());
        }
        let summary = tracker.summary();
        assert_eq!(summary.window_frames, 4);
        assert_eq!(summary.window_capacity, 4);
        // Oldest frames were evicted; the window covers frames 2..=5.
        let stats = summary
            .passes
            .iter()
            .find(|pass| pass.name == "update")
            .unwrap();
        assert_eq!(stats.cpu.unwrap().samples, 4);
        assert!(stats.gpu.is_none());
        assert_eq!(summary.total_cpu.unwrap().samples, 4);
        assert!(summary.total_gpu.is_none());
    }

    #[test]
    fn aggregate_computes_avg_p95_max() {
        let values: Vec<f32> = (1..=20).map(|value| value as f32).collect();
        let aggregate = TimingAggregate::from_values(&values).unwrap();
        assert_eq!(aggregate.samples, 20);
        assert!((aggregate.avg_ms - 10.5).abs() < 0.001);
        // Nearest rank: ceil(20 * 0.95) = 19 -> the 19th value.
        assert!((aggregate.p95_ms - 19.0).abs() < f32::EPSILON);
        assert!((aggregate.max_ms - 20.0).abs() < f32::EPSILON);
        assert!(TimingAggregate::from_values(&[]).is_none());
    }

    #[test]
    fn summary_tracks_gpu_status_and_gpu_aggregates() {
        let mut tracker = FrameTimingTracker::new();
        tracker.finish_frame(0, GpuTimingStatus::Pending, None, Vec::new());
        tracker.finish_frame(
            1,
            GpuTimingStatus::Available,
            Some(0),
            vec![gpu("shadow", 0.5)],
        );
        tracker.finish_frame(
            2,
            GpuTimingStatus::Available,
            Some(1),
            vec![gpu("shadow", 1.5)],
        );
        let summary = tracker.summary();
        assert_eq!(summary.gpu_status, GpuTimingStatus::Available);
        let shadow = summary
            .passes
            .iter()
            .find(|pass| pass.name == "shadow")
            .unwrap();
        let gpu_stats = shadow.gpu.unwrap();
        assert_eq!(gpu_stats.samples, 2);
        assert!((gpu_stats.avg_ms - 1.0).abs() < f32::EPSILON);
        assert!((gpu_stats.max_ms - 1.5).abs() < f32::EPSILON);
        assert!((summary.total_gpu.unwrap().avg_ms - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn frame_timings_serialize_to_the_report_shape() {
        let mut tracker = FrameTimingTracker::new();
        tracker.begin_stage("update");
        tracker.end_stage("update");
        let frame = tracker.finish_frame(3, GpuTimingStatus::Unavailable, None, Vec::new());
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["frame_index"], 3);
        assert_eq!(json["gpu_status"], "unavailable");
        assert_eq!(json["passes"][0]["name"], "update");
        assert!(json["passes"][0]["cpu_ms"].is_number());
        // Optional fields are omitted entirely when absent.
        assert!(json.get("total_gpu_ms").is_none());
        assert!(json.get("gpu_frame_index").is_none());
        assert!(json["passes"][0].get("gpu_ms").is_none());
    }
}
