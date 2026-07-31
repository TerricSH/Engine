mod snapshot;

use std::collections::{BTreeMap, BTreeSet};

use engine_serialize::Diagnostic;
use engine_terrain::{
    PlanetSceneBand, PlanetSceneTransitionConfig, PlanetSceneTransitionController,
    PlanetSceneTransitionError, PlanetSceneTransitionRequest, PlanetTerrainQuery, TerrainVolume,
};

use self::snapshot::{PlanetTransitionFrame, ResolvedTransition, RuntimeIssue};
use super::GameLoop;

/// A host-facing, two-phase request to replace the current project scene.
///
/// The project host must call
/// [`GameLoop::commit_planet_scene_transition`] only after the requested scene
/// is fully loaded and validated. Any rejection or load failure calls
/// [`GameLoop::reject_planet_scene_transition`], preserving the current band
/// and allowing a later frame to retry.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanetSceneTransitionTicket {
    /// Runtime-global transaction identity. Unlike the per-controller request
    /// serial, this does not reset when an authored controller is rebuilt.
    pub transaction_id: u64,
    pub controller_id: String,
    pub terrain_volume_id: String,
    pub request: PlanetSceneTransitionRequest,
}

#[derive(Clone, Debug)]
struct TrackedTransition {
    config: PlanetSceneTransitionConfig,
    terrain_volume_id: String,
    terrain_volume: TerrainVolume,
    controller: PlanetSceneTransitionController,
}

impl TrackedTransition {
    fn expected_scene_id(&self) -> &str {
        match self.controller.current_band() {
            PlanetSceneBand::Orbit => &self.config.orbit_scene_id,
            PlanetSceneBand::Surface => &self.config.surface_scene_id,
        }
    }
}

#[derive(Debug)]
pub(super) struct PlanetSceneTransitionRuntime {
    entries: BTreeMap<String, TrackedTransition>,
    pending: Option<PlanetSceneTransitionTicket>,
    pending_delivered: bool,
    sticky_controller: Option<String>,
    reported_issues: BTreeSet<String>,
    next_transaction_id: Option<u64>,
}

impl Default for PlanetSceneTransitionRuntime {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            pending: None,
            pending_delivered: false,
            sticky_controller: None,
            reported_issues: BTreeSet::new(),
            next_transaction_id: Some(1),
        }
    }
}

impl PlanetSceneTransitionRuntime {
    fn is_waiting_for_ack(&self) -> bool {
        self.pending.is_some()
    }

    fn tick(&mut self, mut frame: PlanetTransitionFrame, delta_seconds: f64) -> Vec<Diagnostic> {
        let mut issues = std::mem::take(&mut frame.issues);
        if self.pending.is_some() {
            return self.new_diagnostics(issues);
        }

        self.drop_stale_sticky_controller(&frame.scene_id);
        let mut active = BTreeSet::new();
        let mut samples = Vec::new();
        for resolved in &frame.resolved {
            active.insert(resolved.controller_id.clone());
            self.reconcile_resolved(resolved);
            if let Some(camera) = frame.camera_logical {
                match PlanetTerrainQuery::new(&resolved.terrain_volume) {
                    Ok(query) => {
                        samples.push((resolved.controller_id.clone(), query.altitude(camera)))
                    }
                    Err(error) => issues.push(RuntimeIssue::terrain_query(
                        &resolved.controller_id,
                        &resolved.terrain_volume_id,
                        error,
                    )),
                }
            }
        }

        self.sample_sticky_controller(&frame, &active, &mut samples, &mut issues);
        self.entries.retain(|controller_id, _| {
            active.contains(controller_id)
                || self.sticky_controller.as_deref() == Some(controller_id.as_str())
        });
        self.select_transition(samples, delta_seconds, &mut issues);
        self.new_diagnostics(issues)
    }

    fn drop_stale_sticky_controller(&mut self, scene_id: &str) {
        let Some(controller_id) = self.sticky_controller.as_deref() else {
            return;
        };
        let keep = self
            .entries
            .get(controller_id)
            .is_some_and(|entry| entry.expected_scene_id() == scene_id);
        if !keep {
            self.sticky_controller = None;
        }
    }

