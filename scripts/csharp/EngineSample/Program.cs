/// <summary>
/// Engine Script Protocol Host — sample implementation.
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

/// <summary>
/// Represents a script object instance with modifiable fields.
/// </summary>
class ScriptInstance
{
    public string InstanceId { get; }
    public string AssemblyId { get; }
    public string ClassName { get; }
    public Dictionary<string, ScriptValue> Fields { get; } = new();

    public ScriptInstance(string instanceId, string assemblyId, string className)
    {
        InstanceId = instanceId;
        AssemblyId = assemblyId;
        ClassName = className;
    }

    public ScriptValue CallMethod(string method, List<ScriptValue> args)
    {
        // TODO: In a real host, this would use reflection to invoke
        // the actual method on a compiled script type.
        Console.Error.WriteLine($"[ScriptHost] Call {ClassName}.{method}({args.Count} args)");
        return ScriptValue.Null();
    }
}

/// <summary>
/// Main protocol host — reads JSON lines from stdin, dispatches them to
/// the appropriate handler, and writes JSON response lines to stdout.
/// </summary>
class ScriptProtocolHost
{
    private readonly Dictionary<string, ScriptInstance> _instances = new();
    private readonly List<string> _loadedTypes = new();

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

        // In a real host we would load the assembly from the base64-decoded bytes.
        // For the sample, we just acknowledge the load.
        Console.Error.WriteLine($"[ScriptHost] LoadAssembly: {id} ({data.Length} chars)");

        // Simulate discovering types in the assembly
        _loadedTypes.Add($"{id}.ScriptType");

        return new ScriptMessage
        {
            Type = "AssemblyLoaded",
            Id = id,
            Types = new List<string>(_loadedTypes)
        };
    }

    ScriptMessage HandleInstantiate(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? Guid.NewGuid().ToString();
        var assemblyId = msg.AssemblyId ?? "unknown";
        var className = msg.ClassName ?? "Unknown";

        var instance = new ScriptInstance(instanceId, assemblyId, className);
        _instances[instanceId] = instance;

        Console.Error.WriteLine($"[ScriptHost] Instantiated {className} as {instanceId}");

        // Return a MethodResult (with null) to acknowledge creation
        return new ScriptMessage
        {
            Type = "MethodResult",
            InstanceId = instanceId,
            Result = ScriptValue.Null()
        };
    }

    ScriptMessage HandleCallMethod(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var method = msg.Method ?? "";
        var args = msg.Args ?? new List<ScriptValue>();

        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError($"Instance not found: {instanceId}");

        var result = instance.CallMethod(method, args);
        return new ScriptMessage
        {
            Type = "MethodResult",
            InstanceId = instanceId,
            Result = result
        };
    }

    ScriptMessage HandleSetField(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var name = msg.Name ?? "";
        var value = msg.Value ?? ScriptValue.Null();

        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError($"Instance not found: {instanceId}");

        instance.Fields[name] = value;

        return new ScriptMessage
        {
            Type = "FieldValue",
            InstanceId = instanceId,
            Name = name,
            Value = value
        };
    }

    ScriptMessage HandleGetField(ScriptMessage msg)
    {
        var instanceId = msg.InstanceId ?? "";
        var name = msg.Name ?? "";

        if (!_instances.TryGetValue(instanceId, out var instance))
            return MakeError($"Instance not found: {instanceId}");

        instance.Fields.TryGetValue(name, out var value);

        return new ScriptMessage
        {
            Type = "FieldValue",
            InstanceId = instanceId,
            Name = name,
            Value = value
        };
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
