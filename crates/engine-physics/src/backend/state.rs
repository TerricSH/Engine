use super::*;

impl RapierBackend {
    /// Remove a collider but keep the body.
    pub fn remove_collider(&mut self, entity: Entity) {
        if let Some((handle, _)) = self.collider_map.remove(&entity) {
            self.colliders
                .remove(handle, &mut self.islands, &mut self.bodies, true);
            self.query_pipeline_dirty = true;
        }
        self.clear_entity_overlaps(entity);
    }

    // ── Transform synchronisation ───────────────────────────────────────

    /// Read back the world-space transform of a body.
    pub fn sync_body_transform(&self, entity: Entity) -> Option<(glam::Vec3, glam::Quat)> {
        let handle = self.body_map.get(&entity)?;
        let body = self.bodies.get(*handle)?;
        Some(from_rapier_isometry(body.position()))
    }

    pub(crate) fn runtime_body_states(&self) -> Vec<(Entity, RigidBodyRuntimeState)> {
        self.body_map
            .iter()
            .filter_map(|(&entity, &handle)| {
                let body = self.bodies.get(handle)?;
                let (position, rotation) = from_rapier_isometry(body.position());
                Some((
                    entity,
                    RigidBodyRuntimeState {
                        position: position.to_array(),
                        rotation: rotation.to_array(),
                        linear_velocity: [body.linvel().x, body.linvel().y, body.linvel().z],
                        angular_velocity: [body.angvel().x, body.angvel().y, body.angvel().z],
                        sleeping: body.is_sleeping(),
                    },
                ))
            })
            .collect()
    }

    pub(crate) fn restore_runtime_body_state(
        &mut self,
        entity: Entity,
        state: &RigidBodyRuntimeState,
    ) -> bool {
        let Some(&handle) = self.body_map.get(&entity) else {
            return false;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return false;
        };
        let rotation = glam::Quat::from_array(state.rotation).normalize();
        body.set_position(
            to_rapier_isometry(glam::Vec3::from_array(state.position), rotation),
            false,
        );
        body.set_linvel(
            to_rapier_vec(glam::Vec3::from_array(state.linear_velocity)),
            false,
        );
        body.set_angvel(
            to_rapier_vec(glam::Vec3::from_array(state.angular_velocity)),
            false,
        );
        if state.sleeping {
            body.sleep();
        } else {
            body.wake_up(true);
        }
        self.query_pipeline_dirty = true;
        true
    }

    pub(crate) fn set_linear_velocity(&mut self, entity: Entity, velocity: glam::Vec3) {
        let Some(&handle) = self.body_map.get(&entity) else {
            return;
        };
        if let Some(body) = self.bodies.get_mut(handle) {
            body.set_linvel(to_rapier_vec(velocity), true);
        }
    }

    pub(crate) fn set_angular_velocity(&mut self, entity: Entity, velocity: glam::Vec3) {
        let Some(&handle) = self.body_map.get(&entity) else {
            return;
        };
        if let Some(body) = self.bodies.get_mut(handle) {
            body.set_angvel(to_rapier_vec(velocity), true);
        }
    }

    pub(crate) fn set_body_type(&mut self, entity: Entity, body_type: BodyType) {
        let Some(&handle) = self.body_map.get(&entity) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return;
        };
        let body_type = match body_type {
            BodyType::Static => RigidBodyType::Fixed,
            BodyType::Dynamic => RigidBodyType::Dynamic,
            BodyType::Kinematic => RigidBodyType::KinematicPositionBased,
        };
        body.set_body_type(body_type, true);
        self.query_pipeline_dirty = true;
    }

    /// Set the world-space transform of a body.
    pub fn set_body_transform(&mut self, entity: Entity, pos: glam::Vec3, rot: glam::Quat) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.set_position(to_rapier_isometry(pos, rot), true);
            }
        }
    }

    /// Translate a body by `offset` without disturbing its simulation state.
    ///
    /// Used by world-origin shifts: the body's rotation, linear/angular
    /// velocities, forces, and sleep state are all preserved — only its
    /// position moves. `wake_up = false` keeps sleeping bodies asleep and
    /// leaves awake bodies awake. Returns `false` when no body is registered
    /// for `entity`.
    pub fn translate_body(&mut self, entity: Entity, offset: glam::Vec3) -> bool {
        let Some(&handle) = self.body_map.get(&entity) else {
            return false;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return false;
        };
        let mut position = *body.position();
        position.translation.vector += to_rapier_vec(offset);
        body.set_position(position, false);
        self.query_pipeline_dirty = true;
        true
    }

    /// Push modified body positions into their attached colliders.
    ///
    /// `RigidBody::set_position` only updates the body; collider poses (and
    /// therefore the query pipeline) observe the move after this
    /// propagation, which the physics step normally performs.
    pub(crate) fn propagate_body_positions_to_colliders(&mut self) {
        self.bodies
            .propagate_modified_body_positions_to_colliders(&mut self.colliders);
    }

    // ── Force / impulse ─────────────────────────────────────────────────

    /// Set the gravity scale of a registered body without waking it.
    ///
    /// Used by the gravity-source system to zero the global-gravity
    /// multiplier on source-driven bodies and to restore the ECS-authored
    /// scale when a body leaves every source's range.
    pub(crate) fn set_body_gravity_scale(&mut self, entity: Entity, scale: f32) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.set_gravity_scale(scale, false);
            }
        }
    }

    /// Total mass of a registered body (additional mass plus collider mass).
    pub(crate) fn body_mass(&self, entity: Entity) -> Option<f32> {
        let handle = self.body_map.get(&entity)?;
        Some(self.bodies.get(*handle)?.mass())
    }

    /// Apply a force at the centre of mass.
    pub fn apply_force(&mut self, entity: Entity, force: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.add_force(to_rapier_vec(force), true);
            }
        }
    }

    /// Apply an impulse at the centre of mass.
    pub fn apply_impulse(&mut self, entity: Entity, impulse: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.apply_impulse(to_rapier_vec(impulse), true);
            }
        }
    }

    /// Apply a torque to a rigid body.
    pub fn apply_torque(&mut self, entity: Entity, torque: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.add_torque(to_rapier_vec(torque), true);
            }
        }
    }

    /// Apply an instantaneous angular impulse to a rigid body.
    pub fn apply_torque_impulse(&mut self, entity: Entity, torque_impulse: glam::Vec3) {
        if let Some(&handle) = self.body_map.get(&entity) {
            if let Some(body) = self.bodies.get_mut(handle) {
                body.apply_torque_impulse(to_rapier_vec(torque_impulse), true);
            }
        }
    }
}
