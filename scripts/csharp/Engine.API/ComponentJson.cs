using System.Collections;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Engine;

/// <summary>
/// Converts between ordinary managed component DTOs and Rust's externally
/// tagged engine_serialize::Value JSON representation.
/// </summary>
internal static class ComponentJson
{
    private static readonly JsonSerializerOptions Options = new()
    {
        IncludeFields = true,
        PropertyNameCaseInsensitive = true,
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        Converters = { new JsonStringEnumConverter() },
    };

    internal static T Deserialize<T>(ReadOnlySpan<byte> utf8Json)
    {
        var tagged = JsonNode.Parse(utf8Json) as JsonObject
            ?? throw new JsonException("A component payload must be a JSON object");
        var plain = DecodeObject(tagged, typeof(T), "$", isComponentRoot: true);
        var value = plain.Deserialize<T>(Options);
        return value is null
            ? throw new JsonException($"Component {typeof(T).Name} deserialized to null")
            : value;
    }

    internal static byte[] Serialize<T>(T value)
    {
        ArgumentNullException.ThrowIfNull(value);
        var plain = JsonSerializer.SerializeToNode(value, Options) as JsonObject
            ?? throw new NotSupportedException(
                $"Component DTO {typeof(T).FullName} must serialize as a JSON object");
        var tagged = EncodeObject(plain, typeof(T), "$", isComponentRoot: true);
        return JsonSerializer.SerializeToUtf8Bytes(tagged, Options);
    }

    private static JsonObject DecodeObject(
        JsonObject source,
        Type targetType,
        string path,
        bool isComponentRoot)
    {
        var memberTypes = SerializableMemberTypes(targetType);
        var result = new JsonObject();
        foreach (var (name, node) in source)
        {
            var memberPath = $"{path}.{name}";
            result[name] = memberTypes.TryGetValue(name, out var memberType)
                ? DecodeValue(node, memberType, memberPath)
                : DecodeTaggedValue(node, memberPath);
        }

        // A component root is already the field map. Nested objects must have
        // arrived through an explicit Value::Map tag, handled by DecodeValue.
        _ = isComponentRoot;
        return result;
    }

    private static JsonNode DecodeValue(JsonNode? node, Type targetType, string path)
    {
        targetType = Nullable.GetUnderlyingType(targetType) ?? targetType;
        var (tag, payload) = ReadTag(node, path);

        if (targetType == typeof(bool))
            return RequireScalarTag(tag, payload, "Bool", path);
        if (IsSignedInteger(targetType))
            return RequireScalarTag(tag, payload, "Int", path);
        if (IsUnsignedInteger(targetType))
            return RequireScalarTag(tag, payload, "UInt", path);
        if (targetType == typeof(float))
            return RequireNumericTag(tag, payload, "Float32", "Float64", path);
        if (targetType == typeof(double))
            return RequireNumericTag(tag, payload, "Float64", "Float32", path);
        if (targetType == typeof(string) || targetType == typeof(char))
            return RequireScalarTag(tag, payload, "Str", path);
        if (targetType.IsEnum)
            return RequireScalarTag(tag, payload, "Enum", path);

        if (targetType == typeof(Vector3))
        {
            RequireTag(tag, "Vec3", path);
            var values = payload as JsonArray
                ?? throw new JsonException($"{path}: Vec3 payload must be an array");
            if (values.Count != 3)
                throw new JsonException($"{path}: Vec3 payload must contain three numbers");
            return new JsonObject
            {
                ["x"] = values[0]?.DeepClone(),
                ["y"] = values[1]?.DeepClone(),
                ["z"] = values[2]?.DeepClone(),
            };
        }

        if (TryGetDictionaryValueType(targetType, out var valueType))
        {
            RequireTag(tag, "Map", path);
            var map = payload as JsonObject
                ?? throw new JsonException($"{path}: Map payload must be an object");
            var result = new JsonObject();
            foreach (var (key, value) in map)
                result[key] = DecodeValue(value, valueType, $"{path}.{key}");
            return result;
        }

        if (TryGetEnumerableElementType(targetType, out var elementType))
        {
            RequireTag(tag, "List", path);
            var list = payload as JsonArray
                ?? throw new JsonException($"{path}: List payload must be an array");
            var result = new JsonArray();
            for (var index = 0; index < list.Count; index++)
                result.Add(DecodeValue(list[index], elementType, $"{path}[{index}]"));
            return result;
        }

        if (CanUseObjectMap(targetType))
        {
            RequireTag(tag, "Map", path);
            var map = payload as JsonObject
                ?? throw new JsonException($"{path}: Map payload must be an object");
            return DecodeObject(map, targetType, path, isComponentRoot: false);
        }

        throw UnsupportedType(targetType, path);
    }

