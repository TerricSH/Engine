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

    // ── Character controller ──────────────────────────────────────────

    [DllImport("engine_ffi")]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool character_move(IntPtr controller, float dirX, float dirZ, float speed, float dt, IntPtr physics);

    [DllImport("engine_ffi")]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool character_jump(IntPtr controller);

    [DllImport("engine_ffi")]
    internal static extern int character_is_grounded(IntPtr controller);

    [DllImport("engine_ffi")]
    internal static extern int character_get_move_state(IntPtr controller);

    [DllImport("engine_ffi")]
    internal static extern float character_get_velocity_x(IntPtr controller);

    [DllImport("engine_ffi")]
    internal static extern float character_get_velocity_y(IntPtr controller);

    [DllImport("engine_ffi")]
    internal static extern float character_get_velocity_z(IntPtr controller);

    [DllImport("engine_ffi")]
    internal static extern void character_set_foot_ik_enabled(IntPtr controller, [MarshalAs(UnmanagedType.I1)] bool enabled);

    [DllImport("engine_ffi")]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool character_get_foot_ik_enabled(IntPtr controller);

    [DllImport("engine_ffi")]
    internal static extern float character_get_ground_normal_x(IntPtr controller);

    [DllImport("engine_ffi")]
    internal static extern float character_get_ground_normal_y(IntPtr controller);

    [DllImport("engine_ffi")]
    internal static extern float character_get_ground_normal_z(IntPtr controller);

    // ── Animation Player ───────────────────────────────────────────────

    [DllImport("engine_ffi")]
    internal static extern void animation_set_param_float(IntPtr player, string name, float value);

    [DllImport("engine_ffi")]
    internal static extern void animation_set_param_bool(IntPtr player, string name, [MarshalAs(UnmanagedType.I1)] bool value);

    [DllImport("engine_ffi")]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool animation_force_state(IntPtr player, string stateName);

    [DllImport("engine_ffi")]
    internal static extern void animation_play_clip(IntPtr player, string clipAsset);

    [DllImport("engine_ffi")]
    internal static extern uint animation_bone_count(IntPtr player);

    [DllImport("engine_ffi")]
    internal static extern uint animation_get_bone_positions(IntPtr player, float[] output, uint maxCount);

    // ── IK Target Component ────────────────────────────────────────────

    [DllImport("engine_ffi")]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ik_set_effector_target(IntPtr ik, string name, float x, float y, float z);

    [DllImport("engine_ffi")]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ik_get_effector_target(IntPtr ik, string name, out float x, out float y, out float z);
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
