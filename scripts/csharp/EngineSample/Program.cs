/// <summary>
/// Engine Script Protocol Host — .NET runtime implementation.
///
/// This program implements the JSON-line protocol that the engine's
/// <c>ProcessHost</c> uses to communicate with a .NET script runtime.
///
/// It reads JSON messages from stdin, processes lifecycle commands, and
/// writes JSON responses to stdout.
///
/// Build:
///   dotnet publish -c Release -o out
///
/// Run (standalone test):
///   echo '{"type":"Shutdown"}' | dotnet run
/// </summary>
using System.Reflection;
using System.Runtime.Loader;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace EngineSample;

/// <summary>
/// Represents the scalar types the engine can pass across the script boundary.
/// </summary>
[JsonConverter(typeof(ScriptValueConverter))]
public class ScriptValue
{
    public string? Type { get; set; }
    public object? Value { get; set; }

    public static ScriptValue Null() => new() { Type = "Null" };
    public static ScriptValue FromBool(bool b) => new() { Type = "Bool", Value = b };
    public static ScriptValue FromInt(long i) => new() { Type = "Int", Value = i };
    public static ScriptValue FromFloat(double f) => new() { Type = "Float", Value = f };
    public static ScriptValue FromString(string s) => new() { Type = "String", Value = s };
}

/// <summary>
/// Custom JSON converter for ScriptValue that matches the Rust
/// ScriptValue's externally-tagged serde representation.
/// </summary>
public class ScriptValueConverter : JsonConverter<ScriptValue>
{
    public override ScriptValue? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        using var doc = JsonDocument.ParseValue(ref reader);
        var root = doc.RootElement;

        // Externally-tagged: the first (and only) property name is the variant
        if (root.ValueKind == JsonValueKind.String && root.GetString() == "Null")
            return ScriptValue.Null();

        if (root.ValueKind != JsonValueKind.Object)
            return ScriptValue.Null();

        foreach (var prop in root.EnumerateObject())
        {
            return prop.Name switch
            {
                "Null" => ScriptValue.Null(),
                "Bool" => ScriptValue.FromBool(prop.Value.GetBoolean()),
                "Int" => ScriptValue.FromInt(prop.Value.GetInt64()),
                "Float" => ScriptValue.FromFloat(prop.Value.GetDouble()),
                "String" => ScriptValue.FromString(prop.Value.GetString()!),
                _ => ScriptValue.Null()
            };
        }
        return ScriptValue.Null();
    }

    public override void Write(Utf8JsonWriter writer, ScriptValue value, JsonSerializerOptions options)
    {
        if (value.Type == "Null")
        {
            writer.WriteStringValue("Null");
            return;
        }

        writer.WriteStartObject();
        switch (value.Type)
        {
            case "Bool":
                writer.WriteBoolean("Bool", (bool)value.Value!);
                break;
            case "Int":
                writer.WriteNumber("Int", (long)value.Value!);
                break;
            case "Float":
                writer.WriteNumber("Float", (double)value.Value!);
                break;
            case "String":
                writer.WriteString("String", (string)value.Value!);
                break;
            default:
                writer.WriteNull("Null");
                break;
        }
        writer.WriteEndObject();
    }
}

/// <summary>
/// All possible messages in the engine-script protocol.
/// </summary>
[JsonConverter(typeof(ScriptMessageConverter))]
public class ScriptMessage
{
    public string Type { get; set; } = "";
    public string? Id { get; set; }
    public string? DataBase64 { get; set; }
    public List<string>? Classes { get; set; }
    public string? AssemblyId { get; set; }
    public string? ClassName { get; set; }
    public string? InstanceId { get; set; }
    public string? Method { get; set; }
    public List<ScriptValue>? Args { get; set; }
    public ScriptValue? Result { get; set; }
    public string? Code { get; set; }
    public string? Operation { get; set; }
    public string? Message { get; set; }
    public string? Name { get; set; }
    public ScriptValue? Value { get; set; }
    public string? ContextJson { get; set; }
    public string? CommandsJson { get; set; }
}

