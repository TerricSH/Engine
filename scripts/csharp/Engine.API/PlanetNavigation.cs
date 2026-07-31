using System.Runtime.InteropServices;

namespace Engine;

public readonly record struct Vector3d(double X, double Y, double Z);
public readonly record struct Quaterniond(double X, double Y, double Z, double W);

public readonly record struct PlanetCoordinates(
    double Latitude,
    double Longitude,
    double Altitude);

public readonly record struct PlanetTangentFrame(
    Vector3d SurfacePoint,
    Vector3d Normal,
    Vector3d East,
    Vector3d North);

public readonly record struct PlanetSurfaceAnchorSettings(
    Vector3d Direction,
    double HeadingRadians,
    double AltitudeOffset,
    double FootprintRadius,
    double MaxSlopeRadians,
    double MaxHeightDelta,
    uint SupportSamples)
{
    public static PlanetSurfaceAnchorSettings Default => new(
        new Vector3d(0.0, 1.0, 0.0),
        0.0,
        0.0,
        1.0,
        Math.PI * 35.0 / 180.0,
        0.5,
        12);
}

public readonly record struct PlanetSurfacePlacement(
    Vector3d Position,
    Vector3d Normal,
    Vector3d Right,
    Vector3d Forward,
    Quaterniond Rotation,
    Vector3d RadialDirection,
    double AngularRadius,
    double MaximumSlopeRadians,
    double SupportHeightSpan);

public readonly record struct PlanetTerrainSettings(
    Vector3d Center,
    double Radius,
    float HeightScale,
    ulong Seed,
    uint Octaves,
    float Frequency,
    float Lacunarity,
    float Gain,
    float DomainWarpAmplitude,
    float DomainWarpFrequency)
{
    public static PlanetTerrainSettings Default => new(
        new Vector3d(0.0, 0.0, 0.0),
        1_000.0,
        24.0f,
        0,
        5,
        0.008f,
        2.0f,
        0.5f,
        0.0f,
        0.01f);
}

/// <summary>
/// Managed owner for the native Rust planetary query library. Height, surface
/// projection, tangent frames, coordinates, and arc distance all use the same
/// implementation as runtime terrain mesh generation.
/// </summary>
public sealed class PlanetTerrainQuery : IDisposable
{
    private IntPtr _handle;

    public PlanetTerrainQuery(PlanetTerrainSettings settings)
    {
        var native = new NativePlanetTerrainConfig
        {
            CenterX = settings.Center.X,
            CenterY = settings.Center.Y,
            CenterZ = settings.Center.Z,
            Radius = settings.Radius,
            HeightScale = settings.HeightScale,
            Seed = settings.Seed,
            Octaves = settings.Octaves,
            Frequency = settings.Frequency,
            Lacunarity = settings.Lacunarity,
            Gain = settings.Gain,
            DomainWarpAmplitude = settings.DomainWarpAmplitude,
            DomainWarpFrequency = settings.DomainWarpFrequency
        };
        _handle = EngineAPI.planet_query_create(ref native);
        if (_handle == IntPtr.Zero)
            throw new ArgumentException("Invalid planetary terrain settings.", nameof(settings));
    }

    ~PlanetTerrainQuery() => Release();

    public double Height(Vector3d direction)
    {
        EnsureAlive();
        return EngineAPI.planet_query_height(_handle, direction.X, direction.Y, direction.Z);
    }

    public Vector3d SurfacePoint(Vector3d direction)
    {
        EnsureAlive();
        if (!EngineAPI.planet_query_surface_point(
                _handle, direction.X, direction.Y, direction.Z, out var result))
            throw new InvalidOperationException("Native planetary surface query failed.");
        return result.ToPublic();
    }

    public double Altitude(Vector3d world)
    {
        EnsureAlive();
        return EngineAPI.planet_query_altitude(_handle, world.X, world.Y, world.Z);
    }

    public PlanetCoordinates Coordinates(Vector3d world)
    {
        EnsureAlive();
        if (!EngineAPI.planet_query_coordinates(
                _handle, world.X, world.Y, world.Z, out var result))
            throw new InvalidOperationException("Native planetary coordinate query failed.");
        return new PlanetCoordinates(result.Latitude, result.Longitude, result.Altitude);
    }

