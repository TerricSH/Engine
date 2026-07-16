use engine_animation::{register_animation_extensions, AnimationPlayer, SkeletonComponent};
use engine_renderer::{DebugDrawRegistry, RenderExtensionRegistry};
use engine_scene::{sample_scene, AssetTypeRegistry, Component, ComponentRegistry, World};
use engine_serialize::AssetId;

fn world_with_animation_registry() -> World {
    let mut component_registry = ComponentRegistry::new();
    let mut asset_type_registry = AssetTypeRegistry::new();
    let mut render_extensions = RenderExtensionRegistry::new();
    let mut debug_draw = DebugDrawRegistry::new();
    let _handles = register_animation_extensions(
        &mut component_registry,
        &mut asset_type_registry,
        &mut render_extensions,
        &mut debug_draw,
    );

    let mut world = World::from_scene(&sample_scene());
    world.set_component_registry(component_registry);
    world
}

#[test]
fn registered_animation_components_are_scene_asset_dependencies() {
    let mut world = world_with_animation_registry();
    let entity = world
        .entity_by_persistent_id("cube-01")
        .expect("sample entity must exist");
    world.add_component(entity, AnimationPlayer::with_clip("walk.anim"));
    world.add_component(entity, SkeletonComponent::new("hero.skeleton"));

    let dependencies = world.to_scene().collect_asset_dependencies();

    assert!(dependencies.contains(&AssetId::new("walk.anim")));
    assert!(dependencies.contains(&AssetId::new("hero.skeleton")));
}

#[test]
fn empty_animation_asset_references_remain_omitted() {
    let mut world = world_with_animation_registry();
    let entity = world
        .entity_by_persistent_id("cube-01")
        .expect("sample entity must exist");
    world.add_component(entity, AnimationPlayer::new());
    world.add_component(
        entity,
        SkeletonComponent {
            skeleton_asset: None,
            bind_shape: [0.5; 3],
        },
    );

    let scene = world.to_scene();
    let entity = scene
        .entities
        .iter()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("serialized sample entity must exist");

    assert!(!entity.components[AnimationPlayer::TYPE_ID]
        .fields
        .contains_key("clip_asset"));
    assert!(!entity.components[SkeletonComponent::TYPE_ID]
        .fields
        .contains_key("skeleton_asset"));
}
