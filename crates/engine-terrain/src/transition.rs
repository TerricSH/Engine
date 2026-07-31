use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanetSceneBand {
    Orbit,
    Surface,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanetSceneTransitionConfig {
    /// Disabled configurations are retained for authoring but never evaluated
    /// by the runtime.
    pub enabled: bool,
    /// Persistent entity ID carrying the target [`crate::TerrainVolume`].
    ///
    /// An empty value is a compatibility shorthand. It resolves only when
    /// the scene contains exactly one enabled, valid cube-sphere terrain
    /// volume; ambiguous scenes fail closed.
    pub terrain_volume_id: String,
    pub orbit_scene_id: String,
    pub surface_scene_id: String,
    /// Descending below this terrain-relative altitude requests the surface
    /// scene.
    pub enter_surface_altitude: f64,
    /// Climbing above this terrain-relative altitude requests the orbit scene.
    /// It must exceed `enter_surface_altitude` to provide hysteresis.
    pub exit_surface_altitude: f64,
    /// Minimum committed time in a band before another transition may begin.
    pub minimum_dwell_seconds: f64,
}

impl Default for PlanetSceneTransitionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            terrain_volume_id: String::new(),
            orbit_scene_id: "orbit".into(),
            surface_scene_id: "surface".into(),
            enter_surface_altitude: 2_000.0,
            exit_surface_altitude: 3_000.0,
            minimum_dwell_seconds: 1.0,
        }
    }
}

