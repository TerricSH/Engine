use glam::Vec3;

// ---------------------------------------------------------------------------
// AiState
// ---------------------------------------------------------------------------

/// The current high-level state of an AI agent's behaviour finite-state
/// machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiState {
    /// Standing still, waiting.
    Idle,
    /// Following a pre-defined route of waypoints.
    Patrol,
    /// Actively pursuing a target (the player).
    Chase,
}

// ---------------------------------------------------------------------------
// AiBehavior
// ---------------------------------------------------------------------------

/// A simple finite-state machine for AI agent behaviour.
///
/// # State diagram
///
/// ```text
///             ┌──────────┐
///   ┌────────>│   Idle   │
///   │         └────┬─────┘
///   │              │  timer expires
///   │         ┌────▼─────┐
///   │         │  Patrol  │◄────────────┐
///   │         └────┬─────┘             │
///   │              │  player in range  │ player out of range (×1.5)
///   │         ┌────▼─────┐             │
///   │         │  Chase   │─────────────┘
///   │         └────┬─────┘
///   │              │  player within arrival radius
///   └──────────────┘
/// ```
///
/// ## Transitions
///
/// | From    | To      | Condition                                    |
/// |---------|---------|----------------------------------------------|
/// | Idle    | Patrol  | `idle_timer ≥ idle_duration`                 |
/// | Patrol  | Chase   | `player_position` within `perception_radius` |
/// | Chase   | Patrol  | `player_position` outside `perception_radius × 1.5` (hysteresis) |
/// | Chase   | Idle    | `player_position` within `arrival_radius`    |
#[derive(Clone, Debug)]
pub struct AiBehavior {
    pub state: AiState,
    /// Current abstract target (updated each frame).
    pub target: Option<Vec3>,
    /// Patrol waypoints.
    pub waypoints: Vec<Vec3>,
    /// Index into `waypoints` for the next patrol target.
    pub current_waypoint: usize,
    /// Distance at which the agent detects the player (m).
    pub perception_radius: f32,
    /// Distance threshold for "arriving" (m).
    pub arrival_radius: f32,
    /// Accumulated idle time (seconds).
    pub idle_timer: f32,
    /// How long the agent stays idle before patrolling (seconds).
    pub idle_duration: f32,
}

impl AiBehavior {
    /// Create a new idle behaviour with default parameters.
    pub fn new() -> Self {
        Self {
            state: AiState::Idle,
            target: None,
            waypoints: Vec::new(),
            current_waypoint: 0,
            perception_radius: 10.0,
            arrival_radius: 1.0,
            idle_timer: 0.0,
            idle_duration: 3.0,
        }
    }

    /// Set the patrol route waypoints and reset waypoint tracking.
    pub fn set_patrol_route(&mut self, waypoints: Vec<Vec3>) {
        self.waypoints = waypoints;
        self.current_waypoint = 0;
    }

    /// Advance the behaviour FSM by `dt` seconds.
    ///
    /// Returns the (possibly new) [`AiState`] and an optional target position.
    /// - `Some(target)` — the agent should move toward this position.
    /// - `None` — no movement target (e.g., idle / waiting).
    ///
    /// `agent_position` is the current world position of the AI entity.
    /// `player_position` is the current world position of the player.
    pub fn update(
        &mut self,
        dt: f32,
        agent_position: Vec3,
        player_position: Vec3,
    ) -> (AiState, Option<Vec3>) {
        let dist_to_player = (agent_position - player_position).length();

        match self.state {
            AiState::Idle => {
                self.idle_timer += dt;

                if self.idle_timer >= self.idle_duration {
                    // Transition: Idle → Patrol
                    self.state = AiState::Patrol;
                    self.current_waypoint = 0;
                    self.idle_timer = 0.0;

                    let target = self
                        .waypoints
                        .first()
                        .copied();
                    self.target = target;
                    (AiState::Patrol, target)
                } else {
                    (AiState::Idle, None)
                }
            }

            AiState::Patrol => {
                // Patrol → Chase when player is within perception range.
                if dist_to_player <= self.perception_radius {
                    self.state = AiState::Chase;
                    self.target = Some(player_position);
                    return (AiState::Chase, Some(player_position));
                }

                // Move along waypoints.
                if self.waypoints.is_empty() {
                    self.target = None;
                    return (AiState::Patrol, None);
                }

                let wp = self.waypoints[self.current_waypoint % self.waypoints.len()];
                let dist_to_wp = (agent_position - wp).length();

                // Advance to the next waypoint when close enough.
                if dist_to_wp <= self.arrival_radius {
                    self.current_waypoint = (self.current_waypoint + 1) % self.waypoints.len();
                }

                let target = self.waypoints[self.current_waypoint % self.waypoints.len()];
                self.target = Some(target);
                (AiState::Patrol, Some(target))
            }

            AiState::Chase => {
                // Chase → Idle when within arrival radius.
                if dist_to_player <= self.arrival_radius {
                    self.state = AiState::Idle;
                    self.idle_timer = 0.0;
                    self.target = None;
                    return (AiState::Idle, None);
                }

                // Chase → Patrol when player escapes (hysteresis).
                if dist_to_player > self.perception_radius * 1.5 {
                    self.state = AiState::Patrol;
                    self.target = None;
                    return (AiState::Patrol, None);
                }

                // Continue chasing the player.
                self.target = Some(player_position);
                (AiState::Chase, Some(player_position))
            }
        }
    }
}

