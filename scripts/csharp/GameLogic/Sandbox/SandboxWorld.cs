namespace Sandbox;

/// <summary>
/// Simulated ECS world for sandbox testing.
/// Provides a higher-level API over MockEngineAPI for writing test scenarios.
/// </summary>
public class SandboxWorld : IDisposable
{
    private bool _initialized;

    /// <summary>
    /// Create a new sandbox world and initialize the mock engine.
    /// </summary>
    public SandboxWorld()
    {
        MockEngineAPI.Initialize();
        _initialized = true;
    }

    /// <summary>
    /// Spawn an entity with components.
    /// </summary>
    public string Spawn(params object[] components)
    {
        var id = $"entity-{Guid.NewGuid():N}";
        MockEngineAPI.CreateEntity(id, components);
        return id;
    }

    /// <summary>
    /// Spawn an entity with a specific ID and components.
    /// </summary>
    public string SpawnWithId(string entityId, params object[] components)
    {
        MockEngineAPI.CreateEntity(entityId, components);
        return entityId;
    }

    /// <summary>
    /// Get a component from an entity.
    /// </summary>
    public T Get<T>(string entityId) where T : class
        => MockEngineAPI.GetComponent<T>(entityId);

    /// <summary>
    /// Set a component on an entity.
    /// </summary>
    public void Set<T>(string entityId, T component)
        => MockEngineAPI.SetComponent(entityId, component);

    /// <summary>
    /// Destroy an entity.
    /// </summary>
    public void Destroy(string entityId)
        => MockEngineAPI.DestroyEntity(entityId);

    /// <summary>
    /// Simulate one frame tick.
    /// </summary>
    public void Tick()
        => MockEngineAPI.Tick();

    /// <summary>
    /// Dump the current world state for debugging.
    /// </summary>
    public void Dump()
        => MockEngineAPI.DumpState();

    /// <summary>
    /// Shutdown the sandbox world.
    /// </summary>
    public void Dispose()
    {
        if (_initialized)
        {
            Console.WriteLine("[Sandbox] World disposed");
            _initialized = false;
        }
    }
}