impl PlanetSceneTransitionConfig {
    pub fn validate(&self) -> Result<(), PlanetSceneTransitionError> {
        if self.orbit_scene_id.trim().is_empty()
            || self.surface_scene_id.trim().is_empty()
            || self.orbit_scene_id == self.surface_scene_id
            || !self.enter_surface_altitude.is_finite()
            || self.enter_surface_altitude < 0.0
            || !self.exit_surface_altitude.is_finite()
            || self.exit_surface_altitude <= self.enter_surface_altitude
            || !self.minimum_dwell_seconds.is_finite()
            || self.minimum_dwell_seconds < 0.0
        {
            return Err(PlanetSceneTransitionError::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanetSceneTransitionRequest {
    pub serial: u64,
    pub from: PlanetSceneBand,
    pub to: PlanetSceneBand,
    pub scene_id: String,
    pub altitude: f64,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum PlanetSceneTransitionError {
    #[error("planet scene transition configuration is invalid")]
    InvalidConfiguration,
    #[error(
        "planet scene transition delta/altitude must be finite and delta must be non-negative"
    )]
    InvalidSample,
    #[error("planet scene transition acknowledgement does not match the pending request")]
    RequestMismatch,
}

/// Two-phase altitude transition policy for projects that deliberately keep
/// orbit and surface in separate scenes.
///
/// Seamless cube-sphere terrain does not require this controller. It exists
/// for memory-constrained projects or scenes with intentionally different
/// simulation sets. The host resolves the returned scene ID transactionally,
/// then calls `commit`; a failed load calls `reject` and leaves the active
/// band unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanetSceneTransitionController {
    config: PlanetSceneTransitionConfig,
    current_band: PlanetSceneBand,
    dwell_seconds: f64,
    next_serial: u64,
    pending: Option<PlanetSceneTransitionRequest>,
}

impl PlanetSceneTransitionController {
    pub fn new(
        config: PlanetSceneTransitionConfig,
        current_band: PlanetSceneBand,
    ) -> Result<Self, PlanetSceneTransitionError> {
        config.validate()?;
        Ok(Self {
            config,
            current_band,
            dwell_seconds: 0.0,
            next_serial: 1,
            pending: None,
        })
    }

    pub fn config(&self) -> &PlanetSceneTransitionConfig {
        &self.config
    }

    pub fn current_band(&self) -> PlanetSceneBand {
        self.current_band
    }

    pub fn pending(&self) -> Option<&PlanetSceneTransitionRequest> {
        self.pending.as_ref()
    }

    pub fn update(
        &mut self,
        terrain_relative_altitude: f64,
        delta_seconds: f64,
    ) -> Result<Option<&PlanetSceneTransitionRequest>, PlanetSceneTransitionError> {
        if !terrain_relative_altitude.is_finite()
            || !delta_seconds.is_finite()
            || delta_seconds < 0.0
        {
            return Err(PlanetSceneTransitionError::InvalidSample);
        }
        if !self.config.enabled {
            return Ok(None);
        }
        self.dwell_seconds += delta_seconds;
        if self.pending.is_some() || self.dwell_seconds < self.config.minimum_dwell_seconds {
            return Ok(self.pending.as_ref());
        }
        let target = match self.current_band {
            PlanetSceneBand::Orbit
                if terrain_relative_altitude <= self.config.enter_surface_altitude =>
            {
                Some((
                    PlanetSceneBand::Surface,
                    self.config.surface_scene_id.clone(),
                ))
            }
            PlanetSceneBand::Surface
                if terrain_relative_altitude >= self.config.exit_surface_altitude =>
            {
                Some((PlanetSceneBand::Orbit, self.config.orbit_scene_id.clone()))
            }
            _ => None,
        };
        if let Some((to, scene_id)) = target {
            self.pending = Some(PlanetSceneTransitionRequest {
                serial: self.next_serial,
                from: self.current_band,
                to,
                scene_id,
                altitude: terrain_relative_altitude,
            });
            self.next_serial = self.next_serial.wrapping_add(1).max(1);
        }
        Ok(self.pending.as_ref())
    }

    pub fn commit(&mut self, serial: u64) -> Result<(), PlanetSceneTransitionError> {
        let Some(request) = self.pending.take() else {
            return Err(PlanetSceneTransitionError::RequestMismatch);
        };
        if request.serial != serial {
            self.pending = Some(request);
            return Err(PlanetSceneTransitionError::RequestMismatch);
        }
        self.current_band = request.to;
        self.dwell_seconds = 0.0;
        Ok(())
    }

    pub fn reject(&mut self, serial: u64) -> Result<(), PlanetSceneTransitionError> {
        if self.pending.as_ref().map(|request| request.serial) != Some(serial) {
            return Err(PlanetSceneTransitionError::RequestMismatch);
        }
        self.pending = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller() -> PlanetSceneTransitionController {
        PlanetSceneTransitionController::new(
            PlanetSceneTransitionConfig {
                enabled: true,
                terrain_volume_id: "planet".into(),
                orbit_scene_id: "orbit".into(),
                surface_scene_id: "surface".into(),
                enter_surface_altitude: 2_000.0,
                exit_surface_altitude: 3_000.0,
                minimum_dwell_seconds: 1.0,
            },
            PlanetSceneBand::Orbit,
        )
        .unwrap()
    }

    #[test]
    fn hysteresis_and_dwell_prevent_boundary_thrashing() {
        let mut controller = controller();
        assert!(controller.update(1_500.0, 0.5).unwrap().is_none());
        let request = controller.update(1_500.0, 0.5).unwrap().unwrap().clone();
        assert_eq!(request.scene_id, "surface");
        controller.commit(request.serial).unwrap();
        assert!(controller.update(3_500.0, 0.5).unwrap().is_none());
        let request = controller.update(3_500.0, 0.5).unwrap().unwrap().clone();
        assert_eq!(request.scene_id, "orbit");
        controller.commit(request.serial).unwrap();
        assert_eq!(controller.current_band(), PlanetSceneBand::Orbit);
    }

    #[test]
    fn failed_load_keeps_the_current_band_and_can_retry() {
        let mut controller = controller();
        let first = controller.update(1_000.0, 1.0).unwrap().unwrap().clone();
        controller.reject(first.serial).unwrap();
        assert_eq!(controller.current_band(), PlanetSceneBand::Orbit);
        let retry = controller.update(1_000.0, 0.0).unwrap().unwrap();
        assert_ne!(retry.serial, first.serial);
        assert_eq!(retry.to, PlanetSceneBand::Surface);
    }

    #[test]
    fn disabled_controller_never_emits_a_request() {
        let mut controller = PlanetSceneTransitionController::new(
            PlanetSceneTransitionConfig::default(),
            PlanetSceneBand::Orbit,
        )
        .unwrap();
        assert!(controller.update(0.0, 10_000.0).unwrap().is_none());
        assert_eq!(controller.current_band(), PlanetSceneBand::Orbit);
    }
}
