use std::collections::{BTreeMap, BTreeSet};

use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::{SphericalNavError, SphericalNavGraph};

/// Geodesic cap removed from pathfinding at runtime. Construction systems can
/// update these by stable ID without rebuilding the static Fibonacci graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphericalNavObstacle {
    pub id: String,
    pub direction: DVec3,
    pub angular_radius: f64,
}

/// Geodesic cap that increases traversal cost without making it impassable.
/// Multipliers are constrained to `>= 1` so the graph's great-circle
/// heuristic remains admissible.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphericalTraversalArea {
    pub id: String,
    pub direction: DVec3,
    pub angular_radius: f64,
    pub cost_multiplier: f64,
}

impl SphericalNavObstacle {
    pub fn new(
        id: impl Into<String>,
        direction: DVec3,
        angular_radius: f64,
    ) -> Result<Self, SphericalNavError> {
        let id = id.into();
        validate_cap(&id, direction, angular_radius)?;
        Ok(Self {
            id,
            direction: direction.normalize(),
            angular_radius,
        })
    }
}

impl SphericalTraversalArea {
    pub fn new(
        id: impl Into<String>,
        direction: DVec3,
        angular_radius: f64,
        cost_multiplier: f64,
    ) -> Result<Self, SphericalNavError> {
        let id = id.into();
        validate_cap(&id, direction, angular_radius)?;
        if !cost_multiplier.is_finite() || cost_multiplier < 1.0 {
            return Err(SphericalNavError::InvalidObstacle);
        }
        Ok(Self {
            id,
            direction: direction.normalize(),
            angular_radius,
            cost_multiplier,
        })
    }
}

impl SphericalNavGraph {
    pub fn dynamic_revision(&self) -> u64 {
        self.dynamic_revision
    }

    pub fn obstacles(&self) -> impl Iterator<Item = &SphericalNavObstacle> {
        self.obstacles.values()
    }

    pub fn traversal_areas(&self) -> impl Iterator<Item = &SphericalTraversalArea> {
        self.traversal_areas.values()
    }

    /// Insert or atomically replace one dynamic obstacle and update only the
    /// lightweight blocked-node overlay.
    pub fn upsert_obstacle(
        &mut self,
        obstacle: SphericalNavObstacle,
    ) -> Result<usize, SphericalNavError> {
        validate_cap(&obstacle.id, obstacle.direction, obstacle.angular_radius)?;
        let mut obstacle = obstacle;
        obstacle.direction = obstacle.direction.normalize();
        if self.obstacles.get(&obstacle.id) == Some(&obstacle) {
            return Ok(self.blocked_nodes.len());
        }
        self.obstacles.insert(obstacle.id.clone(), obstacle);
        self.rebuild_blocked_nodes();
        self.bump_dynamic_revision();
        Ok(self.blocked_nodes.len())
    }

    pub fn remove_obstacle(&mut self, id: &str) -> bool {
        if self.obstacles.remove(id).is_none() {
            return false;
        }
        self.rebuild_blocked_nodes();
        self.bump_dynamic_revision();
        true
    }

    pub fn replace_obstacles(
        &mut self,
        obstacles: impl IntoIterator<Item = SphericalNavObstacle>,
    ) -> Result<usize, SphericalNavError> {
        let mut replacement = BTreeMap::new();
        for mut obstacle in obstacles {
            validate_cap(&obstacle.id, obstacle.direction, obstacle.angular_radius)?;
            obstacle.direction = obstacle.direction.normalize();
            if replacement.insert(obstacle.id.clone(), obstacle).is_some() {
                return Err(SphericalNavError::InvalidObstacle);
            }
        }
        if replacement == self.obstacles {
            return Ok(self.blocked_nodes.len());
        }
        self.obstacles = replacement;
        self.rebuild_blocked_nodes();
        self.bump_dynamic_revision();
        Ok(self.blocked_nodes.len())
    }

