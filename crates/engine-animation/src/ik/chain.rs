//! IK chain definitions.
//!
//! An [`IkChain`] describes a sequence of bones that form a kinematic chain
//! (tip → base) for the IK solver to operate on. Common humanoid chains:
//!
//! * **Arm** (shoulder → upper arm → forearm → hand)
//! * **Leg** (hip → thigh → shin → foot)
//! * **Spine** (pelvis → spine → chest → neck → head)
//!
//! Each chain specifies which solver algorithm to use, convergence parameters,
//! and an overall blend weight.

use serde::{Deserialize, Serialize};

use crate::BoneIndex;

/// IK solver algorithm selection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum IkSolverType {
    /// Forward And Backward Reaching Inverse Kinematics.
    ///
    /// Fast, stable, and handles arbitrary chain lengths well. Does not
    /// natively respect joint angle limits (use constraints separately).
    Fabrik,

    /// Cyclic Coordinate Descent.
    ///
    /// Iteratively rotates each joint in the chain to minimise the distance
    /// to the target. Naturally supports joint angle limits per-bone.
    Ccd,
}

impl Default for IkSolverType {
    fn default() -> Self {
        Self::Fabrik
    }
}

/// An IK chain — a sequence of bones forming one kinematic chain.
///
/// The chain is stored **tip-first** (end effector bone at index 0, base bone
/// at the last index). This matches the expected ordering for both FABRIK
/// and CCD solvers.
///
/// # Example
///
/// For a right arm chain with bones [hand, forearm, upper_arm, shoulder]:
///
/// ```ignore
/// IkChain {
///     name: "arm_r".into(),
///     bones: vec![hand, forearm, upper_arm, shoulder],
///     solver: IkSolverType::Fabrik,
///     iteration_count: 10,
///     tolerance: 0.01,
///     weight: 1.0,
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IkChain {
    /// Human-readable name (e.g. "arm_r", "leg_l").
    pub name: String,

    /// Bones in **tip → base** order (effector tip first).
    ///
    /// Must have at least 2 bones for a solvable chain.
    pub bones: Vec<BoneIndex>,

    /// Which solver algorithm to use for this chain.
    #[serde(default)]
    pub solver: IkSolverType,

    /// Maximum solver iterations per frame.
    #[serde(default = "default_iterations")]
    pub iteration_count: u32,

    /// Convergence tolerance in metres. When the effector is within this
    /// distance of the target, the solver stops early.
    #[serde(default = "default_tolerance")]
    pub tolerance: f32,

    /// Overall blend weight `[0, 1]` for this chain.
    /// `0.0` = no IK, `1.0` = full IK solve.
    #[serde(default = "default_weight")]
    pub weight: f32,

    /// Whether this chain is active.
    pub enabled: bool,
}

const fn default_iterations() -> u32 {
    10
}

const fn default_tolerance() -> f32 {
    0.01
}

const fn default_weight() -> f32 {
    1.0
}

impl IkChain {
    /// Create a new IK chain with the given name and bones (tip → base).
    pub fn new(name: impl Into<String>, bones: Vec<BoneIndex>) -> Self {
        Self {
            name: name.into(),
            bones,
            solver: IkSolverType::Fabrik,
            iteration_count: default_iterations(),
            tolerance: default_tolerance(),
            weight: default_weight(),
            enabled: true,
        }
    }

    /// Builder: set the solver type.
    pub fn with_solver(mut self, solver: IkSolverType) -> Self {
        self.solver = solver;
        self
    }

    /// Builder: set iteration count.
    pub fn with_iterations(mut self, count: u32) -> Self {
        self.iteration_count = count.max(1);
        self
    }

    /// Builder: set convergence tolerance.
    pub fn with_tolerance(mut self, tol: f32) -> Self {
        self.tolerance = tol.max(1e-6);
        self
    }

    /// Builder: set blend weight.
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight.clamp(0.0, 1.0);
        self
    }

    /// Returns the number of bones in this chain.
    pub fn len(&self) -> usize {
        self.bones.len()
    }

    /// Returns `true` if the chain has fewer than 2 bones (unsolvable).
    pub fn is_trivial(&self) -> bool {
        self.bones.len() < 2
    }

    /// The tip (end effector) bone index.
    pub fn tip(&self) -> Option<BoneIndex> {
        self.bones.first().copied()
    }

    /// The base (root) bone index.
    pub fn base(&self) -> Option<BoneIndex> {
        self.bones.last().copied()
    }

    /// Returns `true` if this chain contains `bone`.
    pub fn contains(&self, bone: BoneIndex) -> bool {
        self.bones.contains(&bone)
    }

    /// Returns `true` if this chain is active and has a non-zero weight.
    pub fn is_active(&self) -> bool {
        self.enabled && self.weight > 0.0 && !self.is_trivial()
    }
}

// ---------------------------------------------------------------------------
// Pre-defined humanoid chain helpers
// ---------------------------------------------------------------------------

/// Helper to construct typical humanoid IK chains from bone indices.
pub mod chains {
    use crate::ik::chain::{IkChain, IkSolverType};
    use crate::BoneIndex;

    /// Build a right-arm IK chain: hand → forearm → upper_arm → shoulder.
    pub fn arm_right(
        hand: BoneIndex,
        forearm: BoneIndex,
        upper_arm: BoneIndex,
        shoulder: BoneIndex,
    ) -> IkChain {
        IkChain::new("arm_r", vec![hand, forearm, upper_arm, shoulder])
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(10)
            .with_tolerance(0.01)
    }

