#![forbid(unsafe_code)]

mod agent;
pub mod behavior;
pub mod components;
mod cook;
mod cooker;
pub mod debug;
mod navmesh;
mod pathfinding;

pub use agent::{AgentUpdate, MovementIntent, NavAgent};
pub use behavior::{AiBehavior, AiState};
pub use components::{register_nav_extensions, update_ai_agent, AiAgent};
pub use cooker::NavMeshCooker;
pub use debug::NavMeshDebugDraw;
pub use navmesh::{NavError, NavMesh, PolygonIndex, VertexIndex};
pub use pathfinding::{Path, PathPoint, Pathfinder};

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    // ── VertexIndex tests ────────────────────────────────────────────────

    #[test]
    fn vertex_index_wrapping() {
        let idx = VertexIndex(42);
        assert_eq!(idx.0, 42);
    }

    #[test]
    fn vertex_index_default() {
        let idx = VertexIndex::default();
        assert_eq!(idx.0, 0);
    }

    #[test]
    fn vertex_index_equality() {
        assert_eq!(VertexIndex(1), VertexIndex(1));
        assert_ne!(VertexIndex(1), VertexIndex(2));
    }

    #[test]
    fn vertex_index_ordering() {
        assert!(VertexIndex(1) < VertexIndex(2));
    }

    // ── PolygonIndex tests ───────────────────────────────────────────────

    #[test]
    fn polygon_index_wrapping() {
        let idx = PolygonIndex(7);
        assert_eq!(idx.0, 7);
    }

    #[test]
    fn polygon_index_default() {
        let idx = PolygonIndex::default();
        assert_eq!(idx.0, 0);
    }

    #[test]
    fn polygon_index_equality() {
        assert_eq!(PolygonIndex(3), PolygonIndex(3));
        assert_ne!(PolygonIndex(3), PolygonIndex(5));
    }

    // ── NavError tests ───────────────────────────────────────────────────

    #[test]
    fn nav_error_no_path_found_display() {
        let err = NavError::NoPathFound;
        assert_eq!(
            err.to_string(),
            "No path found between the specified points"
        );
    }

    #[test]
    fn nav_error_invalid_navmesh_display() {
        let err = NavError::InvalidNavMesh("empty mesh".to_string());
        assert_eq!(err.to_string(), "Invalid navigation mesh: empty mesh");
    }

    #[test]
    fn nav_error_agent_not_on_mesh_display() {
        let err = NavError::AgentNotOnMesh;
        assert_eq!(err.to_string(), "Agent is not on the navigation mesh");
    }

    #[test]
    fn nav_error_debug() {
        let err = NavError::NoPathFound;
        let debug = format!("{:?}", err);
        assert!(debug.contains("NoPathFound"));
    }

    // ── Path tests ───────────────────────────────────────────────────────

    #[test]
    fn path_new_and_waypoints() {
        let pts = vec![
            PathPoint {
                position: Vec3::ZERO,
                polygon: PolygonIndex(0),
            },
            PathPoint {
                position: Vec3::new(10.0, 0.0, 0.0),
                polygon: PolygonIndex(1),
            },
        ];
        let path = Path::new(pts.clone());
        assert_eq!(path.waypoints(), &pts[..]);
    }

    #[test]
    fn path_len_and_is_empty() {
        let empty = Path::new(vec![]);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let path = Path::new(vec![PathPoint {
            position: Vec3::ZERO,
            polygon: PolygonIndex(0),
        }]);
        assert!(!path.is_empty());
        assert_eq!(path.len(), 1);
    }

    #[test]
    fn path_first_and_last() {
        let path = Path::new(vec![
            PathPoint {
                position: Vec3::ZERO,
                polygon: PolygonIndex(0),
            },
            PathPoint {
                position: Vec3::new(5.0, 0.0, 0.0),
                polygon: PolygonIndex(1),
            },
            PathPoint {
                position: Vec3::new(10.0, 0.0, 0.0),
                polygon: PolygonIndex(2),
            },
        ]);
        assert_eq!(path.first().unwrap().position, Vec3::ZERO);
        assert_eq!(path.last().unwrap().position, Vec3::new(10.0, 0.0, 0.0));
    }

    #[test]
    fn path_empty_first_last() {
        let path = Path::new(vec![]);
        assert!(path.first().is_none());
        assert!(path.last().is_none());
    }

    #[test]
    fn path_default_is_empty() {
        let path = Path::default();
        assert!(path.is_empty());
    }

    // ── PathPoint tests ──────────────────────────────────────────────────

    #[test]
    fn path_point_construction() {
        let pt = PathPoint {
            position: Vec3::new(1.0, 2.0, 3.0),
            polygon: PolygonIndex(5),
        };
        assert_eq!(pt.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(pt.polygon, PolygonIndex(5));
    }

    #[test]
    fn path_point_equality() {
        let a = PathPoint {
            position: Vec3::ZERO,
            polygon: PolygonIndex(0),
        };
        let b = PathPoint {
            position: Vec3::ZERO,
            polygon: PolygonIndex(0),
        };
        let c = PathPoint {
            position: Vec3::new(1.0, 0.0, 0.0),
            polygon: PolygonIndex(0),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    // ── NavAgent tests ───────────────────────────────────────────────────

    #[test]
    fn nav_agent_defaults() {
        let agent = NavAgent::new();
        assert_eq!(agent.position(), Vec3::ZERO);
        assert_eq!(agent.remaining_distance(), 0.0);
        assert!(agent.is_path_finished());
        assert!(agent.current_target().is_none());
    }

    #[test]
    fn nav_agent_default_impl() {
        let agent = NavAgent::default();
        assert_eq!(agent.position(), Vec3::ZERO);
    }

    #[test]
    fn nav_agent_set_position() {
        let mut agent = NavAgent::new();
        agent.set_position(Vec3::new(10.0, 0.0, 20.0));
        assert_eq!(agent.position(), Vec3::new(10.0, 0.0, 20.0));
    }

    #[test]
    fn nav_agent_stop() {
        let mut agent = NavAgent::new();
        let path = Path::new(vec![
            PathPoint {
                position: Vec3::ZERO,
                polygon: PolygonIndex(0),
            },
            PathPoint {
                position: Vec3::new(5.0, 0.0, 0.0),
                polygon: PolygonIndex(1),
            },
        ]);
        agent.set_path(path);
        assert!(!agent.is_path_finished());
        agent.stop();
        assert!(agent.is_path_finished());
        assert!(agent.current_target().is_none());
    }

    #[test]
    fn nav_agent_update_without_path_returns_stopped() {
        let mut agent = NavAgent::new();
        let (update, intent) = agent.update(0.016);
        assert_eq!(update, AgentUpdate::Stopped);
        assert!(intent.is_none());
    }

    #[test]
    fn nav_agent_update_arrives_immediately_for_single_waypoint() {
        let mut agent = NavAgent::new();
        let path = Path::new(vec![PathPoint {
            position: Vec3::ZERO,
            polygon: PolygonIndex(0),
        }]);
        agent.set_path(path);
        // Single waypoint → agent already at destination
        let (update, intent) = agent.update(0.016);
        assert_eq!(update, AgentUpdate::Arrived);
        assert!(intent.is_none());
    }

    #[test]
    fn nav_agent_moves_toward_target() {
        let mut agent = NavAgent::new();
        agent.set_position(Vec3::ZERO);
        agent.set_speed(1.0);

        let path = Path::new(vec![
            PathPoint {
                position: Vec3::ZERO,
                polygon: PolygonIndex(0),
            },
            PathPoint {
                position: Vec3::new(10.0, 0.0, 0.0),
                polygon: PolygonIndex(1),
            },
        ]);
        agent.set_path(path);

        // After 1 second at 1 m/s, should be at x=1
        let (update, intent) = agent.update(1.0);
        match update {
            AgentUpdate::Moving {
                position,
                target,
                speed,
            } => {
                assert!((position.x - 1.0).abs() < 0.001);
                assert_eq!(target, Vec3::new(10.0, 0.0, 0.0));
                assert_eq!(speed, 1.0);
                // Should produce a MovementIntent when moving
                let intent = intent.expect("Expected MovementIntent when Moving");
                assert!((intent.direction.x - 1.0).abs() < 0.001);
                assert!((intent.direction.y).abs() < 0.001);
                assert!((intent.direction.z).abs() < 0.001);
                assert_eq!(intent.desired_speed, 1.0);
                assert!(!intent.jump_requested);
            }
            _ => panic!("Expected Moving, got {:?}", update),
        }
    }

    #[test]
    fn nav_agent_arrives_at_destination() {
        let mut agent = NavAgent::new();
        agent.set_position(Vec3::ZERO);
        agent.set_speed(100.0); // Fast enough to cover distance in one step

        let path = Path::new(vec![
            PathPoint {
                position: Vec3::ZERO,
                polygon: PolygonIndex(0),
            },
            PathPoint {
                position: Vec3::new(5.0, 0.0, 0.0),
                polygon: PolygonIndex(1),
            },
        ]);
        agent.set_path(path);

        let (update, intent) = agent.update(1.0);
        assert_eq!(update, AgentUpdate::Arrived);
        assert!(intent.is_none());
    }

    #[test]
    fn nav_agent_remaining_distance() {
        let mut agent = NavAgent::new();
        agent.set_position(Vec3::ZERO);

        let path = Path::new(vec![
            PathPoint {
                position: Vec3::ZERO,
                polygon: PolygonIndex(0),
            },
            PathPoint {
                position: Vec3::new(3.0, 0.0, 0.0),
                polygon: PolygonIndex(1),
            },
            PathPoint {
                position: Vec3::new(3.0, 0.0, 4.0),
                polygon: PolygonIndex(2),
            },
        ]);
        agent.set_path(path);

        // Total distance: 3 + 4 = 7
        let remaining = agent.remaining_distance();
        assert!((remaining - 7.0).abs() < 0.001);
    }

    #[test]
    fn agent_update_moving_fields() {
        let update = AgentUpdate::Moving {
            position: Vec3::new(1.0, 0.0, 0.0),
            target: Vec3::new(10.0, 0.0, 0.0),
            speed: 2.5,
        };
        assert_eq!(
            format!("{:?}", update),
            "Moving { position: Vec3(1.0, 0.0, 0.0), target: Vec3(10.0, 0.0, 0.0), speed: 2.5 }"
        );
    }

    #[test]
    fn agent_update_arrived_debug() {
        let debug = format!("{:?}", AgentUpdate::Arrived);
        assert_eq!(debug, "Arrived");
    }

    #[test]
    fn agent_update_stopped_debug() {
        let debug = format!("{:?}", AgentUpdate::Stopped);
        assert_eq!(debug, "Stopped");
    }
}
