namespace Engine;

/// <summary>
/// C# wrapper for the engine's IK Target component.
/// Provides control over IK effector target positions for procedural
/// animation (e.g., reaching, foot placement, look-at).
/// </summary>
public class IkTargetComponent
{
    private readonly IntPtr _nativePtr;

    internal IkTargetComponent(IntPtr nativePtr)
    {
        _nativePtr = nativePtr;
    }

    /// <summary>
    /// Set the target position of a named IK effector.
    /// Common effector names: "hand_ik_r", "hand_ik_l", "foot_ik_r",
    /// "foot_ik_l", "head_lookat", "pelvis_ik".
    /// Returns true if the effector was found.
    /// </summary>
    public bool SetTarget(string effectorName, Vector3 position)
    {
        return EngineAPI.ik_set_effector_target(
            _nativePtr, effectorName, position.X, position.Y, position.Z);
    }

    /// <summary>
    /// Set the target position using individual components.
    /// </summary>
    public bool SetTarget(string effectorName, float x, float y, float z)
    {
        return EngineAPI.ik_set_effector_target(_nativePtr, effectorName, x, y, z);
    }

    /// <summary>
    /// Get the current target position of a named IK effector.
    /// Returns true if the effector was found; position is filled on success.
    /// </summary>
    public bool GetTarget(string effectorName, out Vector3 position)
    {
        bool found = EngineAPI.ik_get_effector_target(
            _nativePtr, effectorName, out float x, out float y, out float z);
        position = found ? new Vector3(x, y, z) : default;
        return found;
    }
}
