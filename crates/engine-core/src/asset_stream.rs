//! Background cooked-asset decoding with frame-budgeted additive commits.
//!
//! [`AssetStreamLoader`] owns a single worker thread that decodes and
//! structurally validates cooked artifacts via
//! [`decode_cooked_batch`](crate::decode_cooked_batch) — pure owned data, so
//! it crosses the thread boundary safely. The main thread drains finished
//! batches at the frame boundary and commits at most a configurable number of
//! assets per call through the additive install path, so per-frame hitch is
//! bounded while remaining work stays queued.

use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use engine_renderer::AssetId;
use engine_scene::registry::AssetTypeRegistry;
use engine_serialize::Diagnostic;

use crate::cooked_assets::{
    additive_conflict_error, cooked_asset_id, decode_cooked_batch, material_texture_available,
    missing_texture_error, DecodedBatch, DecodedCookedAsset, InstallPlan,
};
use crate::EngineRuntime;

/// Default maximum number of streamed assets committed per drain. Registry
/// inserts are cheap; the bound exists so a future streaming driver can keep
/// the frame-boundary commit slice predictable even for large cells.
pub const DEFAULT_STREAM_COMMIT_BUDGET: usize = 8;

struct StreamJob {
    paths: Vec<PathBuf>,
    ids: Vec<AssetId>,
}

struct StreamOutcome {
    ids: Vec<AssetId>,
    result: Result<DecodedBatch, Vec<Diagnostic>>,
}

struct QueuedItem {
    batch_seq: u64,
    asset: DecodedCookedAsset,
}

/// Outcome of one [`EngineRuntime::drain_cooked_asset_stream`] call.
///
/// Failures are reported per batch instead of aborting the drain: a batch
/// that fails to decode or commit is discarded (assets it already installed
/// in earlier drains stay installed; every other batch is unaffected) and its
/// diagnostics appear here and in the runtime diagnostics collector.
#[derive(Clone, Debug, Default)]
pub struct StreamDrainReport {
    /// Assets installed during this drain.
    pub committed: usize,
    /// Assets skipped because an identical payload was already installed.
    pub identical: usize,
    /// Batches discarded during this drain (decode or commit failure).
    pub failed_batches: usize,
    /// Decoded assets still queued for a later drain.
    pub remaining: usize,
    /// Batches still decoding on the worker thread.
    pub decoding: usize,
    /// Per-batch failure diagnostics produced since the previous drain.
    pub diagnostics: Vec<Diagnostic>,
}

impl StreamDrainReport {
    /// Nothing left to decode or commit.
    pub fn is_complete(&self) -> bool {
        self.remaining == 0 && self.decoding == 0
    }

    /// No batch failed during this drain.
    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// Worker-thread decode queue for incremental cooked-asset streaming.
///
/// Construct through [`EngineRuntime`]'s streaming entry points
/// (`enqueue_cooked_asset_stream` / `drain_cooked_asset_stream`); the runtime
/// owns one lazily created instance. All heavy work (file I/O, decode,
/// structural validation) happens on the worker; the main thread only
/// reaps outcomes and performs budgeted additive commits.
pub struct AssetStreamLoader {
    jobs: Option<Sender<StreamJob>>,
    outcomes: Receiver<StreamOutcome>,
    worker: Option<JoinHandle<()>>,
    /// Decoded assets awaiting commit, flattened in commit order.
    commit_queue: VecDeque<QueuedItem>,
    /// Asset IDs enqueued and not yet committed or cleared.
    in_flight_ids: BTreeSet<AssetId>,
    /// Jobs sent to the worker whose outcome has not been reaped yet.
    decoding_batches: usize,
    next_batch_seq: u64,
    budget: usize,
}

impl AssetStreamLoader {
    /// Spawn the decode worker with the default commit budget.
    ///
    /// The asset type registry is cloned into the worker; it contains only
    /// metadata and function pointers, so it is `Send` and decode behaviour
    /// is frozen at construction time.
    ///
    /// # Panics
    ///
    /// Panics if the OS refuses to spawn the worker thread.
    pub fn new(asset_type_registry: &AssetTypeRegistry) -> Self {
        Self::with_commit_budget(asset_type_registry, DEFAULT_STREAM_COMMIT_BUDGET)
    }

