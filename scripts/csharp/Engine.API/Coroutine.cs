using System.Runtime.InteropServices;

namespace Engine;

/// <summary>A handle for a running coroutine.</summary>
public readonly struct CoroutineHandle
{
    public readonly ulong Id;

    public CoroutineHandle(ulong id) => Id = id;

    public static readonly CoroutineHandle Invalid = new(0);
}

[StructLayout(LayoutKind.Sequential)]
internal struct FfiYieldInstruction
{
    internal uint Tag;
    internal uint Reserved;
    internal ulong Payload;
}

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate uint FfiCoroutineMoveNext(
    IntPtr context,
    out FfiYieldInstruction instruction);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate uint FfiCoroutineReadiness(
    IntPtr context,
    ulong token,
    float deltaSeconds);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void FfiCoroutineRelease(IntPtr context);

[StructLayout(LayoutKind.Sequential)]
internal struct FfiManagedCoroutineDescriptor
{
    internal uint AbiVersion;
    internal uint StructSize;
    internal IntPtr Context;
    [MarshalAs(UnmanagedType.FunctionPtr)]
    internal FfiCoroutineMoveNext MoveNext;
    [MarshalAs(UnmanagedType.FunctionPtr)]
    internal FfiCoroutineReadiness Readiness;
    [MarshalAs(UnmanagedType.FunctionPtr)]
    internal FfiCoroutineRelease Release;
}

/// <summary>
/// Starts and stops managed enumerators advanced by the native main-thread
/// scheduler. Native code owns the enumerator root after a successful start.
/// </summary>
public static class Coroutine
{
    private const uint DescriptorAbiVersion = 1;
    private const uint MoveCompleted = 0;
    private const uint MoveYielded = 1;
    private const uint MoveFailed = 2;
    private const uint ReadyWaiting = 0;
    private const uint Ready = 1;
    private const uint ReadyFailed = 2;

    private const uint YieldNextFrame = 0;
    private const uint YieldWaitForSeconds = 1;
    private const uint YieldWaitForAsync = 2;
    private const uint YieldWaitUntil = 3;
    private const uint YieldWaitForAll = 4;

    // The descriptor stores unmanaged thunks for these delegates, so they must
    // remain strongly rooted for the process lifetime.
    private static readonly FfiCoroutineMoveNext MoveNextCallback = MoveNext;
    private static readonly FfiCoroutineReadiness ReadinessCallback = IsReady;
    private static readonly FfiCoroutineRelease ReleaseCallback = Release;

    /// <summary>Start a coroutine. Invalid means native validation failed.</summary>
    public static CoroutineHandle Start(IEnumerator<YieldInstruction> routine)
    {
        ArgumentNullException.ThrowIfNull(routine);

        var state = new ManagedCoroutineState(routine);
        var root = GCHandle.Alloc(state, GCHandleType.Normal);
        var descriptor = new FfiManagedCoroutineDescriptor
        {
            AbiVersion = DescriptorAbiVersion,
            StructSize = checked((uint)Marshal.SizeOf<FfiManagedCoroutineDescriptor>()),
            Context = GCHandle.ToIntPtr(root),
            MoveNext = MoveNextCallback,
            Readiness = ReadinessCallback,
            Release = ReleaseCallback,
        };

        ulong id;
        try
        {
            id = EngineAPI.ffi_coroutine_start(ref descriptor);
        }
        catch
        {
            state.Dispose();
            root.Free();
            throw;
        }

        if (id == 0)
        {
            state.Dispose();
            root.Free();
            return CoroutineHandle.Invalid;
        }

        // Native owns `root` from this point and releases it through the
        // descriptor on completion, cancellation, failure, or runtime clear.
        return new CoroutineHandle(id);
    }

    /// <summary>Cancel a running coroutine. Repeated calls are harmless.</summary>
    public static void Stop(CoroutineHandle handle)
    {
        if (handle.Id != 0)
            EngineAPI.ffi_coroutine_cancel(handle.Id);
    }

    private static uint MoveNext(IntPtr context, out FfiYieldInstruction instruction)
    {
        instruction = default;
        try
        {
            return State(context).MoveNext(out instruction) ? MoveYielded : MoveCompleted;
        }
        catch (Exception error)
        {
            TryRecordFailure(context, error);
            return MoveFailed;
        }
    }

    private static uint IsReady(IntPtr context, ulong token, float deltaSeconds)
    {
        try
        {
            return State(context).IsReady(token, deltaSeconds) ? Ready : ReadyWaiting;
        }
        catch (Exception error)
        {
            TryRecordFailure(context, error);
            return ReadyFailed;
        }
    }

    private static void Release(IntPtr context)
    {
        if (context == IntPtr.Zero)
            return;

        var root = GCHandle.FromIntPtr(context);
        try
        {
            if (root.Target is ManagedCoroutineState state)
                state.Dispose();
        }
        catch
        {
            // Managed exceptions must never escape a reverse P/Invoke callback.
        }
        finally
        {
            if (root.IsAllocated)
                root.Free();
        }
    }

    private static ManagedCoroutineState State(IntPtr context)
    {
        if (context == IntPtr.Zero)
            throw new InvalidOperationException("Native coroutine context is null.");
        return GCHandle.FromIntPtr(context).Target as ManagedCoroutineState
            ?? throw new InvalidOperationException("Native coroutine context is not active.");
    }

    private static void TryRecordFailure(IntPtr context, Exception error)
    {
        TryLogFailure(error);
        try
        {
            State(context).RecordFailure(error);
        }
        catch
        {
            // The scheduler will still fail and release the invalid context.
        }
    }

