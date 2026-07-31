use super::*;

impl Drop for SceneRenderer {
    fn drop(&mut self) {
        // Query pools are device-owned objects; wait for in-flight work so
        // pools are never destroyed while queries are still being written,
        // then destroy them before the logical device goes away.
        self.device.wait_idle();
        self.timestamp_pools
            .destroy(&self.device.logical_device.device);
    }
}
