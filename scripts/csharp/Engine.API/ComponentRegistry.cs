namespace Engine;

/// <summary>
/// Registry mapping .NET types to Rust-side component type IDs.
/// Must be initialized at startup before any entity component access.
/// </summary>
public static class ComponentRegistry
{
    private static readonly Dictionary<Type, int> _typeIds = new();

    /// <summary>
    /// Register a component type with its Rust-side name.
    /// </summary>
    public static void Register<T>(string name) where T : unmanaged
    {
        var id = EngineAPI.ffi_component_type_id(name);
        if (id == 0)
            throw new InvalidOperationException(
                $"Component type '{name}' is not registered in the engine");
        _typeIds[typeof(T)] = id;
    }

    /// <summary>
    /// Look up the numeric ID for a component type.
    /// </summary>
    public static int GetId<T>() where T : unmanaged
    {
        if (!_typeIds.TryGetValue(typeof(T), out var id))
            throw new InvalidOperationException(
                $"Component {typeof(T).Name} is not registered. " +
                $"Call ComponentRegistry.Register<{typeof(T).Name}>() first.");
        return id;
    }

    /// <summary>
    /// Clear all registrations (used when reloading runtime).
    /// </summary>
    public static void Clear() => _typeIds.Clear();
}
