//! Per-pass GPU timestamp queries for the Vulkan backend (ENG-04).
//!
//! Design:
//! - One query pool per frame-in-flight slot, pre-allocated with
//!   [`QUERIES_PER_POOL`] entries (two timestamps per pass, up to
//!   [`MAX_GPU_PASSES`] passes per frame).
//! - Around every render-graph pass the [`SceneRenderer`] writes a start and
//!   an end timestamp into the current slot's pool.
//! - Read-back is asynchronous: results for the frame recorded on a slot are
//!   collected the next time that slot's fence is waited (frames-in-flight
//!   frames later), using a non-blocking `vkGetQueryPoolResults` call. The
//!   pipeline is never stalled for profiling.
//! - Raw ticks are calibrated to nanoseconds with the device's
//!   `timestampPeriod` limit.
//!
//! [`GpuTimestampProfiler`] is the pure state machine (support evaluation,
//! pair bookkeeping, async-delay semantics, tick→ms conversion, degradation);
//! [`TimestampQueryPools`] is the thin Vulkan call wrapper. All policy is
//! unit-tested without a device by scripting tick values through
//! [`GpuTimestampProfiler::deliver_readback`].
//!
//! [`SceneRenderer`]: crate::scene_renderer::SceneRenderer

use ash::vk;

use engine_renderer::{GpuPassTime, GpuTimingStatus};

/// Maximum number of render passes timestamped per frame. Frames with more
/// passes silently skip the excess (CPU timing still covers them).
pub(crate) const MAX_GPU_PASSES: usize = 32;

/// Queries per pool: one start + one end timestamp per pass.
pub(crate) const QUERIES_PER_POOL: u32 = 2 * MAX_GPU_PASSES as u32;

/// Device support decision for timestamp queries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TimestampSupport {
    /// Timestamps supported; `period_ns` converts ticks to nanoseconds.
    Supported { period_ns: f32 },
    /// Timestamps not supported by the device/driver, with a static reason.
    Unsupported(&'static str),
    /// Timestamps disabled by engine configuration.
    Disabled,
}

/// Evaluate device limits + configuration into a support decision.
///
/// `timestampComputeAndGraphics` must be true and `timestampPeriod` must be a
/// positive finite value for timestamps to be usable; anything else reports
/// `Unsupported` (never a hard failure).
pub(crate) fn evaluate_support(
    enabled: bool,
    timestamp_compute_and_graphics: bool,
    timestamp_period: f32,
) -> TimestampSupport {
    if !enabled {
        return TimestampSupport::Disabled;
    }
    if !timestamp_compute_and_graphics {
        return TimestampSupport::Unsupported(
            "device does not support timestamps on graphics/compute queues",
        );
    }
    if !timestamp_period.is_finite() || timestamp_period <= 0.0 {
        return TimestampSupport::Unsupported("device reports no valid timestampPeriod");
    }
    TimestampSupport::Supported {
        period_ns: timestamp_period,
    }
}