    /// Build a left-arm IK chain: hand → forearm → upper_arm → shoulder.
    pub fn arm_left(
        hand: BoneIndex,
        forearm: BoneIndex,
        upper_arm: BoneIndex,
        shoulder: BoneIndex,
    ) -> IkChain {
        IkChain::new("arm_l", vec![hand, forearm, upper_arm, shoulder])
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(10)
            .with_tolerance(0.01)
    }

    /// Build a right-leg IK chain: foot → shin → thigh → hip.
    pub fn leg_right(
        foot: BoneIndex,
        shin: BoneIndex,
        thigh: BoneIndex,
        hip: BoneIndex,
    ) -> IkChain {
        IkChain::new("leg_r", vec![foot, shin, thigh, hip])
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(15)
            .with_tolerance(0.005)
    }

    /// Build a left-leg IK chain: foot → shin → thigh → hip.
    pub fn leg_left(foot: BoneIndex, shin: BoneIndex, thigh: BoneIndex, hip: BoneIndex) -> IkChain {
        IkChain::new("leg_l", vec![foot, shin, thigh, hip])
            .with_solver(IkSolverType::Fabrik)
            .with_iterations(15)
            .with_tolerance(0.005)
    }

    /// Build a head look-at chain: head → neck → spine.
    pub fn head_lookat(head: BoneIndex, neck: BoneIndex, spine: BoneIndex) -> IkChain {
        IkChain::new("head", vec![head, neck, spine])
            .with_solver(IkSolverType::Ccd)
            .with_iterations(5)
            .with_tolerance(0.05)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_new_defaults() {
        let chain = IkChain::new("arm_r", vec![BoneIndex(0), BoneIndex(1), BoneIndex(2)]);
        assert_eq!(chain.name, "arm_r");
        assert_eq!(chain.bones.len(), 3);
        assert_eq!(chain.solver, IkSolverType::Fabrik);
        assert_eq!(chain.iteration_count, 10);
        assert!((chain.tolerance - 0.01).abs() < 1e-6);
        assert!((chain.weight - 1.0).abs() < 1e-6);
        assert!(chain.enabled);
    }

    #[test]
    fn chain_tip_base_accessors() {
        let chain = IkChain::new(
            "leg",
            vec![BoneIndex(3), BoneIndex(2), BoneIndex(1), BoneIndex(0)],
        );
        assert_eq!(chain.tip(), Some(BoneIndex(3)));
        assert_eq!(chain.base(), Some(BoneIndex(0)));
    }

    #[test]
    fn chain_contains_bone() {
        let chain = IkChain::new("test", vec![BoneIndex(1), BoneIndex(2), BoneIndex(5)]);
        assert!(chain.contains(BoneIndex(1)));
        assert!(chain.contains(BoneIndex(5)));
        assert!(!chain.contains(BoneIndex(0)));
    }

    #[test]
    fn chain_trivial_check() {
        assert!(IkChain::new("empty", vec![]).is_trivial());
        assert!(IkChain::new("single", vec![BoneIndex(0)]).is_trivial());
        assert!(!IkChain::new("valid", vec![BoneIndex(0), BoneIndex(1)]).is_trivial());
    }

    #[test]
    fn chain_active_check() {
        let mut chain = IkChain::new("test", vec![BoneIndex(0), BoneIndex(1)]);
        assert!(chain.is_active());

        chain.weight = 0.0;
        assert!(!chain.is_active());

        chain.weight = 1.0;
        chain.enabled = false;
        assert!(!chain.is_active());
    }

    #[test]
    fn chain_builder_methods() {
        let chain = IkChain::new("test", vec![BoneIndex(0), BoneIndex(1)])
            .with_solver(IkSolverType::Ccd)
            .with_iterations(20)
            .with_tolerance(0.001)
            .with_weight(0.5);

        assert_eq!(chain.solver, IkSolverType::Ccd);
        assert_eq!(chain.iteration_count, 20);
        assert!((chain.tolerance - 0.001).abs() < 1e-6);
        assert!((chain.weight - 0.5).abs() < 1e-6);
    }

    #[test]
    fn chain_serialize_roundtrip() {
        let chain = IkChain::new(
            "arm_r",
            vec![BoneIndex(7), BoneIndex(6), BoneIndex(5), BoneIndex(4)],
        );
        let bytes = bincode::serialize(&chain).unwrap();
        let restored: IkChain = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.name, "arm_r");
        assert_eq!(restored.bones.len(), 4);
        assert_eq!(restored.bones[0], BoneIndex(7));
    }

    #[test]
    fn chain_weight_clamped() {
        let chain = IkChain::new("test", vec![BoneIndex(0), BoneIndex(1)]).with_weight(1.5);
        assert!((chain.weight - 1.0).abs() < 1e-6);

        let chain = chain.with_weight(-0.5);
        assert!((chain.weight - 0.0).abs() < 1e-6);
    }

    #[test]
    fn humanoid_chain_builders() {
        let h = BoneIndex(7);
        let f = BoneIndex(6);
        let u = BoneIndex(5);
        let s = BoneIndex(4);

        let arm = chains::arm_right(h, f, u, s);
        assert_eq!(arm.name, "arm_r");
        assert_eq!(arm.bones, vec![h, f, u, s]);
        assert_eq!(arm.solver, IkSolverType::Fabrik);

        let arm_l = chains::arm_left(h, f, u, s);
        assert_eq!(arm_l.name, "arm_l");
    }
}
