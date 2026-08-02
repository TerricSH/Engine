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
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Engine-owned gameplay SDK source"));
    assert!(SCRIPT_SDK_PROJECT.contains("<AssemblyName>EngineGameplay</AssemblyName>"));
    assert!(SCRIPT_SDK_PROJECT.contains(&format!(
        "<Version>{}</Version>",
        engine_script_api::GAMEPLAY_SCRIPT_API_VERSION
    )));
    assert!(SCRIPT_SDK_TARGETS.contains("<Reference Include=\"EngineGameplay\">"));
    assert!(SCRIPT_SDK_TARGETS.contains("<Compile Remove=\"EngineGameplay.cs\" />"));
    assert!(SCRIPT_SDK_TARGETS.contains("<Compile Remove=\"EngineRules.cs\" />"));
    assert!(SCRIPT_SDK_TARGETS.contains("<Compile Remove=\"EngineTactics.cs\" />"));
    assert!(SCRIPT_SDK_TARGETS.contains("<Compile Remove=\"EngineJrpg.cs\" />"));
    assert!(SCRIPT_SDK_TARGETS.contains("<Compile Remove=\"EngineRendering.cs\" />"));
    assert!(SCRIPT_SDK_TARGETS.contains("<Compile Remove=\"EngineRuntimeAssets.cs\" />"));
    assert!(SCRIPT_SDK_TARGETS.contains("<Compile Remove=\"EngineOnlineXr.cs\" />"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains(&format!(
        "public const string Schema = \"{}\"",
        engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA
    )));
    assert!(STARTER_SCRIPT_API_SOURCE.contains(&format!(
        "public const string Version = \"{}\"",
        engine_script_api::GAMEPLAY_SCRIPT_API_VERSION
    )));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Gameplay Script API mismatch"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public abstract partial class EngineBehaviour"));
    assert!(RUNTIME_ASSETS_SCRIPT_API_SOURCE.contains("public sealed class ScriptRuntimeAssets"));
    assert!(RUNTIME_ASSETS_SCRIPT_API_SOURCE.contains("public sealed class ScriptTerrain"));
    assert!(RUNTIME_ASSETS_SCRIPT_API_SOURCE.contains("public RuntimeAssetRequest RegisterMesh("));
    assert!(RUNTIME_ASSETS_SCRIPT_API_SOURCE.contains("public RuntimeAssetRequest ApplyBrush("));
    assert!(ONLINE_XR_SCRIPT_API_SOURCE.contains("public sealed class ScriptNetwork"));
    assert!(ONLINE_XR_SCRIPT_API_SOURCE.contains("public sealed class ScriptXR"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptInput Input"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptUI UI"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class UICanvas"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public class UIElement"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class UIPanel"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class UIText"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class UIButton"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public UICanvas CreateCanvas("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public enum UIScaleMode"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public UIScaleMode ScaleMode"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public UIPanel AddPanel"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public UIText AddText"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public UIButton AddButton"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptTransform Transform"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptScene Scene"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class Entity"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptCharacter"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptCharacter Character"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Move(Vector3 direction"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Jump()"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"character_control\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Character.DrainTo(commands)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptInteraction"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class InteractionTarget"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public PhysicsQuery Probe("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool TryGetTarget("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScriptInteraction Interaction"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptPhysics"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public IReadOnlyList<PhysicsEvent> Events"));
    assert!(
        STARTER_SCRIPT_API_SOURCE.contains("public readonly record struct PhysicsQuery(uint Id)")
    );
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class RaycastHit"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class PhysicsQueryFilter"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public uint? LayerMask { get; set; }"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool IncludeSensors { get; set; }"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public string? ExcludeEntityId { get; set; }"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public PhysicsQuery Raycast("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public PhysicsQuery SphereCast("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public PhysicsQuery OverlapSphere("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void ApplyForce(Entity entity,"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void ApplyImpulse(Entity entity,"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void ApplyTorque(Entity entity,"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void ApplyTorqueImpulse(Entity entity,"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public enum PhysicsJointType"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class PhysicsJointSettings"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void CreateJoint("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void UpdateJoint("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void RemoveJoint(string jointId)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Grab("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void ReleaseGrab(string jointId)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptDamage"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public IReadOnlyList<DamageEvent> Events"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"apply_damage\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"damage_events\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Damage.Replace(context.DamageEvents)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Damage.DrainTo(commands)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptRagdoll"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Activate("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Recover("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"set_ragdoll\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"ragdoll_events\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Ragdoll.Replace(context.RagdollEvents)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Ragdoll.DrainTo(commands)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Kind = \"create_joint\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Kind = \"remove_joint\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"joint_id\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool TryGetRaycastHit("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool TryGetSphereCastHit("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool TryGetOverlapResult("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public enum PhysicsQueryResultKind"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class PhysicsQueryResult"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public List<PhysicsQueryResult> DrainAll()"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Kind = \"sphere_cast\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"physics_query\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"physics_mutation\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"mutation\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"physics_query_results\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Physics.DrainTo(commands)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptComponents"));
    assert!(
        STARTER_SCRIPT_API_SOURCE.contains("public readonly record struct ComponentQuery(uint Id)")
    );
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public readonly record struct ComponentValue"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ComponentSnapshot"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public ComponentQuery Query("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Set("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void SetField("));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public bool TryGet(ComponentQuery query, out ComponentSnapshot snapshot)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool IsMissing(ComponentQuery query)"));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public ComponentQuery QueryComponent(string componentType)"));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public void SetComponentField(string componentType, string field"));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public ScriptComponents Components => Scene.Components;"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"component_query\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"set_component\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"component_query_results\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Components.Replace(context.ComponentQueryResults)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Components.DrainTo(commands)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptPointer"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public ScreenRay? WorldRay"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptCamera"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptAudio"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptAnimation"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void PlayClip("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void SetMorphWeights("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void SetFloat("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptSave"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void SaveJson("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"save_checkpoint\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public sealed class ScriptLogicAssets"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"query_logic_asset\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool WasPressed(string actionName)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool WasReleased(string actionName)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public IReadOnlyList<GameplayUiEvent> Events"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool WasClicked(string callbackId)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public string CanvasId"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public uint ElementId"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public string? CallbackId"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool? BoolValue"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public float? FloatValue"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool IsOn"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public bool IsChecked"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public float Value"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"ui_events\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"canvas_id\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"element_id\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"callback_id\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("UI.Replace(context.UiEvents)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("StringComparison.Ordinal"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("set_toggle_value"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("set_checkbox_value"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("set_slider_value"));
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
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Spawn(string prefabId)"));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public void Spawn(string prefabId, Vector3 translation)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Prefab instantiation is deferred"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"prefab_id\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public void Load(string sceneId)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"set_entity_transform\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"create_entity\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"destroy_entity\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"spawn_prefab\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"load_scene\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"create_canvas\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"add_element\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("Type = \"set_text\""));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("UI.DrainTo(commands)"));
    // World origin (ENG-01): scripts read the double-precision origin;
    // every script-visible position stays origin-relative.
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public readonly record struct ScriptWorldOrigin(double X, double Y, double Z)"));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public ScriptWorldOrigin WorldOrigin { get; private set; }"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("[JsonPropertyName(\"world_origin\")]"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("context.WorldOrigin is { Length: 3 }"));
    // ProcGen (ENG-10): a synchronous, bit-exact C# port of the native
    // engine-procgen primitives - no protocol churn, golden-vector parity.
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public static class ProcGen"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public const string Schema = \"PROCGEN-v1\""));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public static ulong DeriveSeed(ulong parent, string key)"));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public static float Noise2D(ulong seed, float x, float y)"));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public static float Noise3D(ulong seed, float x, float y, float z)"));
    assert!(STARTER_SCRIPT_API_SOURCE
        .contains("public static float Fbm2D(ulong seed, float x, float y, ProcGenFbmParams p)"));
    assert!(STARTER_SCRIPT_API_SOURCE.contains(
        "public static float Fbm3D(ulong seed, float x, float y, float z, ProcGenFbmParams p)"
    ));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public static float WarpedFbm2D("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public static float WarpedFbm3D("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("public readonly record struct ProcGenFbmParams("));
    assert!(STARTER_SCRIPT_API_SOURCE.contains(
        "public readonly record struct ProcGenWarpParams(float Amplitude, float Frequency)"
    ));
    assert!(STARTER_SCRIPT_API_SOURCE.contains("procgen/warp/2d/x"));
    assert!(TACTICS_SCRIPT_API_SOURCE.contains("public sealed class TacticalBoard"));
    assert!(TACTICS_SCRIPT_API_SOURCE.contains("public sealed class TacticalPathfinder"));
    assert!(TACTICS_SCRIPT_API_SOURCE.contains("public sealed class VisibilityMap"));
    assert!(TACTICS_SCRIPT_API_SOURCE.contains("public sealed class TurnDirector"));
    assert!(TACTICS_SCRIPT_API_SOURCE.contains("public sealed class CombatResolver"));
    assert!(TACTICS_SCRIPT_API_SOURCE.contains("public sealed class UtilityBrain"));
    assert!(TACTICS_SCRIPT_API_SOURCE.contains("public sealed class TacticalSession"));
    assert!(RULES_SCRIPT_API_SOURCE.contains("public class DeterministicRandom"));
    assert!(RULES_SCRIPT_API_SOURCE.contains("public sealed class WeightedTable<T>"));
    assert!(JRPG_SCRIPT_API_SOURCE.contains("public sealed class Party"));
    assert!(JRPG_SCRIPT_API_SOURCE.contains("public sealed class Inventory"));
    assert!(JRPG_SCRIPT_API_SOURCE.contains("public sealed class BattleSession"));
    assert!(JRPG_SCRIPT_API_SOURCE.contains("public sealed class QuestJournal"));
    assert!(JRPG_SCRIPT_API_SOURCE.contains("public sealed class DialogueRunner"));
    assert!(JRPG_SCRIPT_API_SOURCE.contains("public sealed class LocalizationCatalog"));
    assert!(JRPG_SCRIPT_API_SOURCE.contains("public sealed class SequenceRunner"));
    assert!(JRPG_SCRIPT_API_SOURCE.contains("public sealed class JrpgSession"));
    assert!(JRPG_SCRIPT_API_SOURCE.contains("public static class JrpgScriptTools"));
    assert!(RENDERING_SCRIPT_API_SOURCE.contains("public sealed class LodGroupSettings"));
    assert!(RENDERING_SCRIPT_API_SOURCE.contains("public sealed class HlodClusterSettings"));
    assert!(RENDERING_SCRIPT_API_SOURCE.contains("ApplyHlodCluster"));
    assert!(RENDERING_SCRIPT_API_SOURCE.contains("public sealed class ParticleEmitterSettings"));
    assert!(RENDERING_SCRIPT_API_SOURCE.contains("public UIColor StartColor"));
    assert!(RENDERING_SCRIPT_API_SOURCE.contains("public float TurbulenceStrength"));
    assert!(RENDERING_SCRIPT_API_SOURCE.contains("public enum ParticleSimulationMode"));
    assert!(RENDERING_SCRIPT_API_SOURCE.contains("[\"simulation_mode\"]"));
    assert!(RENDERING_SCRIPT_API_SOURCE.contains("public static class RenderingScriptTools"));
}
