use std::collections::BTreeMap;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::{SphericalNavError, SphericalNavGraph};

/// Stable runtime handle for one surface-navigation agent.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct SphericalAgentId(pub u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SphericalAgentStatus {
    #[default]
    Idle,
    AwaitingPath,
    Moving,
    Arrived,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SphericalNavRuntimeConfig {
    /// Caps expensive A* replans per frame. Agents are processed by stable ID.
    pub replan_budget_per_tick: usize,
}

impl Default for SphericalNavRuntimeConfig {
    fn default() -> Self {
        Self {
            replan_budget_per_tick: 16,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphericalNavAgent {
    pub id: SphericalAgentId,
    pub position: DVec3,
    pub destination: Option<DVec3>,
    pub speed: f64,
    pub stopping_distance: f64,
    pub status: SphericalAgentStatus,
    route: Vec<DVec3>,
    route_cursor: usize,
    planned_dynamic_revision: Option<u64>,
}

impl SphericalNavAgent {
    pub fn new(
        id: SphericalAgentId,
        position: DVec3,
        speed: f64,
    ) -> Result<Self, SphericalNavError> {
        if !position.is_finite() || !speed.is_finite() || speed <= 0.0 {
            return Err(SphericalNavError::InvalidAgent);
        }
        Ok(Self {
            id,
            position,
            destination: None,
            speed,
            stopping_distance: 0.05,
            status: SphericalAgentStatus::Idle,
            route: Vec::new(),
            route_cursor: 0,
            planned_dynamic_revision: None,
        })
    }

    pub fn route(&self) -> &[DVec3] {
        &self.route
    }

    pub fn route_cursor(&self) -> usize {
        self.route_cursor
    }

    pub fn planned_dynamic_revision(&self) -> Option<u64> {
        self.planned_dynamic_revision
    }

    pub fn set_destination(&mut self, destination: DVec3) -> Result<(), SphericalNavError> {
        if !destination.is_finite() {
            return Err(SphericalNavError::InvalidAgent);
        }
        self.destination = Some(destination);
        self.invalidate_route(SphericalAgentStatus::AwaitingPath);
        Ok(())
    }

    pub fn clear_destination(&mut self) {
        self.destination = None;
        self.invalidate_route(SphericalAgentStatus::Idle);
    }

    pub fn set_position(&mut self, position: DVec3) -> Result<(), SphericalNavError> {
        if !position.is_finite() {
            return Err(SphericalNavError::InvalidAgent);
        }
        self.position = position;
        let status = if self.destination.is_some() {
            SphericalAgentStatus::AwaitingPath
        } else {
            SphericalAgentStatus::Idle
        };
        self.invalidate_route(status);
        Ok(())
    }

    fn invalidate_route(&mut self, status: SphericalAgentStatus) {
        self.route.clear();
        self.route_cursor = 0;
        self.planned_dynamic_revision = None;
        self.status = status;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SphericalNavTick {
    pub replanned: usize,
    pub replan_failures: usize,
    pub moved: usize,
    pub arrived: usize,
    pub awaiting_budget: usize,
}

/// Deterministic runtime path planner and great-circle path follower.
///
/// The static sampled graph is retained across frames. Dynamic obstacle/cost
/// revisions invalidate agent routes, while a per-frame budget prevents a
/// construction update from causing an unbounded replan spike.
#[derive(Clone, Debug)]
pub struct SphericalNavigationRuntime {
    graph: SphericalNavGraph,
    config: SphericalNavRuntimeConfig,
    agents: BTreeMap<SphericalAgentId, SphericalNavAgent>,
}

impl SphericalNavigationRuntime {
    pub fn new(
        graph: SphericalNavGraph,
        config: SphericalNavRuntimeConfig,
    ) -> Result<Self, SphericalNavError> {
        if config.replan_budget_per_tick == 0 {
            return Err(SphericalNavError::InvalidAgent);
        }
        Ok(Self {
            graph,
            config,
            agents: BTreeMap::new(),
        })
    }

    pub fn graph(&self) -> &SphericalNavGraph {
        &self.graph
    }

    /// Mutating dynamic caps increments the graph revision; affected agent
    /// routes are lazily replanned during the next tick.
    pub fn graph_mut(&mut self) -> &mut SphericalNavGraph {
        &mut self.graph
    }

    pub fn agents(&self) -> impl Iterator<Item = &SphericalNavAgent> {
        self.agents.values()
    }

    pub fn agent(&self, id: SphericalAgentId) -> Option<&SphericalNavAgent> {
        self.agents.get(&id)
    }

    pub fn agent_mut(&mut self, id: SphericalAgentId) -> Option<&mut SphericalNavAgent> {
        self.agents.get_mut(&id)
    }

    pub fn upsert_agent(&mut self, agent: SphericalNavAgent) -> Result<bool, SphericalNavError> {
        validate_agent(&agent)?;
        Ok(self.agents.insert(agent.id, agent).is_none())
    }

    pub fn remove_agent(&mut self, id: SphericalAgentId) -> Option<SphericalNavAgent> {
        self.agents.remove(&id)
    }

    pub fn set_destination(
        &mut self,
        id: SphericalAgentId,
        destination: DVec3,
    ) -> Result<(), SphericalNavError> {
        self.agents
            .get_mut(&id)
            .ok_or(SphericalNavError::UnknownAgent)?
            .set_destination(destination)
    }

    pub fn tick(&mut self, delta_seconds: f64) -> Result<SphericalNavTick, SphericalNavError> {
        if !delta_seconds.is_finite() || delta_seconds < 0.0 {
            return Err(SphericalNavError::InvalidAgent);
        }
        let revision = self.graph.dynamic_revision();
        let mut replan_budget = self.config.replan_budget_per_tick;
        let mut stats = SphericalNavTick::default();

        for agent in self.agents.values_mut() {
            validate_agent(agent)?;
            let Some(destination) = agent.destination else {
                agent.status = SphericalAgentStatus::Idle;
                continue;
            };
            let needs_replan = agent.planned_dynamic_revision != Some(revision)
                || agent.status == SphericalAgentStatus::AwaitingPath;
            if needs_replan {
                if replan_budget == 0 {
                    agent.status = SphericalAgentStatus::AwaitingPath;
                    stats.awaiting_budget += 1;
                    continue;
                }
                replan_budget -= 1;
                stats.replanned += 1;
                match self.graph.find_path(agent.position, destination) {
                    Ok(path) => {
                        agent.route = path.waypoints().to_vec();
                        agent.route_cursor = usize::from(agent.route.len() > 1);
                        agent.planned_dynamic_revision = Some(revision);
                        agent.status = SphericalAgentStatus::Moving;
                    }
                    Err(SphericalNavError::NoPathFound | SphericalNavError::EndpointBlocked) => {
                        agent.route.clear();
                        agent.route_cursor = 0;
                        agent.planned_dynamic_revision = Some(revision);
                        agent.status = SphericalAgentStatus::Blocked;
                        stats.replan_failures += 1;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }

            if delta_seconds == 0.0 || agent.status == SphericalAgentStatus::Blocked {
                continue;
            }
            let arrived = advance_agent(
                agent,
                self.graph.center,
                self.graph.reference_radius,
                agent.speed * delta_seconds,
            );
            stats.moved += 1;
            if arrived {
                stats.arrived += 1;
            }
        }
        Ok(stats)
    }
}

fn validate_agent(agent: &SphericalNavAgent) -> Result<(), SphericalNavError> {
    if !agent.position.is_finite()
        || !agent.speed.is_finite()
        || agent.speed <= 0.0
        || !agent.stopping_distance.is_finite()
        || agent.stopping_distance < 0.0
        || agent.destination.is_some_and(|target| !target.is_finite())
    {
        return Err(SphericalNavError::InvalidAgent);
    }
    Ok(())
}

fn advance_agent(
    agent: &mut SphericalNavAgent,
    center: DVec3,
    reference_radius: f64,
    mut distance_budget: f64,
) -> bool {
    while let Some(&waypoint) = agent.route.get(agent.route_cursor) {
        let distance = surface_distance(center, reference_radius, agent.position, waypoint);
        if distance <= agent.stopping_distance || distance_budget >= distance {
            agent.position = waypoint;
            agent.route_cursor += 1;
            distance_budget = (distance_budget - distance).max(0.0);
            continue;
        }
        agent.position = interpolate_surface(
            center,
            agent.position,
            waypoint,
            (distance_budget / distance).clamp(0.0, 1.0),
        );
        agent.status = SphericalAgentStatus::Moving;
        return false;
    }
    if let Some(destination) = agent.destination {
        agent.position = destination;
    }
    agent.status = SphericalAgentStatus::Arrived;
    true
}

fn surface_distance(center: DVec3, radius: f64, from: DVec3, to: DVec3) -> f64 {
    let from = (from - center).normalize_or_zero();
    let to = (to - center).normalize_or_zero();
    from.dot(to).clamp(-1.0, 1.0).acos() * radius
}

fn interpolate_surface(center: DVec3, from: DVec3, to: DVec3, amount: f64) -> DVec3 {
    let from_local = from - center;
    let to_local = to - center;
    let from_radius = from_local.length();
    let to_radius = to_local.length();
    let from_direction = from_local.normalize_or_zero();
    let to_direction = to_local.normalize_or_zero();
    let dot = from_direction.dot(to_direction).clamp(-1.0, 1.0);
    let angle = dot.acos();
    let direction = if angle <= 1.0e-10 {
        from_direction
            .lerp(to_direction, amount)
            .normalize_or_zero()
    } else {
        let sin_angle = angle.sin();
        ((from_direction * ((1.0 - amount) * angle).sin() + to_direction * (amount * angle).sin())
            / sin_angle)
            .normalize_or_zero()
    };
    center + direction * (from_radius + (to_radius - from_radius) * amount)
}

#[cfg(test)]
mod tests;
