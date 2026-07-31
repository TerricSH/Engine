use super::*;

impl GameLoop {
    pub(super) fn bind_scene_character(&mut self) {
        let bound = self
            .runtime
            .with_world_mut(|world| {
                let entities = world
                    .query::<CharacterController>()
                    .map(|(entity, _)| entity)
                    .collect::<Vec<_>>();
                let mut bound = None;
                for entity in entities {
                    let transform_position = world
                        .get::<engine_scene::components::Transform>(entity)
                        .map(|transform| transform.translation);
                    let Some(controller) = world.get_mut::<CharacterController>(entity) else {
                        continue;
                    };
                    if let Some(position) = transform_position {
                        controller.set_position(position);
                    }
                    if bound.is_none() {
                        bound = Some((entity, controller.clone()));
                    }
                }
                bound
            })
            .flatten();
        if let Some((entity, controller)) = bound {
            self.character = Some(controller);
            self.character_entity = Some(entity);
        }
    }

    /// Drive the kinematic character controller and sync its position back to
    /// the ECS world.  Call this each frame after processing player input.
    ///
    /// `direction` is a normalised horizontal movement vector.
    /// `wish_jump` is true when the player wants to jump this frame.
    /// `dt` is the frame delta time in seconds.
    pub fn update_character(&mut self, direction: Vec3, wish_jump: bool, dt: f32) {
        let Some(ref mut ctrl) = self.character else {
            return;
        };

        let input = CharacterMovement {
            direction,
            wish_jump,
            delta_time: dt.min(0.1),
        };

        // Drive the controller.  Physics world is optional — without it the
        // controller still moves but won't do ground collision.
        #[cfg(feature = "subsystem-physics")]
        {
            let physics: Option<&PhysicsWorld> = self.physics.as_ref();
            ctrl.update(&input, physics);
        }
        #[cfg(not(feature = "subsystem-physics"))]
        ctrl.update(&input, None);

        // Write controller position back to the ECS entity's Transform.
        if let Some(entity) = self.character_entity {
            let updated_controller = ctrl.clone();
            self.runtime.with_world_mut(|world| {
                use engine_scene::components::Transform;
                if let Some(t) = world.get_mut::<Transform>(entity) {
                    t.translation = ctrl.position();
                }
                if let Some(component) = world.get_mut::<CharacterController>(entity) {
                    *component = updated_controller;
                }
            });
        }
    }

    /// Refresh the primary controller mirror after script commands have
    /// queued movement intent on the ECS component. The mirror drives the
    /// next frame and would otherwise overwrite those pending commands.
    #[cfg(feature = "subsystem-scripting-csharp")]
    pub(super) fn refresh_primary_character_from_world(&mut self) {
        let Some(entity) = self.character_entity else {
            return;
        };
        if let Some(controller) = self
            .runtime
            .with_world(|world| world.get::<CharacterController>(entity).cloned())
            .flatten()
        {
            self.character = Some(controller);
        }
    }

    #[cfg(feature = "subsystem-gameplay")]
    pub(super) fn resolved_character_input(&self) -> (Vec3, bool) {
        use engine_gameplay::InputValue;

        let digital = |name: &str| {
            self.input_map
                .action(name)
                .map_or(0.0, |action| match &action.current_value {
                    InputValue::Bool(true) => 1.0,
                    InputValue::Float(value) => value.clamp(-1.0, 1.0),
                    InputValue::Bool(false) | InputValue::Vec2(_) => 0.0,
                })
        };
        let analog = self
            .input_map
            .action("move")
            .and_then(|action| match &action.current_value {
                InputValue::Vec2(value) => Some(*value),
                _ => None,
            })
            .unwrap_or(glam::Vec2::ZERO);
        let direction = Vec3::new(
            analog.x + digital("move_right") - digital("move_left"),
            0.0,
            -analog.y - digital("move_forward") + digital("move_backward"),
        )
        .normalize_or_zero();
        let wish_jump = self.input_map.action("jump").is_some_and(|action| {
            matches!(&action.current_value, InputValue::Bool(true))
                || matches!(&action.current_value, InputValue::Float(value) if *value > 0.5)
        });
        (direction, wish_jump)
    }

    /// Advance every non-primary CharacterController so AI characters and
    /// ambient pawns are not frozen merely because they are not player-bound.
    pub(super) fn update_additional_characters(&mut self, dt: f32) {
        let primary = self.character_entity;
        #[cfg(feature = "subsystem-physics")]
        let physics = self.physics.as_ref();
        let _ = self.runtime.with_world_mut(|world| {
            let entities = world
                .query::<CharacterController>()
                .map(|(entity, _)| entity)
                .filter(|entity| Some(*entity) != primary)
                .collect::<Vec<_>>();
            for entity in entities {
                let Some(mut controller) = world.get::<CharacterController>(entity).cloned() else {
                    continue;
                };
                let input = CharacterMovement {
                    direction: Vec3::ZERO,
                    wish_jump: false,
                    delta_time: dt.min(0.1),
                };
                #[cfg(feature = "subsystem-physics")]
                controller.update(&input, physics);
                #[cfg(not(feature = "subsystem-physics"))]
                controller.update(&input, None);
                if let Some(transform) =
                    world.get_mut::<engine_scene::components::Transform>(entity)
                {
                    transform.translation = controller.position();
                }
                if let Some(component) = world.get_mut::<CharacterController>(entity) {
                    *component = controller;
                }
            }
        });
    }
}
