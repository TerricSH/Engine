use thiserror::Error;

use crate::{XrActionSnapshot, XrFrameState, XrSwapchainImage};

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum XrError {
    #[error("XR runtime is unavailable: {0}")]
    RuntimeUnavailable(String),
    #[error("XR runtime error: {0}")]
    Runtime(String),
    #[error("XR frame lifecycle violation: {0}")]
    FrameLifecycle(&'static str),
    #[error("XR graphics bridge error: {0}")]
    Graphics(String),
}

/// Render-backend contract for acquiring, rendering and releasing stereo
/// compositor images.
pub trait XrCompositor: Send {
    fn begin_frame(&mut self, state: &XrFrameState) -> Result<Vec<XrSwapchainImage>, XrError>;
    fn end_frame(
        &mut self,
        state: &XrFrameState,
        rendered_images: &[XrSwapchainImage],
    ) -> Result<(), XrError>;
}

/// Runtime lifecycle consumed by the engine loop. OpenXR and deterministic
/// test runtimes implement the same wait/begin/locate/sync/end contract.
pub trait XrRuntime: Send {
    fn poll_events(&mut self) -> Result<(), XrError>;
    fn is_session_running(&self) -> bool {
        true
    }
    fn wait_frame(&mut self) -> Result<XrFrameState, XrError>;
    fn begin_frame(&mut self) -> Result<(), XrError>;
    fn sync_actions(&mut self) -> Result<XrActionSnapshot, XrError>;
    /// Locate predicted head, hand and stereo view poses after `begin_frame`.
    /// OpenXR implementations use the display time returned by `wait_frame`.
    fn locate_frame(&mut self, _state: &mut XrFrameState) -> Result<(), XrError> {
        Ok(())
    }
    fn end_frame(&mut self, rendered: bool) -> Result<(), XrError>;
    fn should_exit(&self) -> bool;
}

#[derive(Default)]
pub struct XrSystem {
    runtime: Option<Box<dyn XrRuntime>>,
    compositor: Option<Box<dyn XrCompositor>>,
    current_frame: Option<XrFrameState>,
    actions: XrActionSnapshot,
}

impl XrSystem {
    pub fn install(&mut self, runtime: Box<dyn XrRuntime>, compositor: Box<dyn XrCompositor>) {
        self.runtime = Some(runtime);
        self.compositor = Some(compositor);
        self.current_frame = None;
    }

    pub fn is_active(&self) -> bool {
        self.runtime.is_some() && self.compositor.is_some()
    }

    pub fn actions(&self) -> &XrActionSnapshot {
        &self.actions
    }

    pub fn frame(&self) -> Option<&XrFrameState> {
        self.current_frame.as_ref()
    }

    pub fn tick(&mut self) -> Result<Option<Vec<XrSwapchainImage>>, XrError> {
        if self.current_frame.is_some() {
            return Err(XrError::FrameLifecycle(
                "tick called before the previous frame was submitted",
            ));
        }
        let (Some(runtime), Some(compositor)) = (self.runtime.as_mut(), self.compositor.as_mut())
        else {
            return Ok(None);
        };
        runtime.poll_events()?;
        if runtime.should_exit() || !runtime.is_session_running() {
            return Ok(None);
        }
        let mut frame = runtime.wait_frame()?;
        runtime.begin_frame()?;
        self.actions = match runtime.sync_actions() {
            Ok(actions) => actions,
            Err(error) => {
                let _ = runtime.end_frame(false);
                return Err(error);
            }
        };
        if let Err(error) = runtime.locate_frame(&mut frame) {
            let _ = runtime.end_frame(false);
            return Err(error);
        }
        let images = if frame.should_render {
            match compositor.begin_frame(&frame) {
                Ok(images) => images,
                Err(error) => {
                    let _ = runtime.end_frame(false);
                    return Err(error);
                }
            }
        } else {
            Vec::new()
        };
        self.current_frame = Some(frame);
        Ok(Some(images))
    }

    pub fn submit(&mut self, images: &[XrSwapchainImage]) -> Result<(), XrError> {
        let frame = self
            .current_frame
            .take()
            .ok_or(XrError::FrameLifecycle("submit called without tick"))?;
        let runtime = self
            .runtime
            .as_mut()
            .ok_or(XrError::FrameLifecycle("XR runtime was removed"))?;
        let compositor = self
            .compositor
            .as_mut()
            .ok_or(XrError::FrameLifecycle("XR compositor was removed"))?;
        let compositor_result = if frame.should_render {
            compositor.end_frame(&frame, images)
        } else {
            Ok(())
        };
        let runtime_result = runtime.end_frame(frame.should_render && compositor_result.is_ok());
        compositor_result?;
        runtime_result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{XrEye, XrView};

    #[derive(Default)]
    struct Trace {
        calls: Vec<&'static str>,
    }

    struct FakeRuntime {
        trace: Arc<Mutex<Trace>>,
    }

    impl XrRuntime for FakeRuntime {
        fn poll_events(&mut self) -> Result<(), XrError> {
            self.trace.lock().unwrap().calls.push("poll");
            Ok(())
        }

        fn wait_frame(&mut self) -> Result<XrFrameState, XrError> {
            self.trace.lock().unwrap().calls.push("wait");
            Ok(XrFrameState {
                predicted_display_time_nanoseconds: 42,
                should_render: true,
                views: [XrView::default(); 2],
                ..XrFrameState::default()
            })
        }

        fn begin_frame(&mut self) -> Result<(), XrError> {
            self.trace.lock().unwrap().calls.push("begin");
            Ok(())
        }

        fn sync_actions(&mut self) -> Result<XrActionSnapshot, XrError> {
            self.trace.lock().unwrap().calls.push("actions");
            Ok(XrActionSnapshot::default())
        }

        fn end_frame(&mut self, _rendered: bool) -> Result<(), XrError> {
            self.trace.lock().unwrap().calls.push("runtime_end");
            Ok(())
        }

        fn should_exit(&self) -> bool {
            false
        }
    }

    struct FakeCompositor {
        trace: Arc<Mutex<Trace>>,
    }

    impl XrCompositor for FakeCompositor {
        fn begin_frame(&mut self, _state: &XrFrameState) -> Result<Vec<XrSwapchainImage>, XrError> {
            self.trace.lock().unwrap().calls.push("acquire");
            Ok(vec![
                XrSwapchainImage {
                    eye: XrEye::Left,
                    image_index: 1,
                    array_layer: 0,
                    native_image: 11,
                    width: 1024,
                    height: 1024,
                },
                XrSwapchainImage {
                    eye: XrEye::Right,
                    image_index: 2,
                    array_layer: 0,
                    native_image: 12,
                    width: 1024,
                    height: 1024,
                },
            ])
        }

        fn end_frame(
            &mut self,
            _state: &XrFrameState,
            _rendered_images: &[XrSwapchainImage],
        ) -> Result<(), XrError> {
            self.trace.lock().unwrap().calls.push("submit");
            Ok(())
        }
    }

    #[test]
    fn stereo_frame_lifecycle_is_ordered_and_bounded() {
        let trace = Arc::new(Mutex::new(Trace::default()));
        let mut system = XrSystem::default();
        system.install(
            Box::new(FakeRuntime {
                trace: Arc::clone(&trace),
            }),
            Box::new(FakeCompositor {
                trace: Arc::clone(&trace),
            }),
        );
        let images = system.tick().unwrap().unwrap();
        assert_eq!(images.len(), 2);
        assert!(matches!(
            system.tick(),
            Err(XrError::FrameLifecycle(
                "tick called before the previous frame was submitted"
            ))
        ));
        system.submit(&images).unwrap();
        assert_eq!(
            trace.lock().unwrap().calls,
            vec![
                "poll",
                "wait",
                "begin",
                "actions",
                "acquire",
                "submit",
                "runtime_end"
            ]
        );
    }

    #[test]
    fn submit_without_tick_is_rejected() {
        assert!(matches!(
            XrSystem::default().submit(&[]),
            Err(XrError::FrameLifecycle("submit called without tick"))
        ));
    }
}
