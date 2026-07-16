//! Project-level C# script build and runtime integration.
//!
//! Source projects are authoring inputs. Runtime players consume the compiled
//! `script_assembly` declared by `game.project.json` and an engine-owned
//! JSON-line protocol host.

#[cfg(feature = "subsystem-scripting-csharp")]
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use engine_asset::project::GameProject;
use engine_core::EngineRuntime;
use engine_scene::Scene;
use engine_serialize::{DiagnosticSeverity, Value};
use serde::Serialize;

const SCRIPT_COMPONENT_TYPE: &str = "engine.script";
#[cfg(feature = "subsystem-scripting-csharp")]
const SCRIPT_HOST_NAME: &str = "project-dotnet";
const SCRIPT_HOST_SOURCE: &str = include_str!("../../../scripts/csharp/EngineSample/Program.cs");
const SCRIPT_HOST_PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
    <AssemblyName>EngineScriptHost</AssemblyName>
    <RootNamespace>EngineScriptHost</RootNamespace>
  </PropertyGroup>
</Project>
"#;

pub(crate) const STARTER_SCRIPT_PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
    <AssemblyName>GameScripts</AssemblyName>
    <RootNamespace>GameScripts</RootNamespace>
  </PropertyGroup>
</Project>
"#;

pub(crate) const STARTER_SCRIPT_API_SOURCE: &str = r#"using System.Text.Json;
using System.Text.Json.Serialization;

namespace Engine;

public readonly record struct Vector2(float X, float Y);
public readonly record struct Vector3(float X, float Y, float Z);
public readonly record struct Quaternion(float X, float Y, float Z, float W);

public sealed class ScriptTransform
{
    private readonly TransformState _state;
    private readonly Action _markDirty;

    internal ScriptTransform(TransformState state, Action markDirty)
    {
        _state = state;
        _markDirty = markDirty;
    }

    public Vector3 Translation
    {
        get => new(_state.Translation[0], _state.Translation[1], _state.Translation[2]);
        set
        {
            _state.Translation = new[] { value.X, value.Y, value.Z };
            _markDirty();
        }
    }

    public Quaternion Rotation
    {
        get => new(_state.Rotation[0], _state.Rotation[1], _state.Rotation[2], _state.Rotation[3]);
        set
        {
            _state.Rotation = new[] { value.X, value.Y, value.Z, value.W };
            _markDirty();
        }
    }

    public Vector3 Scale
    {
        get => new(_state.Scale[0], _state.Scale[1], _state.Scale[2]);
        set
        {
            _state.Scale = new[] { value.X, value.Y, value.Z };
            _markDirty();
        }
    }

    internal TransformState State => _state;
}

public sealed class ScriptInput
{
    private IReadOnlyDictionary<string, InputValueState> _actions =
        new Dictionary<string, InputValueState>();
    private IReadOnlySet<string> _pressed = new HashSet<string>(StringComparer.Ordinal);
    private IReadOnlySet<string> _released = new HashSet<string>(StringComparer.Ordinal);

    internal void Replace(
        IReadOnlyDictionary<string, InputValueState> actions,
        InputTransitionState transitions)
    {
        _actions = actions;
        _pressed = transitions.Pressed;
        _released = transitions.Released;
    }

    public bool GetBool(string actionName) =>
        Get(actionName, "Bool").Value.GetBoolean();

    public float GetFloat(string actionName) =>
        Get(actionName, "Float").Value.GetSingle();

    public Vector2 GetVector2(string actionName)
    {
        var value = Get(actionName, "Vec2").Value;
        if (value.ValueKind != JsonValueKind.Array || value.GetArrayLength() != 2)
            throw new InvalidOperationException(
                $"Input action '{actionName}' returned an invalid Vec2 payload");
        return new(value[0].GetSingle(), value[1].GetSingle());
    }

    // Edge-triggered input for gameplay actions. These return true for one
    // script update only, even if the button remains held for later frames.
    public bool WasPressed(string actionName)
    {
        EnsureKnownAction(actionName);
        return _pressed.Contains(actionName);
    }

    public bool WasReleased(string actionName)
    {
        EnsureKnownAction(actionName);
        return _released.Contains(actionName);
    }

    private InputValueState Get(string actionName, string expectedType)
    {
        if (!_actions.TryGetValue(actionName, out var action))
            throw new KeyNotFoundException(
                $"Input action '{actionName}' is not configured in the project input map");
        if (action.Type != expectedType)
            throw new InvalidOperationException(
                $"Input action '{actionName}' is '{action.Type}', expected '{expectedType}'");
        return action;
    }

    private void EnsureKnownAction(string actionName)
    {
        if (string.IsNullOrWhiteSpace(actionName))
            throw new ArgumentException("Input action name cannot be empty", nameof(actionName));
        // A dynamically removed action can still have one final release edge.
        if (!_actions.ContainsKey(actionName) &&
            !_pressed.Contains(actionName) && !_released.Contains(actionName))
            throw new KeyNotFoundException(
                $"Input action '{actionName}' is not configured in the project input map");
    }
}

// One click emitted by a runtime canvas during the current script update.
// The source canvas and element are retained even when no callback id was
// authored, so scripts may inspect every routed click through UI.Events.
public sealed class GameplayUiEvent
{
    internal GameplayUiEvent(GameplayUiEventState state)
    {
        CanvasId = state.CanvasId;
        ElementId = state.ElementId;
        CallbackId = state.CallbackId;
    }

    public string CanvasId { get; }
    public uint ElementId { get; }
    public string? CallbackId { get; }
}

public sealed class ScriptUI
{
    private IReadOnlyList<GameplayUiEvent> _events = Array.Empty<GameplayUiEvent>();

    // This is a frame-local click-event snapshot. Toggle, Checkbox, and Slider
    // values are not modified automatically by the gameplay bridge.
    public IReadOnlyList<GameplayUiEvent> Events => _events;

    // Returns true when at least one click in this update has the exact,
    // case-sensitive callback id. Events without a callback remain in Events.
    public bool WasClicked(string callbackId)
    {
        ArgumentNullException.ThrowIfNull(callbackId);
        return _events.Any(uiEvent => string.Equals(
            uiEvent.CallbackId,
            callbackId,
            StringComparison.Ordinal));
    }