/// <summary>
/// Custom JSON converter that reads/writes the <c>"type"</c>-discriminated
/// union matching the Rust <c>#[serde(tag = "type")]</c> representation.
/// </summary>
public class ScriptMessageConverter : JsonConverter<ScriptMessage>
{
    public override ScriptMessage? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        using var doc = JsonDocument.ParseValue(ref reader);
        var root = doc.RootElement;

        if (!root.TryGetProperty("type", out var typeProp))
            throw new JsonException("Missing 'type' discriminator");

        var typeName = typeProp.GetString()!;
        var msg = new ScriptMessage { Type = typeName };

        foreach (var prop in root.EnumerateObject())
        {
            switch (prop.Name)
            {
                case "type": break;
                case "id": msg.Id = prop.Value.GetString(); break;
                case "data_base64": msg.DataBase64 = prop.Value.GetString(); break;
                case "classes": msg.Classes = JsonSerializer.Deserialize<List<string>>(prop.Value.GetRawText()); break;
                case "assembly_id": msg.AssemblyId = prop.Value.GetString(); break;
                case "class_name": msg.ClassName = prop.Value.GetString(); break;
                case "instance_id": msg.InstanceId = prop.Value.GetString(); break;
                case "method": msg.Method = prop.Value.GetString(); break;
                case "args": msg.Args = JsonSerializer.Deserialize<List<ScriptValue>>(prop.Value.GetRawText(), options); break;
                case "result": msg.Result = JsonSerializer.Deserialize<ScriptValue>(prop.Value.GetRawText(), options); break;
                case "code": msg.Code = prop.Value.GetString(); break;
                case "operation": msg.Operation = prop.Value.GetString(); break;
                case "message": msg.Message = prop.Value.GetString(); break;
                case "name": msg.Name = prop.Value.GetString(); break;
                case "value": msg.Value = JsonSerializer.Deserialize<ScriptValue>(prop.Value.GetRawText(), options); break;
                case "context_json": msg.ContextJson = prop.Value.GetString(); break;
                case "commands_json": msg.CommandsJson = prop.Value.GetString(); break;
            }
        }
        return msg;
    }

    public override void Write(Utf8JsonWriter writer, ScriptMessage value, JsonSerializerOptions options)
    {
        writer.WriteStartObject();
        writer.WriteString("type", value.Type);

        WriteProp(writer, "id", value.Id);
        WriteProp(writer, "data_base64", value.DataBase64);
        WriteProp(writer, "assembly_id", value.AssemblyId);
        WriteProp(writer, "class_name", value.ClassName);
        WriteProp(writer, "instance_id", value.InstanceId);
        WriteProp(writer, "method", value.Method);
        WriteProp(writer, "code", value.Code);
        WriteProp(writer, "operation", value.Operation);
        WriteProp(writer, "message", value.Message);
        WriteProp(writer, "name", value.Name);
        WriteProp(writer, "context_json", value.ContextJson);
        WriteProp(writer, "commands_json", value.CommandsJson);

        if (value.Classes != null)
        {
            writer.WritePropertyName("classes");
            JsonSerializer.Serialize(writer, value.Classes, options);
        }
        if (value.Args != null)
        {
            writer.WritePropertyName("args");
            JsonSerializer.Serialize(writer, value.Args, options);
        }
        if (value.Result != null)
        {
            writer.WritePropertyName("result");
            JsonSerializer.Serialize(writer, value.Result, options);
        }
        if (value.Value != null)
        {
            writer.WritePropertyName("value");
            JsonSerializer.Serialize(writer, value.Value, options);
        }

        writer.WriteEndObject();
    }

    private static void WriteProp(Utf8JsonWriter writer, string name, string? value)
    {
        if (value != null)
            writer.WriteString(name, value);
    }
}

// ---------------------------------------------------------------------------
// Runtime instance — wraps a .NET object with its type for reflection
// ---------------------------------------------------------------------------

/// <summary>
/// A script instance backed by a real .NET object, with lazy reflection
/// for method invocation and field access.
/// </summary>
class ScriptInstance
{
    public string InstanceId { get; }
    public Type Type { get; }
    public object Instance { get; }

