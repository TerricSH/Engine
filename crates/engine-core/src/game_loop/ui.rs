use super::*;

#[cfg(feature = "subsystem-ui")]
pub(super) fn embed_scene_ui_batches(
    batches: &mut [engine_renderer::UiBatch],
    viewport: RenderViewportContext,
) {
    let surface_size = viewport.surface_size();
    let output = viewport.output_rect();
    let origin = [
        output.min[0] * surface_size[0] as f32,
        output.min[1] * surface_size[1] as f32,
    ];
    let extent = [
        output.width() * surface_size[0] as f32,
        output.height() * surface_size[1] as f32,
    ];
    for batch in batches {
        for vertex in &mut batch.vertices {
            vertex.position[0] += origin[0];
            vertex.position[1] += origin[1];
        }
        batch.clip_rect.min[0] =
            (batch.clip_rect.min[0] + origin[0]).clamp(origin[0], origin[0] + extent[0]);
        batch.clip_rect.min[1] =
            (batch.clip_rect.min[1] + origin[1]).clamp(origin[1], origin[1] + extent[1]);
        batch.clip_rect.max[0] =
            (batch.clip_rect.max[0] + origin[0]).clamp(origin[0], origin[0] + extent[0]);
        batch.clip_rect.max[1] =
            (batch.clip_rect.max[1] + origin[1]).clamp(origin[1], origin[1] + extent[1]);
    }
}

/// Platform-independent retained UI click produced by a scene Canvas.
///
/// This native event mirrors [`engine_script::GameplayUiEvent`] when the
/// scripting feature is enabled, while remaining available to non-scripted
/// runtime hosts through [`GameLoop::take_ui_events`].
#[cfg(feature = "subsystem-ui")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RuntimeUiValue {
    Bool(bool),
    Float(f32),
}
#[cfg(feature = "subsystem-ui")]
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeUiEvent {
    pub canvas_id: String,
    pub element_id: u32,
    pub callback_id: Option<String>,
    pub value: Option<RuntimeUiValue>,
}

impl GameLoop {
    /// Drain retained Canvas click events for a native host.
    ///
    /// Script-enabled [`update`](Self::update) consumes the same queue once
    /// when building gameplay contexts. Native hosts that want ownership must
    /// therefore call this before that update.
    #[cfg(feature = "subsystem-ui")]
    pub fn take_ui_events(&mut self) -> Vec<RuntimeUiEvent> {
        std::mem::take(&mut self.runtime_ui_events)
    }

    /// Whether a scene Canvas currently owns the primary pointer gesture.
    #[cfg(feature = "subsystem-ui")]
    pub fn ui_has_pointer_capture(&self) -> bool {
        self.runtime_ui_captured_canvas.is_some()
    }

    /// Update the screen viewport used by retained UI scaling and hit tests.
    #[cfg(feature = "subsystem-ui")]
    pub fn set_ui_viewport_size(&mut self, width: u32, height: u32) {
        self.runtime_ui_viewport = [width.max(1) as f32, height.max(1) as f32];
    }

    /// Update the retained UI primary-pointer position in Canvas coordinates.
    ///
    /// While a Canvas owns capture, movement is delivered only to that
    /// Canvas. Otherwise the topmost interactive Canvas under the pointer is
    /// selected using the same persistent-ID order as UI rendering.
    #[cfg(feature = "subsystem-ui")]
    pub fn ui_pointer_move(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            self.cancel_ui_pointer();
            return;
        }
        self.runtime_ui_pointer = [x, y];

        let mut canvases = self.runtime_ui_canvases();
        if let Some(captured_canvas) = self.runtime_ui_captured_canvas.clone() {
            let Some((_, canvas)) = canvases
                .iter_mut()
                .find(|(canvas_id, _)| canvas_id == &captured_canvas)
            else {
                self.cancel_ui_pointer_state();
                return;
            };
            let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
            let value_change = self
                .runtime_ui_input_states
                .entry(captured_canvas.clone())
                .or_default()
                .process_event(
                    canvas,
                    engine_ui::UiPointerEvent::Move {
                        x: canvas_x,
                        y: canvas_y,
                    },
                );
            self.commit_runtime_ui_canvas(&captured_canvas, canvas.clone());
            if let Some(value_change) = value_change {
                self.runtime_ui_events.push(RuntimeUiEvent {
                    canvas_id: captured_canvas,
                    element_id: value_change.element_id.0,
                    callback_id: value_change.callback_id,
                    value: value_change.value.map(|value| match value {
                        engine_ui::UiValue::Bool(value) => RuntimeUiValue::Bool(value),
                        engine_ui::UiValue::Float(value) => RuntimeUiValue::Float(value),
                    }),
                });
            }
            return;
        }

