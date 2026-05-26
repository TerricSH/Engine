#![forbid(unsafe_code)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowDescriptor {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowDescriptor {
    fn default() -> Self {
        Self {
            title: "Engine Sandbox".to_string(),
            width: 1280,
            height: 720,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlatformEvent {
    Resized { width: u32, height: u32 },
    CloseRequested,
    Suspended,
    Resumed,
}
