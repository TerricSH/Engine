namespace Engine;

/// <summary>
/// Base type for all yield instructions (similar to Unity's YieldInstruction).
/// Coroutines yield return these to control when they resume.
/// </summary>
public abstract class YieldInstruction { }

/// <summary>
/// Waits for a specified number of seconds before resuming.
/// </summary>
public sealed class WaitForSeconds : YieldInstruction
{
    public float Seconds { get; }

    public WaitForSeconds(float seconds)
    {
        if (!float.IsFinite(seconds) || seconds < 0)
            throw new ArgumentOutOfRangeException(nameof(seconds), "Delay must be finite and non-negative.");
        Seconds = seconds;
    }
}

/// <summary>
/// Waits for the next frame before resuming (equivalent to yield return null).
/// </summary>
public sealed class WaitForNextFrame : YieldInstruction { }

/// <summary>
/// Waits for an async operation (image load, HTTP request) to complete.
/// </summary>
public sealed class WaitForAsync : YieldInstruction
{
    public AsyncHandle Handle { get; }

    public WaitForAsync(AsyncHandle handle)
        => Handle = handle ?? throw new ArgumentNullException(nameof(handle));
}

/// <summary>
/// Waits for a condition to become true. The condition is checked each frame.
/// </summary>
public sealed class WaitUntil : YieldInstruction
{
    public Func<bool> Condition { get; }

    public WaitUntil(Func<bool> condition)
        => Condition = condition ?? throw new ArgumentNullException(nameof(condition));
}

/// <summary>
/// Waits for all specified yield instructions to complete.
/// </summary>
public sealed class WaitForAll : YieldInstruction
{
    public YieldInstruction[] Instructions { get; }

    public WaitForAll(params YieldInstruction[] instructions)
        => Instructions = instructions?.ToArray()
            ?? throw new ArgumentNullException(nameof(instructions));
}
