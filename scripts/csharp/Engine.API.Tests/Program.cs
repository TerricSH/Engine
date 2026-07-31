using System.Text;
using System.Text.Json;
using Engine;

var nativeLibraryPath = Environment.GetEnvironmentVariable("ENGINE_FFI_LIBRARY");
if (args.Contains("--expect-missing-library-rejection", StringComparer.Ordinal))
{
    ExpectEngineApiInitializationFailure("ENGINE_FFI_LIBRARY");
    Console.WriteLine("Engine.API rejected direct P/Invoke without a host library as expected.");
    return;
}

if (args.Contains("--expect-missing-pid-rejection", StringComparer.Ordinal))
{
    if (string.IsNullOrWhiteSpace(nativeLibraryPath))
        throw new Exception("Native library path is required for host PID test");
    ExpectEngineApiInitializationFailure("ENGINE_FFI_HOST_PID");
    Console.WriteLine("Engine.API rejected direct P/Invoke without a host PID as expected.");
    return;
}

if (args.Contains("--expect-process-rejection", StringComparer.Ordinal))
{
    if (string.IsNullOrWhiteSpace(nativeLibraryPath))
        throw new Exception("Native library path is required for process-boundary test");
    ExpectEngineApiInitializationFailure("ProcessHost");
    Console.WriteLine("Engine.API rejected cross-process direct P/Invoke as expected.");
    return;
}

if (!string.IsNullOrWhiteSpace(nativeLibraryPath))
{
    Environment.SetEnvironmentVariable("ENGINE_FFI_HOST_PID", Environment.ProcessId.ToString());
    Assert(EngineAPI.ffi_registry_abi_version() == 3, "native ABI version P/Invoke");
    Assert(EngineAPI.ffi_registry_struct_size() > 0, "native registry size P/Invoke");
    RunNativeCoroutineSmoke();
    RunNativePlanetAndSpatialNavigationSmoke();
}

var taggedJson = Encoding.UTF8.GetBytes(
    """
    {
      "height": { "Float32": 1.8 },
      "foot_ik_enabled": { "Bool": true },
      "state": { "Enum": "Grounded" },
      "position": { "Vec3": [1.0, 2.0, 3.0] },
      "inventory": { "List": [{ "UInt": 4 }, { "UInt": 9 }] },
      "flags": { "Map": { "visible": { "Bool": true } } }
    }
    """);

var component = ComponentJson.Deserialize<CharacterDto>(taggedJson);
Assert(Math.Abs(component.Height - 1.8f) < 0.0001f, "Float32 decode");
Assert(component.FootIkEnabled, "Bool decode");
Assert(component.State == CharacterState.Grounded, "Enum decode");
Assert(component.Position.X == 1 && component.Position.Y == 2 && component.Position.Z == 3,
    "Vec3 decode");
Assert(component.Inventory.SequenceEqual([4u, 9u]), "List decode");
Assert(component.Flags["visible"], "Map decode");

var encoded = ComponentJson.Serialize(component);
using var document = JsonDocument.Parse(encoded);
var root = document.RootElement;
Assert(root.GetProperty("height").TryGetProperty("Float32", out _), "Float32 encode");
Assert(root.GetProperty("foot_ik_enabled").TryGetProperty("Bool", out _), "Bool encode");
Assert(root.GetProperty("state").GetProperty("Enum").GetString() == "Grounded", "Enum encode");
Assert(root.GetProperty("position").GetProperty("Vec3").GetArrayLength() == 3, "Vec3 encode");
Assert(root.GetProperty("inventory").GetProperty("List").GetArrayLength() == 2, "List encode");
Assert(root.GetProperty("flags").TryGetProperty("Map", out _), "Map encode");

try
{
    _ = ComponentJson.Serialize(new UnsupportedDto { Timestamp = DateTime.UtcNow });
    throw new Exception("Ambiguous CLR type should have been rejected");
}
catch (NotSupportedException)
{
    // Expected: no silent guess for engine Value types without a mapping.
}

Console.WriteLine(
    nativeLibraryPath is null
        ? "Engine.API component JSON ABI tests passed (native smoke skipped)."
        : "Engine.API component JSON ABI and native P/Invoke smoke tests passed.");

static void Assert(bool condition, string name)
{
    if (!condition)
        throw new Exception($"Assertion failed: {name}");
}

