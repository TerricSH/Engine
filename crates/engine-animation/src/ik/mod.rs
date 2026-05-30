//! Inverse Kinematics module.
//!
//! Provides humanoid character control points ([`IkEffector`]), IK chain
//! definitions ([`IkChain`]), solver algorithms (FABRIK and CCD), and
//! joint constraints ([`IkConstraint`], [`IkConstraintSet`]).
//!
//! # Quick start
//!
//! ```ignore
//! use engine_animation::ik;
//! use engine_animation::{BoneIndex, Pose};
//!
//! // 1. Define your skeleton and create a pose.
//! let mut pose = Pose::new(&skeleton);
//!
//! // 2. Define an IK chain (tip → base).
//! let chain = ik::IkChain::new("arm_r", vec![hand, forearm, upper, shoulder]);
//!
//! // 3. Place an effector (control point).
//! let effector = ik::IkEffector::new("hand_target", hand, Vec3::new(0.5, 1.0, 0.3));
//!
//! // 4. Solve!
//! ik::solve_pose(&mut pose, &skeleton, &chain, &effector, &ik::IkConstraintSet::new());
//! ```

pub(crate) mod chain;
pub(crate) mod constraint;
pub(crate) mod debug;
pub(crate) mod effector;
pub(crate) mod solver;

pub use chain::{chains, IkChain, IkSolverType};
pub use constraint::{humanoid_constraints, IkConstraint, IkConstraintSet};
pub use debug::{IkDebugDraw, IkDebugInfo};
pub use effector::{humanoid, IkEffector, IkEffectorSpace};
pub use solver::{solve_pose, solve_pose_multi};