    private static JsonObject EncodeObject(
        JsonObject source,
        Type sourceType,
        string path,
        bool isComponentRoot)
    {
        var memberTypes = SerializableMemberTypes(sourceType);
        var result = new JsonObject();
        foreach (var (name, node) in source)
        {
            if (!memberTypes.TryGetValue(name, out var memberType))
                throw new NotSupportedException(
                    $"{path}.{name}: cannot determine the declared CLR member type");
            result[name] = EncodeValue(node, memberType, $"{path}.{name}");
        }

        _ = isComponentRoot;
        return result;
    }

    private static JsonNode EncodeValue(JsonNode? node, Type sourceType, string path)
    {
        sourceType = Nullable.GetUnderlyingType(sourceType) ?? sourceType;
        if (node is null)
            throw new NotSupportedException($"{path}: engine component values cannot be null");

        if (sourceType == typeof(bool))
            return Wrap("Bool", node);
        if (IsSignedInteger(sourceType))
            return Wrap("Int", node);
        if (IsUnsignedInteger(sourceType))
            return Wrap("UInt", node);
        if (sourceType == typeof(float))
            return Wrap("Float32", node);
        if (sourceType == typeof(double))
            return Wrap("Float64", node);
        if (sourceType == typeof(string) || sourceType == typeof(char))
            return Wrap("Str", node);
        if (sourceType.IsEnum)
            return Wrap("Enum", node);

        if (sourceType == typeof(Vector3))
        {
            var vector = node as JsonObject
                ?? throw new NotSupportedException($"{path}: Vector3 must serialize as an object");
            return Wrap(
                "Vec3",
                new JsonArray(
                    RequiredNode(vector, "x", path),
                    RequiredNode(vector, "y", path),
                    RequiredNode(vector, "z", path)));
        }

        if (TryGetDictionaryValueType(sourceType, out var valueType))
        {
            var map = node as JsonObject
                ?? throw new NotSupportedException($"{path}: dictionary must serialize as an object");
            var encoded = new JsonObject();
            foreach (var (key, value) in map)
                encoded[key] = EncodeValue(value, valueType, $"{path}.{key}");
            return Wrap("Map", encoded);
        }

        if (TryGetEnumerableElementType(sourceType, out var elementType))
        {
            var list = node as JsonArray
                ?? throw new NotSupportedException($"{path}: list must serialize as an array");
            var encoded = new JsonArray();
            for (var index = 0; index < list.Count; index++)
                encoded.Add(EncodeValue(list[index], elementType, $"{path}[{index}]"));
            return Wrap("List", encoded);
        }

        if (CanUseObjectMap(sourceType))
        {
            var map = node as JsonObject
                ?? throw new NotSupportedException($"{path}: nested DTO must serialize as an object");
            return Wrap("Map", EncodeObject(map, sourceType, path, isComponentRoot: false));
        }

        throw UnsupportedType(sourceType, path);
    }

    private static JsonNode DecodeTaggedValue(JsonNode? node, string path)
    {
        var (tag, payload) = ReadTag(node, path);
        switch (tag)
        {
            case "Bool":
            case "Int":
            case "UInt":
            case "Float32":
            case "Float64":
            case "Str":
            case "Vec3":
            case "Quat":
            case "Color":
            case "Asset":
            case "Entity":
            case "Enum":
                return payload.DeepClone();
            case "List":
            {
                var list = payload as JsonArray
                    ?? throw new JsonException($"{path}: List payload must be an array");
                var result = new JsonArray();
                for (var index = 0; index < list.Count; index++)
                    result.Add(DecodeTaggedValue(list[index], $"{path}[{index}]"));
                return result;
            }
            case "Map":
            {
                var map = payload as JsonObject
                    ?? throw new JsonException($"{path}: Map payload must be an object");
                var result = new JsonObject();
                foreach (var (key, value) in map)
                    result[key] = DecodeTaggedValue(value, $"{path}.{key}");
                return result;
            }
            default:
                throw new JsonException($"{path}: unknown engine value tag '{tag}'");
        }
    }

