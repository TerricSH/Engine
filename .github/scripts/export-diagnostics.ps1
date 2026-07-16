[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$PackageRoot,
    [string]$OutputRoot = "",
    [string]$SymbolRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$PackageRoot = [System.IO.Path]::GetFullPath($PackageRoot)
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path $repoRoot "artifacts\diagnostics"
}
$OutputRoot = [System.IO.Path]::GetFullPath($OutputRoot)

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][AllowEmptyString()][string]$Content
    )
    [System.IO.File]::WriteAllText($Path, $Content, [System.Text.UTF8Encoding]::new($false))
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

$releaseMetadataPath = Join-Path $PackageRoot "manifests\release.json"
if (-not (Test-Path -LiteralPath $releaseMetadataPath -PathType Leaf)) {
    throw "Package release metadata is missing: $releaseMetadataPath"
}
$releaseMetadata = Get-Content -LiteralPath $releaseMetadataPath -Raw | ConvertFrom-Json
if ($releaseMetadata.schema -ne "ReleaseMetadata-v0") {
    throw "Unsupported release metadata schema: $($releaseMetadata.schema)"
}
if ([string]::IsNullOrWhiteSpace($SymbolRoot)) {
    $SymbolRoot = Join-Path (Split-Path -Parent $PackageRoot) "$($releaseMetadata.platform)-symbols"
}
$SymbolRoot = [System.IO.Path]::GetFullPath($SymbolRoot)

$bundleName = "$($releaseMetadata.release_id)-$($releaseMetadata.platform)-diagnostics"
$stagingRoot = [System.IO.Path]::GetFullPath((Join-Path $OutputRoot $bundleName))
if (-not $stagingRoot.StartsWith(($OutputRoot.TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to stage diagnostics outside output root: $stagingRoot"
}
if (Test-Path -LiteralPath $stagingRoot) {
    Remove-Item -LiteralPath $stagingRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stagingRoot | Out-Null

foreach ($directoryName in @("logs", "manifests", "checksums", "config")) {
    $source = Join-Path $PackageRoot $directoryName
    if (Test-Path -LiteralPath $source -PathType Container) {
        Copy-Item -LiteralPath $source -Destination (Join-Path $stagingRoot $directoryName) -Recurse
    }
}

$symbolManifestPath = Join-Path $SymbolRoot "symbols.json"
if (-not (Test-Path -LiteralPath $symbolManifestPath -PathType Leaf)) {
    throw "The sidecar symbol manifest is missing: $symbolManifestPath"
}
$symbolManifest = Get-Content -LiteralPath $symbolManifestPath -Raw | ConvertFrom-Json
if (
    $symbolManifest.schema -ne "SymbolManifest-v0" -or
    $symbolManifest.release_id -ne $releaseMetadata.release_id -or
    $symbolManifest.platform -ne $releaseMetadata.platform
) {
    throw "The sidecar symbol manifest does not match the runtime package"
}
$pdbPath = Join-Path $SymbolRoot $symbolManifest.pdb.path
$executablePath = Join-Path $PackageRoot "binaries\sandbox.exe"
if (-not (Test-Path -LiteralPath $pdbPath -PathType Leaf)) {
    throw "The sidecar PDB is missing: $pdbPath"
}
$pdbHash = (Get-FileHash -LiteralPath $pdbPath -Algorithm SHA256).Hash.ToLowerInvariant()
$executableHash = (Get-FileHash -LiteralPath $executablePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($pdbHash -ne $symbolManifest.pdb.sha256 -or $executableHash -ne $symbolManifest.executable.sha256) {
    throw "The sidecar symbols are not linked to the supplied runtime package"
}
$diagnosticSymbolDir = New-Item -ItemType Directory -Force -Path (Join-Path $stagingRoot "symbols")
Copy-Item -LiteralPath $symbolManifestPath -Destination (Join-Path $diagnosticSymbolDir "symbols.json")
$symbols = @([ordered]@{
    bundle = $releaseMetadata.symbol_bundle
    manifest = "symbols/symbols.json"
    pdb = $symbolManifest.pdb
})

$logFiles = @(
    Get-ChildItem -LiteralPath (Join-Path $stagingRoot "logs") -Recurse -File -ErrorAction SilentlyContinue |
        Sort-Object FullName |
        ForEach-Object { Get-RelativePath $stagingRoot $_.FullName }
)
$index = [ordered]@{
    schema = "DiagnosticBundle-v0"
    release_id = $releaseMetadata.release_id
    platform = $releaseMetadata.platform
    commit = $releaseMetadata.commit
    generated_utc = [DateTimeOffset]::UtcNow.ToString("O")
    logs = $logFiles
    symbol_archive = $symbols
}
Write-Utf8NoBom (Join-Path $stagingRoot "diagnostics.json") (($index | ConvertTo-Json -Depth 8) + "`n")

$archivePath = Join-Path $OutputRoot "$bundleName.zip"
New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
if (Test-Path -LiteralPath $archivePath) {
    Remove-Item -LiteralPath $archivePath -Force
}
Compress-Archive -LiteralPath $stagingRoot -DestinationPath $archivePath -CompressionLevel Optimal
$hash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Utf8NoBom "$archivePath.sha256" "$hash  $([System.IO.Path]::GetFileName($archivePath))`n"

Write-Host "Diagnostic bundle: $archivePath"
Write-Host "Release ID: $($releaseMetadata.release_id)"
Write-Host "Logs: $($logFiles.Count)"
Write-Host "Sidecar symbols referenced: $($symbols.Count)"
Write-Host "SHA-256: $hash"
