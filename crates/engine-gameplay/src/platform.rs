use std::sync::Mutex;

/// Performance mode hint for the runtime environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerformanceMode {
    HighQuality,
    Balanced,
    PowerSave,
}

/// Describes the capabilities of the current platform.
#[derive(Debug, Clone)]
pub struct PlatformCapabilities {
    pub has_touch: bool,
    pub has_gamepad: bool,
    pub has_keyboard: bool,
    pub has_vibration: bool,
    pub is_mobile: bool,
    pub max_resolution: (u32, u32),
    pub performance_mode: PerformanceMode,
}

/// Abstract interface for platform-specific operations.
pub trait PlatformFacade: Send {
    fn capabilities(&self) -> PlatformCapabilities;
    fn vibrate(&self, duration_ms: u32, intensity: f32) -> Result<(), String>;
    fn set_performance_mode(&self, mode: PerformanceMode) -> Result<(), String>;
    fn device_name(&self) -> String;
    fn os_version(&self) -> String;
}

// ---------------------------------------------------------------------------
// DesktopPlatform – stub implementation for Windows / Linux / macOS
// ---------------------------------------------------------------------------

pub struct DesktopPlatform;

impl PlatformFacade for DesktopPlatform {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            has_touch: false,
            has_gamepad: cfg!(target_os = "windows"), // XInput on Windows
            has_keyboard: true,
            has_vibration: false,
            is_mobile: false,
            max_resolution: (3840, 2160),
            performance_mode: PerformanceMode::HighQuality,
        }
    }

    fn vibrate(&self, duration_ms: u32, intensity: f32) -> Result<(), String> {
        tracing::warn!(
            "vibrate called on desktop platform (duration_ms={}, intensity={}) — no vibration hardware",
            duration_ms,
            intensity
        );
        Ok(())
    }

    fn set_performance_mode(&self, mode: PerformanceMode) -> Result<(), String> {
        tracing::info!("set_performance_mode({mode:?}) — desktop stub, no-op");
        Ok(())
    }

    fn device_name(&self) -> String {
        if cfg!(target_os = "windows") {
            "Windows Desktop".into()
        } else if cfg!(target_os = "linux") {
            "Linux Desktop".into()
        } else if cfg!(target_os = "macos") {
            "macOS Desktop".into()
        } else {
            "Unknown Desktop".into()
        }
    }

    fn os_version(&self) -> String {
        std::env::consts::OS.to_string()
    }
}

// ---------------------------------------------------------------------------
// MockPlatform – fully configurable for unit tests
// ---------------------------------------------------------------------------

pub struct MockPlatform {
    base_caps: PlatformCapabilities,
    last_vibrate: Mutex<Option<(u32, f32)>>,
    perf_mode: Mutex<PerformanceMode>,
}

impl MockPlatform {
    pub fn new(capabilities: PlatformCapabilities) -> Self {
        let perf_mode = capabilities.performance_mode;
        Self {
            base_caps: capabilities,
            last_vibrate: Mutex::new(None),
            perf_mode: Mutex::new(perf_mode),
        }
    }

    /// Returns the (duration_ms, intensity) of the most recent `vibrate` call.
    pub fn last_vibrate(&self) -> Option<(u32, f32)> {
        *self.last_vibrate.lock().unwrap()
    }

    /// Returns the current performance mode stored by the mock.
    pub fn current_performance_mode(&self) -> PerformanceMode {
        *self.perf_mode.lock().unwrap()
    }
}

impl PlatformFacade for MockPlatform {
    fn capabilities(&self) -> PlatformCapabilities {
        let mut caps = self.base_caps.clone();
        caps.performance_mode = *self.perf_mode.lock().unwrap();
        caps
    }

    fn vibrate(&self, duration_ms: u32, intensity: f32) -> Result<(), String> {
        *self.last_vibrate.lock().unwrap() = Some((duration_ms, intensity));
        Ok(())
    }

    fn set_performance_mode(&self, mode: PerformanceMode) -> Result<(), String> {
        *self.perf_mode.lock().unwrap() = mode;
        Ok(())
    }

    fn device_name(&self) -> String {
        "MockDevice".into()
    }

