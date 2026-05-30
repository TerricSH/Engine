use std::path::Path;

use crate::AudioError;

// ---------------------------------------------------------------------------
// AudioClip
// ---------------------------------------------------------------------------

/// Owns decoded PCM audio data stored as interleaved `f32` samples.
///
/// Construct via [`AudioClip::new`] (raw PCM) or [`AudioClip::decode`] / [`AudioClip::decode_from`]
/// (file or memory via symphonia).
pub struct AudioClip {
    /// Interleaved PCM samples (`L R L R ...` for stereo, `M M ...` for mono).
    samples: Vec<f32>,
    /// Sample rate in Hz (e.g. 44100).
    sample_rate: u32,
    /// Number of interleaved channels (1 = mono, 2 = stereo).
    channels: u16,
}

impl AudioClip {
    /// Create a new `AudioClip` from raw interleaved PCM samples.
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
        }
    }

    /// Borrow the interleaved PCM sample data.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// The sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Number of interleaved channels.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Total duration in seconds.
    pub fn duration_seconds(&self) -> f32 {
        if self.sample_rate == 0 || self.channels == 0 {
            return 0.0;
        }
        let frames = self.samples.len() / self.channels as usize;
        frames as f32 / self.sample_rate as f32
    }

    /// Decode an audio file from disk via symphonia.
    ///
    /// Supported formats depend on the symphonia features enabled at the workspace level
    /// (default: WAV, MP3, FLAC, OGG Vorbis).
    pub fn decode(path: &Path) -> Result<Self, AudioError> {
        let data = std::fs::read(path).map_err(|e| AudioError::DecodeError {
            detail: format!("failed to read '{}': {}", path.display(), e),
        })?;
        tracing::info!("reading audio file: {}", path.display());
        Self::decode_inner(data)
    }

    /// Decode audio from an in-memory byte slice.
    pub fn decode_from(data: &[u8]) -> Result<Self, AudioError> {
        Self::decode_inner(data.to_vec())
    }

    // ------------------------------------------------------------------
    // Internal decode implementation
    // ------------------------------------------------------------------
    fn decode_inner(data: Vec<u8>) -> Result<Self, AudioError> {
        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::codecs::DecoderOptions;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        // Wrap data in a media source stream (takes ownership for 'static bound).
        let source = Box::new(std::io::Cursor::new(data));
        let mss = MediaSourceStream::new(source, Default::default());

        // Probe the format (symphonia auto-detects container from content).
        let mut hint = Hint::new();
        hint.mime_type("audio/*");

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| AudioError::DecodeError {
                detail: format!("format probe failed: {}", e),
            })?;

        let mut format = probed.format;

        // Find the first non-null audio track.
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or_else(|| AudioError::DecodeError {
                detail: "no audio track found".to_string(),
            })?;

        let track_id = track.id;
        let codec_params = &track.codec_params;

        let sample_rate = codec_params
            .sample_rate
            .ok_or_else(|| AudioError::DecodeError {
                detail: "unknown sample rate".to_string(),
            })?;

        let num_channels: u16 =
            codec_params
                .channels
                .map(|c| c.count() as u16)
                .ok_or_else(|| AudioError::DecodeError {
                    detail: "unknown channel count".to_string(),
                })?;

        // Build the decoder for the track's codec.
        let codec_registry = symphonia::default::get_codecs();
        let mut decoder = codec_registry
            .make(codec_params, &DecoderOptions::default())
            .map_err(|e| AudioError::DecodeError {
                detail: format!("decoder creation failed: {}", e),
            })?;

        let mut all_samples: Vec<f32> = Vec::new();

        // Decode every packet.
        loop {
            let packet = match format.next_packet() {
                Ok(pkt) => {
                    if pkt.track_id() != track_id {
                        continue;
                    }
                    pkt
                }
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => {
                    return Err(AudioError::DecodeError {
                        detail: format!("packet read error: {}", e),
                    });
                }
            };

            let decoded = match decoder.decode(&packet) {
                Ok(buf) => buf,
                Err(symphonia::core::errors::Error::DecodeError(msg)) => {
                    tracing::warn!("audio decode warning: {}", msg);
                    continue;
                }
                Err(e) => {
                    return Err(AudioError::DecodeError {
                        detail: format!("decode error: {}", e),
                    });
                }
            };

            // Convert decoded audio to interleaved f32 via SampleBuffer.
            let spec = *decoded.spec();
            let frames = decoded.frames();
            let mut sample_buf = SampleBuffer::<f32>::new(frames as u64, spec);
            sample_buf.copy_interleaved_ref(decoded);
            all_samples.extend_from_slice(sample_buf.samples());
        }

        if all_samples.is_empty() {
            return Err(AudioError::DecodeError {
                detail: "no audio samples decoded".to_string(),
            });
        }

        let duration = all_samples.len() as f32 / (sample_rate as f32 * num_channels as f32);
        tracing::info!(
            "decoded audio: {} samples, {} ch, {} Hz, {:.2}s",
            all_samples.len(),
            num_channels,
            sample_rate,
            duration,
        );

        Ok(Self {
            samples: all_samples,
            sample_rate,
            channels: num_channels,
        })
    }
}
