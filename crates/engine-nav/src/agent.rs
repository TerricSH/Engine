use crate::pathfinding::Path;
use glam::Vec3;
use tracing::debug;

// ---------------------------------------------------------------------------
// AgentUpdate
// ---------------------------------------------------------------------------

/// The result of a single tick of [`NavAgent::update`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AgentUpdate {
    /// The agent is still en route.
    Moving {
        /// Current world position.
        position: Vec3,
        /// The next waypoint the agent is moving toward.
        target: Vec3,
        /// Current movement speed (m/s).
        speed: f32,
    },
    /// The agent has reached the end of its path.
    Arrived,
    /// The agent has no path assigned.
    Stopped,
}

// ---------------------------------------------------------------------------
// NavAgent
// ---------------------------------------------------------------------------

/// An agent that follows a computed [`Path`] by stepping toward each waypoint
/// in sequence.
///
/// # Movement model
///
/// The agent moves at `speed` metres per second toward the *current target*
/// waypoint.  When it reaches a waypoint (within `0.001` units) it advances
/// to the next one.  Once all waypoints are consumed `update` returns
/// [`AgentUpdate::Arrived`] and the path is cleared.
#[derive(Clone, Debug)]
pub struct NavAgent {
    position: Vec3,
    path: Option<Path>,
    /// Index of the next waypoint to move toward.
    next_waypoint: usize,
    speed: f32,
}

impl NavAgent {
    /// Create a new stationary agent with default speed (1.0 m/s).
    pub fn new() -> Self {
        Self {
            position: Vec3::ZERO,
            path: None,
            next_waypoint: 0,
            speed: 1.0,
        }
    }

    /// Assign a new path, resetting the agent's progress along it.
    ///
    /// The agent starts from its current position and moves toward the
    /// *second* waypoint (index 1) since the first is typically the centre of
    /// the start polygon.  For single-waypoint paths the agent is considered
    /// already at its destination.
    pub fn set_path(&mut self, path: Path) {
        // The A* path's first waypoint is the start polygon centre.
        // Skip it so the agent moves toward the *first real target*.
        self.next_waypoint = if path.len() > 1 { 1 } else { 0 };
        self.path = Some(path);
    }

    /// Set movement speed (metres per second).
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
    }

    /// Override the agent's world position.
    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }

    /// Current world position.
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// Advance the agent along its path by `dt` seconds.
    ///
    /// Returns [`AgentUpdate::Arrived`] once the final waypoint is reached.
    /// After arrival the internal path is cleared.
    pub fn update(&mut self, dt: f32) -> AgentUpdate {
        let Some(ref path) = self.path else {
            return AgentUpdate::Stopped;
        };

        if path.is_empty() || self.next_waypoint >= path.len() {
            self.path = None;
            return AgentUpdate::Arrived;
        }

        // Advance through waypoints until we exhaust the path or run out of
        // movement this frame.
        while self.next_waypoint < path.len() {
            let target = path.waypoints()[self.next_waypoint].position;
            let to_target = target - self.position;
            let dist = to_target.length();

            if dist <= 0.001 {
                // At (or past) this waypoint — move to the next.
                self.next_waypoint += 1;
                if self.next_waypoint >= path.len() {
                    self.position = target;
                    self.path = None;
                    debug!("Agent arrived at destination");
                    return AgentUpdate::Arrived;
                }
                continue;
            }

            let step = self.speed * dt;
            if step >= dist {
                // Can reach this waypoint within the frame.
                self.position = target;
                self.next_waypoint += 1;
                if self.next_waypoint >= path.len() {
                    self.path = None;
                    debug!("Agent arrived at destination");
                    return AgentUpdate::Arrived;
                }
                // Continue to the next waypoint (same frame).
                continue;
            }

            // Move part-way toward the target.
            let dir = to_target / dist;
            self.position += dir * step;

            return AgentUpdate::Moving {
                position: self.position,
                target,
                speed: self.speed,
            };
        }

        // Finished all waypoints.
        self.path = None;
        AgentUpdate::Arrived
    }

    /// Whether the agent has finished its path (or never had one).
    pub fn is_path_finished(&self) -> bool {
        self.path.is_none()
            || self.path.as_ref().map(|p| p.is_empty()).unwrap_or(true)
    }

    /// Total remaining distance along the path from the current position to
    /// the final waypoint (XZ-plane only).
    pub fn remaining_distance(&self) -> f32 {
        let Some(ref path) = self.path else {
            return 0.0;
        };

        if path.is_empty() || self.next_waypoint >= path.len() {
            return 0.0;
        }

        let mut total = 0.0f32;
        let mut cp = self.position;

        for i in self.next_waypoint..path.len() {
            let wp = &path.waypoints()[i];
            let dx = wp.position.x - cp.x;
            let dz = wp.position.z - cp.z;
            total += (dx * dx + dz * dz).sqrt();
            cp = wp.position;
        }

        total
    }

    /// The current target waypoint position, or `None` if the agent has no
    /// path or has finished it.
    pub fn current_target(&self) -> Option<Vec3> {
        let path = self.path.as_ref()?;
        if self.next_waypoint < path.len() {
            Some(path.waypoints()[self.next_waypoint].position)
        } else {
            None
        }
    }

    /// Clear the path and stop the agent.
    pub fn stop(&mut self) {
        self.path = None;
        self.next_waypoint = 0;
    }
}

impl Default for NavAgent {
    fn default() -> Self {
        Self::new()
    }
}
