[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Test-Path Variable:\PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $true
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$shaderDirectory = Join-Path $repoRoot "crates\render-vulkan\shaders"
if (-not (Test-Path -LiteralPath $shaderDirectory -PathType Container)) {
    throw "Shader directory is missing: $shaderDirectory"
}

function Find-ShaderTool {
    param([Parameter(Mandatory)][string]$Name)

    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -ne $command) {
        return $command.Source
    }
    if (-not [string]::IsNullOrWhiteSpace($env:VULKAN_SDK)) {
        $candidate = Join-Path $env:VULKAN_SDK "Bin\$Name.exe"
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return $candidate
        }
    }
    throw "$Name was not found. Install the Vulkan SDK or add its Bin directory to PATH."
}

function Invoke-ShaderTool {
    param(
        [Parameter(Mandatory)][string]$Tool,
        [Parameter(Mandatory)][string[]]$Arguments,
        [Parameter(Mandatory)][string]$Label
    )

    & $Tool @Arguments
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "$Label failed with exit code $exitCode"
    }
}

$compiler = Find-ShaderTool "glslangValidator"
$validator = Find-ShaderTool "spirv-val"
$sources = @(
    Get-ChildItem -LiteralPath $shaderDirectory -File |
        Where-Object { $_.Extension.ToLowerInvariant() -in @(".vert", ".frag", ".comp") } |
        Sort-Object Name
)
if ($sources.Count -eq 0) {
    throw "No GLSL shader sources were found in $shaderDirectory"
}

$tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$tempDirectory = Join-Path $tempBase ("engine-shader-check-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $tempDirectory | Out-Null

try {
    foreach ($source in $sources) {
        Write-Host "==> $($source.Name)"
        $freshArtifact = Join-Path $tempDirectory ($source.Name + ".spv")
        $checkedInArtifact = $source.FullName + ".spv"
        if (-not (Test-Path -LiteralPath $checkedInArtifact -PathType Leaf)) {
            throw "Checked-in SPIR-V artifact is missing: $checkedInArtifact"
        }
        if ((Get-Item -LiteralPath $checkedInArtifact).Length -eq 0) {
            throw "Checked-in SPIR-V artifact is empty: $checkedInArtifact"
        }

        Invoke-ShaderTool $compiler @(
            "-V",
            "--target-env",
            "vulkan1.2",
            $source.FullName,
            "-o",
            $freshArtifact
        ) "compile $($source.Name)"
        if (-not (Test-Path -LiteralPath $freshArtifact -PathType Leaf)) {
            throw "Shader compiler did not produce $freshArtifact"
        }
        if ((Get-Item -LiteralPath $freshArtifact).Length -eq 0) {
            throw "Fresh SPIR-V artifact is empty: $freshArtifact"
        }

        Invoke-ShaderTool $validator @(
            "--target-env",
            "vulkan1.2",
            $freshArtifact
        ) "validate freshly compiled $($source.Name)"
        Invoke-ShaderTool $validator @(
            "--target-env",
            "vulkan1.2",
            $checkedInArtifact
        ) "validate checked-in $($source.Name).spv"
    }
    Write-Host "Validated $($sources.Count) GLSL sources and their checked-in SPIR-V artifacts."
}
finally {
    $cleanupPath = [System.IO.Path]::GetFullPath($tempDirectory)
    if (
        $cleanupPath.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase) -and
        $cleanupPath -ne $tempBase -and
        (Test-Path -LiteralPath $cleanupPath)
    ) {
        Remove-Item -LiteralPath $cleanupPath -Recurse -Force
    }
}