    public ScriptInstance(string instanceId, Type type, object instance)
    {
        InstanceId = instanceId;
        Type = type;
        Instance = instance;
    }

    public ScriptValue CallMethod(string method, List<ScriptValue> args)
    {
        var methodInfo = Type.GetMethod(method, BindingFlags.Public | BindingFlags.Instance | BindingFlags.NonPublic);
        if (methodInfo == null)
        {
            // If the method doesn't exist, return null gracefully rather than
            // erroring — lifecycle methods (OnCreate, OnStart, etc.) are
            // optional.
            Console.Error.WriteLine($"[ScriptHost] Method '{method}' not found on {Type.Name}, returning null");
            return ScriptValue.Null();
        }

        var parameters = methodInfo.GetParameters();
        var convertedArgs = new object?[parameters.Length];

        for (int i = 0; i < parameters.Length; i++)
        {
            if (i < args.Count)
                convertedArgs[i] = ConvertScriptValueToObject(args[i], parameters[i].ParameterType);
            else
                convertedArgs[i] = parameters[i].DefaultValue;
        }

        var result = methodInfo.Invoke(Instance, convertedArgs);
        return ConvertObjectToScriptValue(result);
    }

    public ScriptValue GetField(string name)
    {
        var field = Type.GetField(name, BindingFlags.Public | BindingFlags.Instance | BindingFlags.NonPublic);
        if (field != null)
            return ConvertObjectToScriptValue(field.GetValue(Instance));

        var prop = Type.GetProperty(name, BindingFlags.Public | BindingFlags.Instance | BindingFlags.NonPublic);
        if (prop != null)
            return ConvertObjectToScriptValue(prop.GetValue(Instance));

        return ScriptValue.Null();
    }

    public void SetField(string name, ScriptValue value)
    {
        var field = Type.GetField(name, BindingFlags.Public | BindingFlags.Instance | BindingFlags.NonPublic);
        if (field != null)
        {
            field.SetValue(Instance, ConvertScriptValueToObject(value, field.FieldType));
            return;
        }

        var prop = Type.GetProperty(name, BindingFlags.Public | BindingFlags.Instance | BindingFlags.NonPublic);
        if (prop != null)
        {
            prop.SetValue(Instance, ConvertScriptValueToObject(value, prop.PropertyType));
            return;
        }

        throw new MissingMemberException(Type.FullName, name);
    }

    public void SetGameplayContext(string contextJson)
    {
        var method = Type.GetMethod(
            "__EngineSetGameplayContext",
            BindingFlags.Public | BindingFlags.Instance | BindingFlags.NonPublic);
        // Legacy scripts remain lifecycle-compatible but simply do not expose
        // the first-phase gameplay API.
        if (method == null)
            return;
        method.Invoke(Instance, new object?[] { contextJson });
    }

    public string DrainGameplayCommands()
    {
        var method = Type.GetMethod(
            "__EngineDrainGameplayCommands",
            BindingFlags.Public | BindingFlags.Instance | BindingFlags.NonPublic);
        if (method == null)
            return "[]";
        var result = method.Invoke(Instance, Array.Empty<object?>());
        if (result is not string json)
            throw new InvalidOperationException(
                $"{Type.FullName}.__EngineDrainGameplayCommands must return a JSON string");
        return json;
    }

    // ── Value conversion helpers ──────────────────────────────────────────

    static object? ConvertScriptValueToObject(ScriptValue sv, Type targetType)
    {
        if (sv.Type == "Null" || sv.Value == null)
            return targetType.IsValueType ? Activator.CreateInstance(targetType) : null;

        return sv.Type switch
        {
            "Bool" => Convert.ChangeType(sv.Value, targetType),
            "Int" => Convert.ChangeType(sv.Value, targetType),
            "Float" => Convert.ChangeType(sv.Value, targetType),
            "String" => sv.Value.ToString(),
            _ => Convert.ChangeType(sv.Value, targetType)
        };
    }