    /// Atomically replaces one producer-owned obstacle namespace while
    /// preserving dynamic caps owned by other engine systems.
    pub fn replace_obstacle_layer(
        &mut self,
        layer: &str,
        obstacles: impl IntoIterator<Item = (String, DVec3, f64)>,
    ) -> Result<usize, SphericalNavError> {
        if layer.is_empty() || layer.contains(':') {
            return Err(SphericalNavError::InvalidObstacle);
        }
        let prefix = format!("{layer}:");
        let mut replacement = self
            .obstacles
            .iter()
            .filter(|(id, _)| !id.starts_with(&prefix))
            .map(|(id, obstacle)| (id.clone(), obstacle.clone()))
            .collect::<BTreeMap<_, _>>();
        for (id, direction, angular_radius) in obstacles {
            if id.is_empty() {
                return Err(SphericalNavError::InvalidObstacle);
            }
            let obstacle =
                SphericalNavObstacle::new(format!("{prefix}{id}"), direction, angular_radius)?;
            if replacement.insert(obstacle.id.clone(), obstacle).is_some() {
                return Err(SphericalNavError::InvalidObstacle);
            }
        }
        if replacement == self.obstacles {
            return Ok(self.blocked_nodes.len());
        }
        self.obstacles = replacement;
        self.rebuild_blocked_nodes();
        self.bump_dynamic_revision();
        Ok(self.blocked_nodes.len())
    }

    pub fn upsert_traversal_area(
        &mut self,
        area: SphericalTraversalArea,
    ) -> Result<(), SphericalNavError> {
        validate_cap(&area.id, area.direction, area.angular_radius)?;
        if !area.cost_multiplier.is_finite() || area.cost_multiplier < 1.0 {
            return Err(SphericalNavError::InvalidObstacle);
        }
        let mut area = area;
        area.direction = area.direction.normalize();
        if self.traversal_areas.get(&area.id) == Some(&area) {
            return Ok(());
        }
        self.traversal_areas.insert(area.id.clone(), area);
        self.bump_dynamic_revision();
        Ok(())
    }

    pub fn remove_traversal_area(&mut self, id: &str) -> bool {
        if self.traversal_areas.remove(id).is_none() {
            return false;
        }
        self.bump_dynamic_revision();
        true
    }

    pub fn clear_dynamic_overrides(&mut self) {
        if self.obstacles.is_empty() && self.traversal_areas.is_empty() {
            return;
        }
        self.obstacles.clear();
        self.traversal_areas.clear();
        self.blocked_nodes.clear();
        self.bump_dynamic_revision();
    }

    pub fn blocked_node_count(&self) -> usize {
        self.blocked_nodes.len()
    }

    pub(super) fn direction_is_blocked(&self, direction: DVec3) -> bool {
        let direction = direction.normalize_or_zero();
        direction != DVec3::ZERO
            && self.obstacles.values().any(|obstacle| {
                cap_contains(obstacle.direction, obstacle.angular_radius, direction)
            })
    }

    pub(super) fn traversal_multiplier(&self, direction: DVec3) -> f64 {
        self.traversal_areas
            .values()
            .filter(|area| cap_contains(area.direction, area.angular_radius, direction))
            .map(|area| area.cost_multiplier)
            .fold(1.0, f64::max)
    }

    fn rebuild_blocked_nodes(&mut self) {
        self.blocked_nodes = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                self.direction_is_blocked(node.direction)
                    .then(|| u32::try_from(index).ok())
                    .flatten()
            })
            .collect::<BTreeSet<_>>();
    }

    fn bump_dynamic_revision(&mut self) {
        self.dynamic_revision = self.dynamic_revision.wrapping_add(1).max(1);
    }
}

fn validate_cap(id: &str, direction: DVec3, angular_radius: f64) -> Result<(), SphericalNavError> {
    if id.is_empty()
        || !direction.is_finite()
        || direction.length_squared() <= f64::EPSILON
        || !angular_radius.is_finite()
        || !(0.0..std::f64::consts::PI).contains(&angular_radius)
    {
        return Err(SphericalNavError::InvalidObstacle);
    }
    Ok(())
}

