#[cfg(feature = "subsystem-scripting-csharp")]
const MANAGED_COMPONENT_PROBE_BEHAVIOUR: &str = r#"using Engine;

namespace GameScripts;

public sealed class ComponentProbeBehaviour : EngineBehaviour
{
    private ComponentQuery _audioQuery;
    private ComponentQuery _lightQuery;
    private ComponentQuery _cameraQuery;
    private ComponentQuery _renderableQuery;
    private ComponentQuery _missingQuery;
    private ComponentQuery _gravityQuery;
    private ComponentQuery _updatedAudioQuery;
    private ComponentQuery _updatedLightQuery;
    private ComponentQuery _updatedGravityQuery;
    private ComponentQuery _updatedCameraQuery;
    private ComponentQuery _updatedRenderableQuery;
    private NetworkRequest _networkRequest;
    public int UpdateCount = 0;

    public void OnCreate()
    {
    }

    public void OnStart()
    {
    }

    public void OnUpdate(float deltaTime)
    {
        UpdateCount += 1;
        if (UpdateCount == 1)
        {
            // Query handles never resolve on the frame that issued them.
            if (Components.TryGet(_audioQuery, out _))
                throw new InvalidOperationException("query resolved on its issuing frame");
            var cube = Scene.GetEntity("cube-01");
            _audioQuery = cube.QueryComponent("engine.audio_source");
            _lightQuery = Components.Query("light-directional", "engine.light");
            _cameraQuery = Components.Query("camera-main", "engine.camera");
            _renderableQuery = cube.QueryComponent("engine.renderable");
            _missingQuery = Components.Query("cube-01", "engine.light");
            _gravityQuery = Components.Query("planet-01", "engine.gravity_source");
            _networkRequest = Network.Host(0xC0FFEEUL);
            return;
        }
        if (UpdateCount == 2)
        {
            if (!Components.TryGet(_audioQuery, out var audio))
                throw new InvalidOperationException("audio snapshot missing on the next frame");
            if (audio.EntityId != "cube-01" || audio.ComponentType != "engine.audio_source")
                throw new InvalidOperationException("audio snapshot identity mismatch");
            if (Math.Abs(audio.GetFloat("volume") - 0.8f) > 1e-6f)
                throw new InvalidOperationException("unexpected audio volume");
            if (audio.GetBool("playing"))
                throw new InvalidOperationException("audio source should not be playing");
            if (!audio.HasField("max_distance") || audio.HasField("clip_asset"))
                throw new InvalidOperationException("audio snapshot field coverage mismatch");
            if (!Components.TryGet(_lightQuery, out var light))
                throw new InvalidOperationException("light snapshot missing on the next frame");
            if (Math.Abs(light.GetFloat("intensity") - 2.5f) > 1e-6f)
                throw new InvalidOperationException("unexpected light intensity");
            if (light.GetEnum("kind") != "Directional")
                throw new InvalidOperationException("unexpected light kind");
            var lightColor = light.GetVector3("color");
            if (Math.Abs(lightColor.X - 1.0f) > 1e-6f || Math.Abs(lightColor.Y - 0.96f) > 1e-3f)
                throw new InvalidOperationException("unexpected light color");
            if (!Components.TryGet(_cameraQuery, out var camera))
                throw new InvalidOperationException("camera snapshot missing on the next frame");
            if (Math.Abs(camera.GetFloat("near") - 0.1f) > 1e-6f)
                throw new InvalidOperationException("unexpected camera near plane");
            if (camera.GetEnum("projection") != "Perspective")
                throw new InvalidOperationException("unexpected camera projection");
            if (!Components.TryGet(_renderableQuery, out var renderable) ||
                renderable.GetAsset("mesh") != "mesh-cube")
                throw new InvalidOperationException("renderable snapshot missing or invalid");
            var clearColor = camera.GetColor("clear_color");
            if (Math.Abs(clearColor.B - 0.06f) > 1e-3f || Math.Abs(clearColor.A - 1.0f) > 1e-6f)
                throw new InvalidOperationException("unexpected camera clear color");
            if (Components.TryGet(_missingQuery, out _) || !Components.IsMissing(_missingQuery))
                throw new InvalidOperationException("absent component must report IsMissing");
            if (!Components.TryGet(_gravityQuery, out var gravity))
                throw new InvalidOperationException("gravity snapshot missing on the next frame");
            if (gravity.EntityId != "planet-01" || gravity.ComponentType != "engine.gravity_source")
                throw new InvalidOperationException("gravity snapshot identity mismatch");
            if (gravity.GetEnum("mode") != "Point")
                throw new InvalidOperationException("unexpected gravity mode");
            if (Math.Abs(gravity.GetFloat("strength") - 42.0f) > 1e-6f)
                throw new InvalidOperationException("unexpected gravity strength");
            var gravityCenter = gravity.GetVector3("center");
            if (Math.Abs(gravityCenter.Y - (-100.0f)) > 1e-3f)
                throw new InvalidOperationException("unexpected gravity center");
            if (gravity.GetEnum("falloff") != "InverseSquare")
                throw new InvalidOperationException("unexpected gravity falloff");
            if (Math.Abs(gravity.GetFloat("max_radius") - 500.0f) > 1e-3f)
                throw new InvalidOperationException("unexpected gravity max radius");
            if (!Network.TryGetResult(_networkRequest, out var networkResult))
                throw new InvalidOperationException("network command result missing");
            if (networkResult.Success && !Network.Active)
                throw new InvalidOperationException("network host succeeded without an active session");
            if (!networkResult.Success &&
                !(networkResult.Error?.Contains("subsystem-network", StringComparison.Ordinal) ?? false))
                throw new InvalidOperationException("network bridge failed without a feature diagnostic");
            if (XR.Active && XR.Frame is null)
                throw new InvalidOperationException("active XR bridge did not publish a stereo frame");

            // Merge writes: only the provided fields change on the target.
            var cube = Scene.GetEntity("cube-01");
            cube.SetComponentField("engine.audio_source", "volume", ComponentValue.FromFloat(0.25f));
            cube.SetComponentField("engine.audio_source", "playing", true);
            Scene.GetEntity("light-directional")
                .SetComponentField("engine.light", "intensity", 9.0f);
            Scene.GetEntity("planet-01")
                .SetComponentField("engine.gravity_source", "strength", 12.5f);
            Scene.GetEntity("camera-main")
                .SetComponentField("engine.camera", "msaa_samples", ComponentValue.FromUInt(1));
            RuntimeAssets.RegisterMaterial("material.live-paint", new RuntimeMaterialData
            {
                BaseColor = [0.1f, 0.7f, 0.9f, 1.0f],
                Metallic = 0.4f,
                Roughness = 0.25f
            });
            cube.SetComponentField("engine.renderable", "material", ComponentValue.FromAsset("material.live-paint"));
            _updatedAudioQuery = Components.Query("cube-01", "engine.audio_source");
            _updatedLightQuery = Components.Query("light-directional", "engine.light");
            _updatedGravityQuery = Components.Query("planet-01", "engine.gravity_source");
            _updatedCameraQuery = Components.Query("camera-main", "engine.camera");
            _updatedRenderableQuery = Components.Query("cube-01", "engine.renderable");
            return;
        }
        if (UpdateCount >= 3)
        {
            // Results are frame-local and expire after their delivery frame.
            if (Components.TryGet(_audioQuery, out _))
                throw new InvalidOperationException("frame-local query results must expire");
            if (!Components.TryGet(_updatedAudioQuery, out var audio))
                throw new InvalidOperationException("updated audio snapshot missing");
            if (Math.Abs(audio.GetFloat("volume") - 0.25f) > 1e-6f)
                throw new InvalidOperationException("audio volume write did not apply");
            if (!audio.GetBool("playing"))
                throw new InvalidOperationException("audio playing write did not apply");
            // Fields the write did not mention survive the merge.
            if (Math.Abs(audio.GetFloat("max_distance") - 15.0f) > 1e-6f)
                throw new InvalidOperationException("merge dropped unwritten fields");
            if (!Components.TryGet(_updatedLightQuery, out var light))
                throw new InvalidOperationException("updated light snapshot missing");
            if (Math.Abs(light.GetFloat("intensity") - 9.0f) > 1e-6f)
                throw new InvalidOperationException("light intensity write did not apply");
            if (!Components.TryGet(_updatedGravityQuery, out var gravity))
                throw new InvalidOperationException("updated gravity snapshot missing");
            if (Math.Abs(gravity.GetFloat("strength") - 12.5f) > 1e-6f)
                throw new InvalidOperationException("gravity strength write did not apply");
            // Fields the write did not mention survive the merge.
            if (Math.Abs(gravity.GetFloat("max_radius") - 500.0f) > 1e-3f)
                throw new InvalidOperationException("gravity write dropped unwritten fields");
            if (!Components.TryGet(_updatedCameraQuery, out var camera) ||
                camera.GetUInt("msaa_samples") != 1UL)
                throw new InvalidOperationException("uint component wire value did not round-trip");
            if (!Components.TryGet(_updatedRenderableQuery, out var renderable) ||
                renderable.GetAsset("material") != "material.live-paint" ||
                renderable.GetAsset("mesh") != "mesh-cube")
                throw new InvalidOperationException("live renderable material write did not preserve mesh");
        }
    }
}
"#;