    internal void Replace(IReadOnlyList<GameplayUiEventState> events) =>
        _events = events.Select(uiEvent => new GameplayUiEvent(uiEvent)).ToArray();
}

public sealed class PhysicsEvent
{
    private readonly ScriptScene _scene;

    internal PhysicsEvent(PhysicsEventState state, ScriptScene scene)
    {
        Kind = state.Kind;
        OtherEntityId = state.OtherEntityId;
        _scene = scene;
    }

    // collision_entered, collision_stayed, collision_exited,
    // trigger_entered, trigger_stayed, or trigger_exited.
    public string Kind { get; }
    public string OtherEntityId { get; }
    public Entity? Other => _scene.FindEntity(OtherEntityId);
}

public sealed class ScriptPhysics
{
    private readonly ScriptScene _scene;
    private IReadOnlyList<PhysicsEventState> _events = Array.Empty<PhysicsEventState>();

    internal ScriptPhysics(ScriptScene scene) => _scene = scene;

    public IReadOnlyList<PhysicsEvent> Events =>
        _events.Select(state => new PhysicsEvent(state, _scene)).ToArray();

    internal void Replace(IReadOnlyList<PhysicsEventState> events) => _events = events;
}

// A frame-local view of one persistent ECS entity. Entity ids are resolved by
// the engine at the command boundary; managed code never receives raw ECS
// handles that can become stale after a scene change.
public sealed class Entity
{
    private readonly ScriptScene _scene;
    private readonly ScriptTransform? _transform;

    internal Entity(string id, EntitySnapshotState snapshot, ScriptScene scene)
    {
        Id = id;
        _scene = scene;
        _transform = snapshot.Transform == null
            ? null
            : new ScriptTransform(
                snapshot.Transform,
                () => scene.QueueTransform(id, snapshot.Transform));
    }

    public string Id { get; }
    public bool HasTransform => _transform != null;
    public ScriptTransform Transform => _transform ?? throw new InvalidOperationException(
        $"Entity '{Id}' has no Transform component");

    // Destruction is deferred until all script callbacks for the frame have
    // completed. OnDestroy runs before the native entity is released.
    public void Destroy() => _scene.Destroy(Id);
}

public sealed class ScriptScene
{
    private IReadOnlyDictionary<string, EntitySnapshotState> _entities =
        new Dictionary<string, EntitySnapshotState>();
    private readonly Dictionary<string, TransformState> _pendingTransforms =
        new(StringComparer.Ordinal);
    private readonly List<GameplayCommandState> _pendingCommands = new();

    public IEnumerable<Entity> Entities =>
        _entities.Select(pair => new Entity(pair.Key, pair.Value, this));

    public bool Exists(string entityId)
    {
        ValidateEntityId(entityId, nameof(entityId));
        return _entities.ContainsKey(entityId);
    }

    public Entity? FindEntity(string entityId)
    {
        ValidateEntityId(entityId, nameof(entityId));
        return _entities.TryGetValue(entityId, out var snapshot)
            ? new Entity(entityId, snapshot, this)
            : null;
    }

    public Entity GetEntity(string entityId) => FindEntity(entityId)
        ?? throw new KeyNotFoundException($"Entity '{entityId}' does not exist in this frame");

    public void DestroySelf() => _pendingCommands.Add(GameplayCommandState.DestroySelf());

    public void Destroy(string entityId)
    {
        ValidateEntityId(entityId, nameof(entityId));
        _pendingCommands.Add(GameplayCommandState.DestroyEntity(entityId));
    }

    // Creation is deferred until every script callback for this frame has
    // completed. The entity becomes visible through Entities/FindEntity on
    // the next frame; no raw or provisional ECS handle is exposed here.
    public void CreateEntity(string entityId) =>
        CreateEntity(entityId, new Vector3(0.0f, 0.0f, 0.0f));

    public void CreateEntity(string entityId, Vector3 translation)
    {
        ValidateEntityId(entityId, nameof(entityId));
        _pendingCommands.Add(GameplayCommandState.CreateEntity(
            entityId,
            new TransformState
            {
                Translation = new[] { translation.X, translation.Y, translation.Z }
            }));
    }

    // Queue a scene change for the engine to perform after the current script
    // update. sceneId is a key from game.project.json's `scenes` object.
    public void Load(string sceneId)
    {
        if (!IsValidSceneId(sceneId))
            throw new ArgumentException(
                "Scene.Load(sceneId) requires a game.project.json `scenes` key containing " +
                "1 to 128 ASCII letters, digits, hyphens, underscores, or dots " +
                "(but not '.' or '..').",
                nameof(sceneId));
        _pendingCommands.Add(GameplayCommandState.LoadScene(sceneId));
    }

    internal void DrainTo(List<GameplayCommandState> commands)
    {
        foreach (var pair in _pendingTransforms)
            commands.Add(GameplayCommandState.SetEntityTransform(pair.Key, pair.Value));
        commands.AddRange(_pendingCommands);
        _pendingTransforms.Clear();
        _pendingCommands.Clear();
    }

    internal void Replace(
        string ownerId,
        TransformState? ownerTransform,
        IReadOnlyDictionary<string, EntitySnapshotState> entities)
    {
        // Compatibility with older runtimes that only sent the owner's
        // top-level Transform and omitted the `entities` map.
        if (!entities.ContainsKey(ownerId))
        {
            var compatible = new Dictionary<string, EntitySnapshotState>(entities);
            compatible[ownerId] = new EntitySnapshotState { Transform = ownerTransform };
            _entities = compatible;
        }
        else
        {
            _entities = entities;
        }
    }

    internal void QueueTransform(string entityId, TransformState transform) =>
        _pendingTransforms[entityId] = transform;

    private static bool IsValidSceneId(string? sceneId)
    {
        if (string.IsNullOrEmpty(sceneId) || sceneId.Length > 128 ||
            sceneId == "." || sceneId == "..")
            return false;
        foreach (var character in sceneId)
        {
            if (!char.IsAsciiLetterOrDigit(character) &&
                character != '_' && character != '-' && character != '.')
                return false;
        }
        return true;
    }

