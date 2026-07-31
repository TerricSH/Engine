use super::*;

impl GameLoop {
    /// Advance scene animation players and replace their static renderer
    /// extraction with skinned items backed by the loaded extension assets.
    #[cfg(feature = "subsystem-animation")]
    pub(super) fn update_runtime_animation(&mut self, dt: f32) {
        let asset_ids = self.runtime.asset_registry().cached_ids();
        let skeletons = asset_ids
            .iter()
            .filter_map(|id| {
                self.runtime
                    .extension_asset::<engine_animation::Skeleton>("skeleton", id)
                    .map(|handle| (id.id.clone(), handle.get().clone()))
            })
            .collect::<std::collections::HashMap<_, _>>();
        let clips = asset_ids
            .iter()
            .filter_map(|id| {
                self.runtime
                    .extension_asset::<engine_animation::AnimationClip>("animation_clip", id)
                    .map(|handle| (id.id.clone(), handle.get().clone()))
            })
            .collect::<std::collections::HashMap<_, _>>();
        let producer = self
            .runtime
            .animation_extension_handles()
            .skinned_extract
            .clone();

        // Multiple fixed updates may run before one render. Only the latest
        // evaluated pose belongs in the next frame.
        producer.drain();
        let _ = self.runtime.with_world_mut(|world| {
            engine_animation::bridge_skinned_items(world, &skeletons, &clips, &producer, dt);
        });
    }
}
