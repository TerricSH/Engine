using System.Runtime.InteropServices;

namespace Engine;

/// <summary>
/// Async image loading API for C# scripts.
/// Loads images in the background via Rust's thread pool.
/// </summary>
public static class ImageLoader
{
    // A single static thunk stays valid for the process lifetime. Per-request
    // state is rooted separately and removed after its one completion.
    private static readonly FfiAsyncCallback CompletionCallback = OnNativeCompleted;

    /// <summary>
    /// Start loading an image from a URL asynchronously.
    /// The callback is invoked on the main thread when loading completes.
    /// </summary>
    public static AsyncHandle LoadAsync(string url, Action<byte[]>? onLoaded = null)
    {
        ArgumentNullException.ThrowIfNull(url);

        // Register and root the complete operation before entering native code.
        // A host is allowed to invoke the completion callback synchronously.
        var operation = new AsyncLoadOperation(onLoaded);
        var registration = AsyncHandleRegistry.Register(operation);

        try
        {
            var nativeHandle = EngineAPI.ffi_async_load_image(
                url,
                CompletionCallback,
                registration);

            operation.Handle.BindNativeId(nativeHandle);
            if (nativeHandle == 0)
            {
                operation.Handle.Fail(
                    new InvalidOperationException("The native image loader rejected the request."));
                AsyncHandleRegistry.Release(registration);
            }

            return operation.Handle;
        }
        catch (Exception error)
        {
            operation.Handle.Fail(error);
            AsyncHandleRegistry.Release(registration);
            throw;
        }
    }

    private sealed class AsyncLoadOperation
    {
        private readonly Action<byte[]>? _onLoaded;

        internal AsyncLoadOperation(Action<byte[]>? onLoaded)
        {
            _onLoaded = onLoaded;
        }

        internal AsyncHandle Handle { get; } = new(0);

        internal void Complete(ulong id, IntPtr data, uint len)
        {
            Handle.BindNativeId(id);
            if (id == 0)
            {
                Handle.Fail(
                    new InvalidOperationException("The native image callback returned an invalid handle."));
                return;
            }

            // The native async ABI reports a deferred I/O/decode failure as
            // a null pointer with zero bytes. A valid image can never be empty.
            if (data == IntPtr.Zero && len == 0)
            {
                Handle.Fail(new InvalidOperationException("The native image load failed."));
                return;
            }

            if (len > int.MaxValue)
                throw new OverflowException("The native image result is too large for a managed array.");
            if (len != 0 && data == IntPtr.Zero)
                throw new InvalidOperationException("The native image result has a null data pointer.");

            var bytes = len == 0 ? Array.Empty<byte>() : new byte[(int)len];
            if (bytes.Length != 0)
                Marshal.Copy(data, bytes, 0, bytes.Length);

            Handle.Complete(bytes);
            _onLoaded?.Invoke(bytes);
        }
    }

    private static void OnNativeCompleted(ulong id, IntPtr data, uint len, ulong userData)
    {
        // No managed exception may unwind through the native callback frame.
        AsyncLoadOperation? operation = null;
        try
        {
            if (!AsyncHandleRegistry.TryGet(userData, out operation) || operation is null)
                return;

            operation.Complete(id, data, len);
        }
        catch (Exception error)
        {
            operation?.Handle.Fail(error);
        }
        finally
        {
            AsyncHandleRegistry.Release(userData);
        }
    }

    /// <summary>
    /// Owns the GC roots for pending native callbacks. Registrations are removed
    /// exactly once when a callback finishes or a request fails to start.
    /// </summary>
    private static class AsyncHandleRegistry
    {
        private static readonly object Sync = new();
        private static readonly Dictionary<ulong, GCHandle> Handles = new();
        private static ulong _nextRegistration;

        internal static ulong Register(AsyncLoadOperation operation)
        {
            var root = GCHandle.Alloc(operation, GCHandleType.Normal);
            try
            {
                lock (Sync)
                {
                    ulong registration;
                    do
                    {
                        registration = unchecked(++_nextRegistration);
                    }
                    while (registration == 0 || Handles.ContainsKey(registration));

                    Handles.Add(registration, root);
                    return registration;
                }
            }
            catch
            {
                root.Free();
                throw;
            }
        }

        internal static bool TryGet(ulong registration, out AsyncLoadOperation? operation)
        {
            lock (Sync)
            {
                if (Handles.TryGetValue(registration, out var root)
                    && root.Target is AsyncLoadOperation registered)
                {
                    operation = registered;
                    return true;
                }

                operation = null;
                return false;
            }
        }

        internal static void Release(ulong registration)
        {
            GCHandle root;
            lock (Sync)
            {
                if (!Handles.Remove(registration, out root))
                    return;
            }

            // The static callback thunk remains valid, so duplicate callbacks
            // safely observe a missing registration. Release itself must not
            // throw when called from a native callback frame.
            try
            {
                if (root.IsAllocated)
                    root.Free();
            }
            catch (InvalidOperationException)
            {
                // Another release cannot obtain the removed entry. Treat an
                // externally freed/corrupt handle as already released.
            }
        }
    }
}
