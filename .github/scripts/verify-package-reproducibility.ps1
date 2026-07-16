[CmdletBinding()]
param(
    [string]$Version = "reproducibility-check",
    [string]$OutputRoot = "",
    [string]$CargoTargetRoot = "",
    [string]$ProjectPath = "",
    # Must match package-windows.ps1: DX12 is not a shippable project-player
    # backend until it can pass a real windowed launch.
    [ValidateSet("vulkan")]
    [string]$Backend = "vulkan",
    [switch]$AllowDirty,
    [switch]$SkipBuildA,
    [switch]$BuildAOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Test-Path Variable:\PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $true
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path $repoRoot "artifacts\reproducibility"
}
if ([string]::IsNullOrWhiteSpace($CargoTargetRoot)) {
    $CargoTargetRoot = Join-Path $repoRoot "target\package-reproducibility"
}
$OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)
$CargoTargetRoot = [System.IO.Path]::GetFullPath($CargoTargetRoot)
$packageScript = Join-Path $PSScriptRoot "package-windows.ps1"
$platform = "windows-x86_64"
if ([string]::IsNullOrWhiteSpace($ProjectPath)) {
    $ProjectPath = Join-Path $repoRoot "examples\minimal-game\game.project.json"
}
$ProjectPath = [System.IO.Path]::GetFullPath($ProjectPath)

if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$') {
    throw "Invalid reproducibility version '$Version'"
}