    static ScriptValue ConvertObjectToScriptValue(object? obj)
    {
        if (obj == null)
            return ScriptValue.Null();

        var type = obj.GetType();
        if (type == typeof(bool))
            return ScriptValue.FromBool((bool)obj);
        if (type == typeof(int) || type == typeof(long) || type == typeof(short) || type == typeof(byte))
            return ScriptValue.FromInt(Convert.ToInt64(obj));
        if (type == typeof(float) || type == typeof(double) || type == typeof(decimal))
            return ScriptValue.FromFloat(Convert.ToDouble(obj));
        if (type == typeof(string))
            return ScriptValue.FromString((string)obj);
        if (type == typeof(char))
            return ScriptValue.FromString(((char)obj).ToString());

        return ScriptValue.FromString(obj.ToString() ?? "");
    }
}

// ---------------------------------------------------------------------------
// Protocol host — reads JSON lines from stdin, dispatches to handlers
// ---------------------------------------------------------------------------

/// <summary>
/// Main protocol host — reads JSON lines from stdin, dispatches them to
/// the appropriate handler, and writes JSON response lines to stdout.
/// </summary>
class ScriptProtocolHost
{
    private readonly TextWriter _protocolOutput;
    /// One shared context ensures separately uploaded SDK and game assemblies
    /// participate in normal managed dependency resolution by assembly name.
    private readonly AssemblyLoadContext _scriptLoadContext =
        new("EngineScriptAssemblies", isCollectible: false);
    /// Loaded assemblies: assembly_id → Assembly
    private readonly Dictionary<string, Assembly> _assemblies = new();
    /// Reflection-verified EngineBehaviour classes by assembly id.
    private readonly Dictionary<string, HashSet<string>> _verifiedClasses = new();
    /// Runtime instances: instance_id → ScriptInstance
    private readonly Dictionary<string, ScriptInstance> _instances = new();

    public ScriptProtocolHost(TextWriter protocolOutput)
    {
        _protocolOutput = protocolOutput;
    }

    public void Run()
    {
        string? line;
        while ((line = Console.ReadLine()) != null)
        {
            try
            {
                var msg = JsonSerializer.Deserialize<ScriptMessage>(line);
                if (msg == null) continue;

                var response = ProcessMessage(msg);
                Respond(response);
                if (msg.Type == "Shutdown")
                    return;
            }
            catch (Exception ex)
            {
                Respond(MakeError(
                    "PROTOCOL_EXCEPTION",
                    "ProcessMessage",
                    DescribeException(ex)));
            }
        }
    }

    ScriptMessage ProcessMessage(ScriptMessage msg)
    {
        return msg.Type switch
        {
            "LoadAssembly" => HandleLoadAssembly(msg),
            "Instantiate" => HandleInstantiate(msg),
            "CallMethod" => HandleCallMethod(msg),
            "SetField" => HandleSetField(msg),
            "GetField" => HandleGetField(msg),
            "SetGameplayContext" => HandleSetGameplayContext(msg),
            "DrainGameplayCommands" => HandleDrainGameplayCommands(msg),
            "Shutdown" => HandleShutdown(),
            _ => MakeError(
                "UNKNOWN_MESSAGE",
                "ProcessMessage",
                $"Unknown message type: {msg.Type}")
        };
    }

    ScriptMessage HandleLoadAssembly(ScriptMessage msg)
    {
        var id = msg.Id ?? "unknown";
        var data = msg.DataBase64 ?? "";

        try
        {
            if (string.IsNullOrWhiteSpace(id))
                return MakeError(
                    "INVALID_ASSEMBLY_ID",
                    "LoadAssembly",
                    "Assembly id cannot be empty");
            if (_assemblies.ContainsKey(id))
                return MakeError(
                    "DUPLICATE_ASSEMBLY_ID",
                    "LoadAssembly",
                    $"Assembly id '{id}' is already loaded",
                    id);
            var bytes = Convert.FromBase64String(data);
            using var stream = new MemoryStream(bytes, writable: false);
            var assembly = _scriptLoadContext.LoadFromStream(stream);
            _assemblies[id] = assembly;

            var classes = DiscoverBehaviourClasses(assembly);
            _verifiedClasses[id] = classes.ToHashSet(StringComparer.Ordinal);

            Console.Error.WriteLine(
                $"[ScriptHost] LoadAssembly: {id} ({classes.Count} verified behaviours)");

            return new ScriptMessage
            {
                Type = "AssemblyLoaded",
                Id = id,
                Classes = classes
            };
        }
        catch (ReflectionTypeLoadException ex)
        {
            _assemblies.Remove(id);
            _verifiedClasses.Remove(id);
            var loaderErrors = ex.LoaderExceptions
                .Where(error => error != null)
                .Select(error => DescribeException(error!))
                .Distinct(StringComparer.Ordinal);
            return MakeError(
                "REFLECTION_TYPE_LOAD_FAILED",
                "LoadAssembly",
                $"Could not reflect script classes: {string.Join(" | ", loaderErrors)}",
                id);
        }
        catch (Exception ex)
        {
            _assemblies.Remove(id);
            _verifiedClasses.Remove(id);
            return MakeError(
                "ASSEMBLY_LOAD_FAILED",
                "LoadAssembly",
                DescribeException(ex),
                id);
        }
    }

