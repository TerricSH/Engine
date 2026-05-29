using System.Collections.Concurrent;
using Engine;

namespace Sandbox;

/// <summary>
/// In-memory mock of the Rust engine FFI for sandbox development.
///
/// Replaces the real P/Invoke calls to engine-ffi with dictionary-backed
/// storage so C# scripts can be developed and tested without the Rust engine.
///
/// # Usage
///
/// Before running any script code, call:
///   MockEngineAPI.Initialize();
///   MockEngineAPI.CreateEntity("player-001", new Gold { amount = 500 });
/// </summary>
public static class MockEngineAPI
{
    /// <summary>
    /// The simulated ECS World: entity_id → component_type → component_data.
    /// </summary>
    private static readonly ConcurrentDictionary<string, Dictionary<Type, object>> _world = new();

    /// <summary>
    /// Registered coroutine handles (simulated).
    /// </summary>
    private static readonly List<CoroutineHandle> _activeCoroutines = new();

    /// <summary>
    /// Whether the mock has been initialized.
    /// </summary>
    private static bool _initialized;

    /// <summary>
    /// Initialize the mock engine environment.
    /// Call this at the start of every sandbox test.
    /// </summary>
    public static void Initialize()
    {
        _world.Clear();
        _activeCoroutines.Clear();
        _initialized = true;
        Console.WriteLine("[Mock] Engine initialized");
    }

    /// <summary>
    /// Create an entity with the given ID and optional initial components.
    /// </summary>
    public static void CreateEntity(string entityId, params object[] components)
    {
        if (!_initialized) Initialize();

        var comps = new Dictionary<Type, object>();
        foreach (var comp in components)
            comps[comp.GetType()] = comp;

        _world[entityId] = comps;
        Console.WriteLine($"[Mock] Entity created: {entityId}");
    }

    /// <summary>
    /// Destroy an entity.
    /// </summary>
    public static void DestroyEntity(string entityId)
    {
        _world.TryRemove(entityId, out _);
        Console.WriteLine($"[Mock] Entity destroyed: {entityId}");
    }

    /// <summary>
    /// Read a component from an entity. Returns default if not found.
    /// </summary>
    public static T GetComponent<T>(string entityId) where T : class
    {
        if (_world.TryGetValue(entityId, out var comps) &&
            comps.TryGetValue(typeof(T), out var value))
        {
            return (T)value;
        }
        Console.WriteLine($"[Mock] WARNING: {typeof(T).Name} not found on {entityId}");
        return null!;
    }

    /// <summary>
    /// Write a component to an entity.
    /// </summary>
    public static void SetComponent<T>(string entityId, T component)
    {
        if (!_world.ContainsKey(entityId))
        {
            Console.WriteLine($"[Mock] WARNING: Entity {entityId} not found, creating");
            _world[entityId] = new Dictionary<Type, object>();
        }
        _world[entityId][typeof(T)] = component!;
        Console.WriteLine($"[Mock] Set {entityId}.{typeof(T).Name} = {component}");
    }

    /// <summary>
    /// Start a simulated coroutine.
    /// </summary>
    public static CoroutineHandle StartCoroutine(IEnumerator<YieldInstruction> routine)
    {
        var handle = Coroutine.Start(routine);
        _activeCoroutines.Add(handle);
        Console.WriteLine($"[Mock] Coroutine started: {handle.Id}");
        return handle;
    }

    /// <summary>
    /// Stop a simulated coroutine.
    /// </summary>
    public static void StopCoroutine(CoroutineHandle handle)
    {
        _activeCoroutines.Remove(handle);
        Console.WriteLine($"[Mock] Coroutine stopped: {handle.Id}");
    }

    /// <summary>
    /// Simulate a frame tick — advances coroutines by one step.
    /// </summary>
    public static void Tick()
    {
        // Coroutine advancement is handled by the Rust CoroutineSystem.
        // In the sandbox, we just log the tick.
        Console.WriteLine($"[Mock] Tick — {_world.Count} entities, {_activeCoroutines.Count} coroutines");
    }

    /// <summary>
    /// Print the current state of all entities for debugging.
    /// </summary>
    public static void DumpState()
    {
        Console.WriteLine("\n=== Mock World State ===");
        foreach (var (eid, comps) in _world)
        {
            Console.WriteLine($"  {eid}:");
            foreach (var (type, value) in comps)
                Console.WriteLine($"    {type.Name}: {value}");
        }
        Console.WriteLine("========================\n");
    }
}
