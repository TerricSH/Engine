namespace Engine;

/// <summary>
/// Handle for an async I/O operation (image loading, HTTP request, etc.).
/// Returned by LoadAsync methods. Check IsComplete to see if done,
/// or yield on it in a coroutine: yield return new WaitForAsync(handle).
/// </summary>
public class AsyncHandle
{
    private readonly object _sync = new();
    private ulong _id;
    private byte[]? _result;
    private Exception? _error;
    private bool _isComplete;

    internal AsyncHandle(ulong id)
    {
        _id = id;
    }

    /// <summary>
    /// The Rust-side async operation ID. Zero means that native startup failed.
    /// </summary>
    public ulong Id
    {
        get
        {
            lock (_sync)
                return _id;
        }
    }

    /// <summary>
    /// Whether the async operation has completed or failed.
    /// </summary>
    public bool IsComplete
    {
        get
        {
            ulong id;
            lock (_sync)
            {
                if (_isComplete)
                    return true;
                id = _id;
            }

            if (id == 0)
            {
                Fail(new InvalidOperationException("The async operation has no valid native handle."));
                return true;
            }

            try
            {
                if (!EngineAPI.ffi_async_is_complete(id))
                    return false;

                lock (_sync)
                    _isComplete = true;
                return true;
            }
            catch (Exception error)
            {
                Fail(error);
                return true;
            }
        }
    }

    /// <summary>
    /// The result data (only valid after successful completion).
    /// </summary>
    public byte[]? Result
    {
        get
        {
            lock (_sync)
                return _result;
        }
    }

    /// <summary>
    /// The managed error that caused the operation to fail, if any.
    /// </summary>
    public Exception? Error
    {
        get
        {
            lock (_sync)
                return _error;
        }
    }

    /// <summary>
    /// Called by the engine's main-thread callback dispatch when
    /// the async operation completes.
    /// </summary>
    internal void Complete(byte[] data)
    {
        lock (_sync)
        {
            _result = data;
            _isComplete = true;
        }
    }

    internal void BindNativeId(ulong id)
    {
        lock (_sync)
        {
            if (id == 0)
                return;

            if (_id == 0)
            {
                _id = id;
                return;
            }

            if (_id != id)
            {
                _error = new InvalidOperationException(
                    $"Native async handle mismatch: expected {_id}, received {id}.");
                _isComplete = true;
            }
        }
    }

    internal void Fail(Exception error)
    {
        lock (_sync)
        {
            _error ??= error;
            _isComplete = true;
        }
    }
}
