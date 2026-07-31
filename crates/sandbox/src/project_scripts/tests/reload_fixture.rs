#[cfg(all(test, feature = "subsystem-scripting-csharp", feature = "subsystem-ui"))]
const TEST_RELOAD_SCRIPT_SOURCE: &str = r#"using Engine;

namespace ScriptReloadTests;

public abstract class AbstractFixtureBehaviour : EngineBehaviour
{
}

public sealed class UnrelatedFixtureType
{
}

public sealed class ReloadFixtureBehaviour : EngineBehaviour
{
    public float Speed = 3.0f;
    public int UpdateCount = 0;
    public float ElapsedSeconds = 0.0f;
    public bool LastJump = false;
    public bool LastJumpPressed = false;
    public bool LastJumpReleased = false;
    public bool LastStartClicked = false;
    public int LastUiEventCount = 0;
    public string? LastUiCanvasId = null;
    public uint LastUiElementId = 0;
    public string? LastUiCallbackId = null;

    public void OnCreate()
    {
        UpdateCount = 0;
        ElapsedSeconds = 0.0f;
    }

    public void OnStart()
    {
    }

    public void OnUpdate(float deltaTime)
    {
        UpdateCount += 1;
        ElapsedSeconds += deltaTime;
        LastJump = Input.GetBool("jump");
        LastJumpPressed = Input.WasPressed("jump");
        LastJumpReleased = Input.WasReleased("jump");
        LastStartClicked = UI.WasClicked("start-game");
        LastUiEventCount = UI.Events.Count;
        if (UI.Events.Count > 0)
        {
            LastUiCanvasId = UI.Events[0].CanvasId;
            LastUiElementId = UI.Events[0].ElementId;
            LastUiCallbackId = UI.Events[0].CallbackId;
        }
        else
        {
            LastUiCanvasId = null;
            LastUiElementId = 0;
            LastUiCallbackId = null;
        }

        var translation = Transform.Translation;
        Transform.Translation = new Vector3(
            translation.X + Speed * deltaTime,
            translation.Y,
            translation.Z);
    }

    public void OnDestroy()
    {
    }

    // Compile-time contract probe for deferred rigid-body mutations.
    private void PhysicsMutationApiProbe(Entity entity)
    {
        Character.Move(new Vector3(1.0f, 0.0f, 0.0f));
        Character.Move(entity, new Vector3(0.0f, 0.0f, 1.0f), 7.5f);
        Character.Jump();
        Character.Control(entity.Id, default, jump: true);
        var useProbe = Interaction.Probe(
            default,
            new Vector3(0.0f, 0.0f, -1.0f));
        if (Interaction.TryGetTarget(useProbe, out var target) && target.Grabbable)
            Interaction.Grab("probe-use-grab", entity, target, 1000.0f);
        Interaction.ReleaseGrab("probe-use-grab");
        Physics.ApplyForce(entity, new Vector3(1.0f, 0.0f, 0.0f));
        Physics.ApplyImpulse(entity.Id, new Vector3(0.0f, 1.0f, 0.0f));
        Physics.ApplyTorque(entity, new Vector3(0.0f, 0.0f, 1.0f));
        Physics.ApplyTorqueImpulse(entity.Id, new Vector3(0.0f, 1.0f, 0.0f));
        Physics.CreateJoint(
            "probe-hinge",
            entity,
            entity,
            new PhysicsJointSettings
            {
                Type = PhysicsJointType.Revolute,
                Axis = new Vector3(0.0f, 1.0f, 0.0f),
                Limits = new PhysicsJointLimits { Min = -1.0f, Max = 1.0f },
                Motor = new PhysicsJointMotor
                {
                    TargetVelocity = 2.0f,
                    Stiffness = 10.0f,
                    Damping = 1.0f
                },
                BreakForce = 1000.0f,
                BreakTorque = 200.0f
            });
        Physics.RemoveJoint("probe-hinge");
        Damage.Apply(
            entity,
            25.0f,
            DamageKind.Impact,
            new Vector3(1.0f, 2.0f, 3.0f),
            new Vector3(4.0f, 0.0f, 0.0f));
        Ragdoll.Activate(entity, new Vector3(2.0f, 1.0f, 0.0f));
        Ragdoll.Recover(entity.Id, 0.35f);
    }
}
"#;