        let hovered_canvas = canvases
            .iter()
            .rev()
            .find(|(_, canvas)| {
                let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
                engine_ui::hit_test_interactive(canvas, canvas_x, canvas_y).is_some()
            })
            .map(|(canvas_id, _)| canvas_id.clone());
        for (canvas_id, state) in &mut self.runtime_ui_input_states {
            if hovered_canvas.as_deref() != Some(canvas_id.as_str()) {
                state.reset();
            }
        }
        if let Some(canvas_id) = hovered_canvas {
            let canvas = canvases
                .iter_mut()
                .find_map(|(candidate, canvas)| (candidate == &canvas_id).then_some(canvas))
                .expect("hovered Canvas came from the same snapshot");
            let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
            self.runtime_ui_input_states
                .entry(canvas_id)
                .or_default()
                .process_event(
                    canvas,
                    engine_ui::UiPointerEvent::Move {
                        x: canvas_x,
                        y: canvas_y,
                    },
                );
        }
    }

    /// Press the primary pointer at its most recently supplied position.
    ///
    /// Exactly one topmost Canvas can capture a press. Presses outside all
    /// interactive elements leave the UI uncaptured.
    #[cfg(feature = "subsystem-ui")]
    pub fn ui_pointer_left_press(&mut self) {
        self.cancel_ui_pointer_state();
        let [x, y] = self.runtime_ui_pointer;
        let mut canvases = self.runtime_ui_canvases();
        let pressed_canvas = canvases
            .iter()
            .rev()
            .find(|(_, canvas)| {
                let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
                engine_ui::hit_test_interactive(canvas, canvas_x, canvas_y).is_some()
            })
            .map(|(canvas_id, _)| canvas_id.clone());
        let Some(canvas_id) = pressed_canvas else {
            return;
        };
        let canvas = canvases
            .iter_mut()
            .find_map(|(candidate, canvas)| (candidate == &canvas_id).then_some(canvas))
            .expect("pressed Canvas came from the same snapshot");
        let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
        let captured = {
            let state = self
                .runtime_ui_input_states
                .entry(canvas_id.clone())
                .or_default();
            state.process_event(
                canvas,
                engine_ui::UiPointerEvent::Press {
                    x: canvas_x,
                    y: canvas_y,
                },
            );
            state.capture.is_some()
        };
        self.commit_runtime_ui_canvas(&canvas_id, canvas.clone());
        if captured {
            self.runtime_ui_captured_canvas = Some(canvas_id);
        }
    }

    /// Release the primary pointer at its most recently supplied position.
    ///
    /// A click is queued only when the Canvas and element captured by press
    /// still exist and the release remains inside that enabled element.
    #[cfg(feature = "subsystem-ui")]
    pub fn ui_pointer_left_release(&mut self) {
        let [x, y] = self.runtime_ui_pointer;
        let Some(canvas_id) = self.runtime_ui_captured_canvas.take() else {
            return;
        };
        let mut canvases = self.runtime_ui_canvases();
        let Some(canvas) = canvases
            .iter_mut()
            .find_map(|(candidate, canvas)| (candidate == &canvas_id).then_some(canvas))
        else {
            self.runtime_ui_input_states.remove(&canvas_id);
            return;
        };
        let [canvas_x, canvas_y] = self.runtime_ui_canvas_point(canvas, x, y);
        let click = self
            .runtime_ui_input_states
            .entry(canvas_id.clone())
            .or_default()
            .process_event(
                canvas,
                engine_ui::UiPointerEvent::Release {
                    x: canvas_x,
                    y: canvas_y,
                },
            );
        self.commit_runtime_ui_canvas(&canvas_id, canvas.clone());
        if let Some(click) = click {
            self.runtime_ui_events.push(RuntimeUiEvent {
                canvas_id,
                element_id: click.element_id.0,
                callback_id: click.callback_id,
                value: click.value.map(|value| match value {
                    engine_ui::UiValue::Bool(value) => RuntimeUiValue::Bool(value),
                    engine_ui::UiValue::Float(value) => RuntimeUiValue::Float(value),
                }),
            });
        }
    }

    /// Cancel a retained UI gesture without producing a click.
    ///
    /// Window focus loss, suspension, or an editor release over chrome must
    /// use this path so a later release cannot resurrect an old press.
    #[cfg(feature = "subsystem-ui")]
    pub fn cancel_ui_pointer(&mut self) {
        let mut canvases = self.runtime_ui_canvases();
        for (canvas_id, canvas) in &mut canvases {
            if let Some(state) = self.runtime_ui_input_states.get_mut(canvas_id) {
                state.process_event(canvas, engine_ui::UiPointerEvent::Cancel);
            }
        }
        self.cancel_ui_pointer_state();
    }

    #[cfg(feature = "subsystem-ui")]
    pub(super) fn cancel_ui_pointer_state(&mut self) {
        self.runtime_ui_input_states.clear();
        self.runtime_ui_captured_canvas = None;
    }

    #[cfg(feature = "subsystem-ui")]
    pub(super) fn reset_runtime_ui_input(&mut self) {
        self.cancel_ui_pointer_state();
        self.runtime_ui_events.clear();
    }

    /// Snapshot and lay out all retained scene canvases in renderer order.
    #[cfg(feature = "subsystem-ui")]
    pub(super) fn runtime_ui_canvases(&mut self) -> Vec<(String, engine_ui::Canvas)> {
        self.runtime
            .with_world_mut(|world| {
                let mut canvases = world
                    .query::<engine_ui::Canvas>()
                    .filter_map(|(entity, _)| {
                        world
                            .persistent_id(entity)
                            .map(|id| (id.to_owned(), entity))
                    })
                    .collect::<Vec<_>>();
                canvases.sort_by(|left, right| left.0.cmp(&right.0));
                canvases
                    .into_iter()
                    .filter_map(|(canvas_id, entity)| {
                        let canvas = world.get_mut::<engine_ui::Canvas>(entity)?;
                        canvas.layout_all();
                        Some((canvas_id, canvas.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(feature = "subsystem-ui")]
    pub(super) fn runtime_ui_canvas_point(
        &self,
        canvas: &engine_ui::Canvas,
        x: f32,
        y: f32,
    ) -> [f32; 2] {
        let viewport_width = if self.runtime_ui_viewport[0] > 0.0 {
            self.runtime_ui_viewport[0]
        } else {
            canvas.width
        };
        let viewport_height = if self.runtime_ui_viewport[1] > 0.0 {
            self.runtime_ui_viewport[1]
        } else {
            canvas.height
        };
        let scale = engine_ui::canvas_scale(canvas, viewport_width, viewport_height);
        if scale.is_finite() && scale > 0.0 {
            [x / scale, y / scale]
        } else {
            [x, y]
        }
    }

    #[cfg(feature = "subsystem-ui")]
    pub(super) fn commit_runtime_ui_canvas(&mut self, canvas_id: &str, canvas: engine_ui::Canvas) {
        self.runtime.with_world_mut(|world| {
            let Some(entity) = world.entity_by_persistent_id(canvas_id) else {
                return;
            };
            if let Some(target) = world.get_mut::<engine_ui::Canvas>(entity) {
                *target = canvas;
            }
        });
    }

    /// Resolve retained-mode scene canvases into renderer batches for the
    /// current frame. Canvas order is based on persistent entity IDs so the
    /// result is stable even when ECS storage order changes.
    #[cfg(feature = "subsystem-ui")]
    pub(super) fn runtime_ui_batches(&mut self) -> Vec<engine_renderer::UiBatch> {
        let input_states = self.runtime_ui_input_states.clone();
        let viewport = self.runtime_ui_viewport;
        let batches = self
            .runtime
            .with_world_mut(|world| {
                let mut canvases = world
                    .query::<engine_ui::Canvas>()
                    .filter_map(|(entity, _)| {
                        world
                            .persistent_id(entity)
                            .map(|id| (id.to_owned(), entity))
                    })
                    .collect::<Vec<_>>();
                canvases.sort_by(|left, right| left.0.cmp(&right.0));

                canvases
                    .into_iter()
                    .flat_map(|(canvas_id, entity)| {
                        let Some(canvas) = world.get_mut::<engine_ui::Canvas>(entity) else {
                            return Vec::new();
                        };
                        canvas.layout_all();
                        let viewport_width = if viewport[0] > 0.0 {
                            viewport[0]
                        } else {
                            canvas.width
                        };
                        let viewport_height = if viewport[1] > 0.0 {
                            viewport[1]
                        } else {
                            canvas.height
                        };
                        let mut batches = canvas.build_batches_for_viewport(
                            viewport_width,
                            viewport_height,
                            input_states.get(&canvas_id),
                        );
                        for batch in &mut batches {
                            batch.canvas_id.clone_from(&canvas_id);
                        }
                        batches
                    })
                    .collect()
            })
            .unwrap_or_default();

        batches
    }
}
