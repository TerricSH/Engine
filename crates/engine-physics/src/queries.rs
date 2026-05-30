use crate::components::ColliderShape;
use crate::Entity;
use glam::Vec3;

/// A raycast query that can be performed against the physics world.
///
/// Safe to hold across frames — does not reference backend state.
#[derive(Clone, Debug, PartialEq)]
pub struct RaycastQuery {
    pub origin: Vec3,
    pub direction: Vec3,
    pub max_distance: f32,
}

impl RaycastQuery {
    pub fn new(origin: Vec3, direction: Vec3, max_distance: f32) -> Self {
        Self {
            origin,
            direction,
            max_distance,
        }
    }
}

/// An overlap (proximity) query.
///
/// Safe to hold across frames.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlapQuery {
    pub shape: ColliderShape,
    pub position: Vec3,
}

impl OverlapQuery {
    pub fn new(shape: ColliderShape, position: Vec3) -> Self {
        Self { shape, position }
    }
}

/// A sweep (cast shape) query.
///
/// Safe to hold across frames.
#[derive(Clone, Debug, PartialEq)]
pub struct SweepQuery {
    pub shape: ColliderShape,
    pub from: Vec3,
    pub to: Vec3,
}

impl SweepQuery {
    pub fn new(shape: ColliderShape, from: Vec3, to: Vec3) -> Self {
        Self { shape, from, to }
    }
}

/// Result of a single raycast hit with full intersection data.
#[derive(Clone, Debug, PartialEq)]
pub struct RaycastHitResult {
    pub entity: Entity,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

/// Result of a single overlap (proximity) hit.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlapHitResult {
    pub entity: Entity,
}

/// Result of a single sweep (shape cast) hit with full intersection data.
#[derive(Clone, Debug, PartialEq)]
pub struct SweepHitResult {
    pub entity: Entity,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

/// Collected results from one or more batched queries.
///
/// The `hits` field contains a flat list of **all** entities hit by any
/// queued query.  The `*_details` fields contain per-query detailed results
/// indexed in the same order as the queries were pushed into the batcher.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueryResults {
    /// Flat list of all entity hits across all queries.
    pub hits: Vec<Entity>,
    /// Per-query detailed raycast results (one entry per queued raycast).
    pub raycast_details: Vec<Vec<RaycastHitResult>>,
    /// Per-query detailed overlap results (one entry per queued overlap).
    pub overlap_details: Vec<Vec<OverlapHitResult>>,
    /// Per-query detailed sweep results (one entry per queued sweep).
    pub sweep_details: Vec<Vec<SweepHitResult>>,
}

impl QueryResults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    pub fn len(&self) -> usize {
        self.hits.len()
    }
}

/// Batched query dispatcher.
///
/// Collects raycast, overlap, and sweep queries and executes them all at
/// once via [`crate::backend::RapierBackend::execute_batched_queries`].
#[derive(Clone, Debug, Default)]
pub struct QueryBatcher {
    pub raycasts: Vec<RaycastQuery>,
    pub overlaps: Vec<OverlapQuery>,
    pub sweeps: Vec<SweepQuery>,
}

impl QueryBatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.raycasts.is_empty() && self.overlaps.is_empty() && self.sweeps.is_empty()
    }

    pub fn clear(&mut self) {
        self.raycasts.clear();
        self.overlaps.clear();
        self.sweeps.clear();
    }

    pub fn push_raycast(&mut self, query: RaycastQuery) {
        self.raycasts.push(query);
    }

    pub fn push_overlap(&mut self, query: OverlapQuery) {
        self.overlaps.push(query);
    }

    pub fn push_sweep(&mut self, query: SweepQuery) {
        self.sweeps.push(query);
    }
}