    /// Spawn the decode worker with an explicit per-drain commit budget.
    /// A budget of zero is clamped to one so queued work always progresses.
    ///
    /// # Panics
    ///
    /// Panics if the OS refuses to spawn the worker thread.
    pub fn with_commit_budget(asset_type_registry: &AssetTypeRegistry, budget: usize) -> Self {
        let registry = asset_type_registry.clone();
        let (job_tx, job_rx) = mpsc::channel::<StreamJob>();
        let (outcome_tx, outcome_rx) = mpsc::channel::<StreamOutcome>();
        let worker = thread::Builder::new()
            .name("engine-asset-stream".to_string())
            .spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let outcome = StreamOutcome {
                        ids: job.ids,
                        result: decode_cooked_batch(&job.paths, &registry),
                    };
                    if outcome_tx.send(outcome).is_err() {
                        break;
                    }
                }
            })
            .expect("failed to spawn the asset stream decode thread");
        Self {
            jobs: Some(job_tx),
            outcomes: outcome_rx,
            worker: Some(worker),
            commit_queue: VecDeque::new(),
            in_flight_ids: BTreeSet::new(),
            decoding_batches: 0,
            next_batch_seq: 0,
            budget: budget.max(1),
        }
    }

    /// Maximum number of assets committed per drain.
    pub fn commit_budget(&self) -> usize {
        self.budget
    }

    /// Change the per-drain commit budget. Zero is clamped to one.
    pub fn set_commit_budget(&mut self, budget: usize) {
        self.budget = budget.max(1);
    }

    /// Asset IDs still decoding on the worker or queued for commit.
    pub fn pending(&self) -> usize {
        self.in_flight_ids.len()
    }

    fn enqueue(&mut self, paths: Vec<PathBuf>, ids: Vec<AssetId>) -> Option<StreamOutcome> {
        self.in_flight_ids.extend(ids.iter().cloned());
        self.decoding_batches += 1;
        let sender = self.jobs.as_ref().expect("job channel exists until drop");
        match sender.send(StreamJob { paths, ids }) {
            Ok(()) => None,
            Err(error) => {
                // The worker thread is gone; surface the job as failed on the
                // next drain instead of losing it silently.
                self.decoding_batches -= 1;
                let job = error.0;
                Some(StreamOutcome {
                    ids: job.ids,
                    result: Err(vec![Diagnostic::new(
                        "AS0002",
                        engine_serialize::DiagnosticSeverity::Error,
                        "engine-core.cooked-assets",
                        "asset stream decode worker is not running",
                    )]),
                })
            }
        }
    }
}