    private static void ValidateEntityId(string? entityId, string parameterName)
    {
        var valid = !string.IsNullOrEmpty(entityId) && entityId.Length <= 128 &&
            entityId != "." && entityId != "..";
        if (valid)
        {
            foreach (var character in entityId!)
            {
                if (!char.IsAsciiLetterOrDigit(character) &&
                    character != '_' && character != '-' && character != '.')
                {
                    valid = false;
                    break;
                }
            }
        }
        if (!valid)
            throw new ArgumentException(
                "Entity ids must contain 1 to 128 ASCII letters, digits, hyphens, " +
                "underscores, or dots (but not '.' or '..'); entity ids are not file paths.",
                parameterName);
    }
}

public abstract class EngineBehaviour
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNameCaseInsensitive = false
    };

    private ScriptTransform? _transform;
    private bool _transformDirty;

    protected EngineBehaviour()
    {
        Physics = new ScriptPhysics(Scene);
    }

    public string EntityId { get; private set; } = "";
    public ScriptInput Input { get; } = new();
    public ScriptUI UI { get; } = new();
    public ScriptScene Scene { get; } = new();
    public ScriptPhysics Physics { get; }
    public ScriptTransform Transform => _transform ?? throw new InvalidOperationException(
        $"Script entity '{EntityId}' has no Transform component");

    // Reserved protocol hook. Game code should use EntityId, Input and
    // Transform instead of calling this method directly.
    public void __EngineSetGameplayContext(string contextJson)
    {
        var context = JsonSerializer.Deserialize<GameplayContextState>(contextJson, JsonOptions)
            ?? throw new InvalidOperationException("gameplay context JSON was empty");
        EntityId = context.EntityId;
        Input.Replace(context.InputActions, context.InputTransitions);
        UI.Replace(context.UiEvents);
        Physics.Replace(context.PhysicsEvents);
        _transform = context.Transform == null
            ? null
            : new ScriptTransform(context.Transform, () => _transformDirty = true);
        Scene.Replace(EntityId, context.Transform, context.Entities);
        _transformDirty = false;
    }

    // Reserved protocol hook. Commands never contain an entity id; the Rust
    // manager binds them to this script instance's owning entity.
    public string __EngineDrainGameplayCommands()
    {
        var commands = new List<GameplayCommandState>();
        if (_transformDirty && _transform != null)
            commands.Add(GameplayCommandState.SetTransform(_transform.State));
        Scene.DrainTo(commands);
        var json = JsonSerializer.Serialize(commands, JsonOptions);
        _transformDirty = false;
        return json;
    }
}

internal sealed class GameplayContextState
{
    [JsonPropertyName("entity_id")]
    public string EntityId { get; set; } = "";

    [JsonPropertyName("transform")]
    public TransformState? Transform { get; set; }

    [JsonPropertyName("input_actions")]
    public Dictionary<string, InputValueState> InputActions { get; set; } = new();

    [JsonPropertyName("input_transitions")]
    public InputTransitionState InputTransitions { get; set; } = new();

    [JsonPropertyName("physics_events")]
    public List<PhysicsEventState> PhysicsEvents { get; set; } = new();

    [JsonPropertyName("ui_events")]
    public List<GameplayUiEventState> UiEvents { get; set; } = new();

    [JsonPropertyName("entities")]
    public Dictionary<string, EntitySnapshotState> Entities { get; set; } = new();
}

internal sealed class InputTransitionState
{
    [JsonPropertyName("pressed")]
    public HashSet<string> Pressed { get; set; } = new(StringComparer.Ordinal);

    [JsonPropertyName("released")]
    public HashSet<string> Released { get; set; } = new(StringComparer.Ordinal);
}

internal sealed class EntitySnapshotState
{
    [JsonPropertyName("transform")]
    public TransformState? Transform { get; set; }
}

internal sealed class PhysicsEventState
{
    [JsonPropertyName("kind")]
    public string Kind { get; set; } = "";

    [JsonPropertyName("other_entity_id")]
    public string OtherEntityId { get; set; } = "";
}

internal sealed class GameplayUiEventState
{
    [JsonPropertyName("canvas_id")]
    public string CanvasId { get; set; } = "";

    [JsonPropertyName("element_id")]
    public uint ElementId { get; set; }

    [JsonPropertyName("callback_id")]
    public string? CallbackId { get; set; }
}

internal sealed class TransformState
{
    [JsonPropertyName("translation")]
    public float[] Translation { get; set; } = new float[3];

    [JsonPropertyName("rotation")]
    public float[] Rotation { get; set; } = new[] { 0.0f, 0.0f, 0.0f, 1.0f };

    [JsonPropertyName("scale")]
    public float[] Scale { get; set; } = new[] { 1.0f, 1.0f, 1.0f };
}

internal sealed class InputValueState
{
    [JsonPropertyName("type")]
    public string Type { get; set; } = "";

    [JsonPropertyName("value")]
    public JsonElement Value { get; set; }
}

internal sealed class GameplayCommandState
{
    [JsonPropertyName("type")]
    public required string Type { get; init; }

