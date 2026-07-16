use engine_scene::Scene;

/// Current state of the editor's isolated game simulation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorPlayMode {
    #[default]
    Editing,
    Playing,
    Paused,
}

/// Owns the authoring-scene snapshot used to isolate Play mode.
///
/// The runtime loader is supplied by the host so state transitions only
/// become visible after the runtime has accepted the requested scene.  A
/// failed Play or Stop therefore leaves both the current mode and snapshot
/// unchanged.
#[derive(Clone, Debug, Default)]
pub struct EditorPlaySession {
    mode: EditorPlayMode,
    authoring_snapshot: Option<Scene>,
}

impl EditorPlaySession {
    pub fn mode(&self) -> EditorPlayMode {
        self.mode
    }

    pub fn is_editing(&self) -> bool {
        self.mode == EditorPlayMode::Editing
    }

    pub fn should_tick(&self) -> bool {
        self.mode == EditorPlayMode::Playing
    }

    /// Start Play mode from an in-memory authoring scene.
    ///
    /// Returns `Ok(false)` when already in Play/Pause mode.
    pub fn start<E>(
        &mut self,
        authoring_scene: &Scene,
        load_runtime_scene: impl FnOnce(Scene) -> Result<(), E>,
    ) -> Result<bool, E> {
        if !self.is_editing() {
            return Ok(false);
        }

        let snapshot = authoring_scene.clone();
        load_runtime_scene(snapshot.clone())?;
        self.authoring_snapshot = Some(snapshot);
        self.mode = EditorPlayMode::Playing;
        Ok(true)
    }

    pub fn pause(&mut self) -> bool {
        if self.mode != EditorPlayMode::Playing {
            return false;
        }
        self.mode = EditorPlayMode::Paused;
        true
    }

    pub fn resume(&mut self) -> bool {
        if self.mode != EditorPlayMode::Paused {
            return false;
        }
        self.mode = EditorPlayMode::Playing;
        true
    }

    /// Stop simulation and restore the exact authoring snapshot captured by
    /// [`start`](Self::start).
    ///
    /// Returns `Ok(false)` while already editing.
    pub fn stop<E>(
        &mut self,
        load_runtime_scene: impl FnOnce(Scene) -> Result<(), E>,
    ) -> Result<bool, E> {
        if self.is_editing() {
            return Ok(false);
        }

        let snapshot = self
            .authoring_snapshot
            .as_ref()
            .expect("non-editing mode always owns an authoring snapshot")
            .clone();
        load_runtime_scene(snapshot)?;
        self.authoring_snapshot = None;
        self.mode = EditorPlayMode::Editing;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(name: &str) -> Scene {
        let mut scene = engine_scene::sample_scene();
        scene.name = name.to_string();
        scene
    }

    #[test]
    fn play_pause_resume_stop_restores_authoring_snapshot() {
        let authoring = scene("Unsaved authoring scene");
        let mut loaded = None;
        let mut session = EditorPlaySession::default();

        assert!(session
            .start(&authoring, |scene| {
                loaded = Some(scene);
                Ok::<_, ()>(())
            })
            .unwrap());
        assert_eq!(session.mode(), EditorPlayMode::Playing);
        assert!(session.should_tick());
        assert_eq!(loaded.as_ref(), Some(&authoring));

        assert!(session.pause());
        assert_eq!(session.mode(), EditorPlayMode::Paused);
        assert!(!session.should_tick());
        assert!(session.resume());

        loaded = Some(scene("Runtime-mutated scene"));
        assert!(session
            .stop(|scene| {
                loaded = Some(scene);
                Ok::<_, ()>(())
            })
            .unwrap());
        assert_eq!(session.mode(), EditorPlayMode::Editing);
        assert_eq!(loaded.as_ref(), Some(&authoring));
    }

    #[test]
    fn failed_runtime_load_does_not_change_session_state() {
        let authoring = scene("Authoring");
        let mut session = EditorPlaySession::default();

        assert_eq!(
            session.start(&authoring, |_| Err("invalid")),
            Err("invalid")
        );
        assert_eq!(session.mode(), EditorPlayMode::Editing);

        session.start(&authoring, |_| Ok::<_, ()>(())).unwrap();
        assert_eq!(
            session.stop(|_| Err("restore failed")),
            Err("restore failed")
        );
        assert_eq!(session.mode(), EditorPlayMode::Playing);
        assert!(session.should_tick());
    }

    #[test]
    fn invalid_transitions_are_no_ops() {
        let mut session = EditorPlaySession::default();
        assert!(!session.pause());
        assert!(!session.resume());
        assert!(!session.stop(|_| Ok::<_, ()>(())).unwrap());

        session
            .start(&scene("Authoring"), |_| Ok::<_, ()>(()))
            .unwrap();
        assert!(!session.start(&scene("Other"), |_| Ok::<_, ()>(())).unwrap());
    }
}
