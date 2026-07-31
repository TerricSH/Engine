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
$variants = @(
    @{
        Name = "vfx_billboard.frag"
        Source = "forward.frag"
        Artifact = "vfx_billboard.frag.spv"
        Arguments = @("-DVFX_PARTICLE=1", "-S", "frag")
    }
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

    foreach ($variant in $variants) {
        Write-Host "==> $($variant.Name) (variant of $($variant.Source))"
        $source = Join-Path $shaderDirectory $variant.Source
        $freshArtifact = Join-Path $tempDirectory ($variant.Name + ".spv")
        $checkedInArtifact = Join-Path $shaderDirectory $variant.Artifact
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Shader variant source is missing: $source"
        }
        if (-not (Test-Path -LiteralPath $checkedInArtifact -PathType Leaf)) {
            throw "Checked-in shader variant is missing: $checkedInArtifact"
        }
        if ((Get-Item -LiteralPath $checkedInArtifact).Length -eq 0) {
            throw "Checked-in shader variant is empty: $checkedInArtifact"
        }

        $compileArguments = @(
            "-V",
            "--target-env",
            "vulkan1.2"
        ) + $variant.Arguments + @(
            $source,
            "-o",
            $freshArtifact
        )
        Invoke-ShaderTool $compiler $compileArguments "compile $($variant.Name)"
        Invoke-ShaderTool $validator @(
            "--target-env",
            "vulkan1.2",
            $freshArtifact
        ) "validate freshly compiled $($variant.Name)"
        Invoke-ShaderTool $validator @(
            "--target-env",
            "vulkan1.2",
            $checkedInArtifact
        ) "validate checked-in $($variant.Artifact)"
    }

    $expectedArtifacts = @(
        $sources | ForEach-Object { $_.Name + ".spv" }
        $variants | ForEach-Object { $_.Artifact }
    )
    $orphanedArtifacts = @(
        Get-ChildItem -LiteralPath $shaderDirectory -File -Filter "*.spv" |
            Where-Object { $_.Name -notin $expectedArtifacts } |
            Sort-Object Name
    )
    if ($orphanedArtifacts.Count -ne 0) {
        throw "SPIR-V artifacts without a verified source recipe: $($orphanedArtifacts.Name -join ', ')"
    }

    $artifactCount = $sources.Count + $variants.Count
    Write-Host "Validated $artifactCount GLSL recipes and every checked-in SPIR-V artifact."
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