    [JsonPropertyName("transform")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public TransformState? Transform { get; init; }

    [JsonPropertyName("entity_id")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? EntityId { get; init; }

    [JsonPropertyName("scene_id")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? SceneId { get; init; }

    public static GameplayCommandState SetTransform(TransformState transform) =>
        new() { Type = "set_transform", Transform = transform };

    public static GameplayCommandState SetEntityTransform(
        string entityId,
        TransformState transform) =>
        new()
        {
            Type = "set_entity_transform",
            EntityId = entityId,
            Transform = transform
        };

    public static GameplayCommandState CreateEntity(
        string entityId,
        TransformState transform) =>
        new()
        {
            Type = "create_entity",
            EntityId = entityId,
            Transform = transform
        };

    public static GameplayCommandState DestroySelf() =>
        new() { Type = "destroy_self" };

    public static GameplayCommandState DestroyEntity(string entityId) =>
        new() { Type = "destroy_entity", EntityId = entityId };

    public static GameplayCommandState LoadScene(string sceneId) =>
        new() { Type = "load_scene", SceneId = sceneId };
}
"#;

pub(crate) const STARTER_SCRIPT_SOURCE: &str = r#"using Engine;

namespace GameScripts;

public sealed class Main : EngineBehaviour
{
    public float Speed = 3.0f;
    public int UpdateCount = 0;
    public float ElapsedSeconds = 0.0f;
    public bool LastJump = false;
    public bool LastJumpPressed = false;
    public bool LastJumpReleased = false;
    public bool LastStartClicked = false;
    public int LastUiEventCount = 0;
    public string? LastUiCanvasId = null;
    public uint LastUiElementId = 0;
    public string? LastUiCallbackId = null;

    public void OnCreate()
    {
        UpdateCount = 0;
        ElapsedSeconds = 0.0f;
    }

    public void OnStart()
    {
    }

    public void OnUpdate(float deltaTime)
    {
        UpdateCount += 1;
        ElapsedSeconds += deltaTime;
        LastJump = Input.GetBool("jump");
        LastJumpPressed = Input.WasPressed("jump");
        LastJumpReleased = Input.WasReleased("jump");
        LastStartClicked = UI.WasClicked("start-game");
        LastUiEventCount = UI.Events.Count;
        if (UI.Events.Count > 0)
        {
            LastUiCanvasId = UI.Events[0].CanvasId;
            LastUiElementId = UI.Events[0].ElementId;
            LastUiCallbackId = UI.Events[0].CallbackId;
        }
        else
        {
            LastUiCanvasId = null;
            LastUiElementId = 0;
            LastUiCallbackId = null;
        }

        var translation = Transform.Translation;
        Transform.Translation = new Vector3(
            translation.X + Speed * deltaTime,
            translation.Y,
            translation.Z);
    }

    public void OnDestroy()
    {
    }
}
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScriptProjectInspection {
    pub assembly_id: Option<String>,
    pub component_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ScriptBuildReport {
    pub schema: &'static str,
    pub project: String,
    pub assembly_id: String,
    pub assembly: String,
    pub host: String,
    pub dependency_assemblies: usize,
    pub passed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedScriptRuntime {
    pub assemblies: usize,
}

/// Validate the source/runtime script pairing and every scene attachment.
pub(crate) fn inspect_project_scripts(
    project: &GameProject,
    scene: &Scene,
) -> Result<ScriptProjectInspection, String> {
    let configured = match (&project.script_project, &project.script_assembly) {
        (None, None) => None,
        (Some(_), Some(assembly)) => Some(assembly_id_from_path(assembly)?),
        (Some(_), None) => return Err(
            "script_project is configured but script_assembly is missing from game.project.json"
                .into(),
        ),
        (None, Some(_)) => {
            return Err("script_assembly is configured without an authoring script_project".into())
        }
    };

    let mut component_count = 0usize;
    for entity in &scene.entities {
        let Some(component) = entity.components.get(SCRIPT_COMPONENT_TYPE) else {
            continue;
        };
        component_count += 1;
        let assembly_id = match component.fields.get("assembly_id") {
            Some(Value::Str(value)) if !value.trim().is_empty() => value,
            _ => {
                return Err(format!(
                "entity '{}' has an engine.script component without a non-empty string assembly_id",
                entity.persistent_id
            ))
            }
        };
        let class_name = match component.fields.get("class_name") {
            Some(Value::Str(value)) if !value.trim().is_empty() => value,
            _ => {
                return Err(format!(
                "entity '{}' has an engine.script component without a non-empty string class_name",
                entity.persistent_id
            ))
            }
        };
        let Some(expected) = configured.as_deref() else {
            return Err(format!(
                "entity '{}' attaches script '{}', but the project has no script_project/script_assembly configuration",
                entity.persistent_id, class_name
            ));
        };
        if assembly_id != expected {
            return Err(format!(
                "entity '{}' references script assembly '{}'; expected '{}' from script_assembly",
                entity.persistent_id, assembly_id, expected
            ));
        }
    }

    Ok(ScriptProjectInspection {
        assembly_id: configured,
        component_count,
    })
}

/// Validate runtime scene attachments against the compiled DLL. Packaged
/// projects are allowed to omit the authoring-only `script_project`.
pub(crate) fn validate_runtime_script_references(
    project: &GameProject,
    scene: &Scene,
) -> Result<usize, String> {
    let expected = project
        .script_assembly
        .as_deref()
        .map(assembly_id_from_path)
        .transpose()?;
    let mut count = 0usize;
    for entity in &scene.entities {
        let Some(component) = entity.components.get(SCRIPT_COMPONENT_TYPE) else {
            continue;
        };
        count += 1;
        let assembly_id = match component.fields.get("assembly_id") {
            Some(Value::Str(value)) if !value.trim().is_empty() => value,
            _ => {
                return Err(format!(
                    "entity '{}' has an invalid engine.script assembly_id",
                    entity.persistent_id
                ))
            }
        };
        match component.fields.get("class_name") {
            Some(Value::Str(value)) if !value.trim().is_empty() => {}
            _ => {
                return Err(format!(
                    "entity '{}' has an invalid engine.script class_name",
                    entity.persistent_id
                ))
            }
        }
        let Some(expected) = expected.as_deref() else {
            return Err(format!(
                "entity '{}' contains engine.script but script_assembly is not configured",
                entity.persistent_id
            ));
        };
        if assembly_id != expected {
            return Err(format!(
                "entity '{}' references script assembly '{}'; expected '{}'",
                entity.persistent_id, assembly_id, expected
            ));
        }
    }
    Ok(count)
}

/// Build the project's game assembly and publish the engine script host.
///
/// Both outputs are produced in sibling `.next` directories and only replace
/// the last good outputs after the corresponding dotnet command succeeds.
pub(crate) fn build_project_scripts(
    project: &GameProject,
) -> Result<Option<ScriptBuildReport>, String> {
    let (script_project, script_assembly) = match (
        project.script_project.as_deref(),
        project.script_assembly.as_deref(),
    ) {
        (None, None) => return Ok(None),
        (Some(source), Some(assembly)) => (source, assembly),
        (Some(_), None) => return Err(
            "script_project is configured but script_assembly is missing from game.project.json"
                .into(),
        ),
        (None, Some(_)) => return Err("script build requires an authoring script_project".into()),
    };
    if !script_project.is_file() {
        return Err(format!(
            "C# script project is missing: {}",
            script_project.display()
        ));
    }

    let assembly_id = assembly_id_from_path(script_assembly)?;
    let output_dir = script_assembly.parent().ok_or_else(|| {
        format!(
            "script assembly has no output directory: {}",
            script_assembly.display()
        )
    })?;
    ensure_inside_project(&project.root, output_dir, "script_assembly output")?;
    let output_next = sibling_with_suffix(output_dir, ".next")?;
    reset_owned_directory(&project.root, &output_next)?;

    let game_output = Command::new("dotnet")
        .arg("build")
        .arg(script_project)
        .arg("--configuration")
        .arg("Release")
        .arg("--nologo")
        .arg("--output")
        .arg(&output_next)
        .current_dir(script_project.parent().unwrap_or(&project.root))
        .output()
        .map_err(|error| format!("could not launch dotnet build: {error}"))?;
    ensure_command_success("C# game script build", game_output)?;

    let expected_assembly = output_next.join(
        script_assembly
            .file_name()
            .ok_or_else(|| "script_assembly must name a DLL".to_string())?,
    );
    if !expected_assembly.is_file() {
        return Err(format!(
            "dotnet build succeeded but did not produce the declared script assembly {}; set <AssemblyName>{}</AssemblyName> in {}",
            expected_assembly.display(),
            assembly_id,
            script_project.display()
        ));
    }

    // The process host is engine-owned and only changes when the embedded
    // source changes. Reusing a current host is important on Windows: the
    // previous Play session may still have its executable open while a new
    // game assembly is being prepared transactionally.
    let host_source_dir = project.root.join("build/script-host-source");
    ensure_inside_project(&project.root, &host_source_dir, "script host source")?;
    let host_dir = project.root.join("build/script-host");
    let host_executable = host_dir.join(host_executable_name());
    let host_is_current = host_executable.is_file()
        && file_contents_equal(
            &host_source_dir.join("EngineScriptHost.csproj"),
            SCRIPT_HOST_PROJECT,
        )
        && file_contents_equal(&host_source_dir.join("Program.cs"), SCRIPT_HOST_SOURCE);

    if host_is_current {
        self_test_script_host(&host_executable, &host_dir)?;
    } else {
        std::fs::create_dir_all(&host_source_dir).map_err(|error| {
            format!(
                "could not create script host source directory {}: {error}",
                host_source_dir.display()
            )
        })?;
        write_file(
            &host_source_dir.join("EngineScriptHost.csproj"),
            SCRIPT_HOST_PROJECT,
        )?;
        write_file(&host_source_dir.join("Program.cs"), SCRIPT_HOST_SOURCE)?;

        let host_next = project.root.join("build/script-host.next");
        reset_owned_directory(&project.root, &host_next)?;
        let host_output = Command::new("dotnet")
            .arg("publish")
            .arg(host_source_dir.join("EngineScriptHost.csproj"))
            .arg("--configuration")
            .arg("Release")
            .arg("--nologo")
            .arg("--self-contained")
            .arg("false")
            .arg("--output")
            .arg(&host_next)
            .current_dir(&host_source_dir)
            .output()
            .map_err(|error| format!("could not launch dotnet publish: {error}"))?;
        ensure_command_success("C# script host publish", host_output)?;

        let next_host_executable = host_next.join(host_executable_name());
        if !next_host_executable.is_file() {
            return Err(format!(
                "dotnet publish succeeded but did not produce the script host {}",
                next_host_executable.display()
            ));
        }
        self_test_script_host(&next_host_executable, &host_next)?;
        replace_owned_directory(&project.root, &host_next, &host_dir)?;
    }

    replace_owned_directory(&project.root, &output_next, output_dir)?;

    let dependency_assemblies = managed_dependencies(output_dir, script_assembly)?.len();
    Ok(Some(ScriptBuildReport {
        schema: "ProjectScriptBuildReport-v0",
        project: project.manifest.name.clone(),
        assembly_id,
        assembly: report_path(script_assembly),
        host: report_path(&host_dir.join(host_executable_name())),
        dependency_assemblies,
        passed: true,
    }))
}

/// Register the process host and load dependencies plus the game assembly.
pub(crate) fn prepare_project_scripts(
    runtime: &mut EngineRuntime,
    project: &GameProject,
) -> Result<PreparedScriptRuntime, String> {
    let Some(script_assembly) = project.script_assembly.as_deref() else {
        return Ok(PreparedScriptRuntime::default());
    };

    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = (runtime, script_assembly);
        Err(
            "this project contains C# scripts; rebuild sandbox with the `subsystem-scripting-csharp` feature"
                .into(),
        )
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        let (candidate, prepared) = prepare_isolated_project_script_engine(
            project,
            script_assembly,
            &resolve_script_host(project)?,
        )?;
        runtime
            .replace_script_engine(candidate, SCRIPT_HOST_NAME)
            .map_err(|error| format!("could not activate prepared C# script runtime: {error}"))?;
        Ok(prepared)
    }
}

/// Rebuild authoring scripts and replace the active managed runtime only after
/// a fresh process host has loaded every dependency and the game assembly.
///
/// Build, host-launch, or assembly-load failures return before
/// [`EngineRuntime::replace_script_engine`] is called, leaving the last good
/// runtime usable. Projects without managed scripts are an intentional no-op.
#[allow(dead_code)] // Called by the optional editor integration.
pub(crate) fn rebuild_and_reload_project_scripts(
    runtime: &mut EngineRuntime,
    project: &GameProject,
) -> Result<PreparedScriptRuntime, String> {
    if project.script_project.is_none() && project.script_assembly.is_none() {
        return Ok(PreparedScriptRuntime::default());
    }

    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = runtime;
        Err("cannot reload C# scripts without the `subsystem-scripting-csharp` feature".into())
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        build_project_scripts(project)?;
        let script_assembly = project.script_assembly.as_deref().ok_or_else(|| {
            "script_project is configured but script_assembly is missing from game.project.json"
                .to_string()
        })?;
        let host_path = resolve_script_host(project)?;
        let (candidate, prepared) =
            prepare_isolated_project_script_engine(project, script_assembly, &host_path)?;
        runtime
            .replace_script_engine(candidate, SCRIPT_HOST_NAME)
            .map_err(|error| format!("could not activate rebuilt C# script runtime: {error}"))?;
        Ok(prepared)
    }
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn prepare_isolated_project_script_engine(
    project: &GameProject,
    script_assembly: &Path,
    host_path: &Path,
) -> Result<(engine_script::ScriptEngine, PreparedScriptRuntime), String> {
    use engine_script::{ProcessHost, ScriptEngine};

    if !script_assembly.is_file() {
        return Err(format!(
            "compiled script assembly is missing: {}; run `sandbox project build-scripts {}`",
            script_assembly.display(),
            project.root.display()
        ));
    }

    let mut host = ProcessHost::new(SCRIPT_HOST_NAME);
    host.launch(host_path)
        .map_err(|error| format!("could not start C# script host: {error}"))?;
    let mut candidate = ScriptEngine::new();
    candidate.register_host(Box::new(host));

    let mut loaded_ids = BTreeSet::new();
    let mut loaded = 0usize;
    for dependency in managed_dependencies(
        script_assembly.parent().unwrap_or(&project.root),
        script_assembly,
    )? {
        let id = assembly_id_from_path(&dependency)?;
        if !loaded_ids.insert(id.clone()) {
            return Err(format!("duplicate managed assembly id '{id}'"));
        }
        let bytes = std::fs::read(&dependency).map_err(|error| {
            format!(
                "could not read script dependency {}: {error}",
                dependency.display()
            )
        })?;
        candidate
            .load_script(&id, SCRIPT_HOST_NAME, &bytes)
            .map_err(|error| {
                format!(
                    "could not load script dependency {}: {error}",
                    dependency.display()
                )
            })?;
        loaded += 1;
    }

    let assembly_id = assembly_id_from_path(script_assembly)?;
    if !loaded_ids.insert(assembly_id.clone()) {
        return Err(format!("duplicate game script assembly id '{assembly_id}'"));
    }
    let bytes = std::fs::read(script_assembly).map_err(|error| {
        format!(
            "could not read game script assembly {}: {error}",
            script_assembly.display()
        )
    })?;
    candidate
        .load_script(&assembly_id, SCRIPT_HOST_NAME, &bytes)
        .map_err(|error| format!("could not load game script assembly: {error}"))?;
    loaded += 1;

    Ok((candidate, PreparedScriptRuntime { assemblies: loaded }))
}

pub(crate) fn script_runtime_counts(runtime: &EngineRuntime) -> (usize, usize, usize) {
    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        let assemblies = runtime
            .script_engine()
            .managers()
            .iter()
            .map(|manager| manager.assembly_count())
            .sum();
        let instances = runtime
            .script_engine()
            .managers()
            .iter()
            .map(|manager| manager.instance_count())
            .sum();
        let started = runtime
            .script_engine()
            .managers()
            .iter()
            .flat_map(|manager| manager.iter_instances())
            .filter(|(_, _, state)| state.started)
            .count();
        (assemblies, instances, started)
    }
    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = runtime;
        (0, 0, 0)
    }
}

/// Sum an integer field across attached scripts. This is primarily useful for
/// headless smoke reports (the starter template exposes `UpdateCount`).
pub(crate) fn script_int_field_sum(runtime: &EngineRuntime, field: &str) -> Option<i64> {
    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        let values = runtime
            .script_engine()
            .managers()
            .iter()
            .flat_map(|manager| manager.iter_instances())
            .filter_map(|(_, _, state)| match state.instance.get_field(field) {
                Some(engine_script::ScriptValue::Int(value)) => Some(value),
                _ => None,
            })
            .collect::<Vec<_>>();
        if values.is_empty() {
            None
        } else {
            Some(values.into_iter().sum())
        }
    }
    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = (runtime, field);
        None
    }
}

