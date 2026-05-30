using System.Runtime.InteropServices;

namespace Engine;

/// <summary>
/// Async image loading API for C# scripts.
/// Loads images in the background via Rust's thread pool.
/// </summary>
public static class ImageLoader
{
    /// <summary>
    /// Start loading an image from a URL asynchronously.
    /// The callback is invoked on the main thread when loading completes.
    /// </summary>
    public static AsyncHandle LoadAsync(string url, Action<byte[]>? onLoaded = null)
    {
        // Register completion callback
        FfiAsyncCallback callback = (id, data, len, userData) =>
        {
            var bytes = new byte[len];
            Marshal.Copy(data, bytes, 0, (int)len);

            var ownedHandle = AsyncHandleRegistry.Get(id);
            ownedHandle?.Complete(bytes);

            onLoaded?.Invoke(bytes);
        };

        var ffiHandle = EngineAPI.ffi_async_load_image(
            url,
            callback,
            0);
        var handle = new AsyncHandle(ffiHandle);
        AsyncHandleRegistry.Register(ffiHandle, handle);

        return handle;
    }
}

/// <summary>
/// Internal registry mapping FFI async handles to managed AsyncHandle objects.
/// </summary>
internal static class AsyncHandleRegistry
{
    private static readonly Dictionary<ulong, WeakReference<AsyncHandle>> _handles = new();

    public static void Register(ulong id, AsyncHandle handle)
    {
        lock (_handles)
        {
            _handles[id] = new WeakReference<AsyncHandle>(handle);
        }
    }

    public static AsyncHandle? Get(ulong id)
    {
        lock (_handles)
        {
            if (_handles.TryGetValue(id, out var weak) &&
                weak.TryGetTarget(out var handle))
                return handle;
            return null;
        }
    }
}
