[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$OutputRoot = "",
    [string]$CargoTargetDir = "",
    [string]$ProjectPath = "",
    # The project player currently has a real windowed implementation only for
    # Vulkan. Keep the release surface honest until DX12 has one too.
    [ValidateSet("vulkan")]
    [string]$Backend = "vulkan",
    [switch]$SkipBuild,
    [switch]$SkipSmoke,
    [switch]$AllowDirty
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Test-Path Variable:\PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $true
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path $repoRoot "artifacts\release"
}
$OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)
if ([string]::IsNullOrWhiteSpace($CargoTargetDir)) {
    $CargoTargetDir = Join-Path $repoRoot "target"
}
$CargoTargetDir = [System.IO.Path]::GetFullPath($CargoTargetDir)
$cargoReleaseDir = Join-Path $CargoTargetDir "release"
$platform = "windows-x86_64"

if ([string]::IsNullOrWhiteSpace($ProjectPath)) {
    $ProjectPath = Join-Path $repoRoot "examples\minimal-game\game.project.json"
}
$ProjectPath = [System.IO.Path]::GetFullPath($ProjectPath)
if (Test-Path -LiteralPath $ProjectPath -PathType Container) {
    $ProjectPath = Join-Path $ProjectPath "game.project.json"
}
if (-not (Test-Path -LiteralPath $ProjectPath -PathType Leaf)) {
    throw "Game project manifest was not found: $ProjectPath"
}
$projectRoot = [System.IO.Path]::GetDirectoryName($ProjectPath)
$projectManifestJson = Get-Content -LiteralPath $ProjectPath -Raw
# Windows PowerShell already rejects duplicate JSON object keys. PowerShell 7
# may run on a newer JSON implementation, so inspect the raw scenes object when
# System.Text.Json is available and enforce portable uniqueness explicitly.
$jsonDocumentType = [Type]::GetType("System.Text.Json.JsonDocument, System.Text.Json", $false)
if ($null -ne $jsonDocumentType) {
    $jsonDocument = $jsonDocumentType::Parse($projectManifestJson)
    try {
        $sceneObjectCount = 0
        $rawSceneIds = [System.Collections.Generic.HashSet[string]]::new(
            [StringComparer]::OrdinalIgnoreCase
        )
        foreach ($rootProperty in $jsonDocument.RootElement.EnumerateObject()) {
            if ($rootProperty.Name -cne "scenes") {
                continue
            }
            $sceneObjectCount += 1
            if ($sceneObjectCount -gt 1) {
                throw "Game project manifest contains a duplicate 'scenes' field"
            }
            if ($rootProperty.Value.ValueKind.ToString() -ne "Object") {
                continue
            }
            foreach ($sceneProperty in $rootProperty.Value.EnumerateObject()) {
                if (-not $rawSceneIds.Add($sceneProperty.Name)) {
                    throw "Project scene ID '$($sceneProperty.Name)' is duplicated or differs from another ID only by letter case"
                }
            }
        }
    }
    finally {
        $jsonDocument.Dispose()
    }
}
try {
    $projectManifest = $projectManifestJson | ConvertFrom-Json
}
catch {
    throw "Game project manifest is not valid JSON or contains duplicate keys: $($_.Exception.Message)"
}
if ($projectManifest.schema -ne "GameProject-v0") {
    throw "Unsupported game project schema: $($projectManifest.schema)"
}
$scriptProjectProperty = $projectManifest.PSObject.Properties["script_project"]
$scriptAssemblyProperty = $projectManifest.PSObject.Properties["script_assembly"]
$hasScriptProject = $null -ne $scriptProjectProperty -and
    -not [string]::IsNullOrWhiteSpace([string]$scriptProjectProperty.Value)
$hasScriptAssembly = $null -ne $scriptAssemblyProperty -and
    -not [string]::IsNullOrWhiteSpace([string]$scriptAssemblyProperty.Value)
if ($hasScriptProject -ne $hasScriptAssembly) {
    throw "Scripted projects must configure both script_project and script_assembly"
}
$hasScripts = $hasScriptProject -and $hasScriptAssembly

function Invoke-Native {
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][string]$Command,
        [string[]]$Arguments = @()
    )
    $executable = (Get-Command $Command -ErrorAction Stop).Source
    Write-Host "`n==> $Label"
    & $executable @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Label failed with exit code $LASTEXITCODE"
    }
}