/// GPU pass samples read back for one recorded frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GpuPassBatch {
    pub frame_index: u64,
    pub passes: Vec<GpuPassTime>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ProfilerState {
    Disabled,
    Unavailable(&'static str),
    Ready,
}

/// Bookkeeping for the frame currently recording into a slot.
#[derive(Default)]
struct SlotState {
    /// Pass names recorded for the not-yet-submitted frame, in pair order.
    recording: Option<RecordingSlot>,
    /// Submitted frame awaiting asynchronous read-back.
    pending: Option<PendingSlot>,
}

struct RecordingSlot {
    frame_index: u64,
    /// Pass names in pair order; pass `i` uses queries `2*i` and `2*i+1`.
    passes: Vec<String>,
}

struct PendingSlot {
    frame_index: u64,
    passes: Vec<String>,
}

/// Pure per-pass GPU timestamp state machine.
///
/// One instance drives `frames-in-flight` query pool slots. All methods are
/// no-ops unless the profiler was configured with [`TimestampSupport::Supported`].
pub(crate) struct GpuTimestampProfiler {
    state: ProfilerState,
    period_ns: f64,
    slots: Vec<SlotState>,
    active_slot: Option<usize>,
    /// Latest read-back, pending consumption by the frame statistics.
    latest: Option<GpuPassBatch>,
    /// Batches successfully read back since configuration (drives
    /// [`GpuTimingStatus::Pending`] → [`GpuTimingStatus::Available`]).
    batches_received: u64,
    /// Frames whose read-back failed or was dropped.
    lost_frames: u64,
}

impl GpuTimestampProfiler {
    pub(crate) fn new() -> Self {
        Self {
            state: ProfilerState::Unavailable("timestamp support not evaluated"),
            period_ns: 1.0,
            slots: Vec::new(),
            active_slot: None,
            latest: None,
            batches_received: 0,
            lost_frames: 0,
        }
    }

    /// (Re)configure support. `slots` is the frame-in-flight count.
    pub(crate) fn configure(&mut self, support: TimestampSupport, slots: usize) {
        self.active_slot = None;
        self.latest = None;
        self.batches_received = 0;
        self.lost_frames = 0;
        match support {
            TimestampSupport::Disabled => {
                self.state = ProfilerState::Disabled;
                self.slots = Vec::new();
            }
            TimestampSupport::Unsupported(reason) => {
                self.state = ProfilerState::Unavailable(reason);
                self.slots = Vec::new();
            }
            TimestampSupport::Supported { period_ns } => {
                self.state = ProfilerState::Ready;
                self.period_ns = f64::from(period_ns);
                self.slots = (0..slots.max(1)).map(|_| SlotState::default()).collect();
            }
        }
    }

    /// Permanently degrade after a runtime failure (e.g. pool creation).
    pub(crate) fn degrade(&mut self, reason: &'static str) {
        self.state = ProfilerState::Unavailable(reason);
        self.slots.clear();
        self.active_slot = None;
        self.latest = None;
    }

    /// Availability as reported through frame statistics.
    pub(crate) fn status(&self) -> GpuTimingStatus {
        match self.state {
            ProfilerState::Disabled => GpuTimingStatus::Disabled,
            ProfilerState::Unavailable(_) => GpuTimingStatus::Unavailable,
            ProfilerState::Ready => {
                if self.batches_received > 0 {
                    GpuTimingStatus::Available
                } else {
                    GpuTimingStatus::Pending
                }
            }
        }
    }

    /// Static reason when the state is `Unavailable` (for diagnostics).
    #[cfg(test)]
    pub(crate) fn unavailable_reason(&self) -> Option<&'static str> {
        match self.state {
            ProfilerState::Unavailable(reason) => Some(reason),
            _ => None,
        }
    }

    /// Frames whose read-back failed or was dropped.
    #[cfg(test)]
    pub(crate) fn lost_frames(&self) -> u64 {
        self.lost_frames
    }

    fn ready(&self) -> bool {
        self.state == ProfilerState::Ready
    }

    /// Number of queries to read back for `slot` before it is reused, or
    /// `None` when nothing is pending on the slot.
    pub(crate) fn readback_len(&self, slot: usize) -> Option<u32> {
        if !self.ready() {
            return None;
        }
        self.slots
            .get(slot)
            .and_then(|state| state.pending.as_ref())
            .map(|pending| 2 * pending.passes.len() as u32)
    }

    /// Deliver asynchronously read ticks for `slot`. `ticks` must contain at
    /// least the recorded query count; `None` (read failure) drops the frame.
    pub(crate) fn deliver_readback(&mut self, slot: usize, ticks: Option<&[u64]>) {
        if !self.ready() {
            return;
        }
        let Some(state) = self.slots.get_mut(slot) else {
            return;
        };
        let Some(pending) = state.pending.take() else {
            return;
        };
        let Some(ticks) = ticks else {
            self.lost_frames += 1;
            tracing::warn!(
                slot,
                frame_index = pending.frame_index,
                "GPU timestamp read-back failed; dropping frame timings"
            );
            return;
        };
        let mut passes = Vec::with_capacity(pending.passes.len());
        for (index, name) in pending.passes.iter().enumerate() {
            let (Some(&start), Some(&end)) = (ticks.get(2 * index), ticks.get(2 * index + 1))
            else {
                self.lost_frames += 1;
                return;
            };
            let delta = end.saturating_sub(start);
            let ms = (delta as f64 * self.period_ns / 1_000_000.0) as f32;
            passes.push(GpuPassTime {
                name: name.clone(),
                ms,
            });
        }
        self.batches_received += 1;
        self.latest = Some(GpuPassBatch {
            frame_index: pending.frame_index,
            passes,
        });
    }

    /// Begin recording a frame into `slot`. Returns the query-pool capacity
    /// that must be reset on the slot's command buffer before stamping.
    pub(crate) fn begin_recording(&mut self, slot: usize, frame_index: u64) -> Option<u32> {
        if !self.ready() {
            return None;
        }
        let state = self.slots.get_mut(slot)?;
        // A stale recording (frame aborted without `abort_slot`) is replaced;
        // the pending entry, if any, was consumed by read-back above.
        if state.recording.is_some() {
            self.lost_frames += 1;
        }
        state.recording = Some(RecordingSlot {
            frame_index,
            passes: Vec::new(),
        });
        self.active_slot = Some(slot);
        Some(QUERIES_PER_POOL)
    }

    /// Allocate the start-stamp query for pass `name`. `None` when the pool
    /// is full or no frame is recording.
    pub(crate) fn stamp_start(&mut self, name: &str) -> Option<u32> {
        let slot = self.active_slot?;
        let recording = self.slots.get_mut(slot)?.recording.as_mut()?;
        if recording.passes.len() >= MAX_GPU_PASSES {
            return None;
        }
        let query = 2 * recording.passes.len() as u32;
        recording.passes.push(name.to_string());
        Some(query)
    }

    /// Allocate the end-stamp query paired with the most recent start stamp.
    pub(crate) fn stamp_end(&mut self) -> Option<u32> {
        let slot = self.active_slot?;
        let recording = self.slots.get_mut(slot)?.recording.as_mut()?;
        if recording.passes.is_empty() {
            return None;
        }
        Some(2 * (recording.passes.len() as u32 - 1) + 1)
    }

    /// Move the active slot's recording into the pending (awaiting read-back)
    /// state. Called after the frame's command buffer is submitted.
    pub(crate) fn finish_recording(&mut self) {
        let Some(slot) = self.active_slot.take() else {
            return;
        };
        let Some(state) = self.slots.get_mut(slot) else {
            return;
        };
        if let Some(recording) = state.recording.take() {
            state.pending = Some(PendingSlot {
                frame_index: recording.frame_index,
                passes: recording.passes,
            });
        }
    }

    /// Drop all timestamp state for `slot` (frame aborted). Queries that were
    /// never submitted can never be read back.
    pub(crate) fn abort_slot(&mut self, slot: usize) {
        if let Some(state) = self.slots.get_mut(slot) {
            if state.recording.is_some() || state.pending.is_some() {
                self.lost_frames += 1;
            }
            state.recording = None;
            state.pending = None;
        }
        if self.active_slot == Some(slot) {
            self.active_slot = None;
        }
    }

    /// Take the most recent read-back batch for frame-statistics reporting.
    /// Each batch is reported exactly once.
    pub(crate) fn take_latest(&mut self) -> Option<GpuPassBatch> {
        self.latest.take()
    }
}

