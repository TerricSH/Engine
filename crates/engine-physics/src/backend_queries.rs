use std::collections::HashMap;

use rapier3d::na;
use rapier3d::prelude::*;

use crate::backend::{RapierBackend, RaycastHit, to_rapier_shared_shape};
use crate::components::ColliderShape;
use crate::convert::from_rapier_vec;
use crate::queries::{
    OverlapHitResult, QueryBatcher, QueryResults, RaycastHitResult, SweepHitResult,
};
use crate::Entity;

impl RapierBackend {
    // ── Queries ─────────────────────────────────────────────────────────

    /// Cast a ray against all colliders and return the closest hit.
    pub fn raycast(
        &self,
        origin: glam::Vec3,
        dir: glam::Vec3,
        max_dist: f32,
    ) -> Option<RaycastHit> {
        let ray = Ray::new(
            na::Point3::new(origin.x, origin.y, origin.z),
            na::Vector3::new(dir.x, dir.y, dir.z),
        );
        let filter = QueryFilter::default().exclude_sensors();

        let (collider_handle, intersection) = self.query_pipeline.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            max_dist,
            true,
            filter,
        )?;

        let collider = self.colliders.get(collider_handle)?;
        let body_handle = collider.parent()?;

        let entity_index = self
            .body_map
            .iter()
            .find(|(_, &h)| h == body_handle)
            .map(|(&idx, _)| idx)?;

