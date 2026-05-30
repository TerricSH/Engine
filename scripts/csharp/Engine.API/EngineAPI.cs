using System.Runtime.InteropServices;

namespace Engine;

/// <summary>
/// P/Invoke declarations for the Rust engine FFI (engine-ffi crate).
/// These map to the #[no_mangle] extern "C" functions in the Rust side.
/// </summary>
internal static class EngineAPI
{
    // ── Component registry ──────────────────────────────────────────

    [DllImport("engine_ffi")]
    internal static extern int ffi_component_type_id(string name);

    [DllImport("engine_ffi")]
    internal static extern uint ffi_component_type_count();

    // ── Entity lifecycle ────────────────────────────────────────────

    [DllImport("engine_ffi")]
    internal static extern EntityId ffi_entity_spawn(IntPtr world);

    [DllImport("engine_ffi")]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ffi_entity_destroy(IntPtr world, EntityId entity);

    [DllImport("engine_ffi")]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ffi_entity_is_alive(IntPtr world, EntityId entity);

    // ── Component access (pointer-based via registry) ───────────────

    [DllImport("engine_ffi")]
    internal static extern IntPtr ffi_component_get(
        IntPtr world, EntityId entity, int typeId);

    [DllImport("engine_ffi")]
    internal static extern void ffi_component_set(
        IntPtr world, EntityId entity, int typeId, byte[] data, int length);

    // ── Async I/O ───────────────────────────────────────────────────

    [DllImport("engine_ffi")]
    internal static extern ulong ffi_async_load_image(
        string url,
        FfiAsyncCallback callback,
        ulong userData);

    [DllImport("engine_ffi")]
    internal static extern ulong ffi_async_http_get(
        string url,
        FfiAsyncCallback callback,
        ulong userData);

    // ── Coroutines ──────────────────────────────────────────────────

    [DllImport("engine_ffi")]
    internal static extern ulong ffi_coroutine_start(IntPtr enumerator);

    [DllImport("engine_ffi")]
    internal static extern void ffi_coroutine_cancel(ulong handle);

    [DllImport("engine_ffi")]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ffi_async_is_complete(ulong handle);

    // ── Engine services ─────────────────────────────────────────────

    [DllImport("engine_ffi")]
    internal static extern void ffi_log_info(string msg);

    [DllImport("engine_ffi")]
    internal static extern void ffi_log_warn(string msg);

    [DllImport("engine_ffi")]
    internal static extern void ffi_log_error(string msg);

    [DllImport("engine_ffi")]
    internal static extern double ffi_time_seconds();
}

/// <summary>
/// FFI-safe callback signature for async operations.
/// </summary>
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void FfiAsyncCallback(
    ulong handle,
    IntPtr data,
    uint len,
    ulong userData);
