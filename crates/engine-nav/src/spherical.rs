use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};

use glam::DVec3;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod dynamic;
mod runtime;
pub use dynamic::{SphericalNavObstacle, SphericalTraversalArea};
pub use runtime::{
    SphericalAgentId, SphericalAgentStatus, SphericalNavAgent, SphericalNavRuntimeConfig,
    SphericalNavTick, SphericalNavigationRuntime,
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphericalSurfaceSample {
    pub position: DVec3,
    pub traversal_cost: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphericalNavBuildConfig {
    pub node_count: usize,
    pub neighbors_per_node: usize,
    /// Maximum neighbor arc in radians. Zero derives a conservative value
    /// from the Fibonacci sample density.
    pub max_edge_angle: f64,
}

impl Default for SphericalNavBuildConfig {
    fn default() -> Self {
        Self {
            node_count: 2_048,
            neighbors_per_node: 8,
            max_edge_angle: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphericalNavNode {
    pub direction: DVec3,
    pub position: DVec3,
    pub traversal_cost: f64,
    neighbors: Vec<u32>,
}

impl SphericalNavNode {
    pub fn neighbors(&self) -> &[u32] {
        &self.neighbors
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphericalPath {
    waypoints: Vec<DVec3>,
    length: f64,
}

impl SphericalPath {
    pub fn waypoints(&self) -> &[DVec3] {
        &self.waypoints
    }

    pub fn length(&self) -> f64 {
        self.length
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum SphericalNavError {
    #[error("spherical navigation radius must be finite and greater than zero")]
    InvalidRadius,
    #[error("spherical navigation requires at least 12 sample nodes")]
    InsufficientNodes,
    #[error("spherical navigation requires at least one neighbor per node")]
    InvalidNeighborCount,
    #[error(
        "spherical navigation surface binding is ambiguous; select a terrain volume explicitly"
    )]
    AmbiguousSurfaceBinding,
    #[error("spherical surface sampler rejected every node")]
    EmptySurface,
    #[error("no path was found across the spherical surface")]
    NoPathFound,
    #[error("spherical navigation obstacle parameters are invalid")]
    InvalidObstacle,
    #[error("a path endpoint is covered by a dynamic spherical obstacle")]
    EndpointBlocked,
    #[error("spherical navigation agent parameters are invalid")]
    InvalidAgent,
    #[error("the spherical navigation agent does not exist")]
    UnknownAgent,
}

/// Sampled graph for navigation across a complete spherical surface without
/// cube-face seams or planar X/Z assumptions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphericalNavGraph {
    center: DVec3,
    reference_radius: f64,
    nodes: Vec<SphericalNavNode>,
    #[serde(default)]
    blocked_nodes: BTreeSet<u32>,
    #[serde(default)]
    obstacles: BTreeMap<String, SphericalNavObstacle>,
    #[serde(default)]
    traversal_areas: BTreeMap<String, SphericalTraversalArea>,
    #[serde(default)]
    dynamic_revision: u64,
}

impl SphericalNavGraph {
    pub fn fibonacci(
        center: DVec3,
        radius: f64,
        config: SphericalNavBuildConfig,
    ) -> Result<Self, SphericalNavError> {
        Self::fibonacci_sampled(center, radius, config, |direction| {
            Some(SphericalSurfaceSample {
                position: center + direction * radius,
                traversal_cost: 1.0,
            })
        })
    }

    /// Builds a seam-free Fibonacci graph. Returning `None` marks a sample as
    /// non-traversable; returning a displaced position lets terrain queries
    /// place nodes on the exact generated surface.
    pub fn fibonacci_sampled(
        center: DVec3,
        radius: f64,
        config: SphericalNavBuildConfig,
        mut sampler: impl FnMut(DVec3) -> Option<SphericalSurfaceSample>,
    ) -> Result<Self, SphericalNavError> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err(SphericalNavError::InvalidRadius);
        }
        if config.node_count < 12 {
            return Err(SphericalNavError::InsufficientNodes);
        }
        if config.neighbors_per_node == 0 {
            return Err(SphericalNavError::InvalidNeighborCount);
        }

        let golden_angle = std::f64::consts::PI * (3.0 - 5.0f64.sqrt());
        let mut nodes = Vec::with_capacity(config.node_count);
        for index in 0..config.node_count {
            let y = 1.0 - 2.0 * (index as f64 + 0.5) / config.node_count as f64;
            let radial = (1.0 - y * y).max(0.0).sqrt();
            let angle = golden_angle * index as f64;
            let direction = DVec3::new(radial * angle.cos(), y, radial * angle.sin());
            let Some(sample) = sampler(direction) else {
                continue;
            };
            if !sample.position.is_finite()
                || !sample.traversal_cost.is_finite()
                || sample.traversal_cost < 1.0
            {
                continue;
            }
            nodes.push(SphericalNavNode {
                direction,
                position: sample.position,
                traversal_cost: sample.traversal_cost,
                neighbors: Vec::new(),
            });
        }
        if nodes.is_empty() {
            return Err(SphericalNavError::EmptySurface);
        }

        let estimated_spacing = (4.0 * std::f64::consts::PI / config.node_count as f64).sqrt();
        let max_edge_angle = if config.max_edge_angle > 0.0 {
            config.max_edge_angle
        } else {
            estimated_spacing * 2.75
        };
        for index in 0..nodes.len() {
            let mut nearest = nodes
                .iter()
                .enumerate()
                .filter(|(candidate, _)| *candidate != index)
                .map(|(candidate, node)| {
                    (
                        candidate,
                        nodes[index]
                            .direction
                            .dot(node.direction)
                            .clamp(-1.0, 1.0)
                            .acos(),
                    )
                })
                .filter(|(_, angle)| *angle <= max_edge_angle)
                .collect::<Vec<_>>();
            nearest.sort_by(|left, right| left.1.total_cmp(&right.1));
            nearest.truncate(config.neighbors_per_node);
            for (candidate, _) in nearest {
                if let Ok(candidate) = u32::try_from(candidate) {
                    nodes[index].neighbors.push(candidate);
                }
            }
        }

        // Symmetric edges avoid direction-dependent islands near blocked areas.
        let directed = nodes
            .iter()
            .enumerate()
            .flat_map(|(from, node)| {
                node.neighbors
                    .iter()
                    .copied()
                    .map(move |to| (from, to as usize))
            })
            .collect::<Vec<_>>();
        for (from, to) in directed {
            if to < nodes.len() {
                let from = from as u32;
                if !nodes[to].neighbors.contains(&from) {
                    nodes[to].neighbors.push(from);
                }
            }
        }
        for node in &mut nodes {
            node.neighbors.sort_unstable();
            node.neighbors.dedup();
        }

        Ok(Self {
            center,
            reference_radius: radius,
            nodes,
            blocked_nodes: BTreeSet::new(),
            obstacles: BTreeMap::new(),
            traversal_areas: BTreeMap::new(),
            dynamic_revision: 0,
        })
    }

    pub fn center(&self) -> DVec3 {
        self.center
    }

    pub fn nodes(&self) -> &[SphericalNavNode] {
        &self.nodes
    }

    pub fn find_path(&self, from: DVec3, to: DVec3) -> Result<SphericalPath, SphericalNavError> {
        if self.direction_is_blocked(from - self.center)
            || self.direction_is_blocked(to - self.center)
        {
            return Err(SphericalNavError::EndpointBlocked);
        }
        let start = self
            .nearest_node(from)
            .ok_or(SphericalNavError::EmptySurface)?;
        let goal = self
            .nearest_node(to)
            .ok_or(SphericalNavError::EmptySurface)?;
        if start == goal {
            return Ok(self.spherical_path(vec![from, to]));
        }

        let mut open = BinaryHeap::new();
        let mut g_score = BTreeMap::new();
        let mut came_from = BTreeMap::new();
        g_score.insert(start, 0.0);
        open.push(SphericalOpenEntry {
            node: start,
            f_score: self.heuristic(start, goal),
        });

        while let Some(entry) = open.pop() {
            let Some(&current_g) = g_score.get(&entry.node) else {
                continue;
            };
            if entry.f_score > current_g + self.heuristic(entry.node, goal) + 1.0e-4 {
                continue;
            }
            if entry.node == goal {
                let node_path = reconstruct_nodes(&came_from, start, goal)?;
                let mut waypoints = Vec::with_capacity(node_path.len() + 2);
                waypoints.push(from);
                waypoints.extend(
                    node_path
                        .iter()
                        .skip(1)
                        .take(node_path.len().saturating_sub(2))
                        .map(|&index| self.nodes[index as usize].position),
                );
                waypoints.push(to);
                waypoints.dedup_by(|a, b| a.distance_squared(*b) <= f64::EPSILON);
                return Ok(self.spherical_path(waypoints));
            }

            let node = &self.nodes[entry.node as usize];
            for &neighbor in &node.neighbors {
                if self.blocked_nodes.contains(&neighbor) {
                    continue;
                }
                let neighbor_node = &self.nodes[neighbor as usize];
                let arc = node
                    .direction
                    .dot(neighbor_node.direction)
                    .clamp(-1.0, 1.0)
                    .acos()
                    * self.reference_radius;
                let cost = arc * (node.traversal_cost + neighbor_node.traversal_cost) * 0.5;
                let dynamic_cost = (self.traversal_multiplier(node.direction)
                    + self.traversal_multiplier(neighbor_node.direction))
                    * 0.5;
                let cost = cost * dynamic_cost;
                let tentative = current_g + cost;
                if tentative + f64::EPSILON
                    < g_score.get(&neighbor).copied().unwrap_or(f64::INFINITY)
                {
                    came_from.insert(neighbor, entry.node);
                    g_score.insert(neighbor, tentative);
                    open.push(SphericalOpenEntry {
                        node: neighbor,
                        f_score: tentative + self.heuristic(neighbor, goal),
                    });
                }
            }
        }
        Err(SphericalNavError::NoPathFound)
    }

    fn nearest_node(&self, world: DVec3) -> Option<u32> {
        let direction = (world - self.center).normalize_or_zero();
        (direction != DVec3::ZERO).then_some(())?;
        self.nodes
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                u32::try_from(*index).is_ok_and(|index| !self.blocked_nodes.contains(&index))
            })
            .max_by(|left, right| {
                left.1
                    .direction
                    .dot(direction)
                    .total_cmp(&right.1.direction.dot(direction))
            })
            .and_then(|(index, _)| u32::try_from(index).ok())
    }

    fn heuristic(&self, from: u32, to: u32) -> f64 {
        self.nodes[from as usize]
            .direction
            .dot(self.nodes[to as usize].direction)
            .clamp(-1.0, 1.0)
            .acos()
            * self.reference_radius
    }

    fn spherical_path(&self, waypoints: Vec<DVec3>) -> SphericalPath {
        let length = waypoints
            .windows(2)
            .map(|pair| {
                let from = (pair[0] - self.center).normalize_or_zero();
                let to = (pair[1] - self.center).normalize_or_zero();
                from.dot(to).clamp(-1.0, 1.0).acos() * self.reference_radius
            })
            .sum();
        SphericalPath { waypoints, length }
    }
}

#[derive(Clone, Copy, Debug)]
struct SphericalOpenEntry {
    node: u32,
    f_score: f64,
}

impl PartialEq for SphericalOpenEntry {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node && self.f_score.to_bits() == other.f_score.to_bits()
    }
}

impl Eq for SphericalOpenEntry {}

impl PartialOrd for SphericalOpenEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SphericalOpenEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_score
            .total_cmp(&self.f_score)
            .then_with(|| other.node.cmp(&self.node))
    }
}