    private static void TryLogFailure(Exception error)
    {
        try
        {
            EngineAPI.ffi_log_error($"Managed coroutine failed: {error}");
        }
        catch
        {
            // Diagnostics must never make a reverse P/Invoke callback fail.
        }
    }

    private sealed class ManagedCoroutineState : IDisposable
    {
        private readonly IEnumerator<YieldInstruction> _routine;
        private object? _managedWait;
        private ulong _waitToken;
        private int _disposed;

        internal ManagedCoroutineState(IEnumerator<YieldInstruction> routine) => _routine = routine;

        internal Exception? Failure { get; private set; }

        internal bool MoveNext(out FfiYieldInstruction instruction)
        {
            ObjectDisposedException.ThrowIf(_disposed != 0, this);
            _managedWait = null;
            if (!_routine.MoveNext())
            {
                instruction = default;
                return false;
            }

            instruction = ConvertInstruction(_routine.Current);
            return true;
        }

        internal bool IsReady(ulong token, float deltaSeconds)
        {
            ObjectDisposedException.ThrowIf(_disposed != 0, this);
            if (token == 0 || token != _waitToken || _managedWait is null)
                throw new InvalidOperationException("Native scheduler supplied a stale wait token.");

            return _managedWait switch
            {
                WaitUntil wait => wait.Condition(),
                WaitGroup group => group.Tick(deltaSeconds),
                _ => throw new InvalidOperationException("Unsupported managed wait state."),
            };
        }

        internal void RecordFailure(Exception error) => Failure ??= error;

        private FfiYieldInstruction ConvertInstruction(YieldInstruction? instruction)
        {
            switch (instruction)
            {
                case null:
                case WaitForNextFrame:
                    return Tagged(YieldNextFrame, 0);
                case WaitForSeconds seconds:
                    return Tagged(YieldWaitForSeconds, BitConverter.SingleToUInt32Bits(seconds.Seconds));
                case WaitForAsync asyncWait:
                    if (asyncWait.Handle.Id == 0)
                        throw new InvalidOperationException("WaitForAsync requires a valid native handle.");
                    return Tagged(YieldWaitForAsync, asyncWait.Handle.Id);
                case WaitUntil until:
                    return Managed(YieldWaitUntil, until);
                case WaitForAll all:
                    return Managed(YieldWaitForAll, new WaitGroup(all.Instructions));
                default:
                    throw new NotSupportedException(
                        $"Unsupported yield instruction '{instruction.GetType().FullName}'.");
            }
        }

        private FfiYieldInstruction Managed(uint tag, object wait)
        {
            _managedWait = wait;
            _waitToken = _waitToken == ulong.MaxValue ? 1 : _waitToken + 1;
            return Tagged(tag, _waitToken);
        }

        private static FfiYieldInstruction Tagged(uint tag, ulong payload) => new()
        {
            Tag = tag,
            Reserved = 0,
            Payload = payload,
        };

        public void Dispose()
        {
            if (Interlocked.Exchange(ref _disposed, 1) != 0)
                return;
            _managedWait = null;
            try
            {
                _routine.Dispose();
            }
            catch (Exception error)
            {
                RecordFailure(error);
                TryLogFailure(error);
            }
        }
    }

    private sealed class WaitGroup
    {
        private readonly WaitNode[] _nodes;
        private readonly bool[] _completed;

        internal WaitGroup(IReadOnlyList<YieldInstruction> instructions)
        {
            _nodes = new WaitNode[instructions.Count];
            _completed = new bool[instructions.Count];
            for (var index = 0; index < instructions.Count; index++)
                _nodes[index] = WaitNode.Create(instructions[index]);
        }

        internal bool Tick(float deltaSeconds)
        {
            var ready = true;
            for (var index = 0; index < _nodes.Length; index++)
            {
                if (!_completed[index])
                    _completed[index] = _nodes[index].Tick(deltaSeconds);
                ready &= _completed[index];
            }
            return ready;
        }
    }

    private abstract class WaitNode
    {
        internal abstract bool Tick(float deltaSeconds);

        internal static WaitNode Create(YieldInstruction instruction) => instruction switch
        {
            WaitForNextFrame => new NextFrameNode(),
            WaitForSeconds seconds => new SecondsNode(seconds.Seconds),
            WaitForAsync asyncWait when asyncWait.Handle.Id != 0 => new AsyncNode(asyncWait.Handle.Id),
            WaitForAsync => throw new InvalidOperationException(
                "WaitForAll contains WaitForAsync with an invalid native handle."),
            WaitUntil until => new UntilNode(until.Condition),
            WaitForAll all => new GroupNode(new WaitGroup(all.Instructions)),
            null => throw new ArgumentException("WaitForAll cannot contain null instructions."),
            _ => throw new NotSupportedException(
                $"WaitForAll cannot evaluate '{instruction.GetType().FullName}'."),
        };
    }

    private sealed class NextFrameNode : WaitNode
    {
        internal override bool Tick(float deltaSeconds) => true;
    }

    private sealed class SecondsNode(float seconds) : WaitNode
    {
        private float _remaining = seconds;

        internal override bool Tick(float deltaSeconds)
        {
            if (float.IsFinite(deltaSeconds) && deltaSeconds > 0)
                _remaining -= deltaSeconds;
            return _remaining <= 0;
        }
    }

    private sealed class AsyncNode(ulong handle) : WaitNode
    {
        internal override bool Tick(float deltaSeconds) => EngineAPI.ffi_async_is_complete(handle);
    }

    private sealed class UntilNode(Func<bool> condition) : WaitNode
    {
        internal override bool Tick(float deltaSeconds) => condition();
    }

    private sealed class GroupNode(WaitGroup group) : WaitNode
    {
        internal override bool Tick(float deltaSeconds) => group.Tick(deltaSeconds);
    }
}
