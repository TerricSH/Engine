//! ECS components for the audio system.
//!
//! Provides [`AudioSourceComponent`] (playback of a single clip) and
//! [`AudioListenerComponent`] (scene listener marker). Both implement
//! the [`engine_scene::Component`] trait and can be serialized through
//! the `engine-scene` serialization pipeline.
//!
//! # Registration
//!
//! Call [`register_audio_extensions`] during engine initialisation to
//! register these component types with the ECS world.

use std::collections::BTreeMap;

use bincode;
use engine_serialize::Value;

use engine_scene::Component;

// ---------------------------------------------------------------------------
// AudioSourceComponent
// ---------------------------------------------------------------------------

/// ECS component that represents an audio source in the scene.
///
/// Attach this to an entity to give it a sound-emitting capability.
/// The `clip_asset` field references an audio clip by asset path; set
/// `playing = true` to start playback.
#[derive(Clone, Debug)]
pub struct AudioSourceComponent {
    /// Asset path of the audio clip to play (e.g. `"sounds/explosion.wav"`).
    pub clip_asset: Option<String>,
    /// Playback volume in the range `[0, 1]`.
    pub volume: f32,
    /// Whether the clip should loop when it reaches the end.
    pub looping: bool,
    /// Enable 3D spatial audio (position-based panning and attenuation).
    pub spatial: bool,
    /// Maximum distance (in metres) at which the sound is still audible.
    pub max_distance: f32,
    /// Rolloff factor for distance attenuation (`0.0` = no attenuation).
    pub rolloff_factor: f32,
    /// Whether the source is currently playing.
    pub playing: bool,
}

impl Component for AudioSourceComponent {
    const TYPE_ID: &'static str = "engine.audio_source";
}

impl Default for AudioSourceComponent {
    fn default() -> Self {
        Self {
            clip_asset: None,
            volume: 1.0,
            looping: false,
            spatial: false,
            max_distance: 10.0,
            rolloff_factor: 1.0,
            playing: false,
        }
    }
}

impl AudioSourceComponent {
    /// Create a new audio source with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new audio source that plays the given clip asset.
    pub fn with_clip(clip_asset: impl Into<String>) -> Self {
        Self {
            clip_asset: Some(clip_asset.into()),
            playing: true,
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// AudioListenerComponent
// ---------------------------------------------------------------------------

/// ECS component that marks an entity as the audio listener.
///
/// Only one listener should be active in a scene at any time. The
/// listener's transform (position + orientation) drives spatial audio
/// calculations for all [`AudioSourceComponent`] instances with
/// `spatial = true`.
#[derive(Clone, Debug)]
pub struct AudioListenerComponent {
    /// Whether this listener is active. When disabled the engine falls
    /// back to a default listener at the origin.
    pub enabled: bool,
}

impl Component for AudioListenerComponent {
    const TYPE_ID: &'static str = "engine.audio_listener";
}

impl Default for AudioListenerComponent {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl AudioListenerComponent {
    /// Create a new enabled listener component.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new listener component with the given enabled state.
    pub fn with_enabled(enabled: bool) -> Self {
        Self { enabled }
    }
}

// ---------------------------------------------------------------------------
// Serialization hooks
// ---------------------------------------------------------------------------

/// Serialize an [`AudioSourceComponent`] into a field map used by the
/// scene serialization pipeline.
pub fn serialize_audio_source(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let comp = component
        .downcast_ref::<AudioSourceComponent>()
        .expect("AudioSourceComponent expected");
    let mut fields = BTreeMap::new();

    if let Some(ref asset) = comp.clip_asset {
        fields.insert(
            "clip_asset".into(),
            Value::Asset(engine_serialize::AssetId::new(asset)),
        );
    }
    fields.insert("volume".into(), Value::Float32(comp.volume));
    fields.insert("looping".into(), Value::Bool(comp.looping));
    fields.insert("spatial".into(), Value::Bool(comp.spatial));
    fields.insert("max_distance".into(), Value::Float32(comp.max_distance));
    fields.insert("rolloff_factor".into(), Value::Float32(comp.rolloff_factor));
    fields.insert("playing".into(), Value::Bool(comp.playing));

    fields
}

/// Deserialize an [`AudioSourceComponent`] from a field map.
pub fn deserialize_audio_source(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let mut comp = AudioSourceComponent::new();

    if let Some(Value::Asset(asset)) = fields.get("clip_asset") {
        comp.clip_asset = Some(asset.id.clone());
    }
    if let Some(Value::Float32(v)) = fields.get("volume") {
        comp.volume = *v;
    }
    if let Some(Value::Bool(v)) = fields.get("looping") {
        comp.looping = *v;
    }
    if let Some(Value::Bool(v)) = fields.get("spatial") {
        comp.spatial = *v;
    }
    if let Some(Value::Float32(v)) = fields.get("max_distance") {
        comp.max_distance = *v;
    }
    if let Some(Value::Float32(v)) = fields.get("rolloff_factor") {
        comp.rolloff_factor = *v;
    }
    if let Some(Value::Bool(v)) = fields.get("playing") {
        comp.playing = *v;
    }

    Box::new(comp)
}

/// Serialize an [`AudioListenerComponent`] into a field map.
pub fn serialize_audio_listener(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let comp = component
        .downcast_ref::<AudioListenerComponent>()
        .expect("AudioListenerComponent expected");
    let mut fields = BTreeMap::new();

    fields.insert("enabled".into(), Value::Bool(comp.enabled));

    fields
}

/// Deserialize an [`AudioListenerComponent`] from a field map.
pub fn deserialize_audio_listener(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    let enabled = match fields.get("enabled") {
        Some(Value::Bool(v)) => *v,
        _ => true,
    };

    Box::new(AudioListenerComponent::with_enabled(enabled))
}

// ---------------------------------------------------------------------------
// Extension registration
// ---------------------------------------------------------------------------

/// Register audio ECS components and asset types with the Gate 9 extension
/// registries.
///
/// Call this once during engine initialisation:
///
/// ```ignore
/// register_audio_extensions(&mut component_registry, &mut asset_type_registry);
/// ```
pub fn register_audio_extensions(
    component_registry: &mut engine_scene::registry::ComponentRegistry,
    asset_type_registry: &mut engine_scene::registry::AssetTypeRegistry,
) {
    use engine_scene::registry::{ComponentExtension, ComponentMeta};
    use engine_scene::{ComponentStorageDyn, SparseSet};

    fn audio_source_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<AudioSourceComponent>::new())
    }
    fn audio_listener_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<AudioListenerComponent>::new())
    }

