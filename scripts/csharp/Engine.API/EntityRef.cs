namespace Engine;

/// <summary>
/// A managed reference to an entity in the active ECS World.
/// Component values cross the ABI as caller-owned UTF-8 JSON buffers; native
/// ECS pointers never escape the Rust world guard.
/// </summary>
public readonly ref struct EntityRef
{
    private const int MaxReadAttempts = 3;

    private readonly EntityId _id;

    internal EntityRef(EntityId id)
    {
        _id = id;
    }

    /// <summary>
    /// Spawn an empty entity in the active in-process engine runtime.
    /// </summary>
    public static EntityRef Spawn()
    {
        var id = EngineAPI.ffi_entity_spawn();
        if (!id.IsValid)
            throw new InvalidOperationException(
                "The engine did not provide an active in-process ECS world");
        return new EntityRef(id);
    }

    /// <summary>
    /// The raw entity identifier.
    /// </summary>
    public EntityId Id => _id;

    /// <summary>
    /// Whether this entity is still alive in the active world.
    /// </summary>
    public bool IsAlive => EngineAPI.ffi_entity_is_alive(_id);

    /// <summary>
    /// Destroy this entity and all of its components.
    /// </summary>
    public bool Destroy() => EngineAPI.ffi_entity_destroy(_id);

    /// <summary>
    /// Read and deserialize a script-serializable component.
    /// </summary>
    public unsafe T Get<T>()
    {
        var typeId = ComponentRegistry.GetId<T>();

        for (var attempt = 0; attempt < MaxReadAttempts; attempt++)
        {
            _ = EngineAPI.ffi_component_get(
                _id,
                typeId,
                IntPtr.Zero,
                0,
                out var requiredLength);
            if (requiredLength == 0)
                throw MissingComponent<T>();

            var buffer = GC.AllocateUninitializedArray<byte>(checked((int)requiredLength));
            uint actualLength;
            bool copied;
            fixed (byte* bufferPointer = buffer)
            {
                copied = EngineAPI.ffi_component_get(
                    _id,
                    typeId,
                    (IntPtr)bufferPointer,
                    (uint)buffer.Length,
                    out actualLength);
            }

            if (!copied)
            {
                if (actualLength == 0)
                    throw MissingComponent<T>();
                if (actualLength > (uint)buffer.Length)
                    continue;
                throw new InvalidOperationException(
                    $"The engine could not copy component {typeof(T).Name}");
            }
            if (actualLength > (uint)buffer.Length)
                throw new InvalidOperationException(
                    "The engine returned a component length larger than the supplied buffer");

            return ComponentJson.Deserialize<T>(
                buffer.AsSpan(0, checked((int)actualLength)));
        }

        throw new InvalidOperationException(
            $"Component {typeof(T).Name} changed size repeatedly while being read");
    }

    /// <summary>
    /// Serialize and write a script-serializable component.
    /// </summary>
    public unsafe void Set<T>(T value)
    {
        ArgumentNullException.ThrowIfNull(value);
        var typeId = ComponentRegistry.GetId<T>();
        var json = ComponentJson.Serialize(value);

        bool written;
        fixed (byte* jsonPointer = json)
        {
            written = EngineAPI.ffi_component_set(
                _id,
                typeId,
                (IntPtr)jsonPointer,
                checked((uint)json.Length));
        }
        if (!written)
            throw new InvalidOperationException(
                $"The engine rejected component {typeof(T).Name}; the entity may be stale " +
                "or the active type may not provide deserialize hooks");
    }

    private InvalidOperationException MissingComponent<T>() => new(
        $"Entity {_id} does not have a serializable {typeof(T).Name} component " +
        "in the active engine runtime");
}