/// Vulkan query-pool wrapper: creation, reset, timestamp writes, and
/// non-blocking read-back. Owns no policy; all decisions live in
/// [`GpuTimestampProfiler`].
pub(crate) struct TimestampQueryPools {
    pools: [vk::QueryPool; 2],
    created: bool,
}

impl TimestampQueryPools {
    pub(crate) fn new() -> Self {
        Self {
            pools: [vk::QueryPool::null(); 2],
            created: false,
        }
    }

    /// Lazily create both pools. On failure any partially created pool is
    /// destroyed and the caller degrades the profiler.
    pub(crate) fn ensure_created(&mut self, device: &ash::Device) -> Result<(), vk::Result> {
        if self.created {
            return Ok(());
        }
        let create_info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(QUERIES_PER_POOL);
        for (index, pool) in self.pools.iter_mut().enumerate() {
            // SAFETY: `device` is alive; the create info describes a valid
            // timestamp pool.
            match unsafe { device.create_query_pool(&create_info, None) } {
                Ok(created) => *pool = created,
                Err(result) => {
                    for created in self.pools.iter_mut().take(index) {
                        // SAFETY: pools before `index` were created above.
                        unsafe { device.destroy_query_pool(*created, None) };
                        *created = vk::QueryPool::null();
                    }
                    return Err(result);
                }
            }
        }
        self.created = true;
        Ok(())
    }

    /// Record a pool reset at the start of the frame's command buffer.
    pub(crate) fn cmd_reset(&self, device: &ash::Device, cmd: vk::CommandBuffer, slot: usize) {
        if !self.created {
            return;
        }
        // SAFETY: `cmd` is recording; the pool was created on this device and
        // is not in use by the slot (its fence was waited before reset).
        unsafe {
            device.cmd_reset_query_pool(cmd, self.pools[slot], 0, QUERIES_PER_POOL);
        }
    }

