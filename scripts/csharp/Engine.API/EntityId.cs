using System.Runtime.InteropServices;

namespace Engine;

/// <summary>
/// FFI-safe entity identifier, repr(C) compatible with Rust's FfiEntityId.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public readonly struct EntityId
{
    public readonly uint Index;
    public readonly uint Generation;

    public static readonly EntityId Invalid = new(uint.MaxValue, uint.MaxValue);

    public EntityId(uint index, uint generation)
    {
        Index = index;
        Generation = generation;
    }

    public bool IsValid => Index != uint.MaxValue;

    public override string ToString() => $"Entity({Index}:{Generation})";
}