function Get-NativeOutput {
    param(
        [Parameter(Mandatory)][string]$Command,
        [string[]]$Arguments = @()
    )
    $executable = (Get-Command $Command -ErrorAction Stop).Source
    $output = & $executable @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
    return ($output -join "`n").Trim()
}

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][AllowEmptyString()][string]$Content
    )
    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

function Get-RelativeReleasePath {
    param(
        [Parameter(Mandatory)][string]$Base,
        [Parameter(Mandatory)][string]$Path
    )
    $baseUri = [Uri]::new(([System.IO.Path]::GetFullPath($Base).TrimEnd('\') + '\'))
    $pathUri = [Uri]::new([System.IO.Path]::GetFullPath($Path))
    return [Uri]::UnescapeDataString($baseUri.MakeRelativeUri($pathUri).ToString())
}

function Resolve-ProjectRelativePath {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$RelativePath,
        [Parameter(Mandatory)][string]$Field
    )
    if ([string]::IsNullOrWhiteSpace($RelativePath) -or [System.IO.Path]::IsPathRooted($RelativePath)) {
        throw "Project field '$Field' must be a non-empty relative path"
    }
    $segments = $RelativePath -split '[\\/]'
    if ($segments -contains '..') {
        throw "Project field '$Field' may not traverse outside the project root"
    }
    $invalidFileNameCharacters = [System.IO.Path]::GetInvalidFileNameChars()
    foreach ($segment in $segments) {
        if ([string]::IsNullOrEmpty($segment) -or $segment -eq ".") {
            continue
        }
        if (
            $segment.IndexOfAny($invalidFileNameCharacters) -ge 0 -or
            $segment.EndsWith(" ", [StringComparison]::Ordinal) -or
            $segment.EndsWith(".", [StringComparison]::Ordinal)
        ) {
            throw "Project field '$Field' contains a Windows-unsafe path segment '$segment'"
        }
        $deviceName = ($segment -split '\.', 2)[0]
        if ($deviceName -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$') {
            throw "Project field '$Field' contains reserved Windows device name '$segment'"
        }
    }
    $rootPath = [System.IO.Path]::GetFullPath($Root)
    $resolved = [System.IO.Path]::GetFullPath((Join-Path $rootPath $RelativePath))
    $prefix = $rootPath.TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Project field '$Field' resolves outside the project root"
    }
    return $resolved
}

function Get-ProjectSceneCatalog {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][object]$Manifest
    )

    $startupProperty = $Manifest.PSObject.Properties["startup_scene"]
    if ($null -eq $startupProperty -or $startupProperty.Value -isnot [string] -or
        [string]::IsNullOrWhiteSpace([string]$startupProperty.Value)) {
        throw "Project field 'startup_scene' must be a non-empty scene ID or relative .scene.ron path"
    }
    $startupReference = [string]$startupProperty.Value

    $sceneProperties = @()
    $scenesProperty = $Manifest.PSObject.Properties["scenes"]
    if ($null -ne $scenesProperty) {
        if ($null -eq $scenesProperty.Value -or $scenesProperty.Value -isnot [PSCustomObject]) {
            throw "Project field 'scenes' must be a JSON object mapping scene IDs to relative .scene.ron paths"
        }
        $sceneProperties = @($scenesProperty.Value.PSObject.Properties)
    }

    if ($sceneProperties.Count -eq 0) {
        $sceneProperties = @([PSCustomObject]@{
            Name = "main"
            Value = $startupReference
        })
    }

    $sceneIds = [System.Collections.Generic.HashSet[string]]::new(
        [StringComparer]::OrdinalIgnoreCase
    )
    $entries = @(
        $sceneProperties |
            Sort-Object -Property Name -CaseSensitive |
            ForEach-Object {
                $id = [string]$_.Name
                if (
                    $id.Length -lt 1 -or
                    $id.Length -gt 128 -or
                    $id -eq "." -or
                    $id -eq ".." -or
                    $id -notmatch '^[0-9A-Za-z_.-]+$'
                ) {
                    throw "Invalid project scene ID '$id'; use 1..=128 ASCII letters, digits, '.', '_' or '-'"
                }
                if (-not $sceneIds.Add($id)) {
                    throw "Project scene ID '$id' is duplicated or differs from another ID only by letter case"
                }
                if ($_.Value -isnot [string] -or [string]::IsNullOrWhiteSpace([string]$_.Value)) {
                    throw "Project field 'scenes.$id' must be a non-empty relative .scene.ron path"
                }
                $relativePath = [string]$_.Value
                if (-not $relativePath.EndsWith(".scene.ron", [StringComparison]::OrdinalIgnoreCase)) {
                    throw "Project field 'scenes.$id' must end in .scene.ron"
                }
                $sourcePath = Resolve-ProjectRelativePath -Root $Root -RelativePath $relativePath -Field "scenes.$id"
                if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
                    throw "Project scene '$id' was not found: $sourcePath"
                }
                [PSCustomObject]@{
                    id = $id
                    relative_path = $relativePath.Replace('\', '/')
                    source_path = $sourcePath
                }
            }
    )

    $startupScene = $entries | Where-Object {
        [string]::Equals($_.id, $startupReference, [StringComparison]::Ordinal)
    } | Select-Object -First 1
    if ($null -eq $startupScene -and $sceneProperties.Count -gt 0) {
        if ($startupReference.EndsWith(".scene.ron", [StringComparison]::OrdinalIgnoreCase)) {
            $startupPath = Resolve-ProjectRelativePath -Root $Root -RelativePath $startupReference -Field "startup_scene"
            $startupScene = $entries | Where-Object {
                [string]::Equals($_.source_path, $startupPath, [StringComparison]::OrdinalIgnoreCase)
            } | Select-Object -First 1
        }
    }
    if ($null -eq $startupScene) {
        throw "Project field 'startup_scene' must name a scene ID or a path present in the scenes catalog"
    }

    return [PSCustomObject]@{
        entries = @($entries)
        startup = $startupScene
    }
}