    private static (string Tag, JsonNode Payload) ReadTag(JsonNode? node, string path)
    {
        var tagged = node as JsonObject
            ?? throw new JsonException($"{path}: expected an externally tagged engine value");
        if (tagged.Count != 1)
            throw new JsonException($"{path}: engine value must contain exactly one tag");
        var entry = tagged.First();
        return (
            entry.Key,
            entry.Value ?? throw new JsonException($"{path}: engine value payload cannot be null"));
    }

    private static JsonNode RequireScalarTag(
        string actual,
        JsonNode payload,
        string expected,
        string path)
    {
        RequireTag(actual, expected, path);
        return payload.DeepClone();
    }

    private static JsonNode RequireNumericTag(
        string actual,
        JsonNode payload,
        string preferred,
        string accepted,
        string path)
    {
        if (actual != preferred && actual != accepted)
            throw new JsonException(
                $"{path}: expected {preferred} or {accepted}, received {actual}");
        return payload.DeepClone();
    }

    private static void RequireTag(string actual, string expected, string path)
    {
        if (actual != expected)
            throw new JsonException($"{path}: expected {expected}, received {actual}");
    }

    private static JsonObject Wrap(string tag, JsonNode payload) => new()
    {
        [tag] = payload.DeepClone(),
    };

    private static JsonNode RequiredNode(JsonObject source, string name, string path) =>
        source[name]?.DeepClone()
        ?? throw new NotSupportedException($"{path}: Vector3 is missing '{name}'");

    private static Dictionary<string, Type> SerializableMemberTypes(Type type)
    {
        var result = new Dictionary<string, Type>(StringComparer.OrdinalIgnoreCase);
        foreach (var property in type.GetProperties(BindingFlags.Instance | BindingFlags.Public))
        {
            if (property.GetIndexParameters().Length != 0 || IsIgnored(property))
                continue;
            result[JsonName(property)] = property.PropertyType;
        }
        foreach (var field in type.GetFields(BindingFlags.Instance | BindingFlags.Public))
        {
            if (IsIgnored(field))
                continue;
            result[JsonName(field)] = field.FieldType;
        }
        return result;
    }

    private static string JsonName(MemberInfo member) =>
        member.GetCustomAttribute<JsonPropertyNameAttribute>()?.Name
        ?? Options.PropertyNamingPolicy?.ConvertName(member.Name)
        ?? member.Name;

    private static bool IsIgnored(MemberInfo member) =>
        member.GetCustomAttribute<JsonIgnoreAttribute>() is not null;

    private static bool IsSignedInteger(Type type) =>
        type == typeof(sbyte) || type == typeof(short) || type == typeof(int) || type == typeof(long);

    private static bool IsUnsignedInteger(Type type) =>
        type == typeof(byte) || type == typeof(ushort) || type == typeof(uint) || type == typeof(ulong);

    private static bool TryGetDictionaryValueType(Type type, out Type valueType)
    {
        var dictionary = type
            .GetInterfaces()
            .Append(type)
            .FirstOrDefault(candidate =>
                candidate.IsGenericType
                && (candidate.GetGenericTypeDefinition() == typeof(IDictionary<,>)
                    || candidate.GetGenericTypeDefinition() == typeof(IReadOnlyDictionary<,>))
                && candidate.GetGenericArguments()[0] == typeof(string));
        valueType = dictionary?.GetGenericArguments()[1] ?? typeof(void);
        return dictionary is not null;
    }

    private static bool TryGetEnumerableElementType(Type type, out Type elementType)
    {
        if (type == typeof(string) || TryGetDictionaryValueType(type, out _))
        {
            elementType = typeof(void);
            return false;
        }
        if (type.IsArray)
        {
            elementType = type.GetElementType()!;
            return true;
        }
        var enumerable = type
            .GetInterfaces()
            .Append(type)
            .FirstOrDefault(candidate =>
                candidate.IsGenericType
                && candidate.GetGenericTypeDefinition() == typeof(IEnumerable<>));
        elementType = enumerable?.GetGenericArguments()[0] ?? typeof(void);
        return enumerable is not null;
    }

    private static bool CanUseObjectMap(Type type) =>
        !type.IsPrimitive
        && !type.IsPointer
        && !type.IsInterface
        && type != typeof(decimal)
        && type != typeof(DateTime)
        && type != typeof(DateTimeOffset)
        && type != typeof(Guid)
        && type != typeof(object);

    private static NotSupportedException UnsupportedType(Type type, string path) => new(
        $"{path}: CLR type {type.FullName} has no unambiguous engine Value mapping. " +
        "Use an explicit Engine API wrapper for Vec3/Quat/Color/Asset/Entity values.");
}