static void ExpectEngineApiInitializationFailure(string expectedMessage)
{
    try
    {
        _ = EngineAPI.ffi_registry_abi_version();
    }
    catch (TypeInitializationException error)
        when (error.InnerException?.Message.Contains(expectedMessage, StringComparison.Ordinal)
            == true)
    {
        return;
    }
    throw new Exception(
        $"Engine.API did not reject invalid host configuration containing '{expectedMessage}'");
}

static void RunNativeCoroutineSmoke()
{
    EngineAPI.ffi_coroutine_clear();
    var events = new List<string>();
    var condition = false;
    var allCondition = false;
    var releases = 0;
    var handle = Coroutine.Start(NaturalRoutine(
        events,
        () => condition,
        () => allCondition,
        () => releases++));
    Assert(handle.Id != 0, "native coroutine start");
    Assert(EngineAPI.ffi_coroutine_active_count() == 1, "native coroutine active count");

    EngineAPI.ffi_coroutine_tick(0.01f); // enter, NextFrame
    Assert(events.SequenceEqual(["start"]), "NextFrame starts on first tick");
    EngineAPI.ffi_coroutine_tick(0.01f); // WaitForSeconds
    Assert(events.SequenceEqual(["start", "next"]), "NextFrame resumes next tick");
    EngineAPI.ffi_coroutine_tick(0.02f);
    Assert(events.Count == 2, "WaitForSeconds remains waiting");
    EngineAPI.ffi_coroutine_tick(0.02f); // WaitUntil
    Assert(events[^1] == "seconds", "WaitForSeconds resumes after elapsed delta");
    EngineAPI.ffi_coroutine_tick(0.01f);
    Assert(events[^1] == "seconds", "WaitUntil remains waiting");
    condition = true;
    EngineAPI.ffi_coroutine_tick(0.01f); // WaitForAll
    Assert(events[^1] == "until", "WaitUntil resumes when true");
    EngineAPI.ffi_coroutine_tick(0.01f);
    Assert(events[^1] == "until", "WaitForAll waits for every member");
    allCondition = true;
    EngineAPI.ffi_coroutine_tick(0.02f); // complete
    Assert(events[^1] == "all", "WaitForAll resumes when every member is ready");
    Assert(EngineAPI.ffi_coroutine_active_count() == 0, "natural completion removes coroutine");
    Assert(releases == 1, "natural completion releases managed root once");
    Coroutine.Stop(handle);
    Assert(releases == 1, "stop after completion is harmless");

    var stopReleases = 0;
    var stop = Coroutine.Start(InfiniteRoutine(() => stopReleases++));
    EngineAPI.ffi_coroutine_tick(0.01f);
    Coroutine.Stop(stop);
    Coroutine.Stop(stop);
    Assert(stopReleases == 1, "explicit stop releases managed root once");

    var clearReleases = 0;
    _ = Coroutine.Start(InfiniteRoutine(() => clearReleases++));
    EngineAPI.ffi_coroutine_tick(0.01f);
    EngineAPI.ffi_coroutine_clear();
    Assert(clearReleases == 1, "runtime clear releases managed root once");

    var failureReleases = 0;
    _ = Coroutine.Start(FailingRoutine(() => failureReleases++));
    EngineAPI.ffi_coroutine_tick(0.01f);
    EngineAPI.ffi_coroutine_tick(0.01f); // exception becomes MoveFailed, never crosses FFI
    Assert(failureReleases == 1, "managed exception releases root without crossing FFI");
    Assert(EngineAPI.ffi_coroutine_active_count() == 0, "managed failure removes coroutine");
}

