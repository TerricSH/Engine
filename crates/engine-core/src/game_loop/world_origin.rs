use super::*;

impl GameLoop {
    /// Current runtime world origin.
    ///
    /// Every `Transform.translation` (and every other f32 world-space
    /// runtime value) is stored **relative** to this origin; the logical
    /// position of an entity is `world_origin + world_position`. Zero until
    /// the first [`shift_world_origin`](Self::shift_world_origin) and reset
    /// by every scene load.
    pub fn world_origin(&self) -> [f64; 3] {
        self.runtime
            .with_world(|world| world.world_origin())
            .unwrap_or([0.0; 3])
    }

    /// Number of world-origin shifts performed since this loop was created.
    pub fn world_origin_shift_count(&self) -> u64 {
        self.world_origin_shift_count
    }

    /// Details of the most recent world-origin shift, if any.
    pub fn last_world_origin_shift(&self) -> Option<WorldOriginShift> {
        self.last_world_origin_shift
    }

    /// Evaluate the origin-shift trigger once and shift at most once.
    ///
    /// Intended call site: the frame boundary, after `update()` and
    /// scene-transition processing, alongside cell-streaming commits and
    /// before `render()` — never mid-frame. With
    /// [`SceneSettings::origin_shift`] disabled (the default) or without an
    /// active world this is a no-op.
    ///
    /// The reference position is the configured
    /// `reference_entity`'s world position, or the active camera's world
    /// position when unset. When its distance from the origin exceeds
    /// `threshold`, exactly one shift by the full reference position runs, so
    /// the reference lands back at the (relative) origin.
    pub fn tick_world_origin_shift(&mut self) -> Option<WorldOriginShift> {
        let settings = self
            .runtime
            .with_world(|world| world.scene_settings().origin_shift.clone())?;
        if !settings.enabled || !settings.threshold.is_finite() || settings.threshold <= 0.0 {
            return None;
        }
        let reference = self
            .runtime
            .with_world(|world| match settings.reference_entity.as_deref() {
                Some(id) => world
                    .entity_by_persistent_id(id)
                    .and_then(|entity| engine_scene::entity_world_position(world, entity)),
                None => engine_scene::active_camera_world_position(world),
            })
            .flatten()?;
        if !reference.is_finite() {
            return None;
        }
        if (reference.length() as f64) <= f64::from(settings.threshold) {
            return None;
        }
        self.shift_world_origin([
            f64::from(reference.x),
            f64::from(reference.y),
            f64::from(reference.z),
        ])
    }

    /// Shift the world origin by `delta`, preserving logical positions.
    ///
    /// This is the atomic consistency sweep behind
    /// [`tick_world_origin_shift`](Self::tick_world_origin_shift); hosts may
    /// also call it directly (e.g. from tests or a debug console) at a frame
    /// boundary. Every f32 world-space runtime value moves by `-delta` and
    /// [`World::world_origin`] advances by `delta`:
    ///
    /// - every root `Transform` in the ECS (children follow via the
    ///   hierarchy; disabled entities included),
    /// - every physics body, teleported in place with velocities, forces,
    ///   joints, and sleep state preserved (`subsystem-physics` feature),
    /// - every `CharacterController` position, including the primary mirror
    ///   used by [`update_character`](Self::update_character),
    /// - every navigation agent's target and in-progress path
    ///   (`subsystem-navigation` feature),
    /// - every point `GravitySource` center (`subsystem-physics` feature).
    /// - every live world-space CPU particle.
    ///
    /// Audio needs no sweep: emitter/listener snapshots are rebuilt from ECS
    /// transforms every `update()`, and emitters and listener shift together
    /// so relative audio geometry is seamless. Camera-relative rendering
    /// composes unchanged: extraction subtracts the *current* camera
    /// translation each frame, which is exactly what the shift rebased.
    ///
    /// Returns `None` when no world is active.
    pub fn shift_world_origin(&mut self, delta: [f64; 3]) -> Option<WorldOriginShift> {
        let offset = Vec3::new(delta[0] as f32, delta[1] as f32, delta[2] as f32);
        let (transforms, character_controllers, nav_agents, gravity_sources, vfx_particles) =
            self.runtime.with_world_mut(|world| {
                let transforms = world.shift_world_origin(delta);

                let mut characters = 0usize;
                for (_, controller) in world.query_all_mut::<CharacterController>() {
                    let position = controller.position();
                    controller.set_position(position - offset);
                    characters += 1;
                }

                #[cfg(feature = "subsystem-navigation")]
                let nav_agents = {
                    let mut count = 0usize;
                    for (_, agent) in world.query_all_mut::<engine_nav::AiAgent>() {
                        agent.shift_world_positions(-offset);
                        count += 1;
                    }
                    count
                };
                #[cfg(not(feature = "subsystem-navigation"))]
                let nav_agents = 0usize;

                #[cfg(feature = "subsystem-physics")]
                let gravity_sources = engine_physics::shift_gravity_source_centers(world, -offset);
                #[cfg(not(feature = "subsystem-physics"))]
                let gravity_sources = 0usize;

                let vfx_particles = engine_vfx::shift_world_positions(world, -offset);

                (
                    transforms,
                    characters,
                    nav_agents,
                    gravity_sources,
                    vfx_particles,
                )
            })?;

        #[cfg(feature = "subsystem-physics")]
        let physics_bodies = self
            .physics
            .as_mut()
            .map(|physics| physics.translate_bodies(-offset))
            .unwrap_or(0);
        #[cfg(not(feature = "subsystem-physics"))]
        let physics_bodies = 0usize;

        // The primary character mirror is a clone of the component refreshed
        // every frame; keep it consistent between the shift and the next
        // update so a same-frame read cannot observe the pre-shift position.
        if let Some(controller) = self.character.as_mut() {
            let position = controller.position();
            controller.set_position(position - offset);
        }

        let shift = WorldOriginShift {
            delta,
            origin: self.world_origin(),
            transforms,
            physics_bodies,
            character_controllers,
            nav_agents,
            gravity_sources,
            vfx_particles,
        };
        self.world_origin_shift_count += 1;
        self.last_world_origin_shift = Some(shift);
        tracing::info!(
            delta = ?shift.delta,
            origin = ?shift.origin,
            transforms = shift.transforms,
            physics_bodies = shift.physics_bodies,
            character_controllers = shift.character_controllers,
            nav_agents = shift.nav_agents,
            gravity_sources = shift.gravity_sources,
            vfx_particles = shift.vfx_particles,
            count = self.world_origin_shift_count,
            "world origin shifted"
        );
        Some(shift)
    }
}
