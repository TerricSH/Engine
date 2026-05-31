namespace Engine;

/// <summary>
/// C# wrapper for the engine's AnimationPlayer component.
/// Provides animation control: parameters, state transitions, clip playback,
/// and bone position queries.
/// </summary>
public class AnimationPlayer
{
    private readonly IntPtr _nativePtr;

    internal AnimationPlayer(IntPtr nativePtr)
    {
        _nativePtr = nativePtr;
    }

    /// <summary>
    /// Set a float parameter on the animation state machine.
    /// Use this to override parameters like "speed", "vertical_velocity".
    /// </summary>
    public void SetFloat(string name, float value)
    {
        EngineAPI.animation_set_param_float(_nativePtr, name, value);
    }

    /// <summary>
    /// Set a bool parameter on the animation state machine.
    /// Use this to override parameters like "grounded", "is_moving".
    /// </summary>
    public void SetBool(string name, bool value)
    {
        EngineAPI.animation_set_param_bool(_nativePtr, name, value);
    }

    /// <summary>
    /// Force the state machine to transition to a named state immediately,
    /// bypassing normal transition conditions.  Returns true if the state exists.
    /// </summary>
    public bool ForceState(string stateName)
    {
        return EngineAPI.animation_force_state(_nativePtr, stateName);
    }

    /// <summary>
    /// Play a specific animation clip by asset ID, bypassing the state machine.
    /// This is useful for cutscenes, scripted sequences, and one-shot actions.
    /// </summary>
    public void PlayClip(string clipAsset)
    {
        EngineAPI.animation_play_clip(_nativePtr, clipAsset);
    }

    /// <summary>
    /// Number of bones in the skeleton (from last frame's cached positions).
    /// </summary>
    public uint BoneCount => EngineAPI.animation_bone_count(_nativePtr);

    /// <summary>
    /// Get the world-space position of every bone from the last frame's
    /// animation evaluation.  Returns an array of Vector3 (one per bone).
    /// Returns empty array if the player hasn't been evaluated yet.
    /// </summary>
    public Vector3[] GetBonePositions()
    {
        uint count = EngineAPI.animation_bone_count(_nativePtr);
        if (count == 0) return Array.Empty<Vector3>();

        var raw = new float[count * 3];
        uint written = EngineAPI.animation_get_bone_positions(_nativePtr, raw, count);
        var result = new Vector3[written];
        for (int i = 0; i < written; i++)
        {
            result[i] = new Vector3(raw[i * 3], raw[i * 3 + 1], raw[i * 3 + 2]);
        }
        return result;
    }
}

/// <summary>
/// Simple vector type for bone position data.
/// </summary>
public struct Vector3
{
    public float X, Y, Z;
    public Vector3(float x, float y, float z) { X = x; Y = y; Z = z; }
    public override string ToString() => $"({X:F3}, {Y:F3}, {Z:F3})";
}