/// Snapshot the ECS translations of entities that own attached scripts.
pub(crate) fn script_entity_translations(
    runtime: &EngineRuntime,
) -> std::collections::BTreeMap<String, [f32; 3]> {
    #[cfg(feature = "subsystem-scripting-csharp")]
    {
        let scripted_entities = runtime
            .script_engine()
            .managers()
            .iter()
            .flat_map(|manager| manager.iter_instances().map(|(entity_id, _, _)| entity_id))
            .collect::<std::collections::BTreeSet<_>>();
        runtime
            .with_world(|world| {
                world
                    .query_all::<engine_scene::components::Transform>()
                    .filter_map(|(entity, transform)| {
                        let entity_id = world.persistent_id(entity)?;
                        scripted_entities
                            .contains(entity_id)
                            .then(|| (entity_id.to_owned(), transform.translation.to_array()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    #[cfg(not(feature = "subsystem-scripting-csharp"))]
    {
        let _ = runtime;
        std::collections::BTreeMap::new()
    }
}

pub(crate) fn fail_on_script_errors(runtime: &EngineRuntime, phase: &str) -> Result<(), String> {
    let errors = runtime
        .diagnostics_collector()
        .script_diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        })
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("C# script {phase} failed:\n{}", errors.join("\n")))
    }
}

fn assembly_id_from_path(path: &Path) -> Result<String, String> {
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| {
            format!(
                "managed assembly path has no valid file stem: {}",
                path.display()
            )
        })?;
    if id.chars().any(char::is_whitespace) {
        return Err(format!(
            "managed assembly id may not contain whitespace: {id:?}"
        ));
    }
    Ok(id.to_string())
}

fn managed_dependencies(directory: &Path, main_assembly: &Path) -> Result<Vec<PathBuf>, String> {
    let main_name = main_assembly.file_name();
    let mut dependencies = std::fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "could not enumerate managed output {}: {error}",
                directory.display()
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
                && path.file_name() != main_name
        })
        .collect::<Vec<_>>();
    dependencies.sort();
    Ok(dependencies)
}

