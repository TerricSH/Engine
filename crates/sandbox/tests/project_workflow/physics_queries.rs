#[cfg(feature = "subsystem-scripting-csharp")]
const MANAGED_PHYSICS_PROBE_BEHAVIOUR: &str = r#"using Engine;

namespace GameScripts;

public sealed class PhysicsProbeBehaviour : EngineBehaviour
{
    private PhysicsQuery _hitQuery;
    private PhysicsQuery _missQuery;
    private PhysicsQuery _overlapQuery;
    private PhysicsQuery _sweepHitQuery;
    private PhysicsQuery _sweepMissQuery;
    private PhysicsQuery _selfExcludedQuery;
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
            if (Physics.TryGetRaycastHit(_hitQuery, out _))
                throw new InvalidOperationException("query resolved on its issuing frame");

            // Filter validation rejects degenerate masks and ids up front.
            try
            {
                Physics.Raycast(
                    new Vector3(0.0f, 5.0f, 0.0f),
                    new Vector3(0.0f, -1.0f, 0.0f),
                    10.0f,
                    new PhysicsQueryFilter { LayerMask = 0u });
                throw new InvalidOperationException("zero layer mask was not rejected");
            }
            catch (ArgumentException)
            {
            }
            try
            {
                Physics.SphereCast(
                    new Vector3(0.0f, 5.0f, 0.0f),
                    0.0f,
                    new Vector3(0.0f, -1.0f, 0.0f),
                    10.0f);
                throw new InvalidOperationException("zero sweep radius was not rejected");
            }
            catch (ArgumentException)
            {
            }

            _hitQuery = Physics.Raycast(
                new Vector3(0.0f, 5.0f, 0.0f),
                new Vector3(0.0f, -1.0f, 0.0f),
                10.0f);
            _missQuery = Physics.Raycast(
                new Vector3(0.0f, 5.0f, 0.0f),
                new Vector3(0.0f, 1.0f, 0.0f),
                10.0f);
            _overlapQuery = Physics.OverlapSphere(new Vector3(0.0f, 0.0f, 0.0f), 1.0f);
            _sweepHitQuery = Physics.SphereCast(
                new Vector3(0.0f, 5.0f, 0.0f),
                0.5f,
                new Vector3(0.0f, -1.0f, 0.0f),
                10.0f);
            _sweepMissQuery = Physics.SphereCast(
                new Vector3(0.0f, 5.0f, 0.0f),
                0.5f,
                new Vector3(0.0f, 1.0f, 0.0f),
                10.0f);
            // Self-exclusion turns the ground probe into a miss; a layer mask
            // matching the default collision group still hits.
            _selfExcludedQuery = Physics.Raycast(
                new Vector3(0.0f, 5.0f, 0.0f),
                new Vector3(0.0f, -1.0f, 0.0f),
                10.0f,
                new PhysicsQueryFilter { ExcludeEntityId = "cube-01" });
            return;
        }
        if (UpdateCount == 2)
        {
            if (!Physics.TryGetRaycastHit(_hitQuery, out var hit))
                throw new InvalidOperationException("raycast hit missing on the next frame");
            if (hit.EntityId != "cube-01" || hit.Entity?.Id != "cube-01")
                throw new InvalidOperationException("raycast hit the wrong entity");
            if (Math.Abs(hit.Distance - 4.5f) > 1e-3f)
                throw new InvalidOperationException($"unexpected raycast distance {hit.Distance}");
            if (Math.Abs(hit.Point.Y - 0.5f) > 1e-3f || Math.Abs(hit.Normal.Y - 1.0f) > 1e-3f)
                throw new InvalidOperationException("raycast hit geometry mismatch");
            if (Physics.TryGetRaycastHit(_missQuery, out _))
                throw new InvalidOperationException("miss raycast resolved as a hit");
            if (!Physics.TryGetOverlapResult(_overlapQuery, out var entityIds) ||
                !entityIds.Contains("cube-01"))
                throw new InvalidOperationException("overlap sphere missed cube-01");

            // Sphere casts deliver the same hit payload as raycasts.
            if (!Physics.TryGetSphereCastHit(_sweepHitQuery, out var sweepHit))
                throw new InvalidOperationException("sphere cast hit missing on the next frame");
            if (sweepHit.EntityId != "cube-01")
                throw new InvalidOperationException("sphere cast hit the wrong entity");
            // The sphere surface touches the top face after 4.0 units of
            // travel; the contact point and normal match the raycast's.
            if (Math.Abs(sweepHit.Distance - 4.0f) > 1e-3f)
                throw new InvalidOperationException(
                    $"unexpected sphere cast distance {sweepHit.Distance}");
            if (Math.Abs(sweepHit.Point.Y - 0.5f) > 5e-3f ||
                Math.Abs(sweepHit.Normal.Y - 1.0f) > 1e-3f)
                throw new InvalidOperationException("sphere cast hit geometry mismatch");
            if (Physics.TryGetSphereCastHit(_sweepMissQuery, out _))
                throw new InvalidOperationException("miss sphere cast resolved as a hit");

            // The self-excluded ray found nothing to hit.
            if (Physics.TryGetRaycastHit(_selfExcludedQuery, out _))
                throw new InvalidOperationException("self-excluded raycast resolved as a hit");

            // One call drains the whole batch, ordered by query id, and
            // consumes the drained results.
            var drained = Physics.DrainAll();
            if (drained.Count != 6)
                throw new InvalidOperationException($"expected 6 drained results, got {drained.Count}");
            for (var index = 0; index < drained.Count; index += 1)
            {
                if (drained[index].Query.Id != (uint)(index + 1))
                    throw new InvalidOperationException("drained results are not ordered by query id");
            }
            if (drained[0].Kind != PhysicsQueryResultKind.RaycastHit || drained[0].Hit is null)
                throw new InvalidOperationException("drained raycast hit payload mismatch");
            if (drained[1].Kind != PhysicsQueryResultKind.RaycastMiss)
                throw new InvalidOperationException("drained raycast miss kind mismatch");
            if (drained[2].Kind != PhysicsQueryResultKind.OverlapSphere ||
                drained[2].EntityIds is null || !drained[2].EntityIds.Contains("cube-01"))
                throw new InvalidOperationException("drained overlap payload mismatch");
            if (drained[3].Kind != PhysicsQueryResultKind.SphereCastHit || drained[3].Hit is null)
                throw new InvalidOperationException("drained sphere cast hit payload mismatch");
            if (drained[4].Kind != PhysicsQueryResultKind.SphereCastMiss)
                throw new InvalidOperationException("drained sphere cast miss kind mismatch");
            if (drained[5].Kind != PhysicsQueryResultKind.RaycastMiss)
                throw new InvalidOperationException("drained self-excluded miss kind mismatch");
            if (Physics.TryGetRaycastHit(_hitQuery, out _) || Physics.DrainAll().Count != 0)
                throw new InvalidOperationException("drained results must be consumed");
        }
        if (UpdateCount >= 3)
        {
            // Results are frame-local and expire after their delivery frame.
            if (Physics.TryGetRaycastHit(_hitQuery, out _) ||
                Physics.TryGetOverlapResult(_overlapQuery, out _))
                throw new InvalidOperationException("frame-local query results must expire");
        }
    }
}
"#;

