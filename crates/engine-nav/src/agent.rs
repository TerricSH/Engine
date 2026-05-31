use crate::pathfinding::{Path, PathPoint};
use glam::Vec3;
use tracing::debug;

// ---------------------------------------------------------------------------
// MovementIntent
// ---------------------------------------------------------------------------

/// A movement intent produced by [`NavAgent::update`] when the agent is
/// actively moving toward a waypoint.
///
/// This decouples path-following (NavAgent) from actual movement (e.g. a
/// [`CharacterController`](engine_character::CharacterController)): the
/// agent computes *where* it wants to go, and the caller applies the intent
/// to the actual transform or controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementIntent {
    /// Normalised direction toward the current target waypoint (XZ plane).
    pub direction: Vec3,
    /// Desired movement speed (m/s).
    pub desired_speed: f32,
    /// Whether the agent requests a jump.
    pub jump_requested: bool,
}

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
///
/// # Position vs. transform
///
/// [`NavAgent::position`] represents the **desired / planned** position along
/// the path — it is **not** the actual world-space transform of the entity.
/// Callers should use the returned [`MovementIntent`] to drive a
/// [`CharacterController`](engine_character::CharacterController) or
/// other movement system.  The internal position is kept for path-progress
/// bookkeeping only.
#[derive(Clone, Debug)]
pub struct NavAgent {
    /// Desired/planned position along the path (internal bookkeeping).
    /// This is NOT the actual transform position.
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
    /// Returns a tuple of:
    /// - [`AgentUpdate`] describing the agent's progress state.
    /// - An optional [`MovementIntent`] when the agent is actively moving
    ///   (`Some`) or `None` when stopped / arrived.
    ///
    /// The agent's internal [`position`](Self::position) is updated for
    /// path-progress tracking.  Callers should use the returned
    /// `MovementIntent` to drive the actual transform or character controller.
    pub fn update(&mut self, dt: f32) -> (AgentUpdate, Option<MovementIntent>) {
        let Some(ref path) = self.path else {
            return (AgentUpdate::Stopped, None);
        };

        if path.is_empty() || self.next_waypoint >= path.len() {
            self.path = None;
            return (AgentUpdate::Arrived, None);
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
                    return (AgentUpdate::Arrived, None);
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
                    return (AgentUpdate::Arrived, None);
                }
                // Continue to the next waypoint (same frame).
                continue;
            }

            // Move part-way toward the target.
            let dir = to_target / dist;
            self.position += dir * step;

            let intent = MovementIntent {
                direction: dir,
                desired_speed: self.speed,
                jump_requested: false,
            };

            return (
                AgentUpdate::Moving {
                    position: self.position,
                    target,
                    speed: self.speed,
                },
                Some(intent),
            );
        }

        // Finished all waypoints.
        self.path = None;
        (AgentUpdate::Arrived, None)
    }

    /// Whether the agent has finished its path (or never had one).
    pub fn is_path_finished(&self) -> bool {
        self.path.is_none() || self.path.as_ref().map(|p| p.is_empty()).unwrap_or(true)
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

    /// Set a direct target destination.
    ///
    /// Creates a straight‑line path from the agent's current position to
    /// `target`.  This is a convenience for FFI/C# control; for navigation‑
    /// mesh‑aware pathfinding use [`set_path`](Self::set_path) with a path
    /// computed by [`Pathfinder`](crate::Pathfinder).
    pub fn set_target(&mut self, target: Vec3) {
        let from = PathPoint {
            position: self.position,
            polygon: crate::navmesh::PolygonIndex(0),
        };
        let to = PathPoint {
            position: target,
            polygon: crate::navmesh::PolygonIndex(0),
        };
        self.set_path(Path::new(vec![from, to]));
    }

    /// Clear the path and stop the agent.
    pub fn stop(&mut self) {
        self.path = None;
        self.next_waypoint = 0;
    }

    /// Number of waypoints remaining on the current path (including already
    /// passed ones, for a total count).
    pub fn waypoint_count(&self) -> usize {
        self.path.as_ref().map(|p| p.len()).unwrap_or(0)
    }

    /// Get the world-space position of a waypoint by index.
    /// Returns `None` if the index is out of range or no path is set.
    pub fn waypoint_at(&self, index: usize) -> Option<Vec3> {
        self.path
            .as_ref()
            .and_then(|p| p.waypoints().get(index))
            .map(|wp| wp.position)
    }
}

impl Default for NavAgent {
    fn default() -> Self {
        Self::new()
    }
}
