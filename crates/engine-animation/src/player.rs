use std::sync::Arc;

use crate::{AnimationClip, Pose, Skeleton};

// ---------------------------------------------------------------------------
// AnimationPlayer — single-clip player with crossfade support.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ClipState {
    clip: Arc<AnimationClip>,
    elapsed: f32,
    speed: f32,
    looped: bool,
}

/// Internal crossfade tracking.
///
/// The first `update` call after a transition converts a `Pending` into an
/// `Active` crossfade by sampling the *from*-clip pose against the skeleton.
#[derive(Debug, Clone)]
enum Crossfade {
    /// Waiting for the first `update()` to capture the from-pose.
    Pending {
        from_clip: Arc<AnimationClip>,
        from_elapsed: f32,
        duration: f32,
    },
    /// Actively blending between a frozen from-pose and the current clip.
    Active {
        from_pose: Pose,
        duration: f32,
        elapsed: f32,
    },
    /// Fading to rest pose.
    Fading {
        from_pose: Pose,
        duration: f32,
        elapsed: f32,
    },
}

#[derive(Debug, Clone)]
pub struct AnimationPlayer {
    state: Option<ClipState>,
    crossfade: Option<Crossfade>,
    /// Non-None when `stop(fade_duration)` was called with a positive fade.
    /// The `update()` method captures the current pose and transitions to
    /// `Crossfade::Fading` on the first subsequent tick.
    pending_stop_fade: Option<f32>,
}

impl AnimationPlayer {
    pub fn new() -> Self {
        tracing::debug!("AnimationPlayer created");
        Self {
            state: None,
            crossfade: None,
            pending_stop_fade: None,
        }
    }

    /// Start playing `clip`.
    ///
    /// If a different clip is already playing and `blend_duration > 0` the
    /// player crossfades from the previous clip to the new one.  If the same
    /// clip is already playing it is restarted from the beginning.
    pub fn play(&mut self, clip: Arc<AnimationClip>, blend_duration: f32) {
        // Cancel any pending stop.
        self.pending_stop_fade = None;

        // If the same clip is already playing, just restart it.
        if let Some(ref state) = self.state {
            if Arc::ptr_eq(&state.clip, &clip) {
                let speed = state.speed;
                let looped = state.looped;
                self.state = Some(ClipState {
                    clip,
                    elapsed: 0.0,
                    speed,
                    looped,
                });
                self.crossfade = None;
                tracing::debug!("AnimationPlayer restarted same clip");
                return;
            }
        }

        // Capture the outgoing clip state for crossfading.
        let prev = self.state.take();
        self.state = Some(ClipState {
            clip: clip.clone(),
            elapsed: 0.0,
            speed: 1.0,
            looped: true,
        });

        if let Some(prev) = prev {
            if blend_duration > 0.0 {
                tracing::debug!(
                    from = %prev.clip.name(),
                    to = %clip.name(),
                    blend = blend_duration,
                    "AnimationPlayer crossfade started"
                );
                self.crossfade = Some(Crossfade::Pending {
                    from_clip: prev.clip,
                    from_elapsed: prev.elapsed,
                    duration: blend_duration,
                });
            } else {
                self.crossfade = None;
            }
        } else {
            self.crossfade = None;
        }
    }

    /// Stop playback.
    ///
    /// If `fade_duration > 0` the player fades toward the rest pose over the
    /// given period.  Otherwise it stops immediately.
    pub fn stop(&mut self, fade_duration: f32) {
        if fade_duration <= 0.0 || self.state.is_none() {
            self.state = None;
            self.crossfade = None;
            self.pending_stop_fade = None;
            tracing::debug!("AnimationPlayer stopped immediately");
            return;
        }

        tracing::debug!(fade = fade_duration, "AnimationPlayer stop with fade");
        // Cancel any in-flight crossfade — the fade-to-rest takes priority.
        self.crossfade = None;
        // Defer pose capture to the next update() call where we have a skeleton.
        self.pending_stop_fade = Some(fade_duration);
    }

