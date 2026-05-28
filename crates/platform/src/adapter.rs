//! [`PlatformAdapter`] trait — abstract interface over platform lifecycle,
//! input, and device-specific concerns.
//!
//! Desktop backends currently implement this trait trivially (no-op suspend/
//! resume, zero safe-area insets, etc.).  Mobile backends (Android, iOS)
//! provide real implementations backed by the platform runtime (Activity,
//! UIApplication).
//!
//! # Object safety
//!
//! `PlatformAdapter` is designed to be **object-safe** so that consumers
//! can hold `Box<dyn PlatformAdapter>` without generic plumbing.  All
//! methods take `&self` or `&mut self`; there are no associated types or
//! generic parameters.

use crate::PlatformProfile;

/// Abstract interface for platform lifecycle, IME, low-memory, and touch
/// input.  Every platform backend provides one instance of this trait.
///
/// # Lifecycle
///
/// The platform runtime calls [`on_suspend`][PlatformAdapter::on_suspend]
/// when the app is about to enter the background and
/// [`on_resume`][PlatformAdapter::on_resume] when it returns to the
/// foreground.  These correspond to `Activity.onPause`/`onResume` on
/// Android and `UIApplicationDelegate.applicationDidEnterBackground`/
/// `applicationWillEnterForeground` on iOS.
///
/// # Safe-area & IME
///
/// Mobile devices have notches, rounded corners, and software keyboards.
/// The adapter reports the current safe-area insets and IME state so the
/// UI system can avoid content clipping.
///
/// # Low-memory
///
/// [`low_memory_warning`][PlatformAdapter::low_memory_warning] is raised
/// when the OS needs the application to free resources.  The handler
/// should drop caches, compress textures, etc.
///
/// # Touch
///
/// Touch events are delivered through [`touch_event`][PlatformAdapter::touch_event].
/// Desktop platforms may synthesise these from mouse events if needed.
pub trait PlatformAdapter: Send + Sync {
    /// Return the static [`PlatformProfile`] describing this platform's
    /// capabilities.
    fn profile(&self) -> &PlatformProfile;

    /// Called when the application is about to enter the background.
    ///
    /// On mobile this is the place to pause audio, animation, and network
    /// activity.  On desktop this is a no-op unless the window is being
    /// hidden.
    fn on_suspend(&mut self) {}

    /// Called when the application returns to the foreground.
    ///
    /// Resume audio, animation, and network activity that was suspended.
    fn on_resume(&mut self) {}

    /// Current safe-area insets as `[top, right, bottom, left]` in logical
    /// pixels.  Desktop backends return `[0.0; 4]`.
    fn safe_area_insets(&self) -> [f32; 4] {
        [0.0; 4]
    }

    /// Called when the software keyboard (IME) opens.
    ///
    /// `keyboard_height` is the height of the keyboard in logical pixels.
    fn ime_open(&mut self, _keyboard_height: f32) {}

    /// Called when the software keyboard (IME) closes.
    fn ime_close(&mut self) {}

    /// Called when the OS issues a low-memory warning.
    ///
    /// The handler should free as much non-essential memory as possible.
    fn low_memory_warning(&mut self) {}

    /// Deliver a touch event to the platform adapter.
    ///
    /// On mobile this receives raw touch input from the OS.  Desktop
    /// backends may forward mouse events as synthetic touch events.
    fn touch_event(&mut self, _event: TouchEvent) {}
}

/// A single touch contact point.
///
/// Multitouch is supported: each finger gets a unique [`id`][TouchEvent::id]
/// that remains stable for the duration of that touch sequence (Began → Moved
/// → Ended / Cancelled).
#[derive(Clone, Debug)]
pub struct TouchEvent {
    /// Current phase of this touch sequence.
    pub phase: TouchPhase,
    /// X coordinate in logical pixels (left = 0).
    pub x: f32,
    /// Y coordinate in logical pixels (top = 0).
    pub y: f32,
    /// Unique identifier for this touch (finger) across the sequence.
    pub id: u64,
}

/// Phase of a touch event within a multi-touch sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    /// A new finger touched the screen.
    Began,
    /// A finger moved while remaining in contact.
    Moved,
    /// A finger was lifted from the screen.
    Ended,
    /// The touch sequence was interrupted (e.g. incoming call, system
    /// gesture recogniser).
    Cancelled,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal implementation used for testing trait object safety.
    struct TestAdapter {
        profile: &'static PlatformProfile,
    }

    impl PlatformAdapter for TestAdapter {
        fn profile(&self) -> &PlatformProfile {
            self.profile
        }
    }

    // ── Trait object safety ──────────────────────────────────────────────

    #[test]
    fn platform_adapter_is_object_safe() {
        // The whole point: we can hold `Box<dyn PlatformAdapter>`.
        let adapter: Box<dyn PlatformAdapter> = Box::new(TestAdapter {
            profile: &crate::DESKTOP_PROFILE,
        });
        assert_eq!(adapter.profile().name, "desktop");
    }

    // ── Default implementations ──────────────────────────────────────────

    #[test]
    fn default_safe_area_insets_are_zero() {
        let adapter = TestAdapter {
            profile: &crate::DESKTOP_PROFILE,
        };
        assert_eq!(adapter.safe_area_insets(), [0.0; 4]);
    }

    #[test]
    fn default_methods_do_not_panic() {
        let mut adapter = TestAdapter {
            profile: &crate::DESKTOP_PROFILE,
        };
        // These should all be no-ops.
        adapter.on_suspend();
        adapter.on_resume();
        adapter.ime_open(300.0);
        adapter.ime_close();
        adapter.low_memory_warning();
        adapter.touch_event(TouchEvent {
            phase: TouchPhase::Began,
            x: 100.0,
            y: 200.0,
            id: 0,
        });
    }

    // ── Touch Event ───────────────────────────────────────────────────────

    #[test]
    fn touch_event_construction() {
        let ev = TouchEvent {
            phase: TouchPhase::Moved,
            x: 320.0,
            y: 480.0,
            id: 1,
        };
        assert_eq!(ev.phase, TouchPhase::Moved);
        assert!((ev.x - 320.0).abs() < f32::EPSILON);
        assert!((ev.y - 480.0).abs() < f32::EPSILON);
        assert_eq!(ev.id, 1);
    }

    #[test]
    fn touch_phase_debug() {
        assert_eq!(format!("{:?}", TouchPhase::Began), "Began");
        assert_eq!(format!("{:?}", TouchPhase::Moved), "Moved");
        assert_eq!(format!("{:?}", TouchPhase::Ended), "Ended");
        assert_eq!(format!("{:?}", TouchPhase::Cancelled), "Cancelled");
    }

    #[test]
    fn touch_event_clone() {
        let a = TouchEvent {
            phase: TouchPhase::Began,
            x: 1.0,
            y: 2.0,
            id: 42,
        };
        let b = a.clone();
        assert_eq!(a.phase, b.phase);
        assert!((a.x - b.x).abs() < f32::EPSILON);
        assert_eq!(a.id, b.id);
    }
}
