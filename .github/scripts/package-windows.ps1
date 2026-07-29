[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$OutputRoot = "",
    [string]$CargoTargetDir = "",
    [string]$ProjectPath = "",
    # Installed editors pass their immutable distribution root. In this mode
    # packaging consumes prebuilt tools and never invokes Cargo or Git.
    [string]$EngineInstallRoot = "",
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
$installedMode = -not [string]::IsNullOrWhiteSpace($EngineInstallRoot)
$cargoTargetWasRequested = -not [string]::IsNullOrWhiteSpace($CargoTargetDir)
if ($installedMode -and $cargoTargetWasRequested) {
    throw "Installed-engine packaging uses prebuilt tools and does not accept -CargoTargetDir"
}
if ($installedMode -and $SkipBuild) {
    throw "Installed-engine packaging is already prebuilt and does not accept -SkipBuild"
}
if ($installedMode -and $AllowDirty) {
    throw "Installed-engine packaging has immutable verified tools and does not accept -AllowDirty"
}
$platform = "windows-x86_64"

if ([string]::IsNullOrWhiteSpace($ProjectPath)) {
    if ($installedMode) {
        throw "Installed-engine packaging requires -ProjectPath"
    }
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
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = if ($installedMode) {
        Join-Path $projectRoot "Dist"
    }
    else {
        Join-Path $repoRoot "artifacts\release"
    }
}
$OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)
if ($installedMode) {
    $projectPrefix = $projectRoot.TrimEnd('\') + '\'
    if ($OutputRoot -eq $projectRoot -or
        -not $OutputRoot.StartsWith($projectPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Installed-engine package output must be a dedicated directory inside the project workspace"
    }
    $ancestor = $OutputRoot
    while ($ancestor.StartsWith($projectPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        if (Test-Path -LiteralPath $ancestor) {
            $ancestorItem = Get-Item -LiteralPath $ancestor -Force
            if (($ancestorItem.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Installed-engine package output may not traverse a reparse point: $ancestor"
            }
        }
        $ancestor = Split-Path -Parent $ancestor
    }
}
if ([string]::IsNullOrWhiteSpace($CargoTargetDir)) {
    $CargoTargetDir = Join-Path $repoRoot "target"
}
$CargoTargetDir = [System.IO.Path]::GetFullPath($CargoTargetDir)
$cargoReleaseDir = Join-Path $CargoTargetDir "release"

function ConvertFrom-JsonStringToken {
    param(
        [Parameter(Mandatory)][string]$Token
    )
    $builder = [System.Text.StringBuilder]::new()
    for ($index = 1; $index -lt $Token.Length - 1; $index += 1) {
        $character = $Token[$index]
        if ($character -ne '\') {
            [void]$builder.Append($character)
            continue
        }
        $index += 1
        if ($index -ge $Token.Length - 1) {
            throw "JSON string ends with an incomplete escape"
        }
        $escaped = $Token[$index]
        switch ($escaped) {
            '"' { [void]$builder.Append('"') }
            '\' { [void]$builder.Append('\') }
            '/' { [void]$builder.Append('/') }
            'b' { [void]$builder.Append([char]0x08) }
            'f' { [void]$builder.Append([char]0x0c) }
            'n' { [void]$builder.Append([char]0x0a) }
            'r' { [void]$builder.Append([char]0x0d) }
            't' { [void]$builder.Append([char]0x09) }
            'u' {
                if ($index + 4 -ge $Token.Length) {
                    throw "JSON string contains an incomplete Unicode escape"
                }
                $hex = $Token.Substring($index + 1, 4)
                if ($hex -notmatch '^[0-9A-Fa-f]{4}$') {
                    throw "JSON string contains an invalid Unicode escape '\u$hex'"
                }
                [void]$builder.Append([char][Convert]::ToInt32($hex, 16))
                $index += 4
            }
            default {
                throw "JSON string contains an invalid escape '\$escaped'"
            }
        }
    }
    return $builder.ToString()
}

function Assert-NoDuplicateJsonObjectKeys {
    param(
        [Parameter(Mandatory)][string]$Json,
        [Parameter(Mandatory)][string]$Label
    )
    $contexts = [System.Collections.Generic.Stack[object]]::new()
    for ($index = 0; $index -lt $Json.Length; $index += 1) {
        $character = $Json[$index]
        if ($character -eq '{') {
            $contexts.Push([PSCustomObject]@{
                kind = "object"
                keys = [System.Collections.Generic.HashSet[string]]::new(
                    [StringComparer]::OrdinalIgnoreCase
                )
            })
            continue
        }
        if ($character -eq '[') {
            $contexts.Push([PSCustomObject]@{ kind = "array"; keys = $null })
            continue
        }
        if ($character -eq '}' -or $character -eq ']') {
            if ($contexts.Count -gt 0) {
                [void]$contexts.Pop()
            }
            continue
        }
        if ($character -ne '"') {
            continue
        }

        $tokenStart = $index
        $escaped = $false
        for ($index += 1; $index -lt $Json.Length; $index += 1) {
            $character = $Json[$index]
            if ($escaped) {
                $escaped = $false
                continue
            }
            if ($character -eq '\') {
                $escaped = $true
                continue
            }
            if ($character -eq '"') {
                break
            }
        }
        if ($index -ge $Json.Length) {
            throw "$Label contains an unterminated JSON string"
        }
        $lookahead = $index + 1
        while ($lookahead -lt $Json.Length -and [char]::IsWhiteSpace($Json[$lookahead])) {
            $lookahead += 1
        }
        if ($lookahead -ge $Json.Length -or $Json[$lookahead] -ne ':') {
            continue
        }
        if ($contexts.Count -eq 0 -or $contexts.Peek().kind -ne "object") {
            continue
        }
        $token = $Json.Substring($tokenStart, $index - $tokenStart + 1)
        $key = ConvertFrom-JsonStringToken -Token $token
        if (-not $contexts.Peek().keys.Add($key)) {
            throw "$Label contains a duplicate or case-conflicting object key '$key'"
        }
    }
}

$projectManifestJson = Get-Content -LiteralPath $ProjectPath -Raw
Assert-NoDuplicateJsonObjectKeys -Json $projectManifestJson -Label "Game project manifest"
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
$assetSourceProperty = $projectManifest.PSObject.Properties["asset_source"]
$assetSourceRelative = if ($null -eq $assetSourceProperty -or
    [string]::IsNullOrWhiteSpace([string]$assetSourceProperty.Value)) {
    "assets/source"
}
else {
    [string]$assetSourceProperty.Value
}
$cookedAssetsProperty = $projectManifest.PSObject.Properties["cooked_assets"]
$cookedAssetsRelative = if ($null -eq $cookedAssetsProperty -or
    [string]::IsNullOrWhiteSpace([string]$cookedAssetsProperty.Value)) {
    "build/cooked"
}
else {
    [string]$cookedAssetsProperty.Value
}
$inputActionsProperty = $projectManifest.PSObject.Properties["input_actions"]
$inputActionsRelative = if ($null -eq $inputActionsProperty -or
    [string]::IsNullOrWhiteSpace([string]$inputActionsProperty.Value)) {
    $null
}
else {
    [string]$inputActionsProperty.Value
}

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

function Test-PathTreesOverlap {
    param(
        [Parameter(Mandatory)][string]$Left,
        [Parameter(Mandatory)][string]$Right
    )
    $leftPath = [System.IO.Path]::GetFullPath($Left).TrimEnd('\')
    $rightPath = [System.IO.Path]::GetFullPath($Right).TrimEnd('\')
    if ([string]::Equals($leftPath, $rightPath, [StringComparison]::OrdinalIgnoreCase)) {
        return $true
    }
    $leftPrefix = $leftPath + '\'
    $rightPrefix = $rightPath + '\'
    return $leftPath.StartsWith($rightPrefix, [StringComparison]::OrdinalIgnoreCase) -or
        $rightPath.StartsWith($leftPrefix, [StringComparison]::OrdinalIgnoreCase)
}

function Assert-PackagePathDisjointFromProjectBuild {
    param(
        [Parameter(Mandatory)][string]$Candidate,
        [Parameter(Mandatory)][string]$CandidateLabel,
        [Parameter(Mandatory)][object[]]$ProtectedDirectories
    )
    foreach ($protected in $ProtectedDirectories) {
        if (Test-PathTreesOverlap -Left $Candidate -Right ([string]$protected.path)) {
            throw "$CandidateLabel '$Candidate' overlaps the project-owned $($protected.label) directory '$($protected.path)'; choose a dedicated package output directory"
        }
    }
}

function Assert-NoReparsePointBelowRoot {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][string]$Field
    )
    $rootPath = [System.IO.Path]::GetFullPath($Root)
    $rootPrefix = $rootPath.TrimEnd('\') + '\'
    $cursor = [System.IO.Path]::GetFullPath($Path)
    if (-not $cursor.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Project field '$Field' resolves outside the project root"
    }
    while ($cursor.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Project field '$Field' may not traverse a reparse point: $cursor"
            }
        }
        $cursor = Split-Path -Parent $cursor
    }
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

function Get-InstalledFile {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][object]$Manifest,
        [Parameter(Mandatory)][string]$Field
    )
    $property = $Manifest.PSObject.Properties[$Field]
    if ($null -eq $property -or $property.Value -isnot [string] -or
        [string]::IsNullOrWhiteSpace([string]$property.Value)) {
        throw "Engine installation field '$Field' must be a non-empty relative path"
    }
    $relative = ([string]$property.Value).Replace('\', '/')
    $resolved = Resolve-ProjectRelativePath -Root $Root -RelativePath $relative -Field $Field
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "Engine installation file '$Field' was not found: $resolved"
    }
    $hashProperty = $Manifest.files.PSObject.Properties |
        Where-Object { [string]::Equals($_.Name, $relative, [StringComparison]::Ordinal) } |
        Select-Object -First 1
    if ($null -eq $hashProperty -or [string]$hashProperty.Value -notmatch '^[0-9A-Fa-f]{64}$') {
        throw "Engine installation file '$Field' is not covered by a valid files SHA-256: $relative"
    }
    $actual = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash.ToLowerInvariant()
    $expected = ([string]$hashProperty.Value).ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "Engine installation file '$Field' failed SHA-256 verification: $relative"
    }
    return $resolved
}

$protectedProjectBuildDirectories = @(
    [PSCustomObject]@{
        label = "cooked_assets"
        path = Resolve-ProjectRelativePath `
            -Root $projectRoot `
            -RelativePath $cookedAssetsRelative `
            -Field "cooked_assets"
    },
    [PSCustomObject]@{
        label = "managed script SDK"
        path = [System.IO.Path]::GetFullPath((Join-Path $projectRoot "build\script-sdk"))
    },
    [PSCustomObject]@{
        label = "managed script host"
        path = [System.IO.Path]::GetFullPath((Join-Path $projectRoot "build\script-host"))
    }
)
if ($hasScripts) {
    $scriptAssemblyPathForOutputValidation = Resolve-ProjectRelativePath `
        -Root $projectRoot `
        -RelativePath ([string]$scriptAssemblyProperty.Value) `
        -Field "script_assembly"
    $protectedProjectBuildDirectories += [PSCustomObject]@{
        label = "script_assembly output"
        path = [System.IO.Path]::GetDirectoryName($scriptAssemblyPathForOutputValidation)
    }
}
if ($installedMode) {
    Assert-PackagePathDisjointFromProjectBuild `
        -Candidate $OutputRoot `
        -CandidateLabel "Installed-engine package output root" `
        -ProtectedDirectories $protectedProjectBuildDirectories
}

$engineInstallation = $null
$projectToolExe = $null
$sandboxExe = $null
$sandboxPdb = $null
$assetCookExe = $null
$installationNotices = $null
if ($installedMode) {
    $EngineInstallRoot = [System.IO.Path]::GetFullPath($EngineInstallRoot)
    $installationManifestPath = Join-Path $EngineInstallRoot "engine.installation.json"
    if (-not (Test-Path -LiteralPath $installationManifestPath -PathType Leaf)) {
        throw "Engine installation manifest was not found: $installationManifestPath"
    }
    $engineInstallationJson = Get-Content -LiteralPath $installationManifestPath -Raw
    Assert-NoDuplicateJsonObjectKeys `
        -Json $engineInstallationJson `
        -Label "Engine installation manifest"
    $engineInstallation = $engineInstallationJson | ConvertFrom-Json
    if ($engineInstallation.schema -ne "EngineInstallation-v0") {
        throw "Unsupported engine installation schema '$($engineInstallation.schema)'"
    }
    if ($null -eq $engineInstallation.files -or
        @($engineInstallation.files.PSObject.Properties).Count -eq 0) {
        throw "Engine installation manifest contains no file hashes"
    }
    foreach ($fileHash in $engineInstallation.files.PSObject.Properties) {
        $relative = ([string]$fileHash.Name).Replace('\', '/')
        if ([string]$fileHash.Value -notmatch '^[0-9A-Fa-f]{64}$') {
            throw "Engine installation contains an invalid SHA-256 for '$relative'"
        }
        $file = Resolve-ProjectRelativePath -Root $EngineInstallRoot -RelativePath $relative -Field "files.$relative"
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
            throw "Engine installation file listed in the manifest is missing: $file"
        }
        $actual = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne ([string]$fileHash.Value).ToLowerInvariant()) {
            throw "Engine installation file failed SHA-256 verification: $relative"
        }
    }

    $projectToolExe = Get-InstalledFile -Root $EngineInstallRoot -Manifest $engineInstallation -Field "editor"
    $sandboxExe = Get-InstalledFile -Root $EngineInstallRoot -Manifest $engineInstallation -Field "windows_runtime"
    $sandboxPdb = Get-InstalledFile -Root $EngineInstallRoot -Manifest $engineInstallation -Field "windows_symbols"
    $assetCookExe = Get-InstalledFile -Root $EngineInstallRoot -Manifest $engineInstallation -Field "asset_cooker"
    $installationNotices = Get-InstalledFile -Root $EngineInstallRoot -Manifest $engineInstallation -Field "notices"

    $scriptHostRelative = [string]$engineInstallation.script_host
    $scriptHostRoot = Resolve-ProjectRelativePath -Root $EngineInstallRoot -RelativePath $scriptHostRelative -Field "script_host"
    if (-not (Test-Path -LiteralPath $scriptHostRoot -PathType Container)) {
        throw "Engine installation script host directory was not found: $scriptHostRoot"
    }
    $scriptHostFiles = @(Get-ChildItem -LiteralPath $scriptHostRoot -Force)
    if ($scriptHostFiles.Count -eq 0 -or @($scriptHostFiles | Where-Object { -not $_.PSIsContainer }).Count -eq 0) {
        throw "Engine installation script host directory contains no files: $scriptHostRoot"
    }
    if (@($scriptHostFiles | Where-Object { $_.PSIsContainer -or $_.LinkType }).Count -ne 0) {
        throw "Engine installation script host must contain regular top-level files only: $scriptHostRoot"
    }
    foreach ($hostFile in $scriptHostFiles) {
        $relative = (Get-RelativeReleasePath -Base $EngineInstallRoot -Path $hostFile.FullName).Replace('\', '/')
        $hashProperty = $engineInstallation.files.PSObject.Properties |
            Where-Object { [string]::Equals($_.Name, $relative, [StringComparison]::Ordinal) } |
            Select-Object -First 1
        if ($null -eq $hashProperty) {
            throw "Engine installation script host contains an unlisted file: $relative"
        }
    }
}

$projectScenes = Get-ProjectSceneCatalog -Root $projectRoot -Manifest $projectManifest

$operationRoot = if ($installedMode) { $projectRoot } else { $repoRoot }
$packageStagingRoot = $null
Push-Location $operationRoot
try {
    if ($installedMode) {
        $commit = [string]$engineInstallation.source_commit
        $dirty = $false
        $commitEpoch = [long]$engineInstallation.source_date_epoch
        $rustcVersion = [string]$engineInstallation.rustc
        if ([string]::IsNullOrWhiteSpace($commit) -or
            $commitEpoch -lt 315532800 -or
            $commitEpoch -gt 4354819199 -or
            [string]::IsNullOrWhiteSpace($rustcVersion)) {
            throw "Engine installation provenance metadata is incomplete"
        }
        if ([string]::IsNullOrWhiteSpace($Version)) {
            if (-not [string]::IsNullOrWhiteSpace($env:RELEASE_VERSION)) {
                $Version = $env:RELEASE_VERSION
            }
            else {
                $Version = [string]$engineInstallation.engine_version
            }
        }
    }
    else {
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
    }
    if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$') {
        throw "Invalid release version '$Version'"
    }

    $releaseRoot = [System.IO.Path]::GetFullPath((Join-Path $OutputRoot $Version))
    if ($installedMode) {
        Assert-PackagePathDisjointFromProjectBuild `
            -Candidate $releaseRoot `
            -CandidateLabel "Installed-engine package release directory" `
            -ProtectedDirectories $protectedProjectBuildDirectories
    }
    if (Test-Path -LiteralPath $releaseRoot) {
        throw "Release version already exists and will not be overwritten: $releaseRoot"
    }
    New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
    $packageStagingRoot = Join-Path $OutputRoot (".engine-package-{0}-{1}" -f $PID, [Guid]::NewGuid().ToString("N"))
    $workingReleaseRoot = Join-Path $packageStagingRoot $Version
    $stageRoot = [System.IO.Path]::GetFullPath((Join-Path $workingReleaseRoot $platform))
    $symbolStageName = "$platform-symbols"
    $symbolStageRoot = [System.IO.Path]::GetFullPath((Join-Path $workingReleaseRoot $symbolStageName))
    foreach ($ownedPath in @($stageRoot, $symbolStageRoot)) {
        if (-not $ownedPath.StartsWith(($packageStagingRoot.TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to stage outside output root: $ownedPath"
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
    if ($installedMode) {
        # The distributed player is one stable superset build so every project
        # uses the exact runtime covered by the installation manifest.
        $featureList += @("terrain", "subsystem-scripting-csharp")
    }
    elseif ($hasScripts) {
        $featureList += "subsystem-scripting-csharp"
    }
    $features = $featureList -join ','
    if (-not $installedMode -and -not $SkipBuild) {
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

    if (-not $installedMode) {
        $sandboxExe = Join-Path $cargoReleaseDir "sandbox.exe"
        $projectToolExe = $sandboxExe
        $sandboxPdb = Join-Path $cargoReleaseDir "sandbox.pdb"
        $assetCookExe = Join-Path $cargoReleaseDir "asset-cook.exe"
    }
    if (-not (Test-Path -LiteralPath $sandboxExe -PathType Leaf)) {
        throw "Release executable was not produced: $sandboxExe"
    }
    Copy-Item -LiteralPath $sandboxExe -Destination (Join-Path $binaryDir "sandbox.exe")
    if (-not (Test-Path -LiteralPath $sandboxPdb -PathType Leaf)) {
        throw "Release symbols were not produced: $sandboxPdb"
    }
    $stagedPdb = Join-Path $symbolDir "sandbox.pdb"
    Copy-Item -LiteralPath $sandboxPdb -Destination $stagedPdb

    if (-not $installedMode -and -not (Test-Path -LiteralPath $assetCookExe -PathType Leaf)) {
        throw "Release asset cooker was not produced: $assetCookExe"
    }
    $sourceAssetRoot = Resolve-ProjectRelativePath `
        -Root $projectRoot `
        -RelativePath $assetSourceRelative `
        -Field "asset_source"
    $startupScenePath = $projectScenes.startup.source_path
    $inputActionsPath = $null
    if ($null -ne $inputActionsRelative) {
        $inputActionsPath = Resolve-ProjectRelativePath `
            -Root $projectRoot `
            -RelativePath $inputActionsRelative `
            -Field "input_actions"
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
        Invoke-Native "build game scripts and deploy script host" $projectToolExe @(
            "project", "build-scripts", $ProjectPath
        )
        if (-not (Test-Path -LiteralPath $scriptAssemblyPath -PathType Leaf)) {
            throw "Project script build did not produce the declared assembly: $scriptAssemblyPath"
        }
    }

    $projectCookedAssets = $null
    if ($installedMode) {
        $projectCookedAssets = Resolve-ProjectRelativePath `
            -Root $projectRoot `
            -RelativePath $cookedAssetsRelative `
            -Field "cooked_assets"
        Assert-NoReparsePointBelowRoot `
            -Root $projectRoot `
            -Path $projectCookedAssets `
            -Field "cooked_assets"
        $outputPrefix = $OutputRoot.TrimEnd('\') + '\'
        $cookedPrefix = $projectCookedAssets.TrimEnd('\') + '\'
        if ($projectCookedAssets -eq $OutputRoot -or
            $projectCookedAssets.StartsWith($outputPrefix, [StringComparison]::OrdinalIgnoreCase) -or
            $OutputRoot.StartsWith($cookedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Project cooked_assets and installed package output must be disjoint directories"
        }
        Invoke-Native "cook project assets with the installed engine registry" $projectToolExe @(
            "project", "cook", $ProjectPath
        )
        if (-not (Test-Path -LiteralPath $projectCookedAssets -PathType Container)) {
            throw "Installed project cook did not produce the configured cooked directory: $projectCookedAssets"
        }
    }

    $projectCheckReport = Join-Path $manifestDir "project-check.json"
    Invoke-Native "validate game project" $projectToolExe @(
        "project", "check", $ProjectPath, "--report", $projectCheckReport
    )
    $checkReport = Get-Content -LiteralPath $projectCheckReport -Raw | ConvertFrom-Json
    if ($checkReport.schema -ne "ProjectCheckReport-v0" -or -not $checkReport.passed) {
        throw "Game project validation did not pass"
    }

    $stagedProjectPath = Join-Path $stageRoot "game.project.json"
    $projectManifest | Add-Member `
        -NotePropertyName "cooked_assets" `
        -NotePropertyValue "assets/cooked" `
        -Force
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
        $stagedInputActionsPath = Join-Path $stageRoot $inputActionsRelative
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $stagedInputActionsPath) | Out-Null
        Copy-Item -LiteralPath $inputActionsPath -Destination $stagedInputActionsPath
    }

    $assetCookReport = Join-Path $manifestDir "asset-cook.json"
    if ($installedMode) {
        $cookedEntries = @(Get-ChildItem -LiteralPath $projectCookedAssets -Force)
        $reparseEntries = @(
            Get-ChildItem -LiteralPath $projectCookedAssets -Recurse -Force |
                Where-Object {
                    ($_.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0
                }
        )
        if ($reparseEntries.Count -ne 0) {
            throw "Configured cooked assets contain a reparse point: $($reparseEntries[0].FullName)"
        }
        foreach ($entry in $cookedEntries) {
            Copy-Item `
                -LiteralPath $entry.FullName `
                -Destination (Join-Path $assetDir.FullName $entry.Name) `
                -Recurse
        }
        if ([long]$checkReport.cooked_assets -ne [long]$checkReport.declared_assets) {
            throw "Installed project cook produced $($checkReport.cooked_assets) cooked artifacts for $($checkReport.declared_assets) declared assets"
        }
        $cookReport = [ordered]@{
            schema = "AssetCookReport-v0"
            source = "installed-project-cook"
            declared_asset_count = [long]$checkReport.declared_assets
            succeeded_asset_count = [long]$checkReport.cooked_assets
            failed_asset_count = 0
            failed_manifest_count = 0
            diagnostics = @()
        }
        Write-Utf8NoBom $assetCookReport (($cookReport | ConvertTo-Json -Depth 5) + "`n")
    }
    else {
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

    if ($installedMode) {
        Copy-Item -LiteralPath $installationNotices -Destination (Join-Path $manifestDir "NOTICES.txt")
    }
    else {
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
    }
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
    if ($installedMode) {
        $releaseMetadata.engine_installation = [ordered]@{
            schema = [string]$engineInstallation.schema
            engine_version = [string]$engineInstallation.engine_version
            manifest_sha256 = (Get-FileHash -LiteralPath $installationManifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
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
    $archivePath = Join-Path $workingReleaseRoot "$platform.zip"
    New-DeterministicZip $stageRoot $platform $archivePath $timestamp
    $symbolArchivePath = Join-Path $workingReleaseRoot "$symbolStageName.zip"
    New-DeterministicZip $symbolDir $symbolStageName $symbolArchivePath $timestamp

    $archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8NoBom "$archivePath.sha256" "$archiveHash  $([System.IO.Path]::GetFileName($archivePath))`n"
    $symbolArchiveHash = (Get-FileHash -LiteralPath $symbolArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Utf8NoBom "$symbolArchivePath.sha256" "$symbolArchiveHash  $([System.IO.Path]::GetFileName($symbolArchivePath))`n"
    if (Test-Path -LiteralPath $releaseRoot) {
        throw "Release version appeared while packaging and will not be overwritten: $releaseRoot"
    }
    Move-Item -LiteralPath $workingReleaseRoot -Destination $releaseRoot
    $archivePath = Join-Path $releaseRoot "$platform.zip"
    $symbolArchivePath = Join-Path $releaseRoot "$symbolStageName.zip"
    Write-Host "`nRelease package: $archivePath"
    Write-Host "SHA-256: $archiveHash"
    Write-Host "Symbol package: $symbolArchivePath"
    Write-Host "Symbol SHA-256: $symbolArchiveHash"
}
finally {
    Pop-Location
    if ($null -ne $packageStagingRoot -and
        (Test-Path -LiteralPath $packageStagingRoot) -and
        $packageStagingRoot.StartsWith(($OutputRoot.TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $packageStagingRoot -Recurse -Force
    }
}