    fn reconcile_resolved(&mut self, resolved: &ResolvedTransition) {
        let must_replace = self
            .entries
            .get(&resolved.controller_id)
            .is_none_or(|entry| {
                entry.config != resolved.config
                    || entry.terrain_volume_id != resolved.terrain_volume_id
                    || entry.terrain_volume != resolved.terrain_volume
                    || entry.controller.current_band() != resolved.band
            });
        if !must_replace {
            return;
        }
        let Ok(controller) =
            PlanetSceneTransitionController::new(resolved.config.clone(), resolved.band)
        else {
            return;
        };
        self.entries.insert(
            resolved.controller_id.clone(),
            TrackedTransition {
                config: resolved.config.clone(),
                terrain_volume_id: resolved.terrain_volume_id.clone(),
                terrain_volume: resolved.terrain_volume.clone(),
                controller,
            },
        );
    }

    fn sample_sticky_controller(
        &mut self,
        frame: &PlanetTransitionFrame,
        active: &BTreeSet<String>,
        samples: &mut Vec<(String, f64)>,
        issues: &mut Vec<RuntimeIssue>,
    ) {
        let Some(controller_id) = self.sticky_controller.clone() else {
            return;
        };
        if active.contains(&controller_id) {
            return;
        }
        let Some(camera) = frame.camera_logical else {
            issues.push(RuntimeIssue::missing_camera(&controller_id));
            return;
        };
        let Some(entry) = self.entries.get_mut(&controller_id) else {
            self.sticky_controller = None;
            return;
        };
        if let Some(volume) = frame.valid_volumes.get(&entry.terrain_volume_id) {
            entry.terrain_volume = volume.clone();
        }
        match PlanetTerrainQuery::new(&entry.terrain_volume) {
            Ok(query) => samples.push((controller_id, query.altitude(camera))),
            Err(error) => issues.push(RuntimeIssue::terrain_query(
                &controller_id,
                &entry.terrain_volume_id,
                error,
            )),
        }
    }