    List<string> DiscoverBehaviourClasses(Assembly assembly)
    {
        var engineBehaviour = _assemblies.Values
            .Select(candidate => candidate.GetType(
                "Engine.EngineBehaviour",
                throwOnError: false,
                ignoreCase: false))
            .FirstOrDefault(candidate => candidate is { IsClass: true, IsAbstract: true });
        if (engineBehaviour == null)
            return new List<string>();

        return assembly.GetTypes()
            .Where(type =>
                type.IsClass &&
                !type.IsAbstract &&
                type.FullName != null &&
                engineBehaviour.IsAssignableFrom(type))
            .Select(type => type.FullName!)
            .Distinct(StringComparer.Ordinal)
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToList();
    }

    ScriptMessage HandleInstantiate(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? Guid.NewGuid().ToString();
        var assemblyId = msg.AssemblyId ?? "";
        var className = msg.ClassName ?? "";

        if (!_assemblies.TryGetValue(assemblyId, out var assembly))
            return MakeError(
                "ASSEMBLY_NOT_LOADED",
                "Instantiate",
                $"Assembly '{assemblyId}' is not loaded",
                assemblyId);

        if (!_verifiedClasses.TryGetValue(assemblyId, out var verified) ||
            !verified.Contains(className))
            return MakeError(
                "SCRIPT_CLASS_NOT_VERIFIED",
                "Instantiate",
                $"Class '{className}' is not a concrete Engine.EngineBehaviour reported by reflection",
                assemblyId);

        var type = assembly.GetType(className, throwOnError: false, ignoreCase: false);
        if (type == null)
            return MakeError(
                "SCRIPT_CLASS_DISAPPEARED",
                "Instantiate",
                $"Verified class '{className}' can no longer be resolved",
                assemblyId);

        try
        {
            var instance = Activator.CreateInstance(type);
            if (instance == null)
                return MakeError(
                    "SCRIPT_INSTANTIATION_FAILED",
                    "Instantiate",
                    $"Activator returned null for '{className}'",
                    assemblyId);

            var scriptInstance = new ScriptInstance(instanceId, type, instance);
            _instances[instanceId] = scriptInstance;

            Console.Error.WriteLine($"[ScriptHost] Instantiated {className} as {instanceId}");

            return new ScriptMessage
            {
                Type = "MethodResult",
                InstanceId = instanceId,
                Result = ScriptValue.Null()
            };
        }
        catch (Exception ex)
        {
            return MakeError(
                "SCRIPT_INSTANTIATION_FAILED",
                "Instantiate",
                DescribeException(ex),
                assemblyId);
        }
    }

    ScriptMessage HandleCallMethod(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var method = msg.Method ?? "";
        var args = msg.Args ?? new List<ScriptValue>();

        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError("INSTANCE_NOT_FOUND", "CallMethod", $"Instance not found: {instanceId}");

        try
        {
            var result = instance.CallMethod(method, args);
            return new ScriptMessage
            {
                Type = "MethodResult",
                InstanceId = instanceId,
                Result = result
            };
        }
        catch (Exception ex)
        {
            return MakeError("METHOD_FAILED", "CallMethod", $"Method '{method}' failed: {DescribeException(ex)}");
        }
    }

