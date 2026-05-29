using System.Runtime.InteropServices;

namespace Engine;

/// <summary>
/// Handle for a running coroutine. Can be used to cancel it.
/// </summary>
public readonly struct CoroutineHandle
{
    public readonly ulong Id;

    public CoroutineHandle(ulong id) => Id = id;

    public static readonly CoroutineHandle Invalid = new(0);
}

/// <summary>
/// Static API for starting and managing coroutines.
/// Usage matches Unity's StartCoroutine pattern.
/// </summary>
public static class Coroutine
{
    /// <summary>
    /// Start a coroutine from an IEnumerator.
    /// The enumerator runs on the main thread, advanced each frame
    /// by the engine's CoroutineSystem.
    /// </summary>
    public static CoroutineHandle Start(IEnumerator<YieldInstruction> routine)
    {
        // Pass the IEnumerator to the Rust CoroutineSystem via FFI
        unsafe
        {
            var ptr = GCHandle.ToIntPtr(GCHandle.Alloc(routine));
            var id = EngineAPI.ffi_coroutine_start(ptr);
            return new CoroutineHandle(id);
        }
    }

    /// <summary>
    /// Cancel a running coroutine by handle.
    /// </summary>
    public static void Stop(CoroutineHandle handle)
    {
        if (handle.Id != 0)
            EngineAPI.ffi_coroutine_cancel(handle.Id);
    }
}