fn reconstruct_nodes(
    came_from: &BTreeMap<u32, u32>,
    start: u32,
    goal: u32,
) -> Result<Vec<u32>, SphericalNavError> {
    let mut result = vec![goal];
    let mut current = goal;
    while current != start {
        current = *came_from
            .get(&current)
            .ok_or(SphericalNavError::NoPathFound)?;
        result.push(current);
    }
    result.reverse();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_crosses_a_complete_sphere_without_cube_face_seams() {
        let graph = SphericalNavGraph::fibonacci(
            DVec3::ZERO,
            1_000.0,
            SphericalNavBuildConfig {
                node_count: 512,
                neighbors_per_node: 8,
                ..Default::default()
            },
        )
        .unwrap();
        let path = graph
            .find_path(DVec3::X * 1_000.0, -DVec3::X * 1_000.0)
            .unwrap();
        assert!(path.waypoints().len() > 4);
        assert!(path.length() > 1_900.0);
    }

    #[test]
    fn sampler_projects_nodes_and_routes_around_forbidden_surface() {
        let graph = SphericalNavGraph::fibonacci_sampled(
            DVec3::ZERO,
            100.0,
            SphericalNavBuildConfig {
                node_count: 768,
                neighbors_per_node: 8,
                ..Default::default()
            },
            |direction| {
                // Remove a broad equatorial obstacle on the +Z hemisphere.
                if direction.z > 0.35 && direction.y.abs() < 0.45 {
                    None
                } else {
                    Some(SphericalSurfaceSample {
                        position: direction * 103.0,
                        traversal_cost: 1.0,
                    })
                }
            },
        )
        .unwrap();
        assert!(graph
            .nodes()
            .iter()
            .all(|node| (node.position.length() - 103.0).abs() < 1.0e-3));
        let path = graph
            .find_path(DVec3::X * 103.0, -DVec3::X * 103.0)
            .unwrap();
        assert!(path.waypoints().iter().any(|point| point.y.abs() > 45.0));
    }
}