    ScriptMessage HandleSetField(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var name = msg.Name ?? "";
        var value = msg.Value ?? ScriptValue.Null();

        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError("INSTANCE_NOT_FOUND", "SetField", $"Instance not found: {instanceId}");

        try
        {
            instance.SetField(name, value);
            return new ScriptMessage
            {
                Type = "FieldValue",
                InstanceId = instanceId,
                Name = name,
                Value = value
            };
        }
        catch (Exception ex)
        {
            return MakeError("SET_FIELD_FAILED", "SetField", $"SetField '{name}' failed: {DescribeException(ex)}");
        }
    }

    ScriptMessage HandleGetField(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var name = msg.Name ?? "";

        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError("INSTANCE_NOT_FOUND", "GetField", $"Instance not found: {instanceId}");

        try
        {
            var value = instance.GetField(name);
            return new ScriptMessage
            {
                Type = "FieldValue",
                InstanceId = instanceId,
                Name = name,
                Value = value
            };
        }
        catch (Exception ex)
        {
            return MakeError("GET_FIELD_FAILED", "GetField", $"GetField '{name}' failed: {DescribeException(ex)}");
        }
    }

    ScriptMessage HandleSetGameplayContext(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var contextJson = msg.ContextJson
            ?? throw new InvalidOperationException("SetGameplayContext requires context_json");
        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError("INSTANCE_NOT_FOUND", "SetGameplayContext", $"Instance not found: {instanceId}");

        try
        {
            instance.SetGameplayContext(contextJson);
            return new ScriptMessage
            {
                Type = "GameplayContextSet",
                InstanceId = instanceId
            };
        }
        catch (Exception ex)
        {
            return MakeError(
                "SET_GAMEPLAY_CONTEXT_FAILED",
                "SetGameplayContext",
                $"Instance '{instanceId}': {DescribeException(ex)}");
        }
    }

    ScriptMessage HandleDrainGameplayCommands(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError("INSTANCE_NOT_FOUND", "DrainGameplayCommands", $"Instance not found: {instanceId}");

        try
        {
            return new ScriptMessage
            {
                Type = "GameplayCommands",
                InstanceId = instanceId,
                CommandsJson = instance.DrainGameplayCommands()
            };
        }
        catch (Exception ex)
        {
            return MakeError(
                "DRAIN_GAMEPLAY_COMMANDS_FAILED",
                "DrainGameplayCommands",
                $"Instance '{instanceId}': {DescribeException(ex)}");
        }
    }

    ScriptMessage HandleShutdown()
    {
        Console.Error.WriteLine("[ScriptHost] Shutting down");
        return new ScriptMessage { Type = "Shutdown" };
    }

    static ScriptMessage MakeError(
        string code,
        string operation,
        string message,
        string? assemblyId = null)
    {
        return new ScriptMessage
        {
            Type = "Error",
            Code = code,
            Operation = operation,
            Message = message,
            AssemblyId = assemblyId
        };
    }

    internal static string DescribeException(Exception exception)
    {
        while (exception is TargetInvocationException { InnerException: not null })
            exception = exception.InnerException;
        return $"{exception.GetType().Name}: {exception.Message}";
    }

    void Respond(ScriptMessage response)
    {
        var json = JsonSerializer.Serialize(response, new JsonSerializerOptions
        {
            WriteIndented = false,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull
        });
        _protocolOutput.WriteLine(json);
        _protocolOutput.Flush();
    }

}

/// <summary>
/// Entry point.
/// </summary>
class Program
{
    static int Main(string[] args)
    {
        if (args.Contains("--self-test", StringComparer.Ordinal))
            return GameplayBridgeSelfTest.Run();

        var protocolOutput = Console.Out;
        Console.SetOut(Console.Error);
        var host = new ScriptProtocolHost(protocolOutput);
        host.Run();
        return 0;
    }
}

sealed class GameplayBridgeProbe
{
    public string ContextJson { get; private set; } = "";

    public void __EngineSetGameplayContext(string contextJson) => ContextJson = contextJson;

