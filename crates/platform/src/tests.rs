use super::*;

#[test]
fn text_translation_keeps_unicode_and_filters_control_characters() {
    assert_eq!(
        character_events("é中\r\n"),
        vec![
            PlatformEvent::CharacterTyped { character: 'é' },
            PlatformEvent::CharacterTyped { character: '中' },
        ]
    );
}

#[test]
fn winit_keys_map_to_stable_platform_keys() {
    use winit::keyboard::KeyCode as WinitKey;

    assert_eq!(key_code_from_winit(WinitKey::KeyW), KeyCode::W);
    assert_eq!(key_code_from_winit(WinitKey::Digit0), KeyCode::Key0);
    assert_eq!(key_code_from_winit(WinitKey::Space), KeyCode::Space);
    assert_eq!(key_code_from_winit(WinitKey::ArrowLeft), KeyCode::Left);
    assert_eq!(key_code_from_winit(WinitKey::ShiftRight), KeyCode::RShift);
}

// ── WindowDescriptor tests ───────────────────────────────────────────

#[test]
fn window_descriptor_defaults() {
    let desc = WindowDescriptor::default();
    assert_eq!(desc.title, "Engine Sandbox");
    assert_eq!(desc.width, 1280);
    assert_eq!(desc.height, 720);
}

#[test]
fn window_descriptor_custom() {
    let desc = WindowDescriptor {
        title: "My Game".to_string(),
        width: 1920,
        height: 1080,
    };
    assert_eq!(desc.title, "My Game");
    assert_eq!(desc.width, 1920);
    assert_eq!(desc.height, 1080);
}

#[test]
fn window_descriptor_partial_eq() {
    let a = WindowDescriptor::default();
    let b = WindowDescriptor::default();
    let c = WindowDescriptor {
        title: "Custom".to_string(),
        width: 800,
        height: 600,
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn window_descriptor_debug() {
    let desc = WindowDescriptor::default();
    let debug = format!("{:?}", desc);
    assert!(debug.contains("WindowDescriptor"));
    assert!(debug.contains("Engine Sandbox"));
}

#[test]
fn window_descriptor_clone() {
    let desc = WindowDescriptor::default();
    let cloned = desc.clone();
    assert_eq!(desc, cloned);
}

// ── EventFlow tests ──────────────────────────────────────────────────

#[test]
fn event_flow_continue_vs_exit() {
    assert_eq!(EventFlow::Continue, EventFlow::Continue);
    assert_eq!(EventFlow::Exit, EventFlow::Exit);
    assert_ne!(EventFlow::Continue, EventFlow::Exit);
}

#[test]
fn event_flow_debug() {
    assert_eq!(format!("{:?}", EventFlow::Continue), "Continue");
    assert_eq!(format!("{:?}", EventFlow::Exit), "Exit");
}

#[test]
fn event_flow_copy_clone() {
    let a = EventFlow::Continue;
    let b = a;
    let c = a;
    assert_eq!(a, b);
    assert_eq!(a, c);
}

// ── PlatformEvent tests ──────────────────────────────────────────────

#[test]
fn platform_event_resumed() {
    assert_eq!(PlatformEvent::Resumed, PlatformEvent::Resumed);
    assert_ne!(PlatformEvent::Resumed, PlatformEvent::Suspended);
}

#[test]
fn platform_event_suspended() {
    assert_eq!(PlatformEvent::Suspended, PlatformEvent::Suspended);
}

#[test]
fn platform_event_resized() {
    let a = PlatformEvent::Resized {
        width: 1920,
        height: 1080,
    };
    let b = PlatformEvent::Resized {
        width: 1920,
        height: 1080,
    };
    let c = PlatformEvent::Resized {
        width: 800,
        height: 600,
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn platform_event_redraw() {
    assert_eq!(PlatformEvent::Redraw, PlatformEvent::Redraw);
}

#[test]
fn platform_event_close_requested() {
    assert_eq!(PlatformEvent::CloseRequested, PlatformEvent::CloseRequested);
}

#[test]
fn platform_event_debug() {
    assert_eq!(format!("{:?}", PlatformEvent::Redraw), "Redraw");
    assert_eq!(
        format!("{:?}", PlatformEvent::CloseRequested),
        "CloseRequested"
    );
    assert_eq!(
        format!(
            "{:?}",
            PlatformEvent::Resized {
                width: 100,
                height: 200
            }
        ),
        "Resized { width: 100, height: 200 }"
    );
}

#[test]
fn platform_event_clone() {
    let ev = PlatformEvent::Resized {
        width: 640,
        height: 480,
    };
    let cloned = ev.clone();
    assert_eq!(ev, cloned);
}

#[test]
fn platform_file_drop_preserves_the_exact_path() {
    let path = PathBuf::from(r"C:\project assets\hero.glb");
    let event = PlatformEvent::FileDropped { path: path.clone() };
    assert_eq!(event, PlatformEvent::FileDropped { path });
}

// ── PlatformError tests ──────────────────────────────────────────────

#[test]
fn platform_error_event_loop_display() {
    let err = PlatformError::EventLoop("init failed".to_string());
    assert_eq!(
        err.to_string(),
        "event loop initialization failed: init failed"
    );
}

#[test]
fn platform_error_window_creation_display() {
    let err = PlatformError::WindowCreation("no display".to_string());
    assert_eq!(err.to_string(), "window creation failed: no display");
}

#[test]
fn platform_error_debug() {
    let err = PlatformError::EventLoop("err".to_string());
    let debug = format!("{:?}", err);
    assert!(debug.contains("EventLoop"));
}
