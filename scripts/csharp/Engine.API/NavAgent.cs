namespace Engine;

/// <summary>
/// C# wrapper for the engine's NavAgent (AI agent controller).
/// Provides movement commands and state queries without exposing engine internals.
/// </summary>
public class NavAgent
{
    private IntPtr _nativePtr;

    internal NavAgent(IntPtr nativePtr)
    {
        _nativePtr = nativePtr;
    }

    /// <summary>
    /// Set the agent's movement destination.
    /// A straight‑line path is created from the current position to the target.
    /// For navmesh‑aware pathfinding, compute the path externally and use
    /// <see cref="SetPath"/> (when available).
    /// </summary>
    public void SetTarget(float x, float y, float z)
    {
        EngineAPI.nav_agent_set_target(_nativePtr, x, y, z);
    }

    /// <summary>
    /// Get the agent's current world position.
    /// Returns true on success, false if the native pointer is invalid.
    /// </summary>
    public bool GetPosition(out float x, out float y, out float z)
    {
        return EngineAPI.nav_agent_get_position(_nativePtr, out x, out y, out z);
    }

    /// <summary>
    /// Returns true when the agent has reached the end of its path
    /// (or has no path assigned).
    /// </summary>
    public bool IsPathFinished => EngineAPI.nav_agent_is_path_finished(_nativePtr);

    /// <summary>
    /// Total remaining distance along the agent's path.
    /// </summary>
    public float RemainingDistance => EngineAPI.nav_agent_get_remaining_distance(_nativePtr);

    /// <summary>
    /// Number of waypoints on the current path (0 if no path).
    /// </summary>
    public int WaypointCount => EngineAPI.nav_agent_waypoint_count(_nativePtr);

    /// <summary>
    /// Get a waypoint position by index.
    /// Returns true on success, false if out of range.
    /// </summary>
    public bool GetWaypoint(int index, out float x, out float y, out float z)
    {
        return EngineAPI.nav_agent_waypoint_at(_nativePtr, index, out x, out y, out z);
    }
}
