use thiserror::Error;

use crate::EditorError;

#[derive(Debug, Error)]
pub enum PrefabAuthoringError {
    #[error("invalid prefab request: {0}")]
    InvalidRequest(String),
    #[error("invalid prefab data: {0}")]
    InvalidPrefab(String),
    #[error("prefab asset is not loaded: {0}")]
    AssetNotLoaded(String),
    #[error("prefab source I/O failed: {0}")]
    Io(String),
    #[error("prefab manifest failed: {0}")]
    Manifest(String),
    #[error(transparent)]
    Editor(#[from] EditorError),
}

pub(super) fn join_validation_errors(errors: Vec<engine_scene::PrefabValidationError>) -> String {
    errors
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}