#[cfg(feature = "subsystem-scripting-csharp")]
fn resolve_script_host(project: &GameProject) -> Result<PathBuf, String> {
    if let Some(override_path) = std::env::var_os("ENGINE_SCRIPT_HOST") {
        let path = PathBuf::from(override_path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "ENGINE_SCRIPT_HOST does not name a file: {}",
            path.display()
        ));
    }

    let project_host = project
        .root
        .join("build/script-host")
        .join(host_executable_name());
    if project_host.is_file() {
        return Ok(project_host);
    }
    let packaged_host = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf))
        .map(|directory| directory.join("script-host").join(host_executable_name()));
    if let Some(path) = packaged_host.filter(|path| path.is_file()) {
        return Ok(path);
    }
    Err(format!(
        "C# script host is missing; run `sandbox project build-scripts {}` or set ENGINE_SCRIPT_HOST",
        project.root.display()
    ))
}

fn host_executable_name() -> &'static str {
    if cfg!(windows) {
        "EngineScriptHost.exe"
    } else {
        "EngineScriptHost"
    }
}

fn file_contents_equal(path: &Path, expected: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|contents| contents == expected)
}

fn self_test_script_host(executable: &Path, working_directory: &Path) -> Result<(), String> {
    let output = Command::new(executable)
        .arg("--self-test")
        .current_dir(working_directory)
        .output()
        .map_err(|error| format!("could not launch C# script host self-test: {error}"))?;
    ensure_command_success("C# script host gameplay bridge self-test", output)
}

