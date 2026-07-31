use super::*;

impl EditorApp {
    pub(super) fn process_gizmo_inputs(&mut self) {
        if !gizmo_viewport_enabled(
            self.workspace_preferences.gizmos_visible,
            self.play_session.is_editing(),
            self.viewport_tab,
        ) {
            self.gizmo_pointer_events.clear();
            self.gizmo.cancel_drag();
            if let Some(editor_scene) = self.editor_scene.as_mut() {
                let _ = editor_scene.cancel_transform_gizmo_drag();
            }
            return;
        }
        let Some((interaction_min, interaction_max, render_viewport)) = editor_render_viewport(
            self.web_viewport_rect,
            self.window_scale_factor,
            Vec2::new(self.window_w, self.window_h),
        ) else {
            self.gizmo_pointer_events.clear();
            self.gizmo.cancel_drag();
            return;
        };
        let events = std::mem::take(&mut self.gizmo_pointer_events);
        let mut scene_changed = false;
        for event in events {
            if event == GizmoPointerEvent::Cancel {
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    scene_changed |= editor_scene.cancel_transform_gizmo_drag();
                }
                self.gizmo.cancel_drag();
                continue;
            }
            let press = match event {
                GizmoPointerEvent::Press(pointer) => Some(pointer),
                _ => None,
            };
            if press.is_some_and(|pointer| {
                pointer.x < interaction_min.x
                    || pointer.y < interaction_min.y
                    || pointer.x > interaction_max.x
                    || pointer.y > interaction_max.y
            }) {
                continue;
            }
            let selected = self
                .editor_scene
                .as_ref()
                .and_then(|scene| scene.selected_entity.clone());
            let Some(selected) = selected else {
                if let Some(pointer) = press {
                    let picked = self.game_loop.as_ref().and_then(|game_loop| {
                        pick_runtime_entity(
                            &game_loop.runtime,
                            self.frame,
                            render_viewport,
                            interaction_min,
                            interaction_max,
                            pointer,
                        )
                    });
                    if let Some(editor_scene) = self.editor_scene.as_mut() {
                        editor_scene.selected_entity = picked;
                    }
                }
                continue;
            };
            let view = self.game_loop.as_ref().and_then(|game_loop| {
                runtime_gizmo_view(&game_loop.runtime, &selected, self.frame, render_viewport)
                    .and_then(|view| {
                        restrict_gizmo_view_to_rect(view, interaction_min, interaction_max)
                    })
            });
            let Some(view) = view else {
                self.gizmo.cancel_drag();
                continue;
            };
            if let (Some(game_loop), Some(editor_scene)) =
                (self.game_loop.as_ref(), self.editor_scene.as_mut())
            {
                scene_changed |= process_gizmo_pointer_events(
                    vec![event],
                    editor_scene,
                    &mut self.gizmo,
                    &game_loop.runtime,
                    &selected,
                    view,
                );
            }
            if let Some(pointer) = press.filter(|_| !self.gizmo.dragging) {
                let picked = self.game_loop.as_ref().and_then(|game_loop| {
                    pick_runtime_entity(
                        &game_loop.runtime,
                        self.frame,
                        render_viewport,
                        interaction_min,
                        interaction_max,
                        pointer,
                    )
                });
                if let Some(editor_scene) = self.editor_scene.as_mut() {
                    editor_scene.selected_entity = picked;
                }
            }
        }
        if scene_changed {
            if let (Some(game_loop), Some(editor_scene)) =
                (self.game_loop.as_mut(), self.editor_scene.as_mut())
            {
                synchronize_authoring_view(
                    game_loop,
                    editor_scene,
                    &self.scene_view,
                    self.viewport_tab,
                );
            }
        }
    }
}