    /// Advance the player by `dt` seconds and return the resulting pose.
    ///
    /// This is the main update function — it advances time, handles looping,
    /// resolves pending crossfades / stop-fades, and blends if necessary.
    pub fn update(&mut self, dt: f32, skeleton: &Skeleton) -> Pose {
        // ── Handle pending stop-with-fade ──────────────────────────────
        if let Some(fade_duration) = self.pending_stop_fade.take() {
            if let Some(ref state) = self.state {
                let from_pose = state.clip.sample(state.elapsed, skeleton);
                self.crossfade = Some(Crossfade::Fading {
                    from_pose,
                    duration: fade_duration,
                    elapsed: 0.0,
                });
            }
            self.state = None;
        }

        // ── Advance the current clip's playhead ────────────────────────
        if let Some(ref mut state) = self.state {
            state.elapsed += dt * state.speed;
            let dur = state.clip.duration();
            if dur > 0.0 {
                if state.looped {
                    state.elapsed = state.elapsed.rem_euclid(dur);
                } else {
                    state.elapsed = state.elapsed.clamp(0.0, dur);
                }
            }
        }

        // ── Resolve a Pending crossfade (capture the from-pose) ────────
        if let Some(Crossfade::Pending {
            ref from_clip,
            from_elapsed,
            duration,
        }) = self.crossfade.clone()
        {
            let from_elapsed = from_elapsed + dt;
            let from_pose = from_clip.sample(from_elapsed, skeleton);
            self.crossfade = Some(Crossfade::Active {
                from_pose,
                duration,
                elapsed: 0.0,
            });
        }

        // ── Sample the current clip ────────────────────────────────────
        let current_pose = match self.state {
            Some(ref state) => state.clip.sample(state.elapsed, skeleton),
            None => {
                // If there's still a Fading crossfade (no current clip),
                // we continue fading.  Otherwise return a rest pose.
                match self.crossfade {
                    Some(Crossfade::Fading { .. }) => {} // handled below
                    _ => return Pose::new(skeleton),
                }
                Pose::new(skeleton)
            }
        };

        // ── Apply active crossfade ─────────────────────────────────────
        match self.crossfade {
            Some(Crossfade::Active {
                ref from_pose,
                duration,
                ref mut elapsed,
            }) => {
                *elapsed += dt;
                let t = (*elapsed / duration).clamp(0.0, 1.0);
                if t >= 1.0 {
                    self.crossfade = None;
                    current_pose
                } else {
                    Pose::blend(from_pose, &current_pose, t)
                }
            }
            Some(Crossfade::Fading {
                ref from_pose,
                duration,
                ref mut elapsed,
            }) => {
                *elapsed += dt;
                let t = (*elapsed / duration).clamp(0.0, 1.0);
                if t >= 1.0 {
                    self.crossfade = None;
                    Pose::new(skeleton)
                } else {
                    let rest = Pose::new(skeleton);
                    Pose::blend(from_pose, &rest, t)
                }
            }
            _ => current_pose,
        }
    }

    /// Set playback speed multiplier.  `1.0` is normal speed.
    pub fn set_speed(&mut self, factor: f32) {
        if let Some(ref mut state) = self.state {
            state.speed = factor;
        }
    }

    /// Enable or disable looping.
    pub fn set_loop(&mut self, looped: bool) {
        if let Some(ref mut state) = self.state {
            state.looped = looped;
        }
    }

    /// Whether the player has an active clip.
    pub fn is_playing(&self) -> bool {
        self.state.is_some()
    }

    /// Name of the currently playing clip, or `None` if stopped.
    pub fn current_clip_name(&self) -> Option<&str> {
        self.state.as_ref().map(|s| s.clip.name())
    }
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Animator — higher-level animation controller wrapping AnimationPlayer.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Animator {
    player: AnimationPlayer,
}

impl Animator {
    pub fn new() -> Self {
        tracing::debug!("Animator created");
        Self {
            player: AnimationPlayer::new(),
        }
    }

    /// Start playing `clip`, crossfading from the current clip over
    /// `blend_duration` seconds.  Delegates to the internal player.
    pub fn play(&mut self, clip: Arc<AnimationClip>, blend_duration: f32) {
        self.player.play(clip, blend_duration);
    }

    /// Advance the animator and return the evaluated pose.
    pub fn update(&mut self, dt: f32, skeleton: &Skeleton) -> Pose {
        self.player.update(dt, skeleton)
    }
}

impl Default for Animator {
    fn default() -> Self {
        Self::new()
    }
}