    public Vector3d WorldFromCoordinates(PlanetCoordinates coordinates)
    {
        EnsureAlive();
        var native = new NativePlanetCoordinates
        {
            Latitude = coordinates.Latitude,
            Longitude = coordinates.Longitude,
            Altitude = coordinates.Altitude
        };
        if (!EngineAPI.planet_query_world_from_coordinates(_handle, native, out var result))
            throw new InvalidOperationException("Native planetary coordinate projection failed.");
        return result.ToPublic();
    }

    public PlanetTangentFrame TangentFrame(Vector3d direction)
    {
        EnsureAlive();
        if (!EngineAPI.planet_query_tangent_frame(
                _handle, direction.X, direction.Y, direction.Z, out var result))
            throw new InvalidOperationException("Native planetary tangent query failed.");
        return new PlanetTangentFrame(
            result.SurfacePoint.ToPublic(),
            result.Normal.ToPublic(),
            result.East.ToPublic(),
            result.North.ToPublic());
    }

    /// <summary>
    /// Resolves construction position/orientation and validates the complete
    /// footprint against native terrain slope and support-height limits.
    /// </summary>
    public PlanetSurfacePlacement ResolveSurfacePlacement(
        PlanetSurfaceAnchorSettings settings)
    {
        EnsureAlive();
        var native = new NativePlanetSurfaceAnchor
        {
            Direction = NativeVector3d.FromPublic(settings.Direction),
            HeadingRadians = settings.HeadingRadians,
            AltitudeOffset = settings.AltitudeOffset,
            FootprintRadius = settings.FootprintRadius,
            MaxSlopeRadians = settings.MaxSlopeRadians,
            MaxHeightDelta = settings.MaxHeightDelta,
            SupportSamples = settings.SupportSamples
        };
        var status = EngineAPI.planet_query_resolve_surface_placement(
            _handle, native, out var result);
        if (status == -2)
            throw new ArgumentException("Invalid planetary surface anchor settings.", nameof(settings));
        if (status == -3)
            throw new InvalidOperationException("The construction footprint exceeds its slope limit.");
        if (status == -4)
            throw new InvalidOperationException(
                "The construction footprint exceeds its support-height limit.");
        if (status != 0)
            throw new InvalidOperationException("Native planetary placement query failed.");
        return result.ToPublic();
    }

    public double GreatCircleDistance(Vector3d from, Vector3d to)
    {
        EnsureAlive();
        return EngineAPI.planet_query_great_circle_distance(
            _handle, from.X, from.Y, from.Z, to.X, to.Y, to.Z);
    }

    /// <summary>
    /// Builds a seam-free navigation graph whose nodes use this query's exact
    /// seeded terrain height function.
    /// </summary>
    public SphericalSurfaceNavigation CreateSurfaceNavigation(
        uint nodeCount = 2048,
        uint neighborsPerNode = 8)
    {
        EnsureAlive();
        var graph = EngineAPI.spherical_nav_create_for_planet(
            _handle, nodeCount, neighborsPerNode);
        if (graph == IntPtr.Zero)
            throw new ArgumentException("Invalid planetary navigation graph settings.");
        return new SphericalSurfaceNavigation(graph);
    }

    public void Dispose()
    {
        Release();
        GC.SuppressFinalize(this);
    }

    private void EnsureAlive() => ObjectDisposedException.ThrowIf(_handle == IntPtr.Zero, this);

    private void Release()
    {
        if (_handle == IntPtr.Zero)
            return;
        EngineAPI.planet_query_destroy(_handle);
        _handle = IntPtr.Zero;
    }
}

public sealed record NavigationPath(IReadOnlyList<Vector3> Waypoints, float Length);
public sealed record SphericalNavigationPath(
    IReadOnlyList<Vector3d> Waypoints,
    double Length);

/// <summary>Native 26-neighbor voxel navigation for unrestricted 3D movement.</summary>
public sealed class SpaceNavigationGrid : IDisposable
{
    private IntPtr _handle;

    public SpaceNavigationGrid(
        Vector3 origin, uint cellsX, uint cellsY, uint cellsZ, float cellSize)
    {
        _handle = EngineAPI.space_nav_create(
            origin.X, origin.Y, origin.Z, cellsX, cellsY, cellsZ, cellSize);
        if (_handle == IntPtr.Zero)
            throw new ArgumentException("Invalid space navigation grid settings.");
    }

    ~SpaceNavigationGrid() => Release();

    public bool SetBlocked(int x, int y, int z, bool blocked = true)
    {
        EnsureAlive();
        return EngineAPI.space_nav_set_blocked(_handle, x, y, z, blocked);
    }

