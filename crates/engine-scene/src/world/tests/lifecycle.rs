#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    struct TestComponent(u32);

    impl Component for TestComponent {
        const TYPE_ID: &'static str = "test.world_generation";
    }

    #[test]
    fn stale_world_handle_cannot_access_or_mutate_recycled_entity() {
        let mut world = World::new();
        let old = world.create_entity();
        world.add_component(old, TestComponent(1));
        assert!(world.destroy_entity(old));

        let current = world.create_entity();
        assert_eq!(current.index(), old.index());
        assert_ne!(current.generation(), old.generation());
        world.add_component(current, TestComponent(2));

        assert!(world.get::<TestComponent>(old).is_none());
        assert!(world.get_mut::<TestComponent>(old).is_none());
        assert!(!world.has::<TestComponent>(old));
        assert!(world.remove_component::<TestComponent>(old).is_none());
        world.set_enabled(old, false);

        assert!(world.is_enabled(current));
        assert_eq!(
            world.get::<TestComponent>(current).map(|value| value.0),
            Some(2)
        );
        assert_eq!(
            world.query::<TestComponent>().next().map(|item| item.0),
            Some(current)
        );
    }

    #[test]
    fn entity_handles_do_not_alias_across_worlds() {
        let mut first = World::new();
        let old = first.create_entity();
        let mut second = World::new();
        let current = second.create_entity();

        assert_eq!(old.index(), current.index());
        assert_ne!(old.generation(), current.generation());
        assert!(!second.is_alive(old));
    }

    #[test]
    fn clear_invalidates_handles_before_slots_are_reused() {
        let mut world = World::new();
        let old = world.create_entity();
        world.clear();
        let current = world.create_entity();

        assert_eq!(old.index(), current.index());
        assert_ne!(old.generation(), current.generation());
        assert!(!world.is_alive(old));
        assert!(world.is_alive(current));
    }

    #[test]
    fn guessed_generation_for_free_slot_is_rejected_by_world() {
        let mut world = World::new();
        let old = world.create_entity();
        assert!(world.destroy_entity(old));
        let guessed = Entity::new(old.index(), old.generation() + 1);

        assert!(!world.is_alive(guessed));
        assert!(world.get::<TestComponent>(guessed).is_none());
        assert!(!world.set_any(guessed, TestComponent::TYPE_ID, Box::new(TestComponent(3))));
    }
}
