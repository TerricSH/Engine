[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$OutputRoot = "",
    [string]$CargoTargetRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Test-Path Variable:\PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $true
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path $repoRoot "artifacts\engine-installation"
}
if ([string]::IsNullOrWhiteSpace($CargoTargetRoot)) {
    $CargoTargetRoot = Join-Path $repoRoot "target\engine-installation"
}
$OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)
$CargoTargetRoot = [System.IO.Path]::GetFullPath($CargoTargetRoot)
$platform = "windows-x86_64"

function Invoke-Native {
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][string]$Command,
        [string[]]$Arguments = @()
    )
    Write-Host "`n==> $Label"
    $executable = (Get-Command $Command -ErrorAction Stop).Source
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
    [System.IO.File]::WriteAllText(
        $Path,
        $Content,
        [System.Text.UTF8Encoding]::new($false)
    )
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

function Copy-RequiredFile {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination
    )
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "Required engine distribution file was not produced: $Source"
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Destination) | Out-Null
    Copy-Item -LiteralPath $Source -Destination $Destination
}

if ($PSVersionTable.PSEdition -eq "Core" -and -not $IsWindows) {
    throw "The Windows engine installation can only be assembled on Windows"
}

Push-Location $repoRoot
$stagingRoot = $null
try {
    $commit = Get-NativeOutput "git" @("rev-parse", "--verify", "HEAD")
    $dirty = -not [string]::IsNullOrWhiteSpace((Get-NativeOutput "git" @("status", "--porcelain")))
    if ($dirty) {
        throw "Engine installation assembly requires a clean worktree"
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
        throw "Invalid engine installation version '$Version'"
    }
    $sourceDateEpoch = [long](Get-NativeOutput "git" @("show", "-s", "--format=%ct", $commit))
    if ($sourceDateEpoch -lt 315532800 -or $sourceDateEpoch -gt 4354819199) {
        throw "Git source timestamp is outside the deterministic ZIP range"
    }
    $rustcVersion = Get-NativeOutput "rustc" @("--version")

    New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
    $releaseRoot = Join-Path $OutputRoot $Version
    $finalRoot = Join-Path $releaseRoot $platform
    if (Test-Path -LiteralPath $releaseRoot) {
        throw "Engine installation version already exists and will not be overwritten: $releaseRoot"
    }
    $stagingRoot = Join-Path $OutputRoot (".engine-install-{0}-{1}" -f $PID, [Guid]::NewGuid().ToString("N"))
    $workingReleaseRoot = Join-Path $stagingRoot $Version
    $installRoot = Join-Path $workingReleaseRoot $platform
    New-Item -ItemType Directory -Force -Path $installRoot | Out-Null

    $runtimeTarget = Join-Path $CargoTargetRoot "runtime"
    Invoke-Native "Build precompiled game runtime and asset cooker" "cargo" @(
        "build", "--locked", "--release", "--target-dir", $runtimeTarget,
        "-p", "sandbox", "-p", "engine-asset",
        "--features",
        "sandbox/backend-vulkan,sandbox/target-desktop,sandbox/subsystem-scripting-csharp,sandbox/terrain"
    )
    $runtimeRelease = Join-Path $runtimeTarget "release"
    Copy-RequiredFile `
        (Join-Path $runtimeRelease "sandbox.exe") `
        (Join-Path $installRoot "runtime\windows-x86_64\GameRuntime.exe")
    Copy-RequiredFile `
        (Join-Path $runtimeRelease "sandbox.pdb") `
        (Join-Path $installRoot "runtime\windows-x86_64\GameRuntime.pdb")
    Copy-RequiredFile `
        (Join-Path $runtimeRelease "asset-cook.exe") `
        (Join-Path $installRoot "tools\asset-cook.exe")

    # Build the managed SDK and host once from engine-owned sources. Every game
    # project later receives these verified binaries by copy.
    $bootstrapProject = Join-Path $stagingRoot "managed-bootstrap"
    $runtimeTool = Join-Path $runtimeRelease "sandbox.exe"
    $oldSourceRoot = $env:ENGINE_SOURCE_ROOT
    try {
        $env:ENGINE_SOURCE_ROOT = $repoRoot
        Invoke-Native "Create managed SDK bootstrap project" $runtimeTool @(
            "project", "new", $bootstrapProject,
            "--name", "Engine Managed Runtime Bootstrap", "--with-csharp"
        )
        Invoke-Native "Build managed SDK and script host" $runtimeTool @(
            "project", "build-scripts", $bootstrapProject
        )
    }
    finally {
        $env:ENGINE_SOURCE_ROOT = $oldSourceRoot
    }
    Copy-RequiredFile `
        (Join-Path $bootstrapProject "build\script-sdk\EngineGameplay.dll") `
        (Join-Path $installRoot "sdk\EngineGameplay.dll")
    $hostSource = Join-Path $bootstrapProject "build\script-host"
    $hostDestination = Join-Path $installRoot "sdk\script-host"
    New-Item -ItemType Directory -Force -Path $hostDestination | Out-Null
    $hostFiles = @(Get-ChildItem -LiteralPath $hostSource -Force)
    if ($hostFiles.Count -eq 0 -or
        @($hostFiles | Where-Object { $_.PSIsContainer -or $_.LinkType }).Count -ne 0) {
        throw "Managed script host publish must contain regular top-level files only"
    }
    foreach ($hostFile in $hostFiles) {
        Copy-Item -LiteralPath $hostFile.FullName -Destination (Join-Path $hostDestination $hostFile.Name)
    }

    # Use a separate Cargo target directory because the editor and player are
    # different feature builds of the same sandbox binary.
    $editorTarget = Join-Path $CargoTargetRoot "editor"
    Invoke-Native "Build installed editor application" "cargo" @(
        "build", "--locked", "--release", "--target-dir", $editorTarget,
        "-p", "sandbox",
        "--features", "backend-vulkan,tooling-editor,target-desktop"
    )
    Copy-RequiredFile `
        (Join-Path $editorTarget "release\sandbox.exe") `
        (Join-Path $installRoot "bin\EngineEditor.exe")
    Copy-RequiredFile `
        (Join-Path $PSScriptRoot "package-windows.ps1") `
        (Join-Path $installRoot "tools\package-windows.ps1")

    $metadata = (Get-NativeOutput "cargo" @(
        "metadata", "--locked", "--format-version", "1"
    )) | ConvertFrom-Json
    $notices = @(
        "Third-party dependency notices",
        "Generated from the engine Cargo.lock for installation $Version.",
        ""
    )
    $notices += @(
        $metadata.packages |
            Where-Object { $null -ne $_.source } |
            Sort-Object name, version |
            ForEach-Object {
                $license = if ([string]::IsNullOrWhiteSpace($_.license)) { "UNKNOWN" } else { $_.license }
                $repository = if ([string]::IsNullOrWhiteSpace($_.repository)) { "" } else { " $($_.repository)" }
                "$($_.name) $($_.version) | $license$repository"
            }
    )
    Write-Utf8NoBom `
        (Join-Path $installRoot "THIRD_PARTY_NOTICES.txt") `
        (($notices -join "`n") + "`n")

    $contract = Get-Content `
        -LiteralPath (Join-Path $bootstrapProject "scripts\GameScripts\EngineGameplay.contract.json") `
        -Raw | ConvertFrom-Json
    if ($contract.schema -ne "EngineGameplaySdkContract-v1" -or
        [string]$contract.sha256 -notmatch '^[0-9A-Fa-f]{64}$') {
        throw "Managed SDK bootstrap emitted an invalid API contract"
    }

    $fileHashes = [ordered]@{}
    Get-ChildItem -LiteralPath $installRoot -Recurse -File |
        Sort-Object FullName |
        ForEach-Object {
            $relative = (Get-RelativePath -Base $installRoot -Path $_.FullName).Replace('\', '/')
            $fileHashes[$relative] =
                (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    $installation = [ordered]@{
        schema = "EngineInstallation-v0"
        engine_version = $Version
        editor = "bin/EngineEditor.exe"
        windows_runtime = "runtime/windows-x86_64/GameRuntime.exe"
        windows_symbols = "runtime/windows-x86_64/GameRuntime.pdb"
        asset_cooker = "tools/asset-cook.exe"
        package_script = "tools/package-windows.ps1"
        managed_sdk = "sdk/EngineGameplay.dll"
        script_host = "sdk/script-host"
        notices = "THIRD_PARTY_NOTICES.txt"
        script_api = [string]$contract.script_api
        script_api_version = [string]$contract.version
        script_api_sha256 = ([string]$contract.sha256).ToLowerInvariant()
        source_commit = $commit
        source_date_epoch = $sourceDateEpoch
        rustc = $rustcVersion
        files = $fileHashes
    }
    Write-Utf8NoBom `
        (Join-Path $installRoot "engine.installation.json") `
        (($installation | ConvertTo-Json -Depth 8) + "`n")

    $manifestPath = Join-Path $installRoot "engine.installation.json"
    $manifestHash =
        (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8NoBom `
        (Join-Path $workingReleaseRoot "engine.installation.json.sha256") `
        "$manifestHash  $platform/engine.installation.json`n"

    # Publish the complete version directory in one same-volume rename only
    # after the platform tree and its manifest sidecar are final.
    Move-Item -LiteralPath $workingReleaseRoot -Destination $releaseRoot

    Write-Host "`nEngine installation: $finalRoot"
    Write-Host "Manifest SHA-256: $manifestHash"
}
finally {
    Pop-Location
    if ($null -ne $stagingRoot -and
        (Test-Path -LiteralPath $stagingRoot) -and
        $stagingRoot.StartsWith(($OutputRoot.TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $stagingRoot -Recurse -Force
    }
}
