[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Debug",
    [int]$Frames = 120,
    [double]$MaxAverageCpuMs = 50.0,
    [string]$Output = "",
    [string]$ProjectPath = "",
    [string]$ProjectOutput = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Test-Path Variable:\PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $true
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
if ([string]::IsNullOrWhiteSpace($Output)) {
    $Output = Join-Path $repoRoot "artifacts\qa\headless-scene.json"
}
$Output = [System.IO.Path]::GetFullPath($Output)
if ([string]::IsNullOrWhiteSpace($ProjectPath)) {
    $ProjectPath = Join-Path $repoRoot "examples\minimal-game\game.project.json"
}
$ProjectPath = [System.IO.Path]::GetFullPath($ProjectPath)
if ([string]::IsNullOrWhiteSpace($ProjectOutput)) {
    $ProjectOutput = Join-Path ([System.IO.Path]::GetDirectoryName($Output)) "project-run.json"
}
$ProjectOutput = [System.IO.Path]::GetFullPath($ProjectOutput)
$arguments = @("run", "--locked")
if ($Configuration -eq "Release") {
    $arguments += "--release"
}
$arguments += @(
    "-p", "sandbox", "--", "qa-headless",
    "--frames", $Frames.ToString([Globalization.CultureInfo]::InvariantCulture),
    "--max-average-cpu-ms", $MaxAverageCpuMs.ToString([Globalization.CultureInfo]::InvariantCulture),
    "--output", $Output
)

Push-Location $repoRoot
try {
    $cargo = (Get-Command cargo -ErrorAction Stop).Source
    & $cargo @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Headless scene QA failed with exit code $LASTEXITCODE"
    }
    if (-not (Test-Path -LiteralPath $Output -PathType Leaf)) {
        throw "Headless scene QA did not produce its report: $Output"
    }
    $report = Get-Content -LiteralPath $Output -Raw | ConvertFrom-Json
    if ($report.schema -ne "QaReport-v0" -or -not $report.passed) {
        throw "Headless scene QA report is invalid or failed"
    }

    $projectCommandPrefix = @("run", "--locked")
    if ($Configuration -eq "Release") {
        $projectCommandPrefix += "--release"
    }
    $projectArguments = $projectCommandPrefix + @("-p", "sandbox", "--", "project", "check", $ProjectPath)
    & $cargo @projectArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Game project validation failed with exit code $LASTEXITCODE"
    }
    $projectArguments = $projectCommandPrefix + @("-p", "sandbox", "--", "project", "cook", $ProjectPath)
    & $cargo @projectArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Game project asset cook failed with exit code $LASTEXITCODE"
    }
    $projectArguments = $projectCommandPrefix + @(
        "-p", "sandbox", "--", "game", $ProjectPath, "--headless",
        "--frames", $Frames.ToString([Globalization.CultureInfo]::InvariantCulture),
        "--report", $ProjectOutput
    )
    & $cargo @projectArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Game project QA failed with exit code $LASTEXITCODE"
    }
    $projectReport = Get-Content -LiteralPath $ProjectOutput -Raw | ConvertFrom-Json
    if (
        $projectReport.schema -ne "ProjectRunReport-v0" -or
        -not $projectReport.passed -or
        [long]$projectReport.total_draw_calls -lt 1
    ) {
        throw "Game project QA report is invalid or failed"
    }
    Write-Host "QA report: $Output"
    Write-Host "Project QA report: $ProjectOutput"
}
finally {
    Pop-Location
}
