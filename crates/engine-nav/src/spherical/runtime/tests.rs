use glam::DVec3;

use super::*;
use crate::{SphericalNavBuildConfig, SphericalNavObstacle};

fn runtime(replan_budget_per_tick: usize) -> SphericalNavigationRuntime {
    let graph = SphericalNavGraph::fibonacci(
        DVec3::new(1.0e12, -2.0e12, 3.0e12),
        1_000.0,
        SphericalNavBuildConfig {
            node_count: 512,
            neighbors_per_node: 8,
            ..Default::default()
        },
    )
    .unwrap();
    SphericalNavigationRuntime::new(
        graph,
        SphericalNavRuntimeConfig {
            replan_budget_per_tick,
        },
    )
    .unwrap()
}

#[test]
fn runtime_follows_great_circle_without_sinking_through_the_planet() {
    let mut runtime = runtime(4);
    let center = runtime.graph().center();
    let start = center + DVec3::X * 1_000.0;
    let target = center + DVec3::Y * 1_000.0;
    let mut agent = SphericalNavAgent::new(SphericalAgentId(7), start, 250.0).unwrap();
    agent.set_destination(target).unwrap();
    runtime.upsert_agent(agent).unwrap();

    for _ in 0..20 {
        runtime.tick(0.5).unwrap();
        let agent = runtime.agent(SphericalAgentId(7)).unwrap();
        assert!(((agent.position - center).length() - 1_000.0).abs() < 1.0e-3);
        if agent.status == SphericalAgentStatus::Arrived {
            break;
        }
    }
    let agent = runtime.agent(SphericalAgentId(7)).unwrap();
    assert_eq!(agent.status, SphericalAgentStatus::Arrived);
    assert_eq!(agent.position, target);
}

#[test]
fn dynamic_revision_replans_agents_with_a_deterministic_frame_budget() {
    let mut runtime = runtime(1);
    let center = runtime.graph().center();
    for id in 1..=2 {
        let mut agent =
            SphericalNavAgent::new(SphericalAgentId(id), center + DVec3::X * 1_000.0, 10.0)
                .unwrap();
        agent.set_destination(center + DVec3::Y * 1_000.0).unwrap();
        runtime.upsert_agent(agent).unwrap();
    }

    let first = runtime.tick(0.0).unwrap();
    assert_eq!(first.replanned, 1);
    assert_eq!(first.awaiting_budget, 1);
    assert_eq!(
        runtime.agent(SphericalAgentId(1)).unwrap().status,
        SphericalAgentStatus::Moving
    );
    assert_eq!(
        runtime.agent(SphericalAgentId(2)).unwrap().status,
        SphericalAgentStatus::AwaitingPath
    );

    runtime
        .graph_mut()
        .upsert_obstacle(SphericalNavObstacle::new("landing-pad", DVec3::Y, 0.08).unwrap())
        .unwrap();
    let blocked_first = runtime.tick(0.0).unwrap();
    let blocked_second = runtime.tick(0.0).unwrap();
    assert_eq!(blocked_first.replanned, 1);
    assert_eq!(blocked_second.replanned, 1);
    assert_eq!(
        runtime.agent(SphericalAgentId(1)).unwrap().status,
        SphericalAgentStatus::Blocked
    );
    assert_eq!(
        runtime.agent(SphericalAgentId(2)).unwrap().status,
        SphericalAgentStatus::Blocked
    );
}