    fn os_version(&self) -> String {
        "mock-os-1.0".into()
    }
}

// ---------------------------------------------------------------------------
// detect_platform – factory that returns the appropriate PlatformFacade
// ---------------------------------------------------------------------------

/// Detect the current platform and return a suitable [`PlatformFacade`].
///
/// On desktop targets (Windows, Linux, macOS) this returns a [`DesktopPlatform`].
/// The function is designed to be extended for mobile / console targets.
pub fn detect_platform() -> Box<dyn PlatformFacade> {
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    {
        Box::new(DesktopPlatform)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        // Fallback for unknown / future targets – a desktop-like stub.
        Box::new(DesktopPlatform)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_platform_returns_expected_defaults() {
        let platform = DesktopPlatform;
        let caps = platform.capabilities();

        assert!(!caps.has_touch);
        assert!(caps.has_keyboard);
        assert!(!caps.is_mobile);
        assert!(!caps.has_vibration);
        assert_eq!(caps.max_resolution, (3840, 2160));
        // has_gamepad depends on cfg! but we just verify it doesn't panic
        let _ = caps.has_gamepad;
    }

    #[test]
    fn desktop_vibrate_stub_does_not_panic() {
        let platform = DesktopPlatform;
        let result = platform.vibrate(100, 0.5);
        assert!(result.is_ok());
    }

    #[test]
    fn desktop_set_performance_mode_does_not_panic() {
        let platform = DesktopPlatform;
        let result = platform.set_performance_mode(PerformanceMode::Balanced);
        assert!(result.is_ok());
    }

    #[test]
    fn desktop_device_name_non_empty() {
        let platform = DesktopPlatform;
        assert!(!platform.device_name().is_empty());
        assert!(!platform.os_version().is_empty());
    }

    #[test]
    fn mock_platform_configurable() {
        let caps = PlatformCapabilities {
            has_touch: true,
            has_gamepad: false,
            has_keyboard: false,
            has_vibration: true,
            is_mobile: true,
            max_resolution: (1280, 720),
            performance_mode: PerformanceMode::PowerSave,
        };

        let mock = MockPlatform::new(caps.clone());
        assert!(mock.capabilities().has_touch);
        assert!(mock.capabilities().is_mobile);
        assert_eq!(
            mock.capabilities().performance_mode,
            PerformanceMode::PowerSave
        );
    }

    #[test]
    fn mock_vibrate_records_call() {
        let caps = PlatformCapabilities {
            has_touch: false,
            has_gamepad: false,
            has_keyboard: true,
            has_vibration: false,
            is_mobile: false,
            max_resolution: (1920, 1080),
            performance_mode: PerformanceMode::Balanced,
        };
        let mock = MockPlatform::new(caps);

        assert!(mock.last_vibrate().is_none());

        mock.vibrate(200, 0.75).unwrap();
        assert_eq!(mock.last_vibrate(), Some((200, 0.75)));

        mock.vibrate(500, 1.0).unwrap();
        assert_eq!(mock.last_vibrate(), Some((500, 1.0)));
    }

    #[test]
    fn mock_set_performance_mode_updates_capabilities() {
        let caps = PlatformCapabilities {
            has_touch: false,
            has_gamepad: false,
            has_keyboard: true,
            has_vibration: false,
            is_mobile: false,
            max_resolution: (1920, 1080),
            performance_mode: PerformanceMode::HighQuality,
        };
        let mock = MockPlatform::new(caps);

        assert_eq!(
            mock.capabilities().performance_mode,
            PerformanceMode::HighQuality
        );

        mock.set_performance_mode(PerformanceMode::PowerSave)
            .unwrap();
        assert_eq!(
            mock.capabilities().performance_mode,
            PerformanceMode::PowerSave
        );
        assert_eq!(mock.current_performance_mode(), PerformanceMode::PowerSave);
    }

    #[test]
    fn detect_platform_returns_non_null() {
        let plat = detect_platform();
        let caps = plat.capabilities();
        // Desktop default expectations
        assert!(!caps.is_mobile);
        assert!(caps.has_keyboard);
    }
}