// Exercise the deferred physics query pipeline end to end: generated C#
// Physics.Raycast/OverlapSphere -> gameplay command -> process host -> native
// Rapier query -> next frame's gameplay context -> managed result lookup.
#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_physics_queries_round_trip_through_the_process_host() {
    if !common::require_tool("dotnet") {
        return;
    }
    let root = unique_project_root();
    let output = run(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "Physics Probe Game",
        "--with-csharp",
    ]);
    assert_success(&output, "physics query project new");

    let source = root.join("scripts/GameScripts/PhysicsProbeBehaviour.cs");
    std::fs::write(&source, MANAGED_PHYSICS_PROBE_BEHAVIOUR)
        .expect("write physics probe behaviour");

    // Attach the probe to cube-01 with an explicit origin transform, a static
    // rigid body, and the default unit collider so the ray and overlap
    // results are deterministic.
    let scene_path = root.join("assets/scenes/main.scene.ron");
    let mut scene = Scene::load_from_file(&scene_path).expect("load physics probe scene");
    let entity = scene
        .entities
        .iter_mut()
        .find(|entity| entity.persistent_id == "cube-01")
        .expect("physics probe fixture entity");
    entity.components.insert(
        "engine.transform".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([
                (
                    "translation".into(),
                    engine_serialize::Value::Vec3([0.0; 3]),
                ),
                (
                    "rotation".into(),
                    engine_serialize::Value::Quat([0.0, 0.0, 0.0, 1.0]),
                ),
                ("scale".into(), engine_serialize::Value::Vec3([1.0; 3])),
            ]),
        },
    );
    entity.components.insert(
        "engine.physics.rigid_body".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::from([(
                "body_type".into(),
                engine_serialize::Value::Enum("Static".into()),
            )]),
        },
    );
    entity.components.insert(
        "engine.physics.collider".into(),
        engine_scene::ComponentRecord {
            schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
            enabled: true,
            fields: std::collections::BTreeMap::new(),
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
                    engine_serialize::Value::Str("GameScripts.PhysicsProbeBehaviour".into()),
                ),
            ]),
        },
    );
    scene
        .save_to_file(&scene_path)
        .expect("save physics probe scene");

    let report_path = root.join("csharp-physics-query-run.json");
    let output = run(&[
        "game",
        path_text(&root),
        "--headless",
        "--frames",
        "3",
        "--report",
        path_text(&report_path),
    ]);
    assert_success(&output, "managed physics query round trip");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).expect("read physics query report"))
            .expect("parse physics query report");
    assert_eq!(report["passed"], true);
    assert_eq!(report["script_errors"], 0);
    assert_eq!(report["script_update_count"], 3);

    let _ = std::fs::remove_dir_all(root);
}