        Some(RaycastHit {
            entity: Entity::new(entity_index, 0),
            point: origin + dir * intersection.time_of_impact,
            normal: from_rapier_vec(intersection.normal),
            distance: intersection.time_of_impact,
        })
    }

    /// Find all entities whose colliders overlap with the given shape.
    pub fn query_proximity(&self, shape: &ColliderShape, pos: glam::Vec3) -> Vec<Entity> {
        let rapier_shape = to_rapier_shared_shape(shape);
        let iso = Isometry::from_parts(
            na::Translation3::new(pos.x, pos.y, pos.z),
            na::UnitQuaternion::identity(),
        );
        let filter = QueryFilter::default().exclude_sensors();
        let mut entities = Vec::new();

        self.query_pipeline.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &iso,
            &*rapier_shape,
            filter,
            |collider_handle| {
                if let Some(collider) = self.colliders.get(collider_handle) {
                    if let Some(parent) = collider.parent() {
                        if let Some(entity_index) = self
                            .body_map
                            .iter()
                            .find(|(_, &h)| h == parent)
                            .map(|(&i, _)| i)
                        {
                            entities.push(Entity::new(entity_index, 0));
                        }
                    }
                }
                true
            },
        );

        entities
    }

    /// Execute a set of batched queries in a single call.
    ///
    /// Processes all queued raycasts, overlaps, and sweeps, collecting
    /// per-query detailed results and a flat list of all entity hits.
    pub fn execute_batched_queries(&self, batcher: &QueryBatcher) -> QueryResults {
        use rapier3d::parry::query::details::ShapeCastOptions;

        // Build reverse map: collider handle → entity index for O(1) lookup.
        let mut col_handle_to_entity: HashMap<ColliderHandle, u32> = HashMap::new();
        for (&entity_idx, &(ch, _)) in &self.collider_map {
            col_handle_to_entity.insert(ch, entity_idx);
        }

        let mut all_hits: Vec<Entity> = Vec::new();
        let mut raycast_details: Vec<Vec<RaycastHitResult>> = Vec::new();
        let mut overlap_details: Vec<Vec<OverlapHitResult>> = Vec::new();
        let mut sweep_details: Vec<Vec<SweepHitResult>> = Vec::new();

        // ── 1. Raycasts ────────────────────────────────────────────────
        for q in &batcher.raycasts {
            let rapier_origin = na::Point3::new(q.origin.x, q.origin.y, q.origin.z);
            let rapier_dir = na::Vector3::new(q.direction.x, q.direction.y, q.direction.z);
            let ray = Ray::new(rapier_origin, rapier_dir);
            let filter = QueryFilter::default().exclude_sensors();

            let mut hits: Vec<RaycastHitResult> = Vec::new();
            self.query_pipeline.intersections_with_ray(
                &self.bodies,
                &self.colliders,
                &ray,
                q.max_distance,
                true, // solid
                filter,
                |handle, intersection| {
                    if let Some(&entity_idx) = col_handle_to_entity.get(&handle) {
                        let entity = Entity::new(entity_idx, 0);
                        // `RayIntersection` has `time_of_impact`, `normal`, `feature`
                        let toi = intersection.time_of_impact;
                        hits.push(RaycastHitResult {
                            entity,
                            point: q.origin + q.direction * toi,
                            normal: glam::Vec3::new(
                                intersection.normal.x,
                                intersection.normal.y,
                                intersection.normal.z,
                            ),
                            distance: toi,
                        });
                        all_hits.push(entity);
                    }
                    true // continue collecting
                },
            );
            raycast_details.push(hits);
        }

        // ── 2. Overlaps ────────────────────────────────────────────────
        for q in &batcher.overlaps {
            let shape = to_rapier_shared_shape(&q.shape);
            let pos = Isometry::from_parts(
                na::Translation3::new(q.position.x, q.position.y, q.position.z),
                na::UnitQuaternion::identity(),
            );
            let filter = QueryFilter::default().exclude_sensors();

            let mut hits: Vec<OverlapHitResult> = Vec::new();
            self.query_pipeline.intersections_with_shape(
                &self.bodies,
                &self.colliders,
                &pos,
                &*shape,
                filter,
                |handle| {
                    if let Some(&entity_idx) = col_handle_to_entity.get(&handle) {
                        let entity = Entity::new(entity_idx, 0);
                        hits.push(OverlapHitResult { entity });
                        all_hits.push(entity);
                    }
                    true
                },
            );
            overlap_details.push(hits);
        }

        // ── 3. Sweeps (shape casts) ────────────────────────────────────
        for q in &batcher.sweeps {
            let shape = to_rapier_shared_shape(&q.shape);
            let from_pos = Isometry::from_parts(
                na::Translation3::new(q.from.x, q.from.y, q.from.z),
                na::UnitQuaternion::identity(),
            );
            let delta = na::Vector3::new(
                q.to.x - q.from.x,
                q.to.y - q.from.y,
                q.to.z - q.from.z,
            );
            let max_dist = delta.magnitude();
            let dir = if max_dist > f32::EPSILON {
                delta / max_dist
            } else {
                // Zero-length sweep (from == to) — no movement, skip
                let no_hits: Vec<SweepHitResult> = Vec::new();
                sweep_details.push(no_hits);
                continue;
            };

            let filter = QueryFilter::default().exclude_sensors();
            let options = ShapeCastOptions {
                max_time_of_impact: max_dist,
                target_distance: 0.0,
                stop_at_penetration: true,
                compute_impact_geometry_on_penetration: false,
            };

            let mut hits: Vec<SweepHitResult> = Vec::new();
            // `cast_shape` returns the closest hit; Rapier does not have
            // a multi-hit sweep API, so we accept the single closest.
            if let Some((handle, hit)) = self.query_pipeline.cast_shape(
                &self.bodies,
                &self.colliders,
                &from_pos,
                &dir,
                &*shape,
                options,
                filter,
            ) {
                if let Some(&entity_idx) = col_handle_to_entity.get(&handle) {
                    let entity = Entity::new(entity_idx, 0);
                    let point = q.from
                        + glam::Vec3::new(dir.x, dir.y, dir.z) * hit.time_of_impact;
                    hits.push(SweepHitResult {
                        entity,
                        point,
                        normal: glam::Vec3::new(
                            hit.normal1.x,
                            hit.normal1.y,
                            hit.normal1.z,
                        ),
                        distance: hit.time_of_impact,
                    });
                    all_hits.push(entity);
                }
            }
            sweep_details.push(hits);
        }

        QueryResults {
            hits: all_hits,
            raycast_details,
            overlap_details,
            sweep_details,
        }
    }

    /// Update the query pipeline after structural changes.
    pub fn sync_query_pipeline(&mut self) {
        self.query_pipeline.update(&self.colliders);
    }
}
