namespace Engine;

/// <summary>
/// C# wrapper for the engine's CharacterController component.
/// Provides movement commands and state queries without exposing engine internals.
/// </summary>
public class CharacterController
{
    private IntPtr _nativePtr;
    private IntPtr _physicsWorld;

    /// <summary>Movement state values matching the Rust CharacterState enum.</summary>
    public enum MoveState
    {
        Grounded = 0,
        Jumping = 1,
        Falling = 2,
        Landing = 3,
        Free = 4,
    }

    internal CharacterController(IntPtr nativePtr, IntPtr physicsWorld)
    {
        _nativePtr = nativePtr;
        _physicsWorld = physicsWorld;
    }

    /// <summary>Move the character in a direction at a given speed. Call once per frame.</summary>
    public bool Move(float dirX, float dirZ, float speed, float dt)
    {
        return EngineAPI.character_move(_nativePtr, dirX, dirZ, speed, dt, _physicsWorld);
    }

    /// <summary>Request a jump. Returns true if the character started jumping.</summary>
    public bool Jump()
    {
        return EngineAPI.character_jump(_nativePtr);
    }

    /// <summary>Returns true if the character is on the ground.</summary>
    public bool IsGrounded => EngineAPI.character_is_grounded(_nativePtr) != 0;

    /// <summary>Get the current movement state.</summary>
    public MoveState CurrentMoveState => (MoveState)EngineAPI.character_get_move_state(_nativePtr);

    /// <summary>Get velocity components.</summary>
    public float VelocityX => EngineAPI.character_get_velocity_x(_nativePtr);
    public float VelocityY => EngineAPI.character_get_velocity_y(_nativePtr);
    public float VelocityZ => EngineAPI.character_get_velocity_z(_nativePtr);

    /// <summary>
    /// Enable or disable foot IK grounding.  When enabled (default), the
    /// animation pipeline corrects foot positions to contact the ground.
    /// Disable for swimming, climbing, or airborne states.
    /// </summary>
    public bool FootIKEnabled
    {
        get => EngineAPI.character_get_foot_ik_enabled(_nativePtr);
        set => EngineAPI.character_set_foot_ik_enabled(_nativePtr, value);
    }
}