static void RunNativePlanetAndSpatialNavigationSmoke()
{
    using var planet = new PlanetTerrainQuery(PlanetTerrainSettings.Default);
    var surface = planet.SurfacePoint(new Vector3d(0.0, 1.0, 0.0));
    Assert(surface.Y > 975.0 && surface.Y < 1025.0, "native planet surface");
    Assert(Math.Abs(planet.Altitude(surface)) < 1.0e-7, "native planet altitude");
    var coordinates = planet.Coordinates(surface);
    var roundTrip = planet.WorldFromCoordinates(coordinates);
    Assert(Math.Abs(roundTrip.Y - surface.Y) < 1.0e-7, "native planet coordinates");
    var placement = planet.ResolveSurfacePlacement(new PlanetSurfaceAnchorSettings(
        new Vector3d(0.0, 1.0, 0.0),
        0.3,
        2.0,
        0.0,
        Math.PI / 2.0,
        100.0,
        12));
    Assert(placement.Position.Y > surface.Y, "native construction placement altitude");
    Assert(Math.Abs(placement.AngularRadius) < double.Epsilon,
        "native construction footprint diagnostics");
    Assert(Math.Abs(
        placement.Right.X * placement.Normal.X
        + placement.Right.Y * placement.Normal.Y
        + placement.Right.Z * placement.Normal.Z) < 1.0e-9,
        "native construction placement basis");

    using var space = new SpaceNavigationGrid(new Vector3(0, 0, 0), 8, 8, 8, 1.0f);
    var spacePath = space.FindPath(new Vector3(0.5f, 0.5f, 0.5f), new Vector3(7.5f, 7.5f, 7.5f));
    Assert(spacePath is { Waypoints.Count: 2 }, "native 3D path");

    var largeCenter = new Vector3d(1.0e12, -2.0e12, 3.0e12);
    using var sphere = new SphericalSurfaceNavigation(largeCenter, 100.0, 256, 8);
    var surfacePath = sphere.FindPath(
        new Vector3d(largeCenter.X + 100.0, largeCenter.Y, largeCenter.Z),
        new Vector3d(largeCenter.X - 100.0, largeCenter.Y, largeCenter.Z));
    Assert(surfacePath is { Waypoints.Count: > 3 }, "native spherical path");
    Assert(Math.Abs(surfacePath!.Waypoints[0].X - largeCenter.X - 100.0) < 1.0e-7,
        "native spherical path keeps f64 world coordinates");
    Assert(sphere.UpsertObstacle("landing-pad", new Vector3d(1.0, 0.0, 0.0), 0.05),
        "dynamic spherical obstacle");
    Assert(sphere.FindPath(
        new Vector3d(largeCenter.X + 100.0, largeCenter.Y, largeCenter.Z),
        new Vector3d(largeCenter.X - 100.0, largeCenter.Y, largeCenter.Z)) is null,
        "dynamic obstacle blocks covered endpoint");
    Assert(sphere.RemoveObstacle("landing-pad"), "dynamic spherical obstacle removal");

    using var terrainSphere = planet.CreateSurfaceNavigation(256, 8);
    var terrainPath = terrainSphere.FindPath(
        new Vector3d(1000, 0, 0),
        new Vector3d(-1000, 0, 0));
    Assert(terrainPath is { Waypoints.Count: > 3 }, "native terrain-projected spherical path");
}

static IEnumerator<YieldInstruction> NaturalRoutine(
    List<string> events,
    Func<bool> condition,
    Func<bool> allCondition,
    Action released)
{
    try
    {
        events.Add("start");
        yield return new WaitForNextFrame();
        events.Add("next");
        yield return new WaitForSeconds(0.03f);
        events.Add("seconds");
        yield return new WaitUntil(condition);
        events.Add("until");
        yield return new WaitForAll(
            new WaitForNextFrame(),
            new WaitForSeconds(0.02f),
            new WaitUntil(allCondition));
        events.Add("all");
    }
    finally
    {
        released();
    }
}

static IEnumerator<YieldInstruction> InfiniteRoutine(Action released)
{
    try
    {
        while (true)
            yield return new WaitForNextFrame();
    }
    finally
    {
        released();
    }
}

static IEnumerator<YieldInstruction> FailingRoutine(Action released)
{
    try
    {
        yield return new WaitForNextFrame();
        throw new InvalidOperationException("expected managed coroutine failure");
    }
    finally
    {
        released();
    }
}

internal sealed class CharacterDto
{
    public float Height { get; set; }
    public bool FootIkEnabled { get; set; }
    public CharacterState State { get; set; }
    public Vector3 Position { get; set; }
    public List<uint> Inventory { get; set; } = [];
    public Dictionary<string, bool> Flags { get; set; } = [];
}

internal enum CharacterState
{
    Grounded,
    Jumping,
}

internal sealed class UnsupportedDto
{
    public DateTime Timestamp { get; set; }
}
