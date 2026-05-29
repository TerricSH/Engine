using System.Runtime.InteropServices;

namespace Engine;

/// <summary>
/// A managed reference to an entity in the ECS World.
/// Provides type-safe generic access to components.
/// </summary>
public readonly ref struct EntityRef
{
    private readonly IntPtr _worldPtr;
    private readonly EntityId _id;

    internal EntityRef(IntPtr worldPtr, EntityId id)
    {
        _worldPtr = worldPtr;
        _id = id;
    }

    /// <summary>
    /// The raw entity identifier.
    /// </summary>
    public EntityId Id => _id;

    /// <summary>
    /// Whether this entity is still alive in the world.
    /// </summary>
    public bool IsAlive => EngineAPI.ffi_entity_is_alive(_worldPtr, _id);

    // ── Generic component access ────────────────────────────────────
    //
    // These will be replaced with ILRuntime CLR bindings for hotfix DLLs.
    // The current implementation uses reflection-based ComponentRegistry,
    // which is slow but works for sandbox development.
    //
    // For production, use the generated AOT bindings.

    /// <summary>
    /// Read a component from this entity.
    /// </summary>
    public T Get<T>() where T : unmanaged
    {
        var typeId = ComponentRegistry.GetId<T>();
        unsafe
        {
            var ptr = ffi_component_get(_worldPtr, _id, typeId);
            if (ptr == IntPtr.Zero)
                throw new InvalidOperationException(
                    $"Entity {_id} does not have component {typeof(T).Name}");
            return *(T*)ptr;
        }
    }

    /// <summary>
    /// Write a component to this entity.
    /// </summary>
    public void Set<T>(T value) where T : unmanaged
    {
        var typeId = ComponentRegistry.GetId<T>();
        unsafe
        {
            var bytes = new byte[sizeof(T)];
            fixed (byte* p = bytes)
            {
                *(T*)p = value;
            }
            ffi_component_set(_worldPtr, _id, typeId, bytes, bytes.Length);
        }
    }

    // ── FFI wrappers (temporary until proper P/Invoke into engine-ffi) ─

    private static IntPtr ffi_component_get(
        IntPtr world, EntityId entity, int typeId)
    {
        // TODO: replace with actual P/Invoke to engine-ffi
        return IntPtr.Zero;
    }

    private static void ffi_component_set(
        IntPtr world, EntityId entity, int typeId,
        byte[] data, int length)
    {
        // TODO: replace with actual P/Invoke to engine-ffi
    }
}
