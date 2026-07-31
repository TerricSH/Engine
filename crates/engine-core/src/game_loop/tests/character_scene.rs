#[cfg(all(test, feature = "subsystem-physics", feature = "subsystem-gameplay"))]
mod character_scene_tests {
    include!("character_scene/core.rs");
    include!("character_scene/rendering_animation.rs");
    include!("character_scene/ragdoll.rs");
    include!("character_scene/controllers_navigation.rs");
    include!("character_scene/lifecycle_events.rs");
}
