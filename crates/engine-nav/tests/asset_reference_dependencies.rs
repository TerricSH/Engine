use engine_nav::{register_nav_extensions, AiAgent};
use engine_scene::{sample_scene, AssetTypeRegistry, ComponentRegistry, World};
use engine_serialize::AssetId;

#[test]
fn registered_nav_agent_reference_is_a_scene_asset_dependency() {
    let mut component_registry = ComponentRegistry::new();
    let mut asset_type_registry = AssetTypeRegistry::new();
    register_nav_extensions(&mut component_registry, None, &mut asset_type_registry);

    let mut world = World::from_scene(&sample_scene());
    world.set_component_registry(component_registry);
    let entity = world
        .entity_by_persistent_id("cube-01")
        .expect("sample entity must exist");
    let mut agent = AiAgent::new();
    agent.navmesh_ref = Some("level.navmesh".into());
    world.add_component(entity, agent);

    let dependencies = world.to_scene().collect_asset_dependencies();

    assert!(dependencies.contains(&AssetId::new("level.navmesh")));
}
