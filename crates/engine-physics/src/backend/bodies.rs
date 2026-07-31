use super::*;

impl RapierBackend {
    // ── Body management ─────────────────────────────────────────────────

    /// Create a rigid body for the given entity, returning the body count.
    pub fn create_body(
        &mut self,
        entity: Entity,
        body: &RigidBody,
        transform: &Transform,
    ) -> usize {
        if self.body_map.contains_key(&entity) {
            return self.bodies.len();
        }

        // This low-level public API has no World from which it can prove which
        // generation is current. Reject a conflicting slot instead of letting
        // a delayed stale call replace live state (including across wrapping).
        if self
            .body_map
            .keys()
            .any(|stored| stored.index() == entity.index())
        {
            return self.bodies.len();
        }

        self.insert_body(entity, body, transform)
    }

    /// Replace any previous generation in this slot with a World-validated
    /// current entity.
    pub(crate) fn replace_body_for_current_entity(
        &mut self,
        entity: Entity,
        body: &RigidBody,
        transform: &Transform,
    ) -> usize {
        if self.body_map.contains_key(&entity) {
            return self.bodies.len();
        }

        let stale_entities: Vec<Entity> = self
            .body_map
            .keys()
            .copied()
            .filter(|stored| stored.index() == entity.index())
            .collect();
        for stale in stale_entities {
            self.remove_body(stale);
        }

        self.insert_body(entity, body, transform)
    }

    fn insert_body(&mut self, entity: Entity, body: &RigidBody, transform: &Transform) -> usize {
        let iso = to_rapier_isometry(transform.translation, transform.rotation);

        let builder = match body.body_type {
            BodyType::Static => RigidBodyBuilder::fixed(),
            BodyType::Dynamic => RigidBodyBuilder::dynamic(),
            BodyType::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        };

        let rapier_body = builder
            .position(iso)
            .additional_mass(body.mass)
            .linear_damping(body.linear_damping)
            .angular_damping(body.angular_damping)
            .gravity_scale(body.gravity_scale)
            .can_sleep(body.can_sleep)
            .enabled(body.enabled)
            .ccd_enabled(body.ccd_enabled)
            .build();

        let handle = self.bodies.insert(rapier_body);
        self.body_map.insert(entity, handle);
        self.bodies.len()
    }

    /// Create a collider and attach it to the body of the given entity.
    pub fn create_collider(
        &mut self,
        entity: Entity,
        collider: &Collider,
        body_entity: Entity,
        material: Option<&crate::PhysicsMaterial>,
    ) {
        if self.collider_map.contains_key(&entity) {
            return;
        }

        // As with create_body, only an exact free slot may be populated through
        // the public API. Replacement requires World validation.
        if self
            .collider_map
            .keys()
            .any(|stored| stored.index() == entity.index())
        {
            return;
        }

        self.insert_collider(entity, collider, body_entity, material);
    }

    /// Replace any previous generation in this slot with a World-validated
    /// current entity collider.
    pub(crate) fn replace_collider_for_current_entity(
        &mut self,
        entity: Entity,
        collider: &Collider,
        body_entity: Entity,
        material: Option<&crate::PhysicsMaterial>,
    ) {
        if self.collider_map.contains_key(&entity) {
            return;
        }

        let stale_entities: Vec<Entity> = self
            .collider_map
            .keys()
            .copied()
            .filter(|stored| stored.index() == entity.index())
            .collect();
        for stale in stale_entities {
            self.remove_collider(stale);
        }

        self.insert_collider(entity, collider, body_entity, material);
    }

    fn insert_collider(
        &mut self,
        entity: Entity,
        collider: &Collider,
        body_entity: Entity,
        material: Option<&crate::PhysicsMaterial>,
    ) {
        let body_handle = match self.body_map.get(&body_entity) {
            Some(&h) => h,
            None => return,
        };

        let Some(shape) = to_rapier_shared_shape(&collider.shape) else {
            tracing::warn!(entity = ?entity, "ignored invalid collider shape");
            return;
        };

        let density = material.map_or(collider.density, |m| m.density);
        let friction = material.map_or(collider.friction, |m| m.friction);
        let restitution = material.map_or(collider.restitution, |m| m.restitution);

        let groups = InteractionGroups::new(
            Group::from_bits_truncate(collider.collision_group),
            Group::from_bits_truncate(collider.collision_mask),
        );

        let rapier_collider = ColliderBuilder::new(shape)
            .density(density)
            .friction(friction)
            .restitution(restitution)
            .sensor(collider.is_trigger)
            .collision_groups(groups)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();

        let collider_handle =
            self.colliders
                .insert_with_parent(rapier_collider, body_handle, &mut self.bodies);

        self.collider_map
            .insert(entity, (collider_handle, collider.shape.clone()));
        self.query_pipeline_dirty = true;
    }

    /// Remove a rigid body and all its colliders.
    pub fn remove_body(&mut self, entity: Entity) {
        if let Some(handle) = self.body_map.remove(&entity) {
            let attached_colliders: Vec<Entity> = self
                .collider_map
                .iter()
                .filter_map(|(&collider_entity, &(collider_handle, _))| {
                    self.colliders.get(collider_handle).and_then(|collider| {
                        (collider.parent() == Some(handle)).then_some(collider_entity)
                    })
                })
                .collect();
            let removed_colliders = !attached_colliders.is_empty();
            self.bodies.remove(
                handle,
                &mut self.islands,
                &mut self.colliders,
                &mut self.impulse_joints,
                &mut self.multibody_joints,
                true,
            );
            for collider_entity in attached_colliders {
                self.collider_map.remove(&collider_entity);
                self.clear_entity_overlaps(collider_entity);
            }
            if removed_colliders {
                self.query_pipeline_dirty = true;
            }
        }

        self.clear_entity_overlaps(entity);

        // Clean up joint tracking: Rapier's bodies.remove() already removed
        // any joints attached to this body from impulse_joints, so we clean
        // up our own tracking maps to prevent stale handle entries.
        self.joint_entity_map.remove(&entity);
        self.joint_handle_lookup
            .retain(|_, rapier_handle| self.impulse_joints.get(*rapier_handle).is_some());
        let valid_joint_ids: HashSet<u32> = self.joint_handle_lookup.keys().copied().collect();
        self.joint_break_limits
            .retain(|joint_id, _| valid_joint_ids.contains(joint_id));
        self.joint_bodies
            .retain(|joint_id, _| valid_joint_ids.contains(joint_id));
        self.joint_entity_map.retain(|_, joint_ids| {
            joint_ids.retain(|joint_id| valid_joint_ids.contains(joint_id));
            !joint_ids.is_empty()
        });
    }

    pub(super) fn clear_entity_overlaps(&mut self, entity: Entity) {
        self.active_sensor_overlaps
            .retain(|pair| pair.0 != entity && pair.1 != entity);
        self.active_collision_overlaps
            .retain(|pair| pair.0 != entity && pair.1 != entity);
    }
}