impl Default for AiBehavior {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> AiBehavior {
        let mut b = AiBehavior::new();
        b.set_patrol_route(vec![
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(20.0, 0.0, 0.0),
        ]);
        b.perception_radius = 10.0;
        b.arrival_radius = 1.0;
        b.idle_duration = 2.0;
        b
    }

    // ── Transition: Idle → Patrol ─────────────────────────────────────────

    #[test]
    fn idle_to_patrol_when_timer_expires() {
        let mut b = agent();
        assert_eq!(b.state, AiState::Idle);

        // Before the timer expires, stay idle.
        let (state, target) = b.update(1.0, Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0));
        assert_eq!(state, AiState::Idle);
        assert!(target.is_none());

        // After the timer expires, transition to Patrol.
        let (state, target) = b.update(1.0, Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0));
        assert_eq!(state, AiState::Patrol);
        assert_eq!(target, Some(Vec3::new(10.0, 0.0, 0.0)));
    }

    // ── Transition: Patrol → Chase ────────────────────────────────────────

    #[test]
    fn patrol_to_chase_when_player_in_range() {
        let mut b = agent();
        // Skip idle → patrol.
        b.state = AiState::Patrol;
        b.current_waypoint = 0;
        b.target = Some(Vec3::new(10.0, 0.0, 0.0));

        // Player is within perception_radius (= 10).
        let player_pos = Vec3::new(5.0, 0.0, 0.0);
        let (state, target) = b.update(1.0, Vec3::ZERO, player_pos);
        assert_eq!(state, AiState::Chase);
        assert_eq!(target, Some(player_pos));
    }

    #[test]
    fn patrol_stays_patrol_when_player_far() {
        let mut b = agent();
        b.state = AiState::Patrol;
        b.current_waypoint = 0;

        // Player is far away.
        let (state, _target) = b.update(1.0, Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0));
        assert_eq!(state, AiState::Patrol);
    }

    // ── Transition: Chase → Patrol (hysteresis) ───────────────────────────

    #[test]
    fn chase_to_patrol_when_player_escapes() {
        let mut b = agent();
        b.state = AiState::Chase;
        b.target = Some(Vec3::new(5.0, 0.0, 0.0));

        // Player is outside perception_radius * 1.5 (= 15).
        let player_pos = Vec3::new(20.0, 0.0, 0.0);
        let (state, target) = b.update(1.0, Vec3::ZERO, player_pos);
        assert_eq!(state, AiState::Patrol);
        assert!(target.is_none());
    }

    #[test]
    fn chase_stays_chase_when_player_within_hysteresis() {
        let mut b = agent();
        b.state = AiState::Chase;
        b.target = Some(Vec3::new(5.0, 0.0, 0.0));

        // Player is just inside the hysteresis boundary (14 < 15).
        let player_pos = Vec3::new(14.0, 0.0, 0.0);
        let (state, target) = b.update(1.0, Vec3::ZERO, player_pos);
        assert_eq!(state, AiState::Chase);
        assert_eq!(target, Some(player_pos));
    }

    // ── Transition: Chase → Idle ──────────────────────────────────────────

    #[test]
    fn chase_to_idle_when_reaching_player() {
        let mut b = agent();
        b.state = AiState::Chase;
        b.target = Some(Vec3::new(5.0, 0.0, 0.0));

        // Player is within arrival_radius (= 1) of the agent (at origin).
        let player_pos = Vec3::new(0.5, 0.0, 0.0);
        let (state, target) = b.update(1.0, Vec3::ZERO, player_pos);
        assert_eq!(state, AiState::Idle);
        assert!(target.is_none());
    }

    // ── Patrol waypoint cycling ───────────────────────────────────────────

    #[test]
    fn patrol_advances_waypoints() {
        let mut b = agent();
        b.state = AiState::Patrol;
        b.current_waypoint = 0;

        // Agent starts at origin; first waypoint is (10, 0, 0).
        let (state, target) = b.update(1.0, Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0));
        assert_eq!(state, AiState::Patrol);
        assert_eq!(target, Some(Vec3::new(10.0, 0.0, 0.0)));

        // Advance to next waypoint — agent is at waypoint 0, so advance.
        let (state, target) = b.update(1.0, Vec3::new(10.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 0.0));
        assert_eq!(state, AiState::Patrol);
        assert_eq!(target, Some(Vec3::new(20.0, 0.0, 0.0)));

        // Advance again — should wrap to waypoint 0.
        let (state, target) = b.update(1.0, Vec3::new(20.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 0.0));
        assert_eq!(state, AiState::Patrol);
        assert_eq!(target, Some(Vec3::new(10.0, 0.0, 0.0)));
    }

    // ── Defaults ──────────────────────────────────────────────────────────

    #[test]
    fn ai_behavior_default() {
        let b = AiBehavior::default();
        assert_eq!(b.state, AiState::Idle);
        assert!(b.target.is_none());
        assert!(b.waypoints.is_empty());
        assert_eq!(b.current_waypoint, 0);
        assert!((b.perception_radius - 10.0).abs() < 1e-6);
        assert!((b.arrival_radius - 1.0).abs() < 1e-6);
        assert!((b.idle_timer - 0.0).abs() < 1e-6);
        assert!((b.idle_duration - 3.0).abs() < 1e-6);
    }
}
