use engine_scene::WorldSlot;
use engine_serialize::Diagnostic;

use crate::runtime::script_component_diagnostic;

#[cfg_attr(not(feature = "subsystem-animation"), allow(dead_code))]
pub(crate) enum ScriptAnimationCommand {
    PlayClip {
        clip_asset: String,
        looping: bool,
        speed: f32,
        restart: bool,
    },
    SetParameter {
        name: String,
        value: engine_script::GameplayAnimationParameterValue,
    },
    Transition {
        state: String,
    },
    SetPlaying {
        playing: bool,
    },
}

pub(crate) fn apply_script_animation_command(
    world_slot: &WorldSlot,
    requested_by: &str,
    target_id: &str,
    command: ScriptAnimationCommand,
    diagnostics: &mut Vec<Diagnostic>,
) {
    #[cfg(feature = "subsystem-animation")]
    {
        let applied = world_slot.with_world_mut(|world| {
            let entity = world
                .entity_by_persistent_id(target_id)
                .ok_or_else(|| format!("unknown entity '{target_id}'"))?;
            let player = world
                .get_mut::<engine_animation::AnimationPlayer>(entity)
                .ok_or_else(|| format!("entity '{target_id}' has no AnimationPlayer"))?;
            match command {
                ScriptAnimationCommand::PlayClip {
                    clip_asset,
                    looping,
                    speed,
                    restart,
                } => {
                    if restart || player.clip_asset.as_deref() != Some(clip_asset.as_str()) {
                        player.play_clip(&clip_asset);
                    } else {
                        player.playing = true;
                    }
                    player.looping = looping;
                    player.speed = speed;
                    Ok(())
                }
                ScriptAnimationCommand::SetParameter { name, value } => {
                    let state_machine = player.state_machine.as_mut().ok_or_else(|| {
                        format!("entity '{target_id}' has no animation state machine")
                    })?;
                    let value = match value {
                        engine_script::GameplayAnimationParameterValue::Float(value) => {
                            engine_animation::AnimParamValue::Float(value)
                        }
                        engine_script::GameplayAnimationParameterValue::Int(value) => {
                            engine_animation::AnimParamValue::Int(value)
                        }
                        engine_script::GameplayAnimationParameterValue::Bool(value) => {
                            engine_animation::AnimParamValue::Bool(value)
                        }
                    };
                    state_machine.set_param(&name, value);
                    Ok(())
                }
                ScriptAnimationCommand::Transition { state } => {
                    let state_machine = player.state_machine.as_mut().ok_or_else(|| {
                        format!("entity '{target_id}' has no animation state machine")
                    })?;
                    if state_machine.force_transition_to(&state) {
                        Ok(())
                    } else {
                        Err(format!(
                            "entity '{target_id}' animation state machine has no state '{state}'"
                        ))
                    }
                }
                ScriptAnimationCommand::SetPlaying { playing } => {
                    player.playing = playing;
                    Ok(())
                }
            }
        });
        match applied {
            Some(Ok(())) => {}
            Some(Err(reason)) => diagnostics.push(script_component_diagnostic(
                "SCRIPT_ANIMATION_APPLY_FAILED",
                requested_by,
                format!(
                    "script entity '{requested_by}' could not update animation on '{target_id}': {reason}"
                ),
            )),
            None => diagnostics.push(script_component_diagnostic(
                "SCRIPT_WORLD_MISSING",
                requested_by,
                format!(
                    "script entity '{requested_by}' could not update animation on '{target_id}' because no World is active"
                ),
            )),
        }
    }
    #[cfg(not(feature = "subsystem-animation"))]
    {
        let _ = (world_slot, target_id, command);
        diagnostics.push(script_component_diagnostic(
            "SCRIPT_ANIMATION_UNAVAILABLE",
            requested_by,
            format!(
                "script entity '{requested_by}' requested animation control, but engine-core was built without subsystem-animation"
            ),
        ));
    }
}

pub(crate) fn apply_script_morph_weights(
    world_slot: &WorldSlot,
    requested_by: &str,
    target_id: &str,
    weights: Vec<f32>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    #[cfg(feature = "subsystem-animation")]
    {
        let applied = world_slot.with_world_mut(|world| {
            let entity = world
                .entity_by_persistent_id(target_id)
                .ok_or_else(|| format!("unknown entity '{target_id}'"))?;
            let skeleton = world
                .get_mut::<engine_animation::SkeletonComponent>(entity)
                .ok_or_else(|| format!("entity '{target_id}' has no Skeleton component"))?;
            if skeleton.morph_target_set.is_none() {
                return Err(format!(
                    "entity '{target_id}' has no morph_target_set configured"
                ));
            }
            skeleton.morph_weights = weights;
            Ok(())
        });
        match applied {
            Some(Ok(())) => {}
            Some(Err(reason)) => diagnostics.push(script_component_diagnostic(
                "SCRIPT_ANIMATION_APPLY_FAILED",
                requested_by,
                format!(
                    "script entity '{requested_by}' could not update morph weights on '{target_id}': {reason}"
                ),
            )),
            None => diagnostics.push(script_component_diagnostic(
                "SCRIPT_WORLD_MISSING",
                requested_by,
                "morph weights could not be applied because no World is active".into(),
            )),
        }
    }
    #[cfg(not(feature = "subsystem-animation"))]
    {
        let _ = (world_slot, target_id, weights);
        diagnostics.push(script_component_diagnostic(
            "SCRIPT_ANIMATION_UNAVAILABLE",
            requested_by,
            "morph weights require subsystem-animation".into(),
        ));
    }
}
