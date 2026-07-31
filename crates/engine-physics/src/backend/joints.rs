use super::*;

impl RapierBackend {
    // ── Joint management ────────────────────────────────────────────────

    /// Create a joint between two rigid bodies.
    pub fn create_joint(
        &mut self,
        desc: &JointDescriptor,
        body_a_handle: RigidBodyHandle,
        body_b_handle: RigidBodyHandle,
    ) -> Option<JointHandle> {
        desc.validate().ok()?;
        let frame_a = na::Isometry3::from_parts(
            na::Translation3::new(desc.anchor_a[0], desc.anchor_a[1], desc.anchor_a[2]),
            na::UnitQuaternion::identity(),
        );
        let frame_b = na::Isometry3::from_parts(
            na::Translation3::new(desc.anchor_b[0], desc.anchor_b[1], desc.anchor_b[2]),
            na::UnitQuaternion::identity(),
        );
        let anchor_a = na::Point3::new(desc.anchor_a[0], desc.anchor_a[1], desc.anchor_a[2]);
        let anchor_b = na::Point3::new(desc.anchor_b[0], desc.anchor_b[1], desc.anchor_b[2]);

        // Build the appropriate Rapier joint type and insert directly.
        // NOTE: Rapier 0.22 builders implement Into<GenericJoint> so we pass
        // them straight to impulse_joints.insert().
        let rapier_handle = match desc.joint_type {
            JointType::Fixed => {
                let b = FixedJointBuilder::new()
                    .local_frame1(frame_a)
                    .local_frame2(frame_b);
                self.impulse_joints
                    .insert(body_a_handle, body_b_handle, b, true)
            }
            JointType::Revolute => {
                let axis = na::Unit::new_normalize(na::Vector3::new(
                    desc.axis[0],
                    desc.axis[1],
                    desc.axis[2],
                ));
                let mut b = RevoluteJointBuilder::new(axis)
                    .local_anchor1(anchor_a)
                    .local_anchor2(anchor_b);
                if let Some(l) = &desc.limits {
                    b = b.limits([l.min, l.max]);
                }
                if let Some(m) = &desc.motor {
                    b = b.motor(m.target_pos, m.target_vel, m.stiffness, m.damping);
                }
                self.impulse_joints
                    .insert(body_a_handle, body_b_handle, b, true)
            }
            JointType::Prismatic => {
                let axis = na::Unit::new_normalize(na::Vector3::new(
                    desc.axis[0],
                    desc.axis[1],
                    desc.axis[2],
                ));
                let mut b = PrismaticJointBuilder::new(axis)
                    .local_anchor1(anchor_a)
                    .local_anchor2(anchor_b);
                if let Some(l) = &desc.limits {
                    b = b.limits([l.min, l.max]);
                }
                if let Some(m) = &desc.motor {
                    b = b.set_motor(m.target_pos, m.target_vel, m.stiffness, m.damping);
                }
                self.impulse_joints
                    .insert(body_a_handle, body_b_handle, b, true)
            }
            JointType::Spherical => {
                let mut b = SphericalJointBuilder::new()
                    .local_frame1(frame_a)
                    .local_frame2(frame_b);
                if let Some(l) = &desc.limits {
                    use rapier3d::dynamics::JointAxis;
                    b = b
                        .limits(JointAxis::AngX, [l.min, l.max])
                        .limits(JointAxis::AngY, [l.min, l.max])
                        .limits(JointAxis::AngZ, [l.min, l.max]);
                }
                if let Some(m) = &desc.motor {
                    use rapier3d::dynamics::JointAxis;
                    b = b
                        .motor(
                            JointAxis::AngX,
                            m.target_pos,
                            m.target_vel,
                            m.stiffness,
                            m.damping,
                        )
                        .motor(
                            JointAxis::AngY,
                            m.target_pos,
                            m.target_vel,
                            m.stiffness,
                            m.damping,
                        )
                        .motor(
                            JointAxis::AngZ,
                            m.target_pos,
                            m.target_vel,
                            m.stiffness,
                            m.damping,
                        );
                }
                self.impulse_joints
                    .insert(body_a_handle, body_b_handle, b, true)
            }
        };

        // Generate a unique handle ID and store the full Rapier handle.
        let our_id = self.next_joint_id;
        self.next_joint_id += 1;
        self.joint_handle_lookup.insert(our_id, rapier_handle);
        self.joint_break_limits
            .insert(our_id, (desc.break_force, desc.break_torque));
        self.joint_bodies
            .insert(our_id, (desc.entity_a, desc.entity_b));

        // Track entity → joint mapping.
        self.joint_entity_map
            .entry(desc.entity_a)
            .or_default()
            .insert(our_id);
        self.joint_entity_map
            .entry(desc.entity_b)
            .or_default()
            .insert(our_id);

        Some(JointHandle(our_id))
    }

