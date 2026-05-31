//! Bridge between character controller and the animation pipeline.
//!
//! Wires [`CharacterController`] → [`AnimParams`] → state machine → [`update_animation_pipeline`].
//! Call [`update_character_animation`] once per character per frame after the
//! controller has been updated.

use engine_animation::{
    locomotion_state_machine, skeleton, update_animation_pipeline, AnimStateMachineInstance,
    AnimationPlayer, IkTargetComponent,
};

use crate::{AnimParams, CharacterController};

/// Run the full character animation pipeline for one character.
///
/// 1. Extracts [`AnimParams`] from the controller
/// 2. Initialises the state machine if needed (using [`locomotion_state_machine`])
/// 3. Applies parameters to the state machine
/// 4. Calls [`update_animation_pipeline`] (evaluate → blend → IK → skin)
/// 5. Returns bone palette matrices
///
/// Call this once per character per frame **after** `controller.update()`.
pub fn update_character_animation(
    controller: &CharacterController,
    player: &mut AnimationPlayer,
    clips: &[(&str, engine_animation::AnimationClip)],
    skel: &skeleton::Skeleton,
    ik: Option<&IkTargetComponent>,
    dt: f32,
) -> Vec<[[f32; 4]; 4]> {
    // 1. Create a state machine instance if one doesn't exist yet
    if player.state_machine.is_none() {
        let sm_def = locomotion_state_machine();
        player.state_machine = Some(AnimStateMachineInstance::new(sm_def));
    }

    // 2. Extract animation params from controller and apply to SM
    let params = AnimParams::from_controller(controller);
    if let Some(ref mut sm) = player.state_machine {
        params.apply_to_state_machine(sm);
    }

    // 3. Run the full pipeline.
    //    We temporarily take ownership of the state machine to avoid a dual
    //    mutable borrow of `player` (pipeline needs both &mut player and
    //    &mut state_machine). The take/put-back pattern is standard Rust
    //    for this scenario. On the rare event of a panic during evaluation,
    //    the state machine is dropped — this is acceptable because a panic
    //    in the animation pipeline means the engine is in an unrecoverable
    //    state anyway.
    let mut sm = player.state_machine.take();
    let result = update_animation_pipeline(player, &mut sm, clips, skel, ik, dt);
    player.state_machine = sm;
    result
}