fn ensure_command_success(label: &str, output: std::process::Output) -> Result<(), String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{label} failed with {}:\n{}{}{}",
        output.status,
        stdout,
        if stdout.is_empty() || stderr.is_empty() {
            ""
        } else {
            "\n"
        },
        stderr
    ))
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("path has no final component: {}", path.display()))?;
    let mut next_name = OsString::from(name);
    next_name.push(suffix);
    Ok(path.with_file_name(next_name))
}

fn reset_owned_directory(project_root: &Path, directory: &Path) -> Result<(), String> {
    ensure_inside_project(project_root, directory, "generated directory")?;
    if directory.exists() {
        std::fs::remove_dir_all(directory)
            .map_err(|error| format!("could not clear {}: {error}", directory.display()))?;
    }
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))
}

fn replace_owned_directory(
    project_root: &Path,
    next: &Path,
    final_path: &Path,
) -> Result<(), String> {
    ensure_inside_project(project_root, next, "generated next directory")?;
    ensure_inside_project(project_root, final_path, "generated output directory")?;
    let backup = sibling_with_suffix(final_path, ".previous")?;
    if backup.exists() {
        std::fs::remove_dir_all(&backup)
            .map_err(|error| format!("could not clear {}: {error}", backup.display()))?;
    }
    if final_path.exists() {
        std::fs::rename(final_path, &backup).map_err(|error| {
            format!(
                "could not preserve previous generated output {}: {error}",
                final_path.display()
            )
        })?;
    }
    if let Err(error) = std::fs::rename(next, final_path) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, final_path);
        }
        return Err(format!(
            "could not activate generated output {}: {error}",
            final_path.display()
        ));
    }
    if backup.exists() {
        std::fs::remove_dir_all(&backup)
            .map_err(|error| format!("could not remove {}: {error}", backup.display()))?;
    }
    Ok(())
}