function New-DeterministicZip {
    param(
        [Parameter(Mandatory)][string]$SourceRoot,
        [Parameter(Mandatory)][string]$EntryRoot,
        [Parameter(Mandatory)][string]$ArchivePath,
        [Parameter(Mandatory)][DateTimeOffset]$Timestamp
    )

    if (Test-Path -LiteralPath $ArchivePath) {
        Remove-Item -LiteralPath $ArchivePath -Force
    }
    $archiveStream = [System.IO.File]::Open($ArchivePath, [System.IO.FileMode]::CreateNew)
    try {
        $archive = [System.IO.Compression.ZipArchive]::new(
            $archiveStream,
            [System.IO.Compression.ZipArchiveMode]::Create,
            $false
        )
        try {
            Get-ChildItem -LiteralPath $SourceRoot -Recurse -File |
                Sort-Object FullName |
                ForEach-Object {
                    $entryName = "$EntryRoot/$(Get-RelativeReleasePath $SourceRoot $_.FullName)"
                    $entry = $archive.CreateEntry($entryName, [System.IO.Compression.CompressionLevel]::Optimal)
                    $entry.LastWriteTime = $Timestamp
                    $input = [System.IO.File]::OpenRead($_.FullName)
                    $output = $entry.Open()
                    try {
                        $input.CopyTo($output)
                    }
                    finally {
                        $output.Dispose()
                        $input.Dispose()
                    }
                }
        }
        finally {
            $archive.Dispose()
        }
    }
    finally {
        $archiveStream.Dispose()
    }
}

$projectScenes = Get-ProjectSceneCatalog -Root $projectRoot -Manifest $projectManifest

