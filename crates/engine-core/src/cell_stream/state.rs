use std::collections::BTreeSet;

use engine_asset::partition::CellBounds;
use engine_scene::Scene;
use engine_serialize::{AssetId, PersistentId};

use super::CellState;

/// Outcome of one [`crate::cell_stream::CellStreamingDriver::tick`] call.
#[derive(Clone, Debug, Default)]
pub struct CellStreamTickReport {
    /// Camera position used for the desired-set computation, if any.
    pub camera: Option<[f32; 3]>,
    /// Cells the hysteresis evaluation wants live right now.
    pub desired_cells: BTreeSet<String>,
    /// Cooked assets enqueued on the background stream during this tick.
    pub enqueued_assets: usize,
    /// Cells whose scene merge was committed during this tick.
    pub merged_cells: Vec<String>,
    /// Cells whose unload was committed during this tick.
    pub unloaded_cells: Vec<String>,
    /// Cells that entered the failed state during this tick.
    pub failed_cells: Vec<String>,
    /// Persistent IDs that joined the resident set during this tick.
    pub resident_ids_added: Vec<PersistentId>,
}

impl CellStreamTickReport {
    /// The live world changed (a merge or an unload was committed), so the
    /// host should re-synchronise derived state such as the physics world.
    pub fn world_changed(&self) -> bool {
        !self.merged_cells.is_empty() || !self.unloaded_cells.is_empty()
    }
}

pub(super) struct CellRecord {
    pub(super) bounds: CellBounds,
    pub(super) scene: Scene,
    pub(super) state: CellState,
    /// Asset IDs enqueued for this cell and not yet observed as installed.
    pub(super) pending_assets: BTreeSet<AssetId>,
    /// Persistent IDs currently live in the world from this cell's merges.
    pub(super) merged_ids: Vec<PersistentId>,
    /// Cell scene hierarchy roots: records with no parent or with a parent
    /// outside the cell scene's own ID set.
    pub(super) root_ids: Vec<PersistentId>,
}
