using System.Runtime.InteropServices;

namespace Engine;

/// <summary>
/// Exact C ABI declarations exported by the Rust engine-ffi native library.
/// Direct calls are valid only when this assembly is hosted in the same
/// process as the Rust engine and its host callback registry is installed.
/// </summary>
internal static class EngineAPI
{
    private const string NativeLibraryName = "engine_ffi";
    private static readonly IntPtr NativeLibraryHandle;

    static EngineAPI()
    {
        var nativePath = Environment.GetEnvironmentVariable("ENGINE_FFI_LIBRARY");
        if (string.IsNullOrWhiteSpace(nativePath))
        {
            throw new InvalidOperationException(
                "Engine.API direct P/Invoke requires ENGINE_FFI_LIBRARY to be installed " +
                "by the in-process Rust host. ProcessHost scripts must use IPC.");
        }

        var hostPid = Environment.GetEnvironmentVariable("ENGINE_FFI_HOST_PID");
        if (!uint.TryParse(hostPid, out var expectedPid))
        {
            throw new InvalidOperationException(
                "Engine.API direct P/Invoke requires a valid ENGINE_FFI_HOST_PID from " +
                "the in-process Rust host. ProcessHost scripts must use IPC.");
        }
        if (expectedPid != (uint)Environment.ProcessId)
        {
            throw new InvalidOperationException(
                "Engine.API direct P/Invoke was initialized in a different process than its " +
                "Rust host. ProcessHost scripts must use IPC instead of Engine.API native calls.");
        }

        NativeLibraryHandle = NativeLibrary.Load(Path.GetFullPath(nativePath));
        NativeLibrary.SetDllImportResolver(
            typeof(EngineAPI).Assembly,
            (libraryName, _, _) => string.Equals(
                libraryName,
                NativeLibraryName,
                StringComparison.Ordinal)
                ? NativeLibraryHandle
                : IntPtr.Zero);
    }

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern uint ffi_registry_abi_version();

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern uint ffi_registry_struct_size();

    // Component registry

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern uint ffi_component_type_id(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern uint ffi_component_type_count();

    // Entity lifecycle

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern EntityId ffi_entity_spawn();

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ffi_entity_destroy(EntityId entity);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ffi_entity_is_alive(EntityId entity);

    // Component access: caller-owned UTF-8 JSON buffers

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ffi_component_get(
        EntityId entity,
        uint typeId,
        IntPtr buffer,
        uint bufferCapacity,
        out uint requiredLength);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ffi_component_set(
        EntityId entity,
        uint typeId,
        IntPtr data,
        uint length);

    // Async I/O

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern ulong ffi_async_load_image(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string url,
        FfiAsyncCallback callback,
        ulong userData);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern ulong ffi_async_http_get(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string url,
        FfiAsyncCallback callback,
        ulong userData);

    // Coroutines

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern ulong ffi_coroutine_start(ref FfiManagedCoroutineDescriptor descriptor);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void ffi_coroutine_cancel(ulong handle);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void ffi_coroutine_tick(float deltaSeconds);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern uint ffi_coroutine_active_count();

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void ffi_coroutine_clear();

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ffi_async_is_complete(ulong handle);

    // Engine services

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void ffi_log_info(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string msg);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void ffi_log_warn(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string msg);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void ffi_log_error(
        [MarshalAs(UnmanagedType.LPUTF8Str)] string msg);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern double ffi_time_seconds();

    // Character controller

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool character_move(
        IntPtr controller,
        float dirX,
        float dirZ,
        float speed,
        float dt,
        IntPtr physics);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool character_jump(IntPtr controller);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern int character_is_grounded(IntPtr controller);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern int character_get_move_state(IntPtr controller);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern float character_get_velocity_x(IntPtr controller);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern float character_get_velocity_y(IntPtr controller);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern float character_get_velocity_z(IntPtr controller);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void character_set_foot_ik_enabled(
        IntPtr controller,
        [MarshalAs(UnmanagedType.I1)] bool enabled);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool character_get_foot_ik_enabled(IntPtr controller);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern float character_get_ground_normal_x(IntPtr controller);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern float character_get_ground_normal_y(IntPtr controller);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern float character_get_ground_normal_z(IntPtr controller);

    // Animation player

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void animation_set_param_float(
        IntPtr player,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name,
        float value);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void animation_set_param_bool(
        IntPtr player,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name,
        [MarshalAs(UnmanagedType.I1)] bool value);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool animation_force_state(
        IntPtr player,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string stateName);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void animation_play_clip(
        IntPtr player,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string clipAsset);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern uint animation_bone_count(IntPtr player);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern uint animation_get_bone_positions(
        IntPtr player,
        [Out] float[] output,
        uint maxCount);

    // NavAgent

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void nav_agent_set_target(IntPtr agent, float x, float y, float z);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool nav_agent_get_position(
        IntPtr agent,
        out float x,
        out float y,
        out float z);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool nav_agent_is_path_finished(IntPtr agent);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern float nav_agent_get_remaining_distance(IntPtr agent);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern int nav_agent_waypoint_count(IntPtr agent);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool nav_agent_waypoint_at(
        IntPtr agent,
        int index,
        out float x,
        out float y,
        out float z);

    // IK target component

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ik_set_effector_target(
        IntPtr ik,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name,
        float x,
        float y,
        float z);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern bool ik_get_effector_target(
        IntPtr ik,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string name,
        out float x,
        out float y,
        out float z);

    // Audio

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern ulong audio_play_sound(
        IntPtr engine,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string clipAsset,
        float volume,
        [MarshalAs(UnmanagedType.I1)] bool looping);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void audio_stop_sound(IntPtr engine, ulong handleId);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void audio_set_volume(IntPtr engine, ulong handleId, float volume);

    [DllImport(NativeLibraryName, CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
    internal static extern void audio_set_master_volume(IntPtr engine, float volume);
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