fn cap_contains(center: DVec3, angular_radius: f64, direction: DVec3) -> bool {
    let direction = direction.normalize_or_zero();
    direction != DVec3::ZERO
        && center
            .normalize_or_zero()
            .dot(direction)
            .clamp(-1.0, 1.0)
            .acos()
            <= angular_radius
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SphericalNavBuildConfig;

    fn graph() -> SphericalNavGraph {
        SphericalNavGraph::fibonacci(
            DVec3::new(1.0e12, -2.0e12, 3.0e12),
            6_000_000.0,
            SphericalNavBuildConfig {
                node_count: 1_024,
                neighbors_per_node: 8,
                ..Default::default()
            },
        )
        .unwrap()
    }

    #[test]
    fn dynamic_obstacle_blocks_endpoints_and_can_be_removed_without_rebuild() {
        let mut graph = graph();
        let center = graph.center();
        let radius = 6_000_000.0;
        let obstacle = SphericalNavObstacle::new("building-a", DVec3::X, 0.08).unwrap();
        assert!(graph.upsert_obstacle(obstacle).unwrap() > 0);
        let revision = graph.dynamic_revision();
        assert_eq!(
            graph.find_path(center + DVec3::X * radius, center - DVec3::X * radius),
            Err(SphericalNavError::EndpointBlocked)
        );
        assert!(graph.remove_obstacle("building-a"));
        assert!(graph.dynamic_revision() > revision);
        assert!(graph
            .find_path(center + DVec3::X * radius, center - DVec3::X * radius)
            .is_ok());
    }

    #[test]
    fn path_length_is_geodesic_and_large_centers_keep_precision() {
        let graph = graph();
        let center = graph.center();
        let path = graph
            .find_path(
                center + DVec3::X * 6_000_000.0,
                center - DVec3::X * 6_000_000.0,
            )
            .unwrap();
        let half_circumference = std::f64::consts::PI * 6_000_000.0;
        assert!(path.length() >= half_circumference);
        assert!(path.length() < half_circumference * 1.1);
        assert!(path
            .waypoints()
            .iter()
            .all(|point| (*point - center).length() > 5_999_999.0));
    }

    #[test]
    fn traversal_areas_raise_cost_and_preserve_admissible_search() {
        let mut graph = graph();
        let center = graph.center();
        let from = center + DVec3::X * 6_000_000.0;
        let to = center - DVec3::X * 6_000_000.0;
        let baseline = graph.find_path(from, to).unwrap();
        graph
            .upsert_traversal_area(SphericalTraversalArea::new("mud", DVec3::Z, 0.8, 50.0).unwrap())
            .unwrap();
        let avoided = graph.find_path(from, to).unwrap();
        assert!(avoided.length() >= baseline.length());
        assert!(avoided
            .waypoints()
            .iter()
            .any(|point| ((*point - center).normalize().z).abs() < 0.5));
    }

    #[test]
    fn producer_layer_replacement_is_atomic_and_preserves_other_obstacles() {
        let mut graph = graph();
        graph
            .upsert_obstacle(SphericalNavObstacle::new("weather", DVec3::Z, 0.03).unwrap())
            .unwrap();
        let before = graph.dynamic_revision();
        graph
            .replace_obstacle_layer(
                "construction",
                [
                    ("pad-a".to_string(), DVec3::X, 0.02),
                    ("pad-b".to_string(), DVec3::Y, 0.04),
                ],
            )
            .unwrap();
        assert!(graph.dynamic_revision() > before);
        assert!(graph.obstacles().any(|obstacle| obstacle.id == "weather"));
        assert!(graph
            .obstacles()
            .any(|obstacle| obstacle.id == "construction:pad-a"));

        let stable_revision = graph.dynamic_revision();
        graph
            .replace_obstacle_layer(
                "construction",
                [
                    ("pad-a".to_string(), DVec3::X, 0.02),
                    ("pad-b".to_string(), DVec3::Y, 0.04),
                ],
            )
            .unwrap();
        assert_eq!(graph.dynamic_revision(), stable_revision);
        graph
            .replace_obstacle_layer("construction", [("pad-b".to_string(), DVec3::Y, 0.04)])
            .unwrap();
        assert!(!graph
            .obstacles()
            .any(|obstacle| obstacle.id == "construction:pad-a"));
        assert!(graph.obstacles().any(|obstacle| obstacle.id == "weather"));
    }
}
