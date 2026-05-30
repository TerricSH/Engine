namespace Engine;

/// <summary>
/// Handle for an async I/O operation (image loading, HTTP request, etc.).
/// Returned by LoadAsync methods. Check IsComplete to see if done,
/// or yield on it in a coroutine: yield return new WaitForAsync(handle).
/// </summary>
public class AsyncHandle
{
    private readonly ulong _id;
    private byte[]? _result;
    private bool _isComplete;

    internal AsyncHandle(ulong id)
    {
        _id = id;
    }

    /// <summary>
    /// The Rust-side async operation ID.
    /// </summary>
    public ulong Id => _id;

    /// <summary>
    /// Whether the async operation has completed.
    /// </summary>
    public bool IsComplete
    {
        get
        {
            if (!_isComplete)
            {
                _isComplete = EngineAPI.ffi_async_is_complete(_id);
            }
            return _isComplete;
        }
    }

    /// <summary>
    /// The result data (only valid after IsComplete is true).
    /// </summary>
    public byte[]? Result => _result;

    /// <summary>
    /// Called by the engine's main-thread callback dispatch when
    /// the async operation completes.
    /// </summary>
    internal void Complete(byte[] data)
    {
        _result = data;
        _isComplete = true;
    }
}