impl Drop for AssetStreamLoader {
    fn drop(&mut self) {
        // Disconnect the job channel so the worker exits after its current
        // decode, then wait for it to finish.
        drop(self.jobs.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl EngineRuntime {
    fn stream_loader_mut(&mut self) -> &mut AssetStreamLoader {
        if self.stream_loader.is_none() {
            let registry = self.asset_type_registry.clone();
            self.stream_loader = Some(AssetStreamLoader::with_commit_budget(
                &registry,
                self.stream_budget,
            ));
        }
        self.stream_loader
            .as_mut()
            .expect("stream loader initialised above")
    }

    /// Enqueue cooked artifacts for background decode and additive install.
    ///
    /// Each artifact's asset ID is marked
    /// [`engine_asset::AssetState::Loading`] until it commits or its batch
    /// fails. The worker decodes the whole list as one batch: if any path
    /// fails to decode, none of the batch's assets install. Returns the
    /// number of paths accepted. Call
    /// [`drain_cooked_asset_stream`](Self::drain_cooked_asset_stream) at the
    /// frame boundary to commit finished work.
    pub fn enqueue_cooked_asset_stream(&mut self, paths: Vec<PathBuf>) -> usize {
        let ids = paths
            .iter()
            .filter_map(|path| cooked_asset_id(path).ok())
            .collect::<Vec<_>>();
        for id in &ids {
            self.asset_registry.mark_loading(id.clone());
        }
        let accepted = paths.len();
        if let Some(failed) = self.stream_loader_mut().enqueue(paths, ids) {
            self.reap_stream_outcome(failed);
        }
        accepted
    }

    /// Poll the background stream: reap finished decodes and commit at most
    /// [`cooked_asset_stream_budget`](Self::cooked_asset_stream_budget)
    /// assets through the additive install path. Intended to be called once
    /// per frame, at the frame boundary; with nothing in flight it is a cheap
    /// no-op.
    pub fn drain_cooked_asset_stream(&mut self) -> StreamDrainReport {
        let mut report = StreamDrainReport::default();
        let Some(mut loader) = self.stream_loader.take() else {
            return report;
        };

        // Reap every finished decode. Successful batches join the commit
        // queue in FIFO order; failed batches clear their loading marks.
        while let Ok(outcome) = loader.outcomes.try_recv() {
            loader.decoding_batches -= 1;
            match outcome.result {
                Ok(batch) => {
                    let seq = loader.next_batch_seq;
                    loader.next_batch_seq += 1;
                    loader
                        .commit_queue
                        .extend(
                            batch
                                .into_commit_order()
                                .into_iter()
                                .map(|asset| QueuedItem {
                                    batch_seq: seq,
                                    asset,
                                }),
                        );
                }
                Err(diagnostics) => {
                    report.failed_batches += 1;
                    report.diagnostics.extend(diagnostics);
                    for id in &outcome.ids {
                        loader.in_flight_ids.remove(id);
                        self.asset_registry.unmark_loading(id);
                    }
                }
            }
        }

        // Budgeted additive commit. Per-item validation runs against the live
        // registry at commit time, so interleaved registry changes (other
        // batches, replace loads, editor uploads) can never invalidate an
        // already-checked plan.
        let mut processed = 0;
        while processed < loader.budget {
            let Some(item) = loader.commit_queue.pop_front() else {
                break;
            };
            match self.validate_stream_item(&item.asset) {
                Ok(InstallPlan::Conflict) => {
                    unreachable!("validate_stream_item reports conflicts as Err")
                }
                Ok(InstallPlan::Install) => {
                    let id = item.asset.asset_id().clone();
                    self.install_decoded_item(item.asset);
                    report.committed += 1;
                    processed += 1;
                    loader.in_flight_ids.remove(&id);
                    self.asset_registry.unmark_loading(&id);
                }
                Ok(InstallPlan::NoOp) => {
                    report.identical += 1;
                    processed += 1;
                    loader.in_flight_ids.remove(item.asset.asset_id());
                    self.asset_registry.unmark_loading(item.asset.asset_id());
                }
                Err(diagnostic) => {
                    report.failed_batches += 1;
                    report.diagnostics.push(*diagnostic);
                    loader.in_flight_ids.remove(item.asset.asset_id());
                    self.asset_registry.unmark_loading(item.asset.asset_id());
                    // Discard the remainder of the failed batch; assets it
                    // already committed stay installed.
                    let failed_seq = item.batch_seq;
                    while let Some(front) = loader.commit_queue.front() {
                        if front.batch_seq != failed_seq {
                            break;
                        }
                        let discarded = loader.commit_queue.pop_front().expect("front item exists");
                        loader.in_flight_ids.remove(discarded.asset.asset_id());
                        self.asset_registry
                            .unmark_loading(discarded.asset.asset_id());
                    }
                }
            }
        }

        report.remaining = loader.commit_queue.len();
        report.decoding = loader.decoding_batches;
        self.stream_loader = Some(loader);
        if !report.diagnostics.is_empty() {
            self.collector.push_asset_diags(report.diagnostics.clone());
        }
        report
    }

    fn reap_stream_outcome(&mut self, outcome: StreamOutcome) {
        for id in &outcome.ids {
            self.asset_registry.unmark_loading(id);
        }
        if let Err(diagnostics) = outcome.result {
            self.collector.push_asset_diags(diagnostics);
        }
    }

    /// Validate one queued streamed asset against the live registry:
    /// material → texture references must resolve now (same-batch textures
    /// commit earlier in commit order) and the additive conflict rules apply.
    fn validate_stream_item(
        &self,
        asset: &DecodedCookedAsset,
    ) -> Result<InstallPlan, Box<Diagnostic>> {
        let (id, kind, plan) = match asset {
            DecodedCookedAsset::Texture(upload) => (
                &upload.texture_id,
                "texture",
                self.additive_typed_plan(&upload.texture_id, upload),
            ),
            DecodedCookedAsset::Material(path, upload) => {
                if let Some(texture_id) = upload.base_color_texture.as_ref() {
                    let available = material_texture_available(
                        self,
                        &BTreeSet::new(),
                        &BTreeSet::new(),
                        texture_id,
                    );
                    if !available {
                        return Err(Box::new(missing_texture_error(
                            path,
                            &upload.material_id,
                            texture_id,
                        )));
                    }
                }
                (
                    &upload.material_id,
                    "material",
                    self.additive_typed_plan(&upload.material_id, upload),
                )
            }
            DecodedCookedAsset::Mesh(upload) => (
                &upload.mesh_id,
                "mesh",
                self.additive_typed_plan(&upload.mesh_id, upload),
            ),
            DecodedCookedAsset::Extension(extension) => (
                &extension.id,
                extension.type_id.as_str(),
                self.additive_extension_plan(extension),
            ),
            DecodedCookedAsset::Skipped(_) => {
                unreachable!("skipped artifacts are never queued for commit")
            }
        };
        if plan == InstallPlan::Conflict {
            return Err(Box::new(additive_conflict_error(id, kind)));
        }
        Ok(plan)
    }

    /// Asset IDs still decoding on the worker or queued for commit.
    pub fn cooked_asset_stream_pending(&self) -> usize {
        self.stream_loader
            .as_ref()
            .map_or(0, AssetStreamLoader::pending)
    }

    /// Maximum number of streamed assets committed per drain.
    pub fn cooked_asset_stream_budget(&self) -> usize {
        self.stream_loader
            .as_ref()
            .map_or(self.stream_budget, AssetStreamLoader::commit_budget)
    }

    /// Set the per-drain commit budget for streamed assets. Zero is clamped
    /// to one. Applies to the lazily created worker as well as to a worker
    /// that is already running.
    pub fn set_cooked_asset_stream_budget(&mut self, budget: usize) {
        self.stream_budget = budget.max(1);
        if let Some(loader) = &mut self.stream_loader {
            loader.set_commit_budget(budget);
        }
    }
}
