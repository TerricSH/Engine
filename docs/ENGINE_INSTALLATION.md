# Engine installation and external project workspaces

The supported product model separates the engine application from each game:

```text
C:\Engine\v0.1.0\windows-x86_64\    engine treats this as read-only
D:\Games\MyGame\                     writable game-project workspace
```

The editor executable, runtime, cooker, packaging script, managed SDK, and
script host belong to the engine installation. `game.project.json`, source
assets, scenes, game scripts, cooked data, compiled game assemblies, and
exported packages belong to the project workspace.

This separation means an installed editor can open a project outside the
engine repository and does not copy the project into its own directory.

## Installation layout

The current Windows distribution is a versioned, relocatable directory:

```text
windows-x86_64/
  engine.installation.json
  bin/
    EngineEditor.exe
  runtime/windows-x86_64/
    GameRuntime.exe
    GameRuntime.pdb
  tools/
    asset-cook.exe
    package-windows.ps1
  sdk/
    EngineGameplay.dll
    script-host/
      EngineScriptHost.exe
      ...
  THIRD_PARTY_NOTICES.txt
```

`engine.installation.json` uses the `EngineInstallation-v0` schema. It records
the engine version, relative locations of every required tool, the managed API
schema/version/hash, source provenance used for deterministic packages, and a
SHA-256 map of installation files.

The application normally discovers the manifest by walking upward from
`EngineEditor.exe`. `ENGINE_INSTALL_ROOT` may explicitly identify that
installation root, but the running process must resolve to the manifest's
declared editor or runtime executable. A foreign or invalid explicit root is
an error rather than a silent fallback.

Loading an installation validates:

- the manifest schema and required metadata;
- that declared paths are relative and remain under the installation root;
- required editor, runtime, symbols, cooker, package script, SDK, host, and
  notices files;
- every SHA-256 entry; and
- that each required file and each top-level host file is covered by the hash
  map.

These hashes detect an inconsistent or modified directory, but they are not a
publisher signature. Authenticode/provenance signing is not implemented yet.

## Open and work in an external project

Pass either the project directory or its manifest to the installed editor:

```powershell
$engine = "C:\Engine\v0.1.0\windows-x86_64"
$project = "D:\Games\MyGame"

& "$engine\bin\EngineEditor.exe" project editor $project
```

New projects can also be created outside the installation:

```powershell
& "$engine\bin\EngineEditor.exe" project new $project `
  --name MyGame `
  --with-csharp
```

All persistent game outputs are rooted by the directory containing
`game.project.json`:

```text
MyGame/
  game.project.json
  assets/                     authored/imported project content
  config/                     project configuration
  scripts/                    game-authored C#
  build/                      generated project build output
  Dist/                       exported Windows releases
```

Editor Validate, Build, Cook, Play, and Package operations use the project root
as their working directory. A relative package output such as the default
`Dist` resolves against that root, not against the engine installation or
source repository.

## Managed SDK and host deployment

For a project that declares C# scripts, project opening validates the generated
API contract and copies the prebuilt managed tools into the project:

```text
<installation>/sdk/EngineGameplay.dll
  -> <project>/build/script-sdk/EngineGameplay.dll

<installation>/sdk/script-host/*
  -> <project>/build/script-host/*
```

Files are copied through engine-owned staging locations. An identical host is
reused, including while Windows has the process executable loaded. This
deployment does not evaluate the project's MSBuild file or compile game code,
so merely opening a workspace does not execute project-authored build logic.

The editor compiles game C# on explicit Rebuild, Play, or Build. The equivalent
command-line operations are:

```powershell
& "$engine\bin\EngineEditor.exe" project build-scripts $project
& "$engine\bin\EngineEditor.exe" project build $project
```

Installed builds compile the game assembly against the copied SDK. They do not
rebuild `EngineGameplay.dll` or the host from engine source. A .NET 8 SDK is
still needed to compile C# game code, and the framework-dependent host needs a
.NET 8 runtime. Projects without C# do not need .NET.

The generated `build/script-sdk-source/` cache belongs only to source
development. Installed create/sync/build operations do not create or consume
it; if an old cache is already present under the generated `build/` directory,
the installed path ignores it. Installed script builds consume the verified
installation DLL instead.

## Build and package

`project build` cooks assets and compiles configured game scripts into
project-local `build/`:

```powershell
& "$engine\bin\EngineEditor.exe" project build $project
```

Package from the editor's Package action or invoke the installed packaging
tool directly:

```powershell
& "$engine\tools\package-windows.ps1" `
  -EngineInstallRoot $engine `
  -ProjectPath "$project\game.project.json"
```

The output root defaults to `$project\Dist`; the editor supplies the version
entered in its Package panel, while a direct script invocation falls back to
the installation's engine version when neither `-Version` nor
`RELEASE_VERSION` is set. Packaging uses only the prebuilt tools selected by
the installation manifest, validates their hashes, stages the package below
`Dist`, runs a headless smoke test, and moves the completed version into place.
It refuses to overwrite an existing version directory.

Installed packaging does not invoke Cargo, Rust, or Git and does not require a
checkout of this repository. It also rejects source-maintainer options such as
`-CargoTargetDir`, `-SkipBuild`, and `-AllowDirty`.

See [`RELEASE_PACKAGING.md`](RELEASE_PACKAGING.md) for package contents,
diagnostics, reproducibility, and the separate maintainer workflow.

## Source-development fallback

Engine contributors can run binaries directly from this repository. If no
installation manifest is found, source mode is accepted only when the running
executable is below this workspace's default `target` directory. A custom
development target directory must set `ENGINE_SOURCE_ROOT` explicitly to a
valid engine source workspace.

Source mode may compile the SDK/host and release runtime from engine sources,
and may invoke Cargo, Git, Rust, and .NET. Those are maintainer operations, not
dependencies of an installed engine. A copied executable outside `target`
cannot silently reach back into the checkout; it must be placed in a valid
installation or receive an explicit development source root.

Maintainers assemble a distribution with:

```powershell
.\.github\scripts\build-engine-installation.ps1 -Version v0.1.0-rc1
```

The result is written under
`artifacts/engine-installation/<version>/windows-x86_64`. There is currently no
MSI, automatic updater, code signing, or installer-level rollback.
