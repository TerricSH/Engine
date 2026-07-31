use engine_serialize::PersistentId;

/// Default enter factor: the camera must be inside `bounds * 1.0`.
pub const DEFAULT_ENTER_FACTOR: f32 = 1.0;
/// Default exit factor: a loaded cell stays until the camera leaves
/// `bounds * 1.15`.
pub const DEFAULT_EXIT_FACTOR: f32 = 1.15;
/// Default maximum number of cell merges committed per frame-boundary tick.
pub const DEFAULT_MAX_MERGES_PER_COMMIT: usize = 1;
/// Default maximum number of cell unloads committed per frame-boundary tick.
pub const DEFAULT_MAX_UNLOADS_PER_COMMIT: usize = 4;

/// Tunables of a [`crate::cell_stream::CellStreamingDriver`].
///
/// Hysteresis: an unloaded cell becomes desired when the camera enters
/// `bounds * enter_factor`; a loaded (or loading) cell stops being desired
/// only when the camera leaves `bounds * exit_factor`. Keeping
/// `exit_factor >= enter_factor` prevents boundary ping-ponging.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellStreamingConfig {
    /// Bounds scale at which an unloaded cell becomes desired. Must be
    /// finite and greater than zero.
    pub enter_factor: f32,
    /// Bounds scale at which a loaded cell stops being desired. Must be
    /// finite and not smaller than `enter_factor`.
    pub exit_factor: f32,
    /// Maximum cell merges committed per tick. Zero is clamped to one.
    pub max_merges_per_commit: usize,
    /// Maximum cell unloads committed per tick. Zero is clamped to one.
    pub max_unloads_per_commit: usize,
}

impl Default for CellStreamingConfig {
    fn default() -> Self {
        Self {
            enter_factor: DEFAULT_ENTER_FACTOR,
            exit_factor: DEFAULT_EXIT_FACTOR,
            max_merges_per_commit: DEFAULT_MAX_MERGES_PER_COMMIT,
            max_unloads_per_commit: DEFAULT_MAX_UNLOADS_PER_COMMIT,
        }
    }
}

impl CellStreamingConfig {
    pub(super) fn validated(self) -> Result<Self, CellStreamError> {
        if !self.enter_factor.is_finite() || self.enter_factor <= 0.0 {
            return Err(CellStreamError::InvalidConfig(format!(
                "enter_factor must be finite and greater than zero, got {}",
                self.enter_factor
            )));
        }
        if !self.exit_factor.is_finite() || self.exit_factor < self.enter_factor {
            return Err(CellStreamError::InvalidConfig(format!(
                "exit_factor must be finite and not smaller than enter_factor ({}), got {}",
                self.enter_factor, self.exit_factor
            )));
        }
        Ok(Self {
            max_merges_per_commit: self.max_merges_per_commit.max(1),
            max_unloads_per_commit: self.max_unloads_per_commit.max(1),
            ..self
        })
    }
}

/// Streaming state of one partition cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellState {
    /// Not in the world and no load in flight.
    Unloaded,
    /// Cooked assets are decoding/committing on the background stream.
    LoadingAssets,
    /// Assets are committed; the scene merge waits for the commit budget.
    Merging,
    /// Merged into the live world.
    Loaded,
    /// Queued for destruction at the commit budget.
    Unloading,
    /// Terminal error state until the next rebaseline; carries the reason.
    Failed(String),
}

/// Failure returned when constructing a
/// [`crate::cell_stream::CellStreamingDriver`] or validating
/// partition cell scenes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellStreamError {
    /// The hysteresis factors or budgets are invalid.
    InvalidConfig(String),
    /// A cell references a scene that is not in the project catalog.
    UnknownCellScene { cell_id: String, scene_id: String },
    /// A cell scene file could not be read or parsed.
    CellSceneLoad {
        cell_id: String,
        scene_id: String,
        message: String,
    },
    /// A cell scene failed the standard scene validation.
    CellSceneInvalid {
        cell_id: String,
        scene_id: String,
        messages: Vec<String>,
    },
    /// A cell scene contains an `engine.script` component (forbidden in v1).
    ScriptComponentInCell { cell_id: String, entity_id: String },
    /// Two cells contain the same persistent entity ID.
    DuplicatePersistentIdAcrossCells {
        first_cell: String,
        second_cell: String,
        persistent_id: PersistentId,
    },
    /// A cell that does not reference the startup scene shares persistent
    /// entity IDs with it.
    StartupSceneIdConflict {
        cell_id: String,
        persistent_id: PersistentId,
        startup_scene_id: String,
    },
}

impl std::fmt::Display for CellStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(reason) => {
                write!(f, "invalid cell streaming configuration: {reason}")
            }
            Self::UnknownCellScene { cell_id, scene_id } => write!(
                f,
                "world partition cell \"{cell_id}\" references scene \"{scene_id}\", which is not in the project scene catalog"
            ),
            Self::CellSceneLoad {
                cell_id,
                scene_id,
                message,
            } => write!(
                f,
                "world partition cell \"{cell_id}\" scene \"{scene_id}\" could not be loaded: {message}"
            ),
            Self::CellSceneInvalid {
                cell_id,
                scene_id,
                messages,
            } => write!(
                f,
                "world partition cell \"{cell_id}\" scene \"{scene_id}\" is invalid:\n{}",
                messages.join("\n")
            ),
            Self::ScriptComponentInCell { cell_id, entity_id } => write!(
                f,
                "world partition cell \"{cell_id}\" entity \"{entity_id}\" has an engine.script component; scripts in partition cells are not supported (attach scripts in the startup scene instead)"
            ),
            Self::DuplicatePersistentIdAcrossCells {
                first_cell,
                second_cell,
                persistent_id,
            } => write!(
                f,
                "world partition cells \"{first_cell}\" and \"{second_cell}\" both contain persistent entity id \"{persistent_id}\"; cell scene entity ids must be unique across cells"
            ),
            Self::StartupSceneIdConflict {
                cell_id,
                persistent_id,
                startup_scene_id,
            } => write!(
                f,
                "world partition cell \"{cell_id}\" shares persistent entity id \"{persistent_id}\" with startup scene \"{startup_scene_id}\"; a cell that reuses startup content must reference the startup scene itself"
            ),
        }
    }
}

impl std::error::Error for CellStreamError {}