function Reset-OwnedDirectory {
    param(
        [Parameter(Mandatory)][string]$OwnerRoot,
        [Parameter(Mandatory)][string]$Target
    )

    $owner = [System.IO.Path]::GetFullPath($OwnerRoot).TrimEnd('\')
    $resolved = [System.IO.Path]::GetFullPath($Target).TrimEnd('\')
    if (
        $resolved -eq $owner -or
        -not $resolved.StartsWith(($owner + '\'), [StringComparison]::OrdinalIgnoreCase)
    ) {
        throw "Refusing to reset path outside its owned root: $resolved"
    }
    if (Test-Path -LiteralPath $resolved) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $resolved | Out-Null
}

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][AllowEmptyString()][string]$Content
    )
    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

function Get-RelativePath {
    param(
        [Parameter(Mandatory)][string]$Base,
        [Parameter(Mandatory)][string]$Path
    )
    $baseUri = [Uri]::new(([System.IO.Path]::GetFullPath($Base).TrimEnd('\') + '\'))
    $pathUri = [Uri]::new([System.IO.Path]::GetFullPath($Path))
    return [Uri]::UnescapeDataString($baseUri.MakeRelativeUri($pathUri).ToString())
}

function Get-StageHashes {
    param([Parameter(Mandatory)][string]$StageRoot)

    $hashes = @{}
    Get-ChildItem -LiteralPath $StageRoot -Recurse -File |
        Sort-Object FullName |
        ForEach-Object {
            $relative = Get-RelativePath $StageRoot $_.FullName
            $hashes[$relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    return $hashes
}

function Resolve-PackagedScenePath {
    param(
        [Parameter(Mandatory)][string]$StageRoot,
        [Parameter(Mandatory)][string]$RelativePath,
        [Parameter(Mandatory)][string]$Field
    )

    $segments = $RelativePath -split '[\\/]'
    if (
        [string]::IsNullOrWhiteSpace($RelativePath) -or
        [System.IO.Path]::IsPathRooted($RelativePath) -or
        $segments -contains '..' -or
        -not $RelativePath.EndsWith(".scene.ron", [StringComparison]::OrdinalIgnoreCase)
    ) {
        throw "Packaged field '$Field' is not a safe relative .scene.ron path: $RelativePath"
    }
    $invalidFileNameCharacters = [System.IO.Path]::GetInvalidFileNameChars()
    foreach ($segment in $segments) {
        if ([string]::IsNullOrEmpty($segment) -or $segment -eq ".") {
            continue
        }
        $deviceName = ($segment -split '\.', 2)[0]
        if (
            $segment.IndexOfAny($invalidFileNameCharacters) -ge 0 -or
            $segment.EndsWith(" ", [StringComparison]::Ordinal) -or
            $segment.EndsWith(".", [StringComparison]::Ordinal) -or
            $deviceName -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$'
        ) {
            throw "Packaged field '$Field' contains an unsafe Windows path segment: $segment"
        }
    }
    $root = [System.IO.Path]::GetFullPath($StageRoot)
    $resolved = [System.IO.Path]::GetFullPath((Join-Path $root $RelativePath))
    if (-not $resolved.StartsWith(($root.TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)) {
        throw "Packaged field '$Field' resolves outside the runtime stage: $RelativePath"
    }
    return $resolved
}

function Get-PackagedSceneValidation {
    param([Parameter(Mandatory)][string]$StageRoot)

    $releasePath = Join-Path $StageRoot "manifests\release.json"
    $projectPath = Join-Path $StageRoot "game.project.json"
    foreach ($requiredPath in @($releasePath, $projectPath)) {
        if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) {
            throw "Packaged scene metadata input is missing: $requiredPath"
        }
    }
    $release = Get-Content -LiteralPath $releasePath -Raw | ConvertFrom-Json
    $project = Get-Content -LiteralPath $projectPath -Raw | ConvertFrom-Json
    $releaseScenesProperty = $release.project.PSObject.Properties["scenes"]
    $startupIdProperty = $release.project.PSObject.Properties["startup_scene_id"]
    $startupPathProperty = $release.project.PSObject.Properties["startup_scene_path"]
    if (
        $null -eq $releaseScenesProperty -or
        $null -eq $startupIdProperty -or
        $null -eq $startupPathProperty
    ) {
        throw "Release metadata does not contain the packaged scene catalog and startup scene ID/path"
    }
    $releaseScenes = @($releaseScenesProperty.Value)
    if ($releaseScenes.Count -lt 1) {
        throw "Release metadata scene catalog is empty"
    }

    $expectedScenes = @()
    $projectScenesProperty = $project.PSObject.Properties["scenes"]
    if ($null -ne $projectScenesProperty -and $null -ne $projectScenesProperty.Value) {
        $expectedScenes = @(
            $projectScenesProperty.Value.PSObject.Properties |
                ForEach-Object {
                    [PSCustomObject]@{ id = [string]$_.Name; path = [string]$_.Value }
                }
        )
    }
    if ($expectedScenes.Count -eq 0) {
        $expectedScenes = @([PSCustomObject]@{
            id = "main"
            path = [string]$project.startup_scene
        })
    }
    if ($releaseScenes.Count -ne $expectedScenes.Count) {
        throw "Release metadata scene count does not match the staged project manifest"
    }

    $seenIds = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($expected in $expectedScenes) {
        if (-not $seenIds.Add([string]$expected.id)) {
            throw "Staged project contains a duplicate portable scene ID: $($expected.id)"
        }
        $entry = $releaseScenes | Where-Object { $_.id -ceq $expected.id } | Select-Object -First 1
        if ($null -eq $entry) {
            throw "Release metadata is missing scene '$($expected.id)'"
        }
        $expectedPath = Resolve-PackagedScenePath $StageRoot ([string]$expected.path) "game.project.json scenes.$($expected.id)"
        $entryPath = Resolve-PackagedScenePath $StageRoot ([string]$entry.path) "release.json scenes.$($expected.id).path"
        if (-not [string]::Equals($expectedPath, $entryPath, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Release metadata path does not match scene '$($expected.id)' in game.project.json"
        }
        if (-not (Test-Path -LiteralPath $entryPath -PathType Leaf)) {
            throw "Packaged scene '$($expected.id)' is missing: $entryPath"
        }
        $actualHash = (Get-FileHash -LiteralPath $entryPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualHash -ne [string]$entry.sha256) {
            throw "Release metadata SHA-256 does not match packaged scene '$($expected.id)'"
        }
    }

    $startupId = [string]$startupIdProperty.Value
    $startupPath = Resolve-PackagedScenePath $StageRoot ([string]$startupPathProperty.Value) "release.json startup_scene_path"
    $startupEntry = $releaseScenes | Where-Object { $_.id -ceq $startupId } | Select-Object -First 1
    if ($null -eq $startupEntry) {
        throw "Release startup scene ID '$startupId' is not present in its scene catalog"
    }
    $startupEntryPath = Resolve-PackagedScenePath $StageRoot ([string]$startupEntry.path) "release.json startup scene"
    if (-not [string]::Equals($startupPath, $startupEntryPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Release startup scene ID/path refer to different catalog entries"
    }
    if ([string]$release.project.startup_scene -ne [string]$startupPathProperty.Value) {
        throw "Legacy release startup_scene field no longer mirrors startup_scene_path"
    }
    if ([string]$release.project.startup_scene_sha256 -ne [string]$startupEntry.sha256) {
        throw "Release startup scene SHA-256 does not match its catalog entry"
    }

    $projectStartupReference = [string]$project.startup_scene
    $expectedStartup = $expectedScenes |
        Where-Object { $_.id -ceq $projectStartupReference } |
        Select-Object -First 1
    if ($null -eq $expectedStartup) {
        $projectStartupPath = Resolve-PackagedScenePath $StageRoot $projectStartupReference "game.project.json startup_scene"
        $expectedStartup = $expectedScenes |
            Where-Object {
                $candidate = Resolve-PackagedScenePath $StageRoot ([string]$_.path) "game.project.json scenes.$($_.id)"
                [string]::Equals($candidate, $projectStartupPath, [StringComparison]::OrdinalIgnoreCase)
            } |
            Select-Object -First 1
    }
    if ($null -eq $expectedStartup -or $startupId -cne [string]$expectedStartup.id) {
        throw "Release startup scene does not match game.project.json"
    }

    return [ordered]@{
        valid = $true
        scene_count = $releaseScenes.Count
        startup_scene_id = $startupId
        startup_scene_path = [string]$startupPathProperty.Value
    }
}

function Get-SymbolValidation {
    param(
        [Parameter(Mandatory)][string]$RunRoot,
        [Parameter(Mandatory)][string]$RuntimeStage
    )

    $symbolRoot = Join-Path $RunRoot "$Version\$platform-symbols"
    $manifestPath = Join-Path $symbolRoot "symbols.json"
    $pdbPath = Join-Path $symbolRoot "sandbox.pdb"
    $executablePath = Join-Path $RuntimeStage "binaries\sandbox.exe"
    foreach ($requiredPath in @($manifestPath, $pdbPath, $executablePath)) {
        if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) {
            throw "Reproducibility input is missing: $requiredPath"
        }
    }

    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    $executableHash = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $pdbHash = (Get-FileHash -LiteralPath $pdbPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $valid =
        $manifest.schema -eq "SymbolManifest-v0" -and
        $manifest.release_id -eq $Version -and
        $manifest.platform -eq $platform -and
        $manifest.executable.sha256 -eq $executableHash -and
        $manifest.pdb.sha256 -eq $pdbHash

    return [ordered]@{
        valid = $valid
        executable_sha256 = $executableHash
        pdb_sha256 = $pdbHash
        manifest_sha256 = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}

function Invoke-PackageBuild {
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][string]$RunOutput,
        [Parameter(Mandatory)][string]$CargoTarget
    )

    Write-Host "`n==> $Label"
    $arguments = @{
        Version = $Version
        OutputRoot = $RunOutput
        CargoTargetDir = $CargoTarget
        ProjectPath = $ProjectPath
        Backend = $Backend
        SkipSmoke = $true
    }
    if ($AllowDirty) {
        $arguments.AllowDirty = $true
    }
    & $packageScript @arguments
    if (-not $?) {
        throw "$Label failed"
    }
    if ($LASTEXITCODE -ne 0) {
        throw "$Label failed with exit code $LASTEXITCODE"
    }
}

$reproRoot = Join-Path $OutputRoot $Version
$cargoBuildRoot = Join-Path $CargoTargetRoot $Version
$runA = Join-Path $reproRoot "run-a"
$runB = Join-Path $reproRoot "run-b"
$archiveA = Join-Path $runA "$Version\$platform.zip"
$stageA = Join-Path $runA "$Version\$platform"

if (-not $SkipBuildA) {
    Reset-OwnedDirectory $OutputRoot $reproRoot
    Reset-OwnedDirectory $CargoTargetRoot $cargoBuildRoot
    Invoke-PackageBuild "independent Release build A" $runA $cargoBuildRoot
}
elseif (
    -not (Test-Path -LiteralPath $archiveA -PathType Leaf) -or
    -not (Test-Path -LiteralPath $stageA -PathType Container)
) {
    throw "Cannot skip Build A because its archive/stage is missing under $runA"
}

$hashA = (Get-FileHash -LiteralPath $archiveA -Algorithm SHA256).Hash.ToLowerInvariant()
$stageHashesA = Get-StageHashes $stageA
$symbolsA = Get-SymbolValidation $runA $stageA
$scenesA = Get-PackagedSceneValidation $stageA

if ($BuildAOnly) {
    Write-Host "`nBuild A is ready for a resumed reproducibility check."
    Write-Host "Run again with -SkipBuildA to build and compare Build B."
    return
}

Reset-OwnedDirectory $CargoTargetRoot $cargoBuildRoot
Invoke-PackageBuild "independent Release build B" $runB $cargoBuildRoot
$archiveB = Join-Path $runB "$Version\$platform.zip"
$stageB = Join-Path $runB "$Version\$platform"
$hashB = (Get-FileHash -LiteralPath $archiveB -Algorithm SHA256).Hash.ToLowerInvariant()
$stageHashesB = Get-StageHashes $stageB
$symbolsB = Get-SymbolValidation $runB $stageB
$scenesB = Get-PackagedSceneValidation $stageB

$allPaths = @($stageHashesA.Keys + $stageHashesB.Keys | Sort-Object -Unique)
$differences = @(
    foreach ($path in $allPaths) {
        $left = $stageHashesA[$path]
        $right = $stageHashesB[$path]
        if ($left -ne $right) {
            [ordered]@{
                path = $path
                build_a_sha256 = $left
                build_b_sha256 = $right
            }
        }
    }
)
$runtimeReproducible = $hashA -eq $hashB -and $differences.Count -eq 0
$symbolsReproducible = $symbolsA.pdb_sha256 -eq $symbolsB.pdb_sha256
$symbolLinkageValid = $symbolsA.valid -and $symbolsB.valid
$passed = $runtimeReproducible -and $symbolLinkageValid
$commit = (& git rev-parse --verify HEAD).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "Could not resolve the source commit"
}
$rustc = (& rustc --version).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "Could not resolve the Rust compiler version"
}
$report = [ordered]@{
    schema = "PackageReproducibilityReport-v0"
    version = $Version
    platform = $platform
    backend = $Backend
    project_manifest_sha256 = (Get-FileHash -LiteralPath $ProjectPath -Algorithm SHA256).Hash.ToLowerInvariant()
    commit = $commit
    rustc = $rustc
    build_a_sha256 = $hashA
    build_b_sha256 = $hashB
    differing_files = $differences
    runtime_reproducible = $runtimeReproducible
    symbols = [ordered]@{
        linkage_valid = $symbolLinkageValid
        reproducible = $symbolsReproducible
        build_a = $symbolsA
        build_b = $symbolsB
    }
    scenes = [ordered]@{
        build_a = $scenesA
        build_b = $scenesB
    }
    passed = $passed
}
$reportPath = Join-Path $reproRoot "report.json"
Write-Utf8NoBom $reportPath (($report | ConvertTo-Json -Depth 8) + "`n")

if (-not $runtimeReproducible) {
    $differenceNames = ($differences | ForEach-Object { $_.path }) -join ", "
    throw "Independent runtime Release packages are not reproducible. Differing files: $differenceNames. Report: $reportPath"
}
if (-not $symbolLinkageValid) {
    throw "At least one sidecar symbol manifest is not linked to its packaged executable/PDB. Report: $reportPath"
}

Write-Host "`nIndependent runtime Release packages are reproducible."
Write-Host "SHA-256: $hashA"
Write-Host "Sidecar symbols reproducible: $symbolsReproducible"
Write-Host "Report: $reportPath"