fn ensure_inside_project(root: &Path, path: &Path, field: &str) -> Result<(), String> {
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) && !path.is_absolute()
    {
        return Err(format!(
            "{field} contains unsafe path traversal: {}",
            path.display()
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    if !absolute.starts_with(root) || absolute == root {
        return Err(format!(
            "{field} must remain inside the project root: {}",
            path.display()
        ));
    }
    Ok(())
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    std::fs::write(path, content)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

fn report_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembly_id_is_derived_from_runtime_dll() {
        assert_eq!(
            assembly_id_from_path(Path::new("build/scripts/GameScripts.dll")).unwrap(),
            "GameScripts"
        );
        assert!(assembly_id_from_path(Path::new("build/scripts/bad name.dll")).is_err());
    }

    #[test]
    fn starter_project_and_source_have_stable_contract() {
        assert!(STARTER_SCRIPT_PROJECT.contains("<AssemblyName>GameScripts</AssemblyName>"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public abstract class EngineBehaviour"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptInput Input"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptUI UI"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptTransform Transform"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptScene Scene"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class Entity"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptPhysics"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public IReadOnlyList<PhysicsEvent> Events"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool WasPressed(string actionName)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool WasReleased(string actionName)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public IReadOnlyList<GameplayUiEvent> Events"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool WasClicked(string callbackId)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public string CanvasId"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public uint ElementId"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public string? CallbackId"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"ui_events\")]"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"canvas_id\")]"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"element_id\")]"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"callback_id\")]"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("UI.Replace(context.UiEvents)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("StringComparison.Ordinal"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("values are not modified automatically"));
        assert!(STARTER_SCRIPT_API_SOURCE
            .contains("Input.Replace(context.InputActions, context.InputTransitions)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool Exists(string entityId)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public Entity GetEntity(string entityId)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public void DestroySelf()"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Destroy(string entityId)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public void CreateEntity(string entityId)"));
        assert!(STARTER_SCRIPT_API_SOURCE
            .contains("public void CreateEntity(string entityId, Vector3 translation)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("Creation is deferred"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Load(string sceneId)"));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"set_entity_transform\""));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"create_entity\""));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"destroy_entity\""));
        assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"load_scene\""));
        assert!(STARTER_SCRIPT_SOURCE.contains("namespace GameScripts;"));
        assert!(STARTER_SCRIPT_SOURCE.contains("Main : EngineBehaviour"));
        assert!(STARTER_SCRIPT_SOURCE.contains("void OnUpdate(float deltaTime)"));
        assert!(STARTER_SCRIPT_SOURCE.contains("Input.GetBool(\"jump\")"));
        assert!(STARTER_SCRIPT_SOURCE.contains("UI.WasClicked(\"start-game\")"));
        assert!(STARTER_SCRIPT_SOURCE.contains("UI.Events[0].CanvasId"));
        assert!(STARTER_SCRIPT_SOURCE.contains("UI.Events[0].ElementId"));
        assert!(STARTER_SCRIPT_SOURCE.contains("UI.Events[0].CallbackId"));
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn script_refresh_is_a_noop_for_projects_without_managed_scripts() {
        let root = PathBuf::from("unused-no-script-project");
        let project = GameProject {
            manifest: engine_asset::project::ProjectManifest::new("No Scripts"),
            manifest_path: root.join("game.project.json"),
            startup_scene: root.join("assets/scenes/main.scene.ron"),
            asset_source: root.join("assets-src"),
            cooked_assets: root.join("build/cooked"),
            script_project: None,
            script_assembly: None,
            input_actions: None,
            root,
        };
        let mut runtime = EngineRuntime::new(engine_core::EngineConfig::default());

        let refreshed = rebuild_and_reload_project_scripts(&mut runtime, &project)
            .expect("script-free projects should not invoke dotnet or mutate the runtime");

        assert_eq!(refreshed, PreparedScriptRuntime::default());
        assert_eq!(runtime.script_engine().host_count(), 0);
    }

    #[cfg(feature = "subsystem-scripting-csharp")]
    #[test]
    fn repeated_script_refresh_replaces_once_and_failed_build_keeps_last_good_runtime() {
        use engine_asset::project::ProjectManifest;
        use engine_scene::ComponentRecord;
        use engine_serialize::{SchemaVersion, Value};

        let temporary = tempfile::tempdir().expect("temporary script project");
        let root = temporary.path();
        std::fs::create_dir_all(root.join("assets/source")).expect("asset source directory");
        std::fs::create_dir_all(root.join("assets/scenes")).expect("scene directory");
        std::fs::create_dir_all(root.join("scripts/GameScripts")).expect("script source directory");

        let mut manifest = ProjectManifest::new("Transactional Scripts");
        manifest.input_actions = None;
        manifest.script_project = Some(PathBuf::from("scripts/GameScripts/GameScripts.csproj"));
        manifest.script_assembly = Some(PathBuf::from("build/scripts/GameScripts.dll"));

        let mut scene = engine_scene::sample_scene();
        let script_entity = scene
            .entities
            .iter_mut()
            .find(|entity| entity.persistent_id == "cube-01")
            .expect("sample script entity");
        script_entity.components.insert(
            SCRIPT_COMPONENT_TYPE.to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: std::collections::BTreeMap::from([
                    ("assembly_id".into(), Value::Str("GameScripts".into())),
                    ("class_name".into(), Value::Str("GameScripts.Main".into())),
                ]),
            },
        );
        scene
            .save_to_file(&root.join(&manifest.startup_scene))
            .expect("starter scene");
        std::fs::write(
            root.join("scripts/GameScripts/GameScripts.csproj"),
            STARTER_SCRIPT_PROJECT,
        )
        .expect("script project");
        std::fs::write(
            root.join("scripts/GameScripts/EngineGameplay.cs"),
            STARTER_SCRIPT_API_SOURCE,
        )
        .expect("script API");
        let script_source = root.join("scripts/GameScripts/Main.cs");
        std::fs::write(&script_source, STARTER_SCRIPT_SOURCE).expect("script source");
        manifest.write_to_root(root).expect("project manifest");
        let project = GameProject::load(root).expect("load script project");

        let mut runtime = EngineRuntime::new(engine_core::EngineConfig::default());
        let first = rebuild_and_reload_project_scripts(&mut runtime, &project)
            .expect("initial isolated script runtime");
        assert!(first.assemblies >= 1);
        assert_eq!(runtime.script_engine().host_count(), 1);

        let changed_source = STARTER_SCRIPT_SOURCE
            .replace("public int UpdateCount = 0;", "public int UpdateCount = 7;")
            .replace(
                "        ElapsedSeconds = 0.0f;\n    }",
                "        ElapsedSeconds = 0.0f;\n        Scene.CreateEntity(\"managed-spawn\", new Vector3(4.0f, 5.0f, 6.0f));\n    }",
            )
            .replace(
                "UpdateCount += 1;",
                "if (!Scene.Exists(\"managed-spawn\"))\n            throw new InvalidOperationException(\"deferred entity was not visible on the next frame\");\n        UpdateCount += 1;",
            )
            .replace(
                "        var translation = Transform.Translation;\n        Transform.Translation = new Vector3(\n            translation.X + Speed * deltaTime,\n            translation.Y,\n            translation.Z);",
                "        // This fixture's sample owner intentionally has no Transform.",
            );
        assert_ne!(changed_source, STARTER_SCRIPT_SOURCE);
        std::fs::write(&script_source, changed_source).expect("changed script source");
        let second = rebuild_and_reload_project_scripts(&mut runtime, &project)
            .expect("second isolated script runtime");
        assert_eq!(second.assemblies, first.assemblies);
        assert_eq!(
            runtime.script_engine().host_count(),
            1,
            "reload must replace the old process host rather than register another one"
        );

        std::fs::write(&script_source, "this is not valid C#").expect("invalid script source");
        let error = rebuild_and_reload_project_scripts(&mut runtime, &project)
            .expect_err("invalid source must fail before runtime activation");
        assert!(error.contains("C# game script build failed"));
        assert_eq!(runtime.script_engine().host_count(), 1);
        assert_eq!(
            runtime.script_engine().managers()[0].assembly_count(),
            second.assemblies
        );

        runtime
            .load_scene(scene)
            .expect("last good process host should remain usable after failed refresh");
        fail_on_script_errors(&runtime, "post-refresh attachment")
            .expect("last good managed assembly should still instantiate");
        let input_actions = std::collections::BTreeMap::from([(
            "jump".into(),
            engine_script::GameplayInputValue::Bool(false),
        )]);
        runtime.tick_scripts_with_input(0.016, &input_actions);
        fail_on_script_errors(&runtime, "deferred entity next-frame snapshot")
            .expect("managed script must observe its deferred entity on the next frame");
        runtime
            .with_world(|world| {
                let entity = world
                    .entity_by_persistent_id("managed-spawn")
                    .expect("managed OnCreate must create a persistent entity");
                let transform = world
                    .get::<engine_scene::components::Transform>(entity)
                    .expect("managed-created entity must have Transform");
                assert_eq!(transform.translation.to_array(), [4.0, 5.0, 6.0]);
            })
            .expect("script lifecycle must keep an active World");
        drop(runtime);
    }
}
