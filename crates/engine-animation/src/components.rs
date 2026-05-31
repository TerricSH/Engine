use crate::ik::chain::IkChain;
use crate::ik::constraint::IkConstraintSet;
use crate::ik::effector::IkEffector;
use crate::layers::AnimLayer;
use crate::skeleton::BoneTransform;
use crate::state_machine::{AnimParamValue, AnimStateMachineInstance};
use engine_scene::Component;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnimationPlayer {
    pub clip_asset: Option<String>,
    pub playing: bool,
    pub looping: bool,
    pub speed: f32,
    pub current_time: f32,
    pub layer: u32,
    #[serde(default)]
    pub state_machine: Option<AnimStateMachineInstance>,
    #[serde(default)]
    pub layers: Vec<AnimLayer>,
    /// Cached bone world-space positions, populated by the pipeline after evaluation.
    #[serde(skip)]
    pub cached_bone_positions: Vec<[f32; 3]>,
}

impl Component for AnimationPlayer {
    const TYPE_ID: &'static str = "engine.animation_player";
}

impl AnimationPlayer {
    pub fn new() -> Self {
        Self {
            clip_asset: None,
            playing: false,
            looping: true,
            speed: 1.0,
            current_time: 0.0,
            layer: 0,
            state_machine: None,
            layers: vec![AnimLayer::new("base")],
            cached_bone_positions: Vec::new(),
        }
    }
    pub fn with_clip(clip_asset: impl Into<String>) -> Self {
        Self {
            clip_asset: Some(clip_asset.into()),
            playing: true,
            looping: true,
            speed: 1.0,
            current_time: 0.0,
            layer: 0,
            state_machine: None,
            layers: vec![AnimLayer::new("base")],
            cached_bone_positions: Vec::new(),
        }
    }
    pub fn set_state_machine(&mut self, sm: AnimStateMachineInstance) {
        self.state_machine = Some(sm);
    }
    pub fn set_anim_param(&mut self, name: &str, value: AnimParamValue) {
        if let Some(ref mut sm) = self.state_machine {
            sm.set_param(name, value);
        }
    }
    pub fn add_layer(&mut self, layer: AnimLayer) {
        self.layers.push(layer);
    }
    /// Play a specific animation clip by asset ID, bypassing the state machine.
    /// Sets state_machine to None to force direct clip playback.
    pub fn play_clip(&mut self, clip_asset: &str) {
        self.clip_asset = Some(clip_asset.to_string());
        self.playing = true;
        self.current_time = 0.0;
        self.state_machine = None;
    }
    /// Set the cached bone world positions (called by the pipeline after evaluation).
    pub fn set_cached_bone_positions(&mut self, global_transforms: &[BoneTransform]) {
        self.cached_bone_positions = global_transforms
            .iter()
            .map(|bt| bt.translation.to_array())
            .collect();
    }
}

impl Default for AnimationPlayer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkeletonComponent {
    pub skeleton_asset: Option<String>,
    pub bind_shape: [f32; 3],
}

impl Component for SkeletonComponent {
    const TYPE_ID: &'static str = "engine.skeleton";
}

impl SkeletonComponent {
    pub fn new(skeleton_asset: impl Into<String>) -> Self {
        Self {
            skeleton_asset: Some(skeleton_asset.into()),
            bind_shape: [0.5, 0.5, 0.5],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IkTargetComponent {
    pub effectors: Vec<IkEffector>,
    pub chains: Vec<IkChain>,
    #[serde(default)]
    pub constraints: IkConstraintSet,
    pub enabled: bool,
    #[serde(default = "default_ik_weight")]
    pub blend_weight: f32,
}

fn default_ik_weight() -> f32 {
    1.0
}

impl Component for IkTargetComponent {
    const TYPE_ID: &'static str = "engine.ik_target";
}

impl IkTargetComponent {
    pub fn new() -> Self {
        Self {
            effectors: Vec::new(),
            chains: Vec::new(),
            constraints: IkConstraintSet::new(),
            enabled: true,
            blend_weight: 1.0,
        }
    }
    pub fn add_effector(&mut self, effector: IkEffector) {
        self.effectors.push(effector);
    }
    pub fn add_chain(&mut self, chain: IkChain) {
        self.chains.push(chain);
    }
    pub fn effector(&self, name: &str) -> Option<&IkEffector> {
        self.effectors.iter().find(|e| e.name == name)
    }
    pub fn effector_mut(&mut self, name: &str) -> Option<&mut IkEffector> {
        self.effectors.iter_mut().find(|e| e.name == name)
    }
    pub fn set_target(&mut self, name: &str, position: glam::Vec3) {
        if let Some(e) = self.effector_mut(name) {
            e.position = position;
        }
    }
}

impl Default for IkTargetComponent {
    fn default() -> Self {
        Self::new()
    }
}
