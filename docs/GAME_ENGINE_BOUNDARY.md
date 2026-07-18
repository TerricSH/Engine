# Game script and engine boundary

The architectural rule is:

> The engine provides reusable capabilities. Game scripts and project data
> compose those capabilities into a particular game.

Changing ordinary gameplay must not require an edit under `crates/`. When it
does, treat that as a missing reusable Script API capability first. An engine
change is appropriate only when the requirement needs platform access,
renderer/physics/audio/navigation implementation, ECS or serialization rules,
resource cooking, editor integration, or another reusable runtime primitive.

## Ownership

Game-owned inputs:

- `game.project.json` and project configuration;
- scene and prefab data;
- source assets and input maps;
- C# gameplay assemblies, rules, state machines, AI decisions, quests, combat,
  UI flow, and game-specific identifiers;
- balancing values and content-specific data.

Engine-owned implementation:

- crates, native component storage, entity lifetime, scheduling, and scene I/O;
- renderer, physics, audio, animation, navigation, platform, and asset cooker;
- process-host protocol, validation, Script API implementation, and editor;
- generic capabilities exposed to scripts as commands, events, snapshots, or
  versioned data types.

Engine production code must never depend on a particular game, level, entity,
asset, or example project. Game scripts must never receive raw ECS entities,
native pointers, renderer handles, or backend objects.

## Decision rule

Use this order for every feature request:

1. If existing Script API calls and project data can express it, implement it
   entirely in the game project.
2. If it is game-specific state, keep it in C# fields/data even when no native
   component exists yet.
3. If a reusable native primitive is missing, implement that primitive once in
   the engine, expose a narrow validated Script API command/event, then keep the
   actual game rule in C#.
4. Do not hard-code a game workaround in an engine system.

Examples:

| Requirement | Owner |
| --- | --- |
| Jump timing, damage, inventory, quests | Game script |
| Which input triggers jump | Project data/script |
| Character-controller movement primitive | Engine + Script API |
| When an enemy chooses to chase | Game script |
| Navigation path calculation | Engine + Script API |
| Bullet damage and lifetime | Game script |
| Collision generation and ray-cast primitive | Engine + Script API |
| Concrete scene/entity/asset IDs | Game project |

## Versioned Script API contract

`engine-script-api` is the data-only dependency boundary shared by project
tooling and the runtime. It owns the current schema and version identifiers and
must not depend on ECS, renderer, editor, platform, or a game project.

Projects created with `--with-csharp` keep game-authored code separate from the
engine-owned SDK integration:

```text
scripts/GameScripts/
  EngineGameplay.targets            # generated reference to EngineGameplay.dll
  EngineGameplay.contract.json      # schema, SDK version, owner, SHA-256
  GameScripts.csproj                # game-owned build definition
  PlayerController.cs              # explicitly created game-owned behaviour
build/script-sdk-source/
  EngineGameplay.cs                 # generated SDK implementation
  EngineGameplay.csproj             # generated SDK build definition
build/script-sdk/
  EngineGameplay.dll                # independently compiled engine SDK
build/scripts/
  EngineGameplay.dll                # runtime dependency copied with the game
  GameScripts.dll                   # game-authored assembly
```

`GameScripts.dll` references `EngineGameplay.dll`; it no longer compiles the
engine API source into the game assembly. The process host loads the SDK into a
shared managed load context before loading the game assembly.

Each frame context carries the Script API schema. The SDK checks it before
invoking game code. Script builds validate the generated MSBuild integration
and sidecar, compile the SDK and game assembly transactionally, then copy the
SDK runtime dependency beside the game DLL. After an engine upgrade, refresh
the integration explicitly:

```powershell
sandbox project sync-script-api <project>
sandbox project build-scripts <project>
```

The build report records the Script API schema, concrete version, source
SHA-256, and SDK assembly path so packaged diagnostics can identify the exact
boundary used by a game assembly. `sync-script-api` also removes the legacy
`scripts/GameScripts/EngineGameplay.cs` layout to prevent game projects from
silently compiling engine implementation into their own assembly.

## Review gate

A change under `crates/` that was requested by gameplay must answer all of the
following:

1. What reusable engine capability is missing?
2. Why can the rule not remain in C# or project data?
3. What is the smallest command/event/snapshot surface needed by scripts?
4. How are untrusted script inputs validated?
5. Is the capability free of concrete game IDs and content assumptions?
6. Is there a script-side example and an automated boundary test?

`architecture_boundary.rs` additionally guards dependency direction: engine
crates cannot depend on the `sandbox` application, production crate sources
cannot reference the repository example game, and `engine-script-api` must
remain data-only.

## Current gaps

The current boundary does not yet expose arbitrary component access, full
physics queries/movement, or every audio, animation,
and navigation operation. A game needing one of these should add a generic
engine capability and Script API binding; it should not put game-specific code
into the Rust implementation.
