# Transform Ownership Rules

> Status: Adopted — Gate 12 (Character Controller)
> Applies to: engine-scene `Transform` component, engine-animation `Pose`, engine-physics collider transforms, engine-character controller

## Purpose

Define which system owns which transforms at each stage of the frame, preventing conflicting writes between physics, animation, gameplay scripts, and the character controller.

## Ownership Hierarchy

```
┌──────────────────────────────────────────────────────────────────────┐
│ AUTHORITATIVE (one writer)                                          │
│                                                                      │
│  ECS Transform component — world-space position/rotation/scale       │
│    Written by: CharacterController  (movement resolution)            │
│    Written by: Physics              (rigidbody sync)                 │
│    Read by:    Renderer             (view culling, world matrices)    │
│    Read by:    Animation            (root bone offset)               │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
┌──────────────────────────────────────────────────────────────────────┐
│ TRANSIENT (per-frame, recomputed)                                    │
│                                                                      │
│  Animation Pose — local-space bone transforms (Vec<BoneTransform>)   │
│    Written by: AnimationEvaluator  (clip sampling)                   │
│    Modified by: IK solver          (pose correction)                 │
│    Read by:    Pose::skin_matrices (skin matrix generation)          │
│    Lifespan:   Single frame — discarded after skinning               │
└──────────────────────────────────────────────────────────────────────┘
         │
         ▼
┌──────────────────────────────────────────────────────────────────────┐
│ DERIVED (computed from authoritative + transient)                    │
│                                                                      │
│  Skin matrices — bone palette for GPU skinning                      │
│    Written by: Pose::skin_matrices()  (current * inverse bind)       │
│    Consumed by: SkinnedExtractProducer → GPU                         │
│    Lifespan:   Single frame — submitted to renderer                  │
└──────────────────────────────────────────────────────────────────────┘
```

## Detailed Rules

### 1. ECS Transform (`engine_scene::Transform`)
- **Single writer per frame**: Either the character controller (kinematic movement), physics (rigidbody simulation), or gameplay scripts (teleport/respawn). They must not both write in the same frame.
- **Animation does not write Transform**. Animation writes to `Pose` (local-space bone transforms), never to the entity's world-space Transform.
- **Root motion**: If enabled, root motion delta is extracted from the animation pose (`Pose.local[0]`), converted to a movement delta, and injected as a *request* into the character controller's command buffer. The controller resolves it through collision queries. Root motion does NOT directly write Transform.

### 2. Animation Pose (`engine_animation::Pose`)
- **Per-frame transient value**. The Pose is created at the start of the animation update, populated by clip evaluation, optionally modified by IK, consumed for skin matrix generation, then discarded.
- **Never persisted to ECS**. There is no "component" that stores bone transforms between frames.
- **Clip evaluation** writes to `Pose.local[i]` for each animated joint. Non-animated joints retain their rest-pose transform.
- **IK solver** mutates `Pose.local[i]` for bones in IK chains. The solver clamps rotations to constraint limits.
- **Layers** blend between multiple poses via `Pose::blend()`. Each layer contributes weight-controlled additive or overwrite blending.

### 3. Character Controller (`engine_character::CharacterController`)
- **Movement authority**. The controller determines position, velocity, and grounded state.
- **Writes** to the entity's `Transform` component (position) and its own internal state (velocity, movement mode).
- **Reads** from physics (collision query results, ground normal).
- **Outputs animation parameters** (speed, grounded, vertical_velocity, is_moving, move_state) to `AnimStateMachineInstance` via `AnimParams::apply_to_state_machine()`.
- **Does not write** to `Pose`, `Skeleton`, or any animation internal state.

### 4. Physics (`engine_physics`)
- **Collision resolver only**. Physics does not set character position or velocity — it responds to controller queries (sweeps, ground checks).
- **For kinematic characters**: The controller queries physics to detect obstacles, slopes, and ground. Physics returns contact points, normals, and penetration depth.
- **For dynamic rigidbodies**: Physics writes rigidbody transforms. The character controller is not used in this mode.

### 5. Gameplay Scripts (C# / Rust)
- **Command-based interaction**. Scripts do not directly write transforms. They submit movement commands (`MoveCharacter(direction, speed)`, `Jump()`) and query state (`IsGrounded()`, `GetMoveState()`, `GetVelocity()`).
- **Animation control**: Scripts can set `AnimStateMachineInstance` parameters (e.g., trigger a "hurt" state). They cannot write `Pose` directly.

## Pipeline Sequence (per frame)

```
Step 1: Input/AI  ──→  CharacterController::update()
         Writes commands to controller command buffer

Step 2: Controller ──→  process_movement()
         Reads physics for collision resolution
         Writes Transform (position)
         Writes controller state (velocity, grounded)

Step 3: Animation  ──→  extract AnimParams from controller
         Applies params to AnimStateMachineInstance
         Updates state machine (drives transitions)
         Evaluates clip(s) → Pose
         Applies IK (if IkTargetComponent present)
         Computes skin matrices

Step 4: Extract    ──→  SkinnedExtractProducer::push()
         Writes skin matrices + world transform + mesh info
         to pending queue

Step 5: Render     ──→  SkinnedExtractProducer::produce()
         Drains pending items into RenderFrameInput
         GPU skinning uses bone palette + world transform
```

## Enforcement

- `engine-animation` has `#![forbid(unsafe_code)]` — no raw pointer aliasing of transforms.
- The controller uses physics public APIs only (`PhysicsWorld::cast_shape`, etc.) — no direct physics internal mutation.
- C# FFI exposes movement *intent* and *state query* only — no transform handles cross the FFI boundary.
- The `Pose` type's `local` field is `pub(crate)` — only code within engine-animation can mutate it directly.
- Git hooks / CI will flag any new `unsafe` block in animation/character crates.

## Exceptions & Escalation

- **Root motion with controller**: If root motion is enabled, the animation system extracts the root bone delta and forwards it as a desired movement command to the controller. The controller resolves this through collision queries. This is the ONLY path where animation output influences world-space position.
- **Ragdoll (future)**: A ragdoll mode will transfer authority from the character controller to physics. The animation system will blend between the last animated pose and the physics-driven pose. This is NOT yet implemented.
- **Procedural animation (future)**: Additive layers may write to `Pose` directly (e.g., breathing, weapon sway). These must be additive (LayerBlendMode::Additive) and cannot conflict with base layer bones unless intentionally overriding.