    /// Record a timestamp write into `query`.
    pub(crate) fn cmd_write(
        &self,
        device: &ash::Device,
        cmd: vk::CommandBuffer,
        query: u32,
        slot: usize,
        stage: vk::PipelineStageFlags,
    ) {
        if !self.created {
            return;
        }
        // SAFETY: `cmd` is recording; `query` is within the pool's capacity
        // (guaranteed by GpuTimestampProfiler's pair allocation).
        unsafe {
            device.cmd_write_timestamp(cmd, stage, self.pools[slot], query);
        }
    }

    /// Non-blocking read of the whole pool. Returns `None` when results are
    /// not yet available or the read failed — never stalls.
    pub(crate) fn read(&self, device: &ash::Device, slot: usize) -> Option<Vec<u64>> {
        if !self.created {
            return None;
        }
        let mut data = vec![0u64; QUERIES_PER_POOL as usize];
        // SAFETY: the pool is valid; `data` covers QUERIES_PER_POOL u64s and
        // the WITH_AVAILABILITY flag is not set. Without the WAIT flag this
        // call never blocks.
        unsafe {
            device
                .get_query_pool_results(
                    self.pools[slot],
                    0,
                    &mut data,
                    vk::QueryResultFlags::TYPE_64,
                )
                .ok()?;
        }
        Some(data)
    }

