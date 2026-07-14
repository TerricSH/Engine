using System.Collections.Concurrent;

namespace Engine;

/// <summary>
/// Registry mapping .NET types to Rust-side component type IDs.
/// Must be initialized at startup before any entity component access.
/// </summary>
public static class ComponentRegistry
{
    private static readonly ConcurrentDictionary<Type, uint> TypeIds = new();

    /// <summary>
    /// Register a component type with its Rust-side name.
    /// </summary>
    public static void Register<T>(string name)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(name);
        var id = EngineAPI.ffi_component_type_id(name);
        if (id == 0)
            throw new InvalidOperationException(
                $"Component type '{name}' is not registered or is not script-serializable " +
                "in the active engine runtime");
        TypeIds[typeof(T)] = id;
    }

    /// <summary>
    /// Look up the numeric ID for a component type.
    /// </summary>
    public static uint GetId<T>()
    {
        if (!TypeIds.TryGetValue(typeof(T), out var id))
            throw new InvalidOperationException(
                $"Component {typeof(T).Name} is not registered. " +
                $"Call ComponentRegistry.Register<{typeof(T).Name}>() first.");
        return id;
    }

    /// <summary>
    /// Clear all registrations (used when reloading runtime).
    /// </summary>
    public static void Clear() => TypeIds.Clear();
}