// Exercise deferred typed component access end to end: generated C#
// Components.Query/SetComponent -> gameplay command -> process host -> native
// component snapshot/merge -> next frame's gameplay context -> managed reads.
#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_component_access_round_trips_through_the_process_host() {
    if !common::require_tool("dotnet") {
        return;
    }
    let root = unique_project_root();
    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Component Probe Game",
        "--with-csharp",
    ]);
    assert_success(&output, "component probe project new");

    let source = root.join("scripts/GameScripts/ComponentProbeBehaviour.cs");
    std::fs::write(&source, MANAGED_COMPONENT_PROBE_BEHAVIOUR)
        .expect("write component probe behaviour");

    // Attach the probe to cube-01 with an authored audio source so snapshot
    // reads, missing-component detection, and merge writes are deterministic.
    let scene_path = root.join("assets/scenes/main.scene.ron");
    let mut scene = Scene::load_from_file(&scene_path).expect("load component probe scene");
    let entity = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("component probe fixture entity");
    entity.components.insert(
        "engine.audio_source".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                ("volume".into(), engine_serialize::Value::Float32(0.8)),
                ("playing".into(), engine_serialize::Value::Bool(false)),
                (
                    "max_distance".into(),
                    engine_serialize::Value::Float32(15.0),
                ),
            ]),
        },
    );
    entity.components.insert(
        "engine.script".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                (
                    "assembly_id".into(),
                    engine_serialize::Value::Str("GameScripts".into()),
                ),
                (
                    "class_name".into(),
                    engine_serialize::Value::Str("GameScripts.ComponentProbeBehaviour".into()),
                ),
            ]),
        },
    );
    // A planet entity exercises engine.gravity_source through the same bridge.
    scene.entities.push(engine_scene::EntityRecord {
        persistent_id: "planet-01".into(),
        parent: None,
        name: Some("Planet".into()),
        enabled: true,
        components: std::collections::BTreeMap::from([
            (
                "engine.transform".into(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: std::collections::BTreeMap::from([
                        (
                            "translation".into(),
                            engine_serialize::Value::Vec3([0.0, -100.0, 0.0]),
                        ),
                        (
                            "rotation".into(),
                            engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
                        ),
                        ("scale".into(), engine_serialize::Value::Vec3([1.0; 3])),
                    ]),
                },
            ),
            (
                "engine.gravity_source".into(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: std::collections::BTreeMap::from([
                        ("mode".into(), engine_serialize::Value::Enum("Point".into())),
                        ("strength".into(), engine_serialize::Value::Float32(42.0)),
                        (
                            "center".into(),
                            engine_serialize::Value::Vec3([0.0, -100.0, 0.0]),
                        ),
                        (
                            "falloff".into(),
                            engine_serialize::Value::Enum("InverseSquare".into()),
                        ),
                        ("max_radius".into(), engine_serialize::Value::Float32(500.0)),
                    ]),
                },
            ),
        ]),
    });
    scene
        .save_to_file(&scene_path)
        .expect("save component probe scene");

    let report_path = root.join("csharp-component-run.json");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&report_path),
    ]);
    assert_success(&output, "managed component access round trip");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).expect("read component report"))
            .expect("parse component report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["script_errors"], 0);
    assert_eq!(report["script_update_count"], 3);

    // Unsupported component type keys fail closed with an actionable diagnostic.
    let unknown_source = MANAGED_COMPONENT_PROBE_BEHAVIOUR.replace(
        "_audioQuery = cube.QueryComponent(\"engine.audio_source\");",
        "_audioQuery = cube.QueryComponent(\"engine.nope\");",
    );
    assert_ne!(unknown_source, MANAGED_COMPONENT_PROBE_BEHAVIOUR);
    std::fs::write(&source, unknown_source).expect("write unknown component script");
    let output = run(&["project", "build-scripts", path_text(&root)]);
    assert_success(&output, "unknown component script build");
    let output = run(&["game", path_text(&root), "--headless", "--frames", "1"]);
    assert_failure(&output, "unknown component diagnostic");
    let messages = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        messages.contains("SCRIPT_COMPONENT_UNKNOWN") && messages.contains("engine.nope"),
        "unknown component type lost its diagnostic: {messages}"
    );

    let _ = std::fs::remove_dir_all(root);
}