    let _ = component_registry.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: AudioSourceComponent::TYPE_ID,
            display_name: "Audio Source",
            schema_version: (0, 1, 0),
            has_editor: true,
            has_script_binding: true,
        },
        storage_factory: audio_source_storage,
        serialize: Some(serialize_audio_source),
        deserialize: Some(deserialize_audio_source),
    });

    let _ = component_registry.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: AudioListenerComponent::TYPE_ID,
            display_name: "Audio Listener",
            schema_version: (0, 1, 0),
            has_editor: true,
            has_script_binding: true,
        },
        storage_factory: audio_listener_storage,
        serialize: Some(serialize_audio_listener),
        deserialize: Some(deserialize_audio_listener),
    });

    // Register audio clip asset type (cooked = passthrough for now).
    use engine_scene::registry::{AssetTypeExtension, AssetTypeMeta};

    let audio_clip_ext = AssetTypeExtension {
        meta: AssetTypeMeta {
            type_id: "audio_clip",
            source_extensions: vec!["wav", "mp3", "ogg", "flac"],
            display_name: "Audio Clip",
        },
        cooker: Some(audio_clip_cooker),
        loader: Some(audio_clip_loader),
    };
    let _ = asset_type_registry.register(audio_clip_ext);
}

// ── Audio clip asset cooker / loader ───────────────────────────────────

/// Metadata header embedded at the start of every cooked audio asset.
///
/// The header is serialised with bincode before the raw clip data, allowing
/// the asset system to inspect properties (looping, streaming, compression)
/// without decoding the full PCM buffer.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CookedAudioHeader {
    /// Hint: should this clip loop by default?
    looping: bool,
    /// Hint: should this clip be streamed from disk (not fully loaded)?
    streaming: bool,
    /// Compression codec used for the PCM data (`"none"`, `"adpcm"`, …).
    compression: String,
    /// Original source format (e.g. `"wav"`, `"ogg"`, `"mp3"`).
    source_format: String,
    /// Sample rate in Hz.
    sample_rate: u32,
    /// Number of interleaved channels.
    channels: u16,
    /// Duration in seconds (approximate).
    duration_secs: f32,
}

fn audio_clip_cooker(source: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
    // Decode to extract metadata.
    let clip = crate::AudioClip::decode_from(source)
        .map_err(|e| format!("audio clip cook validation failed: {e}"))?;

    let header = CookedAudioHeader {
        // Default hints — overridable via cook manifest (future).
        looping: false,
        streaming: false,
        compression: "none".into(),
        source_format: clip.source_format().to_string(),
        sample_rate: clip.sample_rate(),
        channels: clip.channels(),
        duration_secs: clip.duration_seconds(),
    };

    // Write header followed by original source bytes.
    let header_bytes = bincode::serialize(&header)
        .map_err(|e| format!("failed to serialize audio header: {e}"))?;
    output.extend_from_slice(&header_bytes);
    output.extend_from_slice(source);

    Ok(())
}