    public NavigationPath? FindPath(Vector3 from, Vector3 to)
    {
        EnsureAlive();
        var path = EngineAPI.space_nav_find_path(
            _handle, from.X, from.Y, from.Z, to.X, to.Y, to.Z);
        if (path == IntPtr.Zero)
            return null;
        try
        {
            return ReadPath(
                path,
                EngineAPI.space_path_count,
                EngineAPI.space_path_length,
                EngineAPI.space_path_point);
        }
        finally
        {
            EngineAPI.space_path_destroy(path);
        }
    }

    public void Dispose()
    {
        Release();
        GC.SuppressFinalize(this);
    }

    private void EnsureAlive() => ObjectDisposedException.ThrowIf(_handle == IntPtr.Zero, this);

    private void Release()
    {
        if (_handle == IntPtr.Zero)
            return;
        EngineAPI.space_nav_destroy(_handle);
        _handle = IntPtr.Zero;
    }

    internal static NavigationPath ReadPath(
        IntPtr path,
        Func<IntPtr, uint> count,
        Func<IntPtr, float> length,
        PathPointReader point)
    {
        var waypoints = new List<Vector3>((int)count(path));
        for (uint index = 0; index < count(path); index++)
        {
            if (!point(path, index, out var x, out var y, out var z))
                throw new InvalidOperationException("Native navigation path became invalid.");
            waypoints.Add(new Vector3(x, y, z));
        }
        return new NavigationPath(waypoints, length(path));
    }
}

/// <summary>Seam-free native navigation graph spanning a complete sphere.</summary>
public sealed class SphericalSurfaceNavigation : IDisposable
{
    private IntPtr _handle;

    internal SphericalSurfaceNavigation(IntPtr handle)
    {
        _handle = handle != IntPtr.Zero
            ? handle
            : throw new ArgumentException("Native spherical navigation handle is null.", nameof(handle));
    }

    public SphericalSurfaceNavigation(
        Vector3d center,
        double radius,
        uint nodeCount = 2048,
        uint neighborsPerNode = 8)
    {
        _handle = EngineAPI.spherical_nav_create(
            center.X, center.Y, center.Z, radius, nodeCount, neighborsPerNode);
        if (_handle == IntPtr.Zero)
            throw new ArgumentException("Invalid spherical navigation settings.");
    }

    public SphericalSurfaceNavigation(
        Vector3 center,
        float radius,
        uint nodeCount = 2048,
        uint neighborsPerNode = 8)
        : this(
            new Vector3d(center.X, center.Y, center.Z),
            radius,
            nodeCount,
            neighborsPerNode)
    {
    }

    ~SphericalSurfaceNavigation() => Release();

    public ulong DynamicRevision
    {
        get
        {
            EnsureAlive();
            return EngineAPI.spherical_nav_dynamic_revision(_handle);
        }
    }

    public bool UpsertObstacle(
        string obstacleId,
        Vector3d direction,
        double angularRadius)
    {
        EnsureAlive();
        ArgumentException.ThrowIfNullOrWhiteSpace(obstacleId);
        return EngineAPI.spherical_nav_upsert_obstacle(
            _handle,
            obstacleId,
            direction.X,
            direction.Y,
            direction.Z,
            angularRadius);
    }

    public bool RemoveObstacle(string obstacleId)
    {
        EnsureAlive();
        ArgumentException.ThrowIfNullOrWhiteSpace(obstacleId);
        return EngineAPI.spherical_nav_remove_obstacle(_handle, obstacleId);
    }

    public bool UpsertTraversalArea(
        string areaId,
        Vector3d direction,
        double angularRadius,
        double costMultiplier)
    {
        EnsureAlive();
        ArgumentException.ThrowIfNullOrWhiteSpace(areaId);
        return EngineAPI.spherical_nav_upsert_traversal_area(
            _handle,
            areaId,
            direction.X,
            direction.Y,
            direction.Z,
            angularRadius,
            costMultiplier);
    }

    public bool RemoveTraversalArea(string areaId)
    {
        EnsureAlive();
        ArgumentException.ThrowIfNullOrWhiteSpace(areaId);
        return EngineAPI.spherical_nav_remove_traversal_area(_handle, areaId);
    }

    public void ClearDynamicOverrides()
    {
        EnsureAlive();
        if (!EngineAPI.spherical_nav_clear_dynamic(_handle))
            throw new InvalidOperationException("Native spherical navigation graph is invalid.");
    }