    /// Remove a joint by handle.
    pub fn remove_joint(&mut self, handle: JointHandle) {
        if let Some(rapier_handle) = self.joint_handle_lookup.remove(&handle.0) {
            self.impulse_joints.remove(rapier_handle, true);
        }
        self.joint_break_limits.remove(&handle.0);
        self.joint_bodies.remove(&handle.0);
        // Clean up entity tracking.
        self.joint_entity_map.retain(|_, handles| {
            handles.remove(&handle.0);
            !handles.is_empty()
        });
    }

    /// Number of active impulse joints.
    pub fn joint_count(&self) -> usize {
        self.impulse_joints.len()
    }

    pub fn has_joint(&self, handle: JointHandle) -> bool {
        self.joint_handle_lookup
            .get(&handle.0)
            .is_some_and(|rapier_handle| self.impulse_joints.get(*rapier_handle).is_some())
    }

    /// Remove joints whose solver impulse exceeded an authored force/torque
    /// threshold during the completed fixed step.
    ///
    /// Rapier exposes constraint impulses (N·s and N·m·s). Dividing their
    /// linear/angular norms by Rapier's final TGS substep duration yields the
    /// reaction force/torque estimate used for deterministic break decisions.
    pub(super) fn break_overloaded_joints(&mut self) -> Vec<JointBreakEvent> {
        let substep_dt = self.integration.dt / self.integration.num_solver_iterations.get() as f32;
        if !substep_dt.is_finite() || substep_dt <= 0.0 {
            return Vec::new();
        }
        let mut broken = Vec::new();
        for (&joint_id, &rapier_handle) in &self.joint_handle_lookup {
            let Some(&(break_force, break_torque)) = self.joint_break_limits.get(&joint_id) else {
                continue;
            };
            if break_force <= 0.0 && break_torque <= 0.0 {
                continue;
            }
            let Some(joint) = self.impulse_joints.get(rapier_handle) else {
                continue;
            };
            let linear_impulse =
                na::Vector3::new(joint.impulses[0], joint.impulses[1], joint.impulses[2]);
            let angular_impulse =
                na::Vector3::new(joint.impulses[3], joint.impulses[4], joint.impulses[5]);
            let force = linear_impulse.norm() / substep_dt;
            let torque = angular_impulse.norm() / substep_dt;
            let force_broke = break_force > 0.0 && (!force.is_finite() || force > break_force);
            let torque_broke = break_torque > 0.0 && (!torque.is_finite() || torque > break_torque);
            if force_broke || torque_broke {
                let Some(&(entity_a, entity_b)) = self.joint_bodies.get(&joint_id) else {
                    continue;
                };
                broken.push(JointBreakEvent {
                    handle: JointHandle(joint_id),
                    joint_entity: None,
                    entity_a,
                    entity_b,
                    force,
                    torque,
                });
            }
        }
        for event in &broken {
            self.remove_joint(event.handle);
        }
        broken
    }
}