    public string __EngineDrainGameplayCommands() =>
        "[{\"type\":\"set_transform\",\"transform\":{\"translation\":[1,2,3],\"rotation\":[0,0,0,1],\"scale\":[1,1,1]}}," +
        "{\"type\":\"set_entity_transform\",\"entity_id\":\"enemy-01\",\"transform\":{\"translation\":[4,5,6],\"rotation\":[0,0,0,1],\"scale\":[2,2,2]}}," +
        "{\"type\":\"destroy_entity\",\"entity_id\":\"enemy-01\"}," +
        "{\"type\":\"destroy_self\"}," +
        "{\"type\":\"load_scene\",\"scene_id\":\"level_two\"}]";

    public void ThrowBridgeError() => throw new InvalidOperationException("bridge boom");
}

static class GameplayBridgeSelfTest
{
    public static int Run()
    {
        try
        {
            const string contextJson =
                "{\"entity_id\":\"player\",\"transform\":null,\"input_actions\":{\"jump\":{\"type\":\"Bool\",\"value\":true}},\"ui_events\":[{\"canvas_id\":\"hud\",\"element_id\":7,\"callback_id\":\"start-game\"}],\"entities\":{\"player\":{\"transform\":null}}}";
            var probeObject = new GameplayBridgeProbe();
            var probe = new ScriptInstance("self-test", typeof(GameplayBridgeProbe), probeObject);
            probe.SetGameplayContext(contextJson);
            Require(probeObject.ContextJson == contextJson, "context reflection hook did not run");
            using (var contextDocument = JsonDocument.Parse(probeObject.ContextJson))
            {
                var uiEvent = contextDocument.RootElement.GetProperty("ui_events")[0];
                Require(
                    uiEvent.GetProperty("canvas_id").GetString() == "hud" &&
                    uiEvent.GetProperty("element_id").GetUInt32() == 7 &&
                    uiEvent.GetProperty("callback_id").GetString() == "start-game",
                    "context reflection hook did not preserve the gameplay UI event");
            }

            var commands = probe.DrainGameplayCommands();
            using (var document = JsonDocument.Parse(commands))
            {
                Require(document.RootElement.GetArrayLength() == 5, "command hook returned commands incorrectly");
                Require(
                    document.RootElement[0].GetProperty("type").GetString() == "set_transform",
                    "command hook did not preserve set_transform");
                Require(
                    document.RootElement[1].GetProperty("type").GetString() == "set_entity_transform" &&
                    document.RootElement[1].GetProperty("entity_id").GetString() == "enemy-01",
                    "command hook did not preserve set_entity_transform");
                Require(
                    document.RootElement[2].GetProperty("type").GetString() == "destroy_entity" &&
                    document.RootElement[2].GetProperty("entity_id").GetString() == "enemy-01",
                    "command hook did not preserve destroy_entity");
                Require(
                    document.RootElement[3].GetProperty("type").GetString() == "destroy_self",
                    "command hook did not preserve destroy_self");
                Require(
                    document.RootElement[4].GetProperty("type").GetString() == "load_scene" &&
                    document.RootElement[4].GetProperty("scene_id").GetString() == "level_two",
                    "command hook did not preserve load_scene");
            }

            var wire = JsonSerializer.Deserialize<ScriptMessage>(
                "{\"type\":\"SetGameplayContext\",\"instance_id\":\"self-test\",\"context_json\":\"{}\"}");
            Require(wire?.ContextJson == "{}", "protocol did not decode context_json");

            try
            {
                probe.CallMethod("ThrowBridgeError", new List<ScriptValue>());
                throw new InvalidOperationException("throwing probe method unexpectedly succeeded");
            }
            catch (Exception exception)
            {
                Require(
                    ScriptProtocolHost.DescribeException(exception) ==
                        "InvalidOperationException: bridge boom",
                    "reflection error did not preserve the inner script diagnostic");
            }

            Console.WriteLine("EngineSample gameplay bridge self-test passed.");
            return 0;
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine($"EngineSample gameplay bridge self-test failed: {exception}");
            return 1;
        }
    }

    static void Require(bool condition, string message)
    {
        if (!condition)
            throw new InvalidOperationException(message);
    }
}