    public SphericalNavigationPath? FindPath(Vector3d from, Vector3d to)
    {
        EnsureAlive();
        var path = EngineAPI.spherical_nav_find_path(
            _handle, from.X, from.Y, from.Z, to.X, to.Y, to.Z);
        if (path == IntPtr.Zero)
            return null;
        try
        {
            var waypoints = new List<Vector3d>((int)EngineAPI.spherical_path_count(path));
            for (uint index = 0; index < EngineAPI.spherical_path_count(path); index++)
            {
                if (!EngineAPI.spherical_path_point(path, index, out var x, out var y, out var z))
                    throw new InvalidOperationException(
                        "Native spherical navigation path became invalid.");
                waypoints.Add(new Vector3d(x, y, z));
            }
            return new SphericalNavigationPath(
                waypoints,
                EngineAPI.spherical_path_length(path));
        }
        finally
        {
            EngineAPI.spherical_path_destroy(path);
        }
    }

    public NavigationPath? FindPath(Vector3 from, Vector3 to)
    {
        var path = FindPath(
            new Vector3d(from.X, from.Y, from.Z),
            new Vector3d(to.X, to.Y, to.Z));
        return path is null
            ? null
            : new NavigationPath(
                path.Waypoints
                    .Select(point => new Vector3((float)point.X, (float)point.Y, (float)point.Z))
                    .ToArray(),
                (float)path.Length);
    }

    public void Dispose()
    {
        Release();
        GC.SuppressFinalize(this);
    }

    private void EnsureAlive() => ObjectDisposedException.ThrowIf(_handle == IntPtr.Zero, this);

    private void Release()
    {
        if (_handle == IntPtr.Zero)
            return;
        EngineAPI.spherical_nav_destroy(_handle);
        _handle = IntPtr.Zero;
    }
}

internal delegate bool PathPointReader(
    IntPtr path, uint index, out float x, out float y, out float z);

[StructLayout(LayoutKind.Sequential)]
internal struct NativePlanetTerrainConfig
{
    public double CenterX;
    public double CenterY;
    public double CenterZ;
    public double Radius;
    public float HeightScale;
    public ulong Seed;
    public uint Octaves;
    public float Frequency;
    public float Lacunarity;
    public float Gain;
    public float DomainWarpAmplitude;
    public float DomainWarpFrequency;
}

[StructLayout(LayoutKind.Sequential)]
internal struct NativeVector3d
{
    public double X;
    public double Y;
    public double Z;

    public static NativeVector3d FromPublic(Vector3d value) =>
        new() { X = value.X, Y = value.Y, Z = value.Z };

    public readonly Vector3d ToPublic() => new(X, Y, Z);
}

[StructLayout(LayoutKind.Sequential)]
internal struct NativePlanetCoordinates
{
    public double Latitude;
    public double Longitude;
    public double Altitude;
}

[StructLayout(LayoutKind.Sequential)]
internal struct NativePlanetTangentFrame
{
    public NativeVector3d SurfacePoint;
    public NativeVector3d Normal;
    public NativeVector3d East;
    public NativeVector3d North;
}

[StructLayout(LayoutKind.Sequential)]
internal struct NativePlanetSurfaceAnchor
{
    public NativeVector3d Direction;
    public double HeadingRadians;
    public double AltitudeOffset;
    public double FootprintRadius;
    public double MaxSlopeRadians;
    public double MaxHeightDelta;
    public uint SupportSamples;
}

[StructLayout(LayoutKind.Sequential)]
internal struct NativePlanetSurfacePlacement
{
    public NativeVector3d Position;
    public NativeVector3d Normal;
    public NativeVector3d Right;
    public NativeVector3d Forward;
    public double RotationX;
    public double RotationY;
    public double RotationZ;
    public double RotationW;
    public NativeVector3d RadialDirection;
    public double AngularRadius;
    public double MaximumSlopeRadians;
    public double SupportHeightSpan;

    public readonly PlanetSurfacePlacement ToPublic() => new(
        Position.ToPublic(),
        Normal.ToPublic(),
        Right.ToPublic(),
        Forward.ToPublic(),
        new Quaterniond(RotationX, RotationY, RotationZ, RotationW),
        RadialDirection.ToPublic(),
        AngularRadius,
        MaximumSlopeRadians,
        SupportHeightSpan);
}
