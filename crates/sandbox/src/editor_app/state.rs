use super::*;

pub(super) struct EditorBackgroundJob {
    pub(super) id: u64,
    pub(super) label: String,
    pub(super) receiver: mpsc::Receiver<Result<EditorJobOutput, String>>,
    pub(super) reload_assets: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum EditorJobOutput {
    #[default]
    None,
    SelectAsset(String),
    SelectFolder(String),
    ClearAssetSelection,
}

#[derive(Clone, Debug)]
pub(super) enum EditorOperationState {
    Running,
    Succeeded,
    CommittedWithWarning(String),
    Failed(String),
}

#[derive(Clone, Debug)]
pub(super) struct EditorOperationStatus {
    pub(super) id: u64,
    pub(super) label: String,
    pub(super) state: EditorOperationState,
}

#[derive(Default)]
pub(super) struct WebViewportInputState {
    pub(super) pointer_id: Option<i64>,
    pub(super) pointer: Option<Vec2>,
    pub(super) buttons: u16,
    pub(super) modifiers: InputModifiers,
    pub(super) keys: BTreeSet<String>,
    pub(super) focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SceneDocumentAction {
    Open(String),
    Create {
        scene_id: String,
        folder: PathBuf,
    },
    SaveAs(String),
    Duplicate {
        source_id: String,
        new_id: String,
    },
    SetStartup(String),
    Rename {
        old_id: String,
        new_id: String,
    },
    Delete {
        scene_id: String,
        replacement_startup: Option<String>,
    },
    CancelSwitch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CloseDocumentAction {
    SaveAndClose,
    DiscardAndClose,
    Cancel,
}

#[derive(Clone, Debug)]
pub(super) struct ProjectSettingsDraft {
    pub(super) title: String,
    pub(super) width: u32,
    pub(super) height: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ViewportTab {
    #[default]
    Scene,
    Game,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditorFrameOutcome {
    Completed,
    Failed,
}
