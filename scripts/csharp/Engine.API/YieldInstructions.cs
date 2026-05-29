namespace Engine;

/// <summary>
/// Base type for all yield instructions (similar to Unity's YieldInstruction).
/// Coroutines yield return these to control when they resume.
/// </summary>
public abstract class YieldInstruction { }

/// <summary>
/// Waits for a specified number of seconds before resuming.
/// </summary>
public class WaitForSeconds : YieldInstruction
{
    public float Seconds { get; }

    public WaitForSeconds(float seconds) => Seconds = seconds;
}

/// <summary>
/// Waits for the next frame before resuming (equivalent to yield return null).
/// </summary>
public class WaitForNextFrame : YieldInstruction { }

/// <summary>
/// Waits for an async operation (image load, HTTP request) to complete.
/// </summary>
public class WaitForAsync : YieldInstruction
{
    public AsyncHandle Handle { get; }

    public WaitForAsync(AsyncHandle handle) => Handle = handle;
}

/// <summary>
/// Waits for a condition to become true. The condition is checked each frame.
/// </summary>
public class WaitUntil : YieldInstruction
{
    public Func<bool> Condition { get; }

    public WaitUntil(Func<bool> condition) => Condition = condition;
}

/// <summary>
/// Waits for all specified yield instructions to complete.
/// </summary>
public class WaitForAll : YieldInstruction
{
    public YieldInstruction[] Instructions { get; }

    public WaitForAll(params YieldInstruction[] instructions)
        => Instructions = instructions;
}