    /// Destroy both pools (device must be idle).
    pub(crate) fn destroy(&mut self, device: &ash::Device) {
        for pool in &mut self.pools {
            if *pool != vk::QueryPool::null() {
                // SAFETY: caller guarantees the device is idle and the pool
                // was created on this device.
                unsafe { device.destroy_query_pool(*pool, None) };
                *pool = vk::QueryPool::null();
            }
        }
        self.created = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_profiler() -> GpuTimestampProfiler {
        let mut profiler = GpuTimestampProfiler::new();
        profiler.configure(
            evaluate_support(true, true, 1.0), // 1 ns per tick
            2,
        );
        profiler
    }

    /// Simulate one recorded frame: begin on `slot`, stamp `passes`, finish.
    fn record_frame(profiler: &mut GpuTimestampProfiler, slot: usize, frame: u64, passes: &[&str]) {
        assert_eq!(
            profiler.begin_recording(slot, frame),
            Some(QUERIES_PER_POOL)
        );
        for pass in passes {
            let start = profiler.stamp_start(pass).expect("start stamp");
            let end = profiler.stamp_end().expect("end stamp");
            assert_eq!(end, start + 1);
        }
        profiler.finish_recording();
    }

    /// Synthesise ticks where pass `i` takes `durations[i]` ticks.
    fn ticks_for(durations: &[u64]) -> Vec<u64> {
        let mut ticks = vec![0u64; QUERIES_PER_POOL as usize];
        let mut clock = 1_000u64;
        for (index, duration) in durations.iter().enumerate() {
            ticks[2 * index] = clock;
            clock += duration;
            ticks[2 * index + 1] = clock;
        }
        ticks
    }

    #[test]
    fn support_evaluation_covers_disabled_and_unsupported_paths() {
        assert_eq!(
            evaluate_support(false, true, 1.0),
            TimestampSupport::Disabled
        );
        assert!(matches!(
            evaluate_support(true, false, 1.0),
            TimestampSupport::Unsupported(_)
        ));
        assert!(matches!(
            evaluate_support(true, true, 0.0),
            TimestampSupport::Unsupported(_)
        ));
        assert!(matches!(
            evaluate_support(true, true, f32::NAN),
            TimestampSupport::Unsupported(_)
        ));
        assert_eq!(
            evaluate_support(true, true, 83.0),
            TimestampSupport::Supported { period_ns: 83.0 }
        );
    }

    #[test]
    fn disabled_and_unsupported_states_report_status_without_recording() {
        let mut profiler = GpuTimestampProfiler::new();
        profiler.configure(evaluate_support(false, true, 1.0), 2);
        assert_eq!(profiler.status(), GpuTimingStatus::Disabled);
        assert_eq!(profiler.begin_recording(0, 0), None);
        assert_eq!(profiler.stamp_start("forward"), None);
        assert_eq!(profiler.readback_len(0), None);

        profiler.configure(evaluate_support(true, false, 1.0), 2);
        assert_eq!(profiler.status(), GpuTimingStatus::Unavailable);
        assert!(profiler.unavailable_reason().is_some());
        assert_eq!(profiler.begin_recording(0, 0), None);
    }

    #[test]
    fn readback_arrives_with_frames_in_flight_delay() {
        let mut profiler = ready_profiler();
        assert_eq!(profiler.status(), GpuTimingStatus::Pending);

        // Frame 0 records on slot 0, frame 1 on slot 1: nothing to read yet.
        record_frame(&mut profiler, 0, 0, &["shadow", "forward"]);
        record_frame(&mut profiler, 1, 1, &["shadow", "forward"]);
        assert_eq!(profiler.take_latest(), None);

        // Frame 2 reuses slot 0: the fence wait guarantees frame 0's results.
        assert_eq!(profiler.readback_len(0), Some(4));
        let ticks = ticks_for(&[500_000, 2_000_000]);
        profiler.deliver_readback(0, Some(&ticks));
        let batch = profiler.take_latest().expect("batch for frame 0");
        assert_eq!(batch.frame_index, 0);
        assert_eq!(batch.passes.len(), 2);
        assert!((batch.passes[0].ms - 0.5).abs() < f32::EPSILON);
        assert!((batch.passes[1].ms - 2.0).abs() < f32::EPSILON);
        assert_eq!(batch.passes[0].name, "shadow");
        assert_eq!(profiler.status(), GpuTimingStatus::Available);

        // The batch is reported exactly once.
        assert_eq!(profiler.take_latest(), None);
    }

    #[test]
    fn timestamp_period_calibrates_ticks_to_nanoseconds() {
        let mut profiler = GpuTimestampProfiler::new();
        profiler.configure(TimestampSupport::Supported { period_ns: 83.333 }, 2);
        record_frame(&mut profiler, 0, 0, &["forward"]);
        // 12_000 ticks * 83.333 ns = 999_996 ns ~= 1.0 ms.
        let ticks = ticks_for(&[12_000]);
        profiler.deliver_readback(0, Some(&ticks));
        let batch = profiler.take_latest().unwrap();
        assert!((batch.passes[0].ms - 1.0).abs() < 0.001);
    }

    #[test]
    fn failed_readback_drops_frame_and_keeps_profiler_healthy() {
        let mut profiler = ready_profiler();
        record_frame(&mut profiler, 0, 0, &["forward"]);
        assert_eq!(profiler.readback_len(0), Some(2));
        profiler.deliver_readback(0, None);
        assert_eq!(profiler.lost_frames(), 1);
        assert_eq!(profiler.take_latest(), None);
        // The slot can record again immediately.
        record_frame(&mut profiler, 0, 2, &["forward"]);
        assert_eq!(profiler.readback_len(0), Some(2));
    }

    #[test]
    fn short_readback_drops_frame() {
        let mut profiler = ready_profiler();
        record_frame(&mut profiler, 0, 0, &["a", "b"]);
        let ticks = ticks_for(&[100]);
        profiler.deliver_readback(0, Some(&ticks[..2]));
        assert_eq!(profiler.lost_frames(), 1);
        assert_eq!(profiler.take_latest(), None);
    }

    #[test]
    fn abort_discards_recording_and_pending_state() {
        let mut profiler = ready_profiler();
        record_frame(&mut profiler, 0, 0, &["forward"]);
        profiler.abort_slot(0);
        assert_eq!(profiler.readback_len(0), None);
        assert_eq!(profiler.lost_frames(), 1);
        // Aborted frame leaves no batch behind.
        profiler.deliver_readback(0, Some(&ticks_for(&[10])));
        assert_eq!(profiler.take_latest(), None);
    }

    #[test]
    fn pass_capacity_is_bounded() {
        let mut profiler = ready_profiler();
        assert!(profiler.begin_recording(0, 0).is_some());
        for index in 0..MAX_GPU_PASSES {
            assert!(profiler.stamp_start("p").is_some(), "pass {index}");
            assert!(profiler.stamp_end().is_some());
        }
        assert_eq!(profiler.stamp_start("overflow"), None);
        // Unbalanced stamp_end without a start is rejected.
        let mut fresh = ready_profiler();
        assert_eq!(fresh.stamp_end(), None);
    }

    #[test]
    fn unbalanced_stamp_end_is_rejected() {
        let mut profiler = ready_profiler();
        assert!(profiler.begin_recording(0, 0).is_some());
        assert_eq!(profiler.stamp_end(), None);
    }

    #[test]
    fn stale_recording_is_replaced_on_slot_reuse() {
        let mut profiler = ready_profiler();
        assert!(profiler.begin_recording(0, 0).is_some());
        profiler.stamp_start("forward");
        // Slot reused without finish/abort (e.g. end_frame failure path).
        assert!(profiler.begin_recording(0, 1).is_some());
        assert_eq!(profiler.lost_frames(), 1);
        profiler.finish_recording();
        assert_eq!(profiler.readback_len(0), Some(0));
    }
}