    fn select_transition(
        &mut self,
        samples: Vec<(String, f64)>,
        delta_seconds: f64,
        issues: &mut Vec<RuntimeIssue>,
    ) {
        let mut candidates = Vec::new();
        for (controller_id, altitude) in samples {
            let Some(entry) = self.entries.get_mut(&controller_id) else {
                continue;
            };
            match entry.controller.update(altitude, delta_seconds) {
                Ok(Some(request)) => candidates.push((
                    altitude.abs(),
                    controller_id,
                    entry.terrain_volume_id.clone(),
                    request.clone(),
                )),
                Ok(None) => {}
                Err(error) => issues.push(RuntimeIssue::sample(&controller_id, error)),
            }
        }
        candidates.sort_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
        });
        let Some((_, selected_id, terrain_volume_id, request)) = candidates.first().cloned() else {
            return;
        };
        let Some(transaction_id) = self.next_transaction_id else {
            for (_, controller_id, _, request) in candidates {
                if let Some(entry) = self.entries.get_mut(&controller_id) {
                    let _ = entry.controller.reject(request.serial);
                }
            }
            issues.push(RuntimeIssue::transaction_ids_exhausted());
            return;
        };
        self.next_transaction_id = transaction_id.checked_add(1);
        for (_, controller_id, _, request) in candidates.into_iter().skip(1) {
            if let Some(entry) = self.entries.get_mut(&controller_id) {
                let _ = entry.controller.reject(request.serial);
            }
        }
        self.pending = Some(PlanetSceneTransitionTicket {
            transaction_id,
            controller_id: selected_id,
            terrain_volume_id,
            request,
        });
        self.pending_delivered = false;
    }

    fn new_diagnostics(&mut self, issues: Vec<RuntimeIssue>) -> Vec<Diagnostic> {
        let mut active = BTreeSet::new();
        let mut diagnostics = Vec::new();
        for issue in issues {
            active.insert(issue.key.clone());
            if !self.reported_issues.contains(&issue.key) {
                diagnostics.push(issue.diagnostic);
            }
        }
        self.reported_issues = active;
        diagnostics
    }

    fn take_pending(&mut self) -> Option<PlanetSceneTransitionTicket> {
        if self.pending_delivered {
            return None;
        }
        let ticket = self.pending.clone()?;
        self.pending_delivered = true;
        Some(ticket)
    }

    fn commit(
        &mut self,
        ticket: &PlanetSceneTransitionTicket,
    ) -> Result<(), PlanetSceneTransitionError> {
        self.matching_ticket(ticket)?;
        let entry = self
            .entries
            .get_mut(&ticket.controller_id)
            .ok_or(PlanetSceneTransitionError::RequestMismatch)?;
        entry.controller.commit(ticket.request.serial)?;
        if ticket.request.to == PlanetSceneBand::Surface {
            self.sticky_controller = Some(ticket.controller_id.clone());
        } else {
            self.sticky_controller = None;
        }
        self.pending = None;
        self.pending_delivered = false;
        Ok(())
    }

    fn reject(
        &mut self,
        ticket: &PlanetSceneTransitionTicket,
    ) -> Result<(), PlanetSceneTransitionError> {
        self.matching_ticket(ticket)?;
        self.entries
            .get_mut(&ticket.controller_id)
            .ok_or(PlanetSceneTransitionError::RequestMismatch)?
            .controller
            .reject(ticket.request.serial)?;
        self.pending = None;
        self.pending_delivered = false;
        Ok(())
    }

    fn matching_ticket(
        &self,
        ticket: &PlanetSceneTransitionTicket,
    ) -> Result<(), PlanetSceneTransitionError> {
        self.pending
            .as_ref()
            .filter(|pending| *pending == ticket)
            .map(|_| ())
            .ok_or(PlanetSceneTransitionError::RequestMismatch)
    }

    fn cancel_pending(&mut self) -> bool {
        let Some(ticket) = self.pending.clone() else {
            return false;
        };
        self.reject(&ticket).is_ok()
    }
}

impl GameLoop {
    /// Evaluate authored planet scene transitions using the active camera's
    /// logical f64 position. Ordinary [`Self::update`] calls this once per
    /// frame after terrain streaming.
    pub fn tick_planet_scene_transitions(&mut self, delta_seconds: f64) {
        if self.planet_scene_transitions.is_waiting_for_ack() {
            return;
        }
        let Some(scene_id) = self.runtime.scene_ref().map(|scene| scene.scene_id.clone()) else {
            return;
        };
        let Some(frame) = self
            .runtime
            .with_world(|world| PlanetTransitionFrame::capture(world, scene_id))
        else {
            return;
        };
        let diagnostics = self.planet_scene_transitions.tick(frame, delta_seconds);
        if !diagnostics.is_empty() {
            self.runtime
                .diagnostics_collector_mut()
                .push_scene_diags(diagnostics);
        }
    }

    /// Deliver the next scene transition once. The request remains pending
    /// until the host explicitly commits or rejects it.
    pub fn take_pending_planet_scene_transition(&mut self) -> Option<PlanetSceneTransitionTicket> {
        self.planet_scene_transitions.take_pending()
    }

    pub fn commit_planet_scene_transition(
        &mut self,
        ticket: &PlanetSceneTransitionTicket,
    ) -> Result<(), PlanetSceneTransitionError> {
        self.planet_scene_transitions.commit(ticket)
    }

    pub fn reject_planet_scene_transition(
        &mut self,
        ticket: &PlanetSceneTransitionTicket,
    ) -> Result<(), PlanetSceneTransitionError> {
        self.planet_scene_transitions.reject(ticket)
    }

    /// Reject an undelivered altitude transition when a higher-priority
    /// explicit scene request changes the active scene.
    pub fn cancel_pending_planet_scene_transition(&mut self) -> bool {
        self.planet_scene_transitions.cancel_pending()
    }
}

#[cfg(test)]
#[path = "planet_scene_transition/tests.rs"]
mod tests;
