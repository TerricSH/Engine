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
    public List<string>? Types { get; set; }
    public string? AssemblyId { get; set; }
    public string? ClassName { get; set; }
    public string? InstanceId { get; set; }
    public string? Method { get; set; }
    public List<ScriptValue>? Args { get; set; }
    public ScriptValue? Result { get; set; }
    public string? Message { get; set; }
    public string? Name { get; set; }
    public ScriptValue? Value { get; set; }
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
                case "types": msg.Types = JsonSerializer.Deserialize<List<string>>(prop.Value.GetRawText()); break;
                case "assembly_id": msg.AssemblyId = prop.Value.GetString(); break;
                case "class_name": msg.ClassName = prop.Value.GetString(); break;
                case "instance_id": msg.InstanceId = prop.Value.GetString(); break;
                case "method": msg.Method = prop.Value.GetString(); break;
                case "args": msg.Args = JsonSerializer.Deserialize<List<ScriptValue>>(prop.Value.GetRawText(), options); break;
                case "result": msg.Result = JsonSerializer.Deserialize<ScriptValue>(prop.Value.GetRawText(), options); break;
                case "message": msg.Message = prop.Value.GetString(); break;
                case "name": msg.Name = prop.Value.GetString(); break;
                case "value": msg.Value = JsonSerializer.Deserialize<ScriptValue>(prop.Value.GetRawText(), options); break;
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
        WriteProp(writer, "message", value.Message);
        WriteProp(writer, "name", value.Name);

        if (value.Types != null)
        {
            writer.WritePropertyName("types");
            JsonSerializer.Serialize(writer, value.Types, options);
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
        }
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
    /// Loaded assemblies: assembly_id → Assembly
    private readonly Dictionary<string, Assembly> _assemblies = new();
    /// Runtime instances: instance_id → ScriptInstance
    private readonly Dictionary<string, ScriptInstance> _instances = new();

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
            }
            catch (Exception ex)
            {
                RespondError(ex.Message);
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
            "Shutdown" => HandleShutdown(),
            _ => MakeError($"Unknown message type: {msg.Type}")
        };
    }

    ScriptMessage HandleLoadAssembly(ScriptMessage msg)
    {
        var id = msg.Id ?? "unknown";
        var data = msg.DataBase64 ?? "";

        try
        {
            var bytes = Convert.FromBase64String(data);
            var assembly = Assembly.Load(bytes);
            _assemblies[id] = assembly;

            // Discover all public types in the assembly
            var types = assembly.GetExportedTypes()
                .Select(t => t.FullName ?? t.Name)
                .ToList();

            Console.Error.WriteLine($"[ScriptHost] LoadAssembly: {id} ({types.Count} types)");

            return new ScriptMessage
            {
                Type = "AssemblyLoaded",
                Id = id,
                Types = types
            };
        }
        catch (Exception ex)
        {
            return MakeError($"Failed to load assembly '{id}': {ex.Message}");
        }
    }

    ScriptMessage HandleInstantiate(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? Guid.NewGuid().ToString();
        var assemblyId = msg.AssemblyId ?? "";
        var className = msg.ClassName ?? "";

        if (!_assemblies.TryGetValue(assemblyId, out var assembly))
            return MakeError($"Assembly not found: {assemblyId}");

        var type = assembly.GetType(className);
        if (type == null)
        {
            // Try searching all types in the assembly
            type = assembly.GetExportedTypes()
                .FirstOrDefault(t => t.FullName == className || t.Name == className);
            if (type == null)
                return MakeError($"Type '{className}' not found in assembly '{assemblyId}'");
        }

        try
        {
            var instance = Activator.CreateInstance(type);
            if (instance == null)
                return MakeError($"Failed to create instance of '{className}'");

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
            return MakeError($"Failed to instantiate '{className}': {ex.Message}");
        }
    }

    ScriptMessage HandleCallMethod(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var method = msg.Method ?? "";
        var args = msg.Args ?? new List<ScriptValue>();

        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError($"Instance not found: {instanceId}");

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
            return MakeError($"Method '{method}' failed: {ex.Message}");
        }
    }

    ScriptMessage HandleSetField(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var name = msg.Name ?? "";
        var value = msg.Value ?? ScriptValue.Null();

        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError($"Instance not found: {instanceId}");

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
            return MakeError($"SetField '{name}' failed: {ex.Message}");
        }
    }

    ScriptMessage HandleGetField(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var name = msg.Name ?? "";

        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError($"Instance not found: {instanceId}");

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
            return MakeError($"GetField '{name}' failed: {ex.Message}");
        }
    }

    ScriptMessage HandleShutdown()
    {
        Console.Error.WriteLine("[ScriptHost] Shutting down");
        return new ScriptMessage { Type = "Shutdown" };
    }

    static ScriptMessage MakeError(string message)
    {
        return new ScriptMessage
        {
            Type = "Error",
            Message = message
        };
    }

    void Respond(ScriptMessage response)
    {
        var json = JsonSerializer.Serialize(response, new JsonSerializerOptions
        {
            WriteIndented = false,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull
        });
        Console.WriteLine(json);
        Console.Out.Flush();
    }

    void RespondError(string message)
    {
        Respond(new ScriptMessage { Type = "Error", Message = message });
    }
}

/// <summary>
/// Entry point.
/// </summary>
class Program
{
    static void Main()
    {
        var host = new ScriptProtocolHost();
        host.Run();
    }
}
