use serde::{Deserialize, Serialize};

/// Project-owned additive world streaming policy. Integer percentages keep
/// the manifest deterministic and `Eq` while runtime converts them to scale
/// factors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectWorldStreaming {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_stream_enter_percent")]
    pub enter_percent: u16,
    #[serde(default = "default_stream_exit_percent")]
    pub exit_percent: u16,
    #[serde(default = "default_stream_merges")]
    pub max_merges_per_frame: u16,
    #[serde(default = "default_stream_unloads")]
    pub max_unloads_per_frame: u16,
    /// Disable legacy altitude-triggered scene replacement and rely on
    /// additive planetary cells across space, atmosphere and surface bands.
    #[serde(default)]
    pub seamless_planetary: bool,
}

impl Default for ProjectWorldStreaming {
    fn default() -> Self {
        Self {
            enabled: false,
            enter_percent: default_stream_enter_percent(),
            exit_percent: default_stream_exit_percent(),
            max_merges_per_frame: default_stream_merges(),
            max_unloads_per_frame: default_stream_unloads(),
            seamless_planetary: false,
        }
    }
}

const fn default_stream_enter_percent() -> u16 {
    100
}

const fn default_stream_exit_percent() -> u16 {
    115
}

const fn default_stream_merges() -> u16 {
    1
}

const fn default_stream_unloads() -> u16 {
    4
}