fn audio_clip_loader(cooked: &[u8]) -> Result<Box<dyn std::any::Any>, String> {
    // Determine header size so we can skip to the source bytes.
    let header_size: usize = match bincode::deserialize::<CookedAudioHeader>(cooked) {
        Ok(hdr) => {
            bincode::serialized_size(&hdr).map_err(|e| format!("header size: {e}"))? as usize
        }
        Err(_) => {
            // Legacy cooked data without header — treat entire buffer as source.
            0
        }
    };

    let source_data = if header_size > 0 && header_size < cooked.len() {
        &cooked[header_size..]
    } else {
        cooked
    };

    let clip = crate::AudioClip::decode_from(source_data)
        .map_err(|e| format!("audio clip load failed: {e}"))?;
    Ok(Box::new(clip))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_source_component_creation() {
        let comp = AudioSourceComponent::new();
        assert_eq!(comp.clip_asset, None);
        assert!((comp.volume - 1.0).abs() < 1e-6);
        assert!(!comp.looping);
        assert!(!comp.spatial);
    }

    #[test]
    fn audio_source_component_with_clip() {
        let comp = AudioSourceComponent::with_clip("sounds/test.wav");
        assert_eq!(comp.clip_asset.as_deref(), Some("sounds/test.wav"));
        assert!(comp.playing);
    }

    #[test]
    fn audio_source_component_type_id() {
        assert_eq!(AudioSourceComponent::TYPE_ID, "engine.audio_source");
    }

    #[test]
    fn audio_listener_component_creation() {
        let comp = AudioListenerComponent::new();
        assert!(comp.enabled);
    }

    #[test]
    fn audio_listener_component_disabled() {
        let comp = AudioListenerComponent::with_enabled(false);
        assert!(!comp.enabled);
    }

    #[test]
    fn audio_listener_component_type_id() {
        assert_eq!(AudioListenerComponent::TYPE_ID, "engine.audio_listener");
    }

    #[test]
    fn audio_source_serde_roundtrip() {
        let mut comp = AudioSourceComponent::new();
        comp.clip_asset = Some("sounds/ambient.ogg".into());
        comp.volume = 0.75;
        comp.looping = true;
        comp.spatial = true;
        comp.max_distance = 25.0;
        comp.rolloff_factor = 0.5;
        comp.playing = true;

        let serialized = serialize_audio_source(&comp);
        let deserialized = deserialize_audio_source(&serialized);
        let restored: &AudioSourceComponent = deserialized.downcast_ref().unwrap();

        assert_eq!(restored.clip_asset.as_deref(), Some("sounds/ambient.ogg"));
        assert!((restored.volume - 0.75).abs() < 1e-6);
        assert!(restored.looping);
        assert!(restored.spatial);
        assert!((restored.max_distance - 25.0).abs() < 1e-6);
        assert!((restored.rolloff_factor - 0.5).abs() < 1e-6);
        assert!(restored.playing);
    }

    #[test]
    fn audio_source_serde_defaults_on_empty() {
        let fields = BTreeMap::new();
        let deserialized = deserialize_audio_source(&fields);
        let restored: &AudioSourceComponent = deserialized.downcast_ref().unwrap();

        assert_eq!(restored.clip_asset, None);
        assert!((restored.volume - 1.0).abs() < 1e-6);
        assert!(!restored.looping);
        assert!(!restored.spatial);
        assert!((restored.max_distance - 10.0).abs() < 1e-6);
        assert!((restored.rolloff_factor - 1.0).abs() < 1e-6);
        assert!(!restored.playing);
    }

    #[test]
    fn audio_listener_serde_roundtrip() {
        let comp = AudioListenerComponent::with_enabled(false);

        let serialized = serialize_audio_listener(&comp);
        let deserialized = deserialize_audio_listener(&serialized);
        let restored: &AudioListenerComponent = deserialized.downcast_ref().unwrap();

        assert!(!restored.enabled);
    }

    #[test]
    fn audio_listener_serde_defaults_on_empty() {
        let fields = BTreeMap::new();
        let deserialized = deserialize_audio_listener(&fields);
        let restored: &AudioListenerComponent = deserialized.downcast_ref().unwrap();

        // Default when field is missing
        assert!(restored.enabled);
    }
}