Push-Location $repoRoot
try {
    $commit = Get-NativeOutput "git" @("rev-parse", "--verify", "HEAD")
    $dirty = -not [string]::IsNullOrWhiteSpace((Get-NativeOutput "git" @("status", "--porcelain")))
    if ($dirty -and -not $AllowDirty) {
        throw "Release packaging requires a clean worktree. Use -AllowDirty only for a local dry run."
    }

    if ([string]::IsNullOrWhiteSpace($Version)) {
        if (-not [string]::IsNullOrWhiteSpace($env:RELEASE_VERSION)) {
            $Version = $env:RELEASE_VERSION
        }
        else {
            $Version = Get-NativeOutput "git" @("describe", "--tags", "--always")
        }
    }
    if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$') {
        throw "Invalid release version '$Version'"
    }

    $releaseRoot = [System.IO.Path]::GetFullPath((Join-Path $OutputRoot $Version))
    $stageRoot = [System.IO.Path]::GetFullPath((Join-Path $releaseRoot $platform))
    $symbolStageName = "$platform-symbols"
    $symbolStageRoot = [System.IO.Path]::GetFullPath((Join-Path $releaseRoot $symbolStageName))
    foreach ($ownedPath in @($stageRoot, $symbolStageRoot)) {
        if (-not $ownedPath.StartsWith(($OutputRoot.TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to stage outside output root: $ownedPath"
        }
        if (Test-Path -LiteralPath $ownedPath) {
            Remove-Item -LiteralPath $ownedPath -Recurse -Force
        }
    }

    $binaryDir = New-Item -ItemType Directory -Force -Path (Join-Path $stageRoot "binaries")
    $assetDir = New-Item -ItemType Directory -Force -Path (Join-Path $stageRoot "assets\cooked")
    $manifestDir = New-Item -ItemType Directory -Force -Path (Join-Path $stageRoot "manifests")
    $symbolDir = New-Item -ItemType Directory -Force -Path $symbolStageRoot
    $logDir = New-Item -ItemType Directory -Force -Path (Join-Path $stageRoot "logs")
    $checksumDir = New-Item -ItemType Directory -Force -Path (Join-Path $stageRoot "checksums")
    $configDir = New-Item -ItemType Directory -Force -Path (Join-Path $stageRoot "config")

    $featureList = @("backend-$Backend", "target-desktop")
    if ($hasScripts) {
        $featureList += "subsystem-scripting-csharp"
    }
    $features = $featureList -join ','
    if (-not $SkipBuild) {
        $workspaceFeatureList = @("sandbox/backend-$Backend", "sandbox/target-desktop")
        if ($hasScripts) {
            $workspaceFeatureList += "sandbox/subsystem-scripting-csharp"
        }
        $workspaceFeatures = $workspaceFeatureList -join ','
        Invoke-Native "locked Release runtime and asset cooker build ($features)" "cargo" @(
            "build", "--locked", "--release", "--target-dir", $CargoTargetDir,
            "-p", "sandbox", "-p", "engine-asset", "--features", $workspaceFeatures
        )
    }

    $sandboxExe = Join-Path $cargoReleaseDir "sandbox.exe"
    if (-not (Test-Path -LiteralPath $sandboxExe -PathType Leaf)) {
        throw "Release executable was not produced: $sandboxExe"
    }
    Copy-Item -LiteralPath $sandboxExe -Destination (Join-Path $binaryDir "sandbox.exe")
    $sandboxPdb = Join-Path $cargoReleaseDir "sandbox.pdb"
    if (-not (Test-Path -LiteralPath $sandboxPdb -PathType Leaf)) {
        throw "Release symbols were not produced: $sandboxPdb"
    }
    $stagedPdb = Join-Path $symbolDir "sandbox.pdb"
    Copy-Item -LiteralPath $sandboxPdb -Destination $stagedPdb

    $assetCookExe = Join-Path $cargoReleaseDir "asset-cook.exe"
    if (-not (Test-Path -LiteralPath $assetCookExe -PathType Leaf)) {
        throw "Release asset cooker was not produced: $assetCookExe"
    }
    $sourceAssetRoot = Resolve-ProjectRelativePath -Root $projectRoot -RelativePath ([string]$projectManifest.asset_source) -Field "asset_source"
    $startupScenePath = $projectScenes.startup.source_path
    $inputActionsPath = $null
    if ($null -ne $projectManifest.input_actions -and -not [string]::IsNullOrWhiteSpace([string]$projectManifest.input_actions)) {
        $inputActionsPath = Resolve-ProjectRelativePath -Root $projectRoot -RelativePath ([string]$projectManifest.input_actions) -Field "input_actions"
    }
    $scriptProjectPath = $null
    $scriptAssemblyPath = $null
    if ($hasScripts) {
        $scriptProjectPath = Resolve-ProjectRelativePath -Root $projectRoot -RelativePath ([string]$projectManifest.script_project) -Field "script_project"
        $scriptAssemblyPath = Resolve-ProjectRelativePath -Root $projectRoot -RelativePath ([string]$projectManifest.script_assembly) -Field "script_assembly"
    }
    if (-not (Test-Path -LiteralPath $sourceAssetRoot -PathType Container)) {
        throw "Project source asset directory was not found: $sourceAssetRoot"
    }
    if ($null -ne $inputActionsPath -and -not (Test-Path -LiteralPath $inputActionsPath -PathType Leaf)) {
        throw "Project input action map was not found: $inputActionsPath"
    }
    if ($hasScripts -and -not (Test-Path -LiteralPath $scriptProjectPath -PathType Leaf)) {
        throw "Project C# source project was not found: $scriptProjectPath"
    }

    if ($hasScripts) {
        Invoke-Native "build game scripts and publish script host" $sandboxExe @(
            "project", "build-scripts", $ProjectPath
        )
        if (-not (Test-Path -LiteralPath $scriptAssemblyPath -PathType Leaf)) {
            throw "Project script build did not produce the declared assembly: $scriptAssemblyPath"
        }
    }

    $projectCheckReport = Join-Path $manifestDir "project-check.json"
    Invoke-Native "validate game project" $sandboxExe @(
        "project", "check", $ProjectPath, "--report", $projectCheckReport
    )
    $checkReport = Get-Content -LiteralPath $projectCheckReport -Raw | ConvertFrom-Json
    if ($checkReport.schema -ne "ProjectCheckReport-v0" -or -not $checkReport.passed) {
        throw "Game project validation did not pass"
    }

    $stagedProjectPath = Join-Path $stageRoot "game.project.json"
    $projectManifest.cooked_assets = "assets/cooked"
    if ($hasScripts) {
        $scriptStageDir = New-Item -ItemType Directory -Force -Path (Join-Path $stageRoot "scripts")
        $scriptOutputDir = Split-Path -Parent $scriptAssemblyPath
        Get-ChildItem -LiteralPath $scriptOutputDir -File |
            Where-Object { $_.Extension -eq ".dll" -or $_.Name -match "\.(deps|runtimeconfig)\.json$" } |
            ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $scriptStageDir $_.Name)
        }
        $scriptHostSourceDir = Join-Path $projectRoot "build\script-host"
        if (-not (Test-Path -LiteralPath (Join-Path $scriptHostSourceDir "EngineScriptHost.exe") -PathType Leaf)) {
            throw "Published EngineScriptHost.exe was not found: $scriptHostSourceDir"
        }
        $scriptHostStageDir = New-Item -ItemType Directory -Force -Path (Join-Path $binaryDir "script-host")
        Get-ChildItem -LiteralPath $scriptHostSourceDir -File |
            Where-Object { $_.Extension -in @(".exe", ".dll") -or $_.Name -match "\.(deps|runtimeconfig)\.json$" } |
            ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $scriptHostStageDir $_.Name)
        }
        $projectManifest.script_assembly = "scripts/$([System.IO.Path]::GetFileName($scriptAssemblyPath))"
        $projectManifest.PSObject.Properties.Remove("script_project")
    }
    Write-Utf8NoBom $stagedProjectPath (($projectManifest | ConvertTo-Json -Depth 10) + "`n")
    $stagedScenePaths = @{}
    foreach ($scene in $projectScenes.entries) {
        $stagedScenePathForId = Resolve-ProjectRelativePath `
            -Root $stageRoot `
            -RelativePath $scene.relative_path `
            -Field "scenes.$($scene.id)"
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $stagedScenePathForId) | Out-Null
        Copy-Item -LiteralPath $scene.source_path -Destination $stagedScenePathForId
        $stagedScenePaths[$scene.id] = $stagedScenePathForId
    }
    $stagedScenePath = [string]$stagedScenePaths[$projectScenes.startup.id]
    if ($null -ne $inputActionsPath) {
        $stagedInputActionsPath = Join-Path $stageRoot ([string]$projectManifest.input_actions)
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $stagedInputActionsPath) | Out-Null
        Copy-Item -LiteralPath $inputActionsPath -Destination $stagedInputActionsPath
    }

    $assetCookReport = Join-Path $manifestDir "asset-cook.json"
    Invoke-Native "strict deterministic asset cook" $assetCookExe @(
        "--source", $sourceAssetRoot,
        "--output", $assetDir.FullName,
        "--report", $assetCookReport
    )
    $cookReport = Get-Content -LiteralPath $assetCookReport -Raw | ConvertFrom-Json
    if ($cookReport.schema -ne "AssetCookReport-v0") {
        throw "Asset cooker emitted an unexpected report schema: $($cookReport.schema)"
    }
    if ($cookReport.succeeded_asset_count -ne $cookReport.declared_asset_count) {
        throw "Asset cook did not produce every declared asset"
    }

    $assetEntries = @(
        Get-ChildItem -LiteralPath $assetDir -Recurse -File |
            Sort-Object FullName |
            ForEach-Object {
                [ordered]@{
                    path = Get-RelativeReleasePath $assetDir $_.FullName
                    size = $_.Length
                    sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                }
            }
    )
    $assetManifestJson = if ($assetEntries.Count -eq 0) {
        "[]"
    }
    else {
        $assetEntries | ConvertTo-Json -Depth 5
    }
    Write-Utf8NoBom (Join-Path $manifestDir "assets.json") ($assetManifestJson + "`n")

    $metadata = (Get-NativeOutput "cargo" @("metadata", "--locked", "--format-version", "1")) | ConvertFrom-Json
    $notices = @(
        $metadata.packages |
            Where-Object { $null -ne $_.source } |
            Sort-Object name, version |
            ForEach-Object {
                $license = if ([string]::IsNullOrWhiteSpace($_.license)) { "UNKNOWN" } else { $_.license }
                $repository = if ([string]::IsNullOrWhiteSpace($_.repository)) { "" } else { " $($_.repository)" }
                "$($_.name) $($_.version) | $license$repository"
            }
    )
    $noticeHeader = @(
        "Third-party dependency notices",
        "Generated from Cargo.lock/Cargo metadata for release $Version.",
        ""
    )
    Write-Utf8NoBom (Join-Path $manifestDir "NOTICES.txt") ((($noticeHeader + $notices) -join "`n") + "`n")

    $commitEpoch = [long](Get-NativeOutput "git" @("show", "-s", "--format=%ct", $commit))
    $rustcVersion = Get-NativeOutput "rustc" @("--version")
    $sceneReleaseEntries = @(
        foreach ($scene in $projectScenes.entries) {
            $sceneStagePath = [string]$stagedScenePaths[$scene.id]
            [ordered]@{
                id = $scene.id
                path = $scene.relative_path
                sha256 = (Get-FileHash -LiteralPath $sceneStagePath -Algorithm SHA256).Hash.ToLowerInvariant()
            }
        }
    )
    $startupSceneId = [string]$projectScenes.startup.id
    $startupSceneRelativePath = [string]$projectScenes.startup.relative_path
    $startupSceneSha256 = (Get-FileHash -LiteralPath $stagedScenePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $releaseMetadata = [ordered]@{
        schema = "ReleaseMetadata-v0"
        release_id = $Version
        platform = $platform
        backend = $Backend
        features = @($features.Split(','))
        commit = $commit
        dirty = $dirty
        source_date_epoch = $commitEpoch
        rustc = $rustcVersion
        asset_count = $assetEntries.Count
        project = [ordered]@{
            name = [string]$projectManifest.name
            manifest = "game.project.json"
            manifest_sha256 = (Get-FileHash -LiteralPath $stagedProjectPath -Algorithm SHA256).Hash.ToLowerInvariant()
            # Keep startup_scene as the resolved path for older release tooling.
            startup_scene = $startupSceneRelativePath
            startup_scene_id = $startupSceneId
            startup_scene_path = $startupSceneRelativePath
            startup_scene_sha256 = $startupSceneSha256
            scenes = $sceneReleaseEntries
        }
        launch = "binaries/sandbox.exe game game.project.json"
        symbol_bundle = "$symbolStageName.zip"
    }
    Write-Utf8NoBom (Join-Path $manifestDir "release.json") (($releaseMetadata | ConvertTo-Json -Depth 8) + "`n")
    Write-Utf8NoBom (Join-Path $configDir "runtime.json") (([ordered]@{
        release_id = $Version
        asset_root = "assets/cooked"
        project = "game.project.json"
        startup_scene = $startupSceneRelativePath
        startup_scene_id = $startupSceneId
        startup_scene_path = $startupSceneRelativePath
        log_root = "logs"
    } | ConvertTo-Json) + "`n")

    $stagedExecutable = Join-Path $binaryDir "sandbox.exe"
    $symbolMetadata = [ordered]@{
        schema = "SymbolManifest-v0"
        release_id = $Version
        platform = $platform
        executable = [ordered]@{
            path = "$platform/binaries/sandbox.exe"
            size = (Get-Item -LiteralPath $stagedExecutable).Length
            sha256 = (Get-FileHash -LiteralPath $stagedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        pdb = [ordered]@{
            path = "sandbox.pdb"
            size = (Get-Item -LiteralPath $stagedPdb).Length
            sha256 = (Get-FileHash -LiteralPath $stagedPdb -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    Write-Utf8NoBom (Join-Path $symbolDir "symbols.json") (($symbolMetadata | ConvertTo-Json -Depth 5) + "`n")

    if (-not $SkipSmoke) {
        $oldLogDir = $env:ENGINE_LOG_DIR
        $projectRunReport = Join-Path $manifestDir "project-run.json"
        Push-Location $stageRoot
        try {
            $env:ENGINE_LOG_DIR = "off"
            Invoke-Native "packaged game project headless smoke" (Join-Path $binaryDir "sandbox.exe") @(
                "game", "game.project.json", "--headless", "--frames", "3", "--report", $projectRunReport
            )
        }
        finally {
            $env:ENGINE_LOG_DIR = $oldLogDir
            Pop-Location
        }
        $runReport = Get-Content -LiteralPath $projectRunReport -Raw | ConvertFrom-Json
        if ($runReport.schema -ne "ProjectRunReport-v0" -or -not $runReport.passed) {
            throw "Packaged game project smoke emitted an invalid or failed report"
        }
        if (
            [long]$runReport.total_draw_calls -lt 1 -or
            [long]$runReport.last_visible_drawables -lt 1 -or
            [long]$runReport.total_triangles -lt 1
        ) {
            throw "Packaged game project produced no visible indexed geometry"
        }
        if ($hasScripts) {
            if (
                [long]$runReport.script_assemblies -lt 1 -or
                [long]$runReport.script_instances -lt 1 -or
                [long]$runReport.script_started_instances -lt 1 -or
                [long]$runReport.script_errors -ne 0
            ) {
                throw "Packaged game project did not execute its managed script lifecycle"
            }
        }
    }

    $checksumLines = @(
        Get-ChildItem -LiteralPath $stageRoot -Recurse -File |
            Where-Object { -not $_.FullName.StartsWith($checksumDir.FullName, [StringComparison]::OrdinalIgnoreCase) } |
            Sort-Object FullName |
            ForEach-Object {
                $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                $relative = Get-RelativeReleasePath $stageRoot $_.FullName
                "$hash  $relative"
            }
    )
    Write-Utf8NoBom (Join-Path $checksumDir "SHA256SUMS.txt") (($checksumLines -join "`n") + "`n")

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $timestamp = [DateTimeOffset]::FromUnixTimeSeconds($commitEpoch)
    $archivePath = Join-Path $releaseRoot "$platform.zip"
    New-DeterministicZip $stageRoot $platform $archivePath $timestamp
    $symbolArchivePath = Join-Path $releaseRoot "$symbolStageName.zip"
    New-DeterministicZip $symbolDir $symbolStageName $symbolArchivePath $timestamp

    $archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8NoBom "$archivePath.sha256" "$archiveHash  $([System.IO.Path]::GetFileName($archivePath))`n"
    $symbolArchiveHash = (Get-FileHash -LiteralPath $symbolArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8NoBom "$symbolArchivePath.sha256" "$symbolArchiveHash  $([System.IO.Path]::GetFileName($symbolArchivePath))`n"
    Write-Host "`nRelease package: $archivePath"
    Write-Host "SHA-256: $archiveHash"
    Write-Host "Symbol package: $symbolArchivePath"
    Write-Host "Symbol SHA-256: $symbolArchiveHash"
}
finally {
    Pop-Location
}
