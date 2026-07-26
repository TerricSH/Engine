# Physics joints

`PhysicsJoint` is a persistent, scene-serializable constraint component. Put it
on a dedicated constraint entity and reference both rigid bodies by persistent
ID. Backend handles are rebuilt after scene load, cell streaming, entity-handle
recycling, or checkpoint restore.

Supported types are fixed, revolute (hinge), prismatic (slider), and spherical.
Anchors are local to each body. Revolute and prismatic axes must be finite and
non-zero. Optional limits and motors are applied to the joint's free axis.
`break_force` and `break_torque` use newtons and newton-metres; zero means
unbreakable.

The physics reconciler is incremental:

- adding or enabling the component creates the backend joint once both bodies
  are live;
- changing anchors, type, limits, motor, or break thresholds replaces the old
  backend joint without creating a duplicate;
- removing/disabling the component, unloading either body, or recycling an ECS
  handle removes the backend joint;
- a broken joint removes the component so it is not recreated or captured by a
  later checkpoint.

## C# API

Scripts use the typed API instead of generic component writes:

```csharp
var settings = new PhysicsJointSettings
{
    Type = PhysicsJointType.Revolute,
    Axis = new Vector3(0.0f, 1.0f, 0.0f),
    Limits = new PhysicsJointLimits
    {
        Min = -1.4f,
        Max = 1.4f,
        Stiffness = 20.0f,
        Damping = 2.0f
    },
    Motor = new PhysicsJointMotor
    {
        TargetVelocity = 0.8f,
        TargetPosition = 0.0f,
        Stiffness = 10.0f,
        Damping = 1.0f
    },
    BreakForce = 5000.0f,
    BreakTorque = 800.0f
};

Physics.CreateJoint("door-hinge", doorFrame, door, settings);

// Reusing the ID updates/replaces the same persistent constraint.
settings.Motor.TargetPosition = 1.0f;
Physics.UpdateJoint("door-hinge", doorFrame, door, settings);

Physics.RemoveJoint("door-hinge");
```

For gravity-gun or hand-style interaction, drive a kinematic rigid body as the
grab anchor:

```csharp
Physics.Grab("player-grab", grabAnchor, prop, breakForce: 3000.0f);
Physics.ReleaseGrab("player-grab");
```

Linear and angular rigid-body mutations are also available through
`ApplyForce`, `ApplyImpulse`, `ApplyTorque`, and `ApplyTorqueImpulse`. They are
validated, bounded, and executed at the next safe physics step.

When a constraint breaks, scripts on both bodies receive a `joint_broken`
entry in `Physics.Events`. `OtherEntityId` identifies the opposite body;
`JointId`, `Force`, and `Torque` report the constraint and measured reaction
load.

## Current boundary

This foundation does not yet generate ragdolls from skeletons or provide a
destructible-fracture authoring system. Those higher-level systems can now be
built on persistent joints, safe script control, break events, and checkpoint
reconstruction.
