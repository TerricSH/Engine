[CmdletBinding()]
param(
    [ValidateSet(
        "All",
        "Rust",
        "Format",
        "Clippy",
        "FeatureCheck",
        "Test",
        "Fixture",
        "Managed",
        "Shaders",
        "Release"
    )]
    [string]$Task = "All"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (Test-Path Variable:\PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $true
}

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$isWindowsPlatform = $env:OS -eq "Windows_NT"
$manifest = Join-Path $repoRoot "Cargo.toml"
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
    throw "Repository root could not be resolved from $PSScriptRoot"
}

function Invoke-Native {
    param(
        [Parameter(Mandatory)]
        [string]$Label,
        [Parameter(Mandatory)]
        [string]$Command,
        [string[]]$Arguments = @()
    )

    $executable = (Get-Command $Command -ErrorAction Stop).Source
    Write-Host "`n==> $Label"
    & $executable @Arguments
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0) {
        throw "$Label failed with exit code $exitCode"
    }
}

function Invoke-Format {
    Invoke-Native "cargo fmt --check" "cargo" @("fmt", "--all", "--", "--check")
}

function Invoke-Clippy {
    Invoke-Native "cargo clippy --workspace --all-targets -D warnings" "cargo" @(
        "clippy",
        "--workspace",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings"
    )
}

function Invoke-FeatureCheck {
    Invoke-Native "sandbox desktop backend feature check" "cargo" @(
        "check",
        "--locked",
        "-p",
        "sandbox",
        "--features",
        "backend-vulkan,backend-dx12,tooling-editor,target-desktop,subsystem-scripting-csharp"
    )
    Invoke-Native "engine-core Vulkan/C#/gameplay feature check" "cargo" @(
        "check",
        "--locked",
        "-p",
        "engine-core",
        "--features",
        "backend-vulkan,subsystem-scripting-csharp,gameplay"
    )
}

function Invoke-WorkspaceTests {
    Invoke-Native "cargo test --workspace" "cargo" @("test", "--workspace", "--locked")
}

function Invoke-FixtureTest {
    $fixtureFiles = @(
        "assets\models\resource-chain.gltf",
        "assets\models\resource-chain.bin",
        "assets\models\resource-chain.png"
    )
    foreach ($relativePath in $fixtureFiles) {
        $fixture = Join-Path $repoRoot $relativePath
        if (-not (Test-Path -LiteralPath $fixture -PathType Leaf)) {
            throw "Required glTF fixture file is missing: $relativePath"
        }
        if ((Get-Item -LiteralPath $fixture).Length -eq 0) {
            throw "Required glTF fixture file is empty: $relativePath"
        }
    }

    Invoke-Native "fixed glTF resource-chain test" "cargo" @(
        "test",
        "--locked",
        "-p",
        "engine-asset",
        "--lib",
        "gltf::tests::resource_chain_preserves_all_indices_and_instances",
        "--",
        "--exact"
    )
}

function Get-ManagedAssembly {
    param(
        [Parameter(Mandatory)][string]$ArtifactsRoot,
        [Parameter(Mandatory)][string]$AssemblyName
    )

    $binRoot = Join-Path $ArtifactsRoot "bin"
    $candidates = @(
        Get-ChildItem -LiteralPath $binRoot -Recurse -File -Filter "$AssemblyName.dll" |
            Where-Object { $_.FullName -notmatch "[\\/]ref(int)?[\\/]" }
    )
    if ($candidates.Count -ne 1) {
        $found = ($candidates.FullName -join ", ")
        throw "Expected one $AssemblyName.dll in $binRoot, found $($candidates.Count): $found"
    }
    return $candidates[0].FullName
}

function Add-ProcessArgument {
    param(
        [Parameter(Mandatory)][System.Diagnostics.ProcessStartInfo]$StartInfo,
        [Parameter(Mandatory)][string]$Value
    )

    if ($StartInfo.PSObject.Properties.Name -contains "ArgumentList") {
        $StartInfo.ArgumentList.Add($Value)
    }
    else {
        $escaped = $Value.Replace('"', '\"')
        $StartInfo.Arguments += " `"$escaped`""
    }
}

function Invoke-DotnetAssembly {
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][string]$AssemblyPath,
        [string[]]$ProgramArguments = @(),
        [int]$TimeoutSeconds = 60
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = (Get-Command "dotnet" -ErrorAction Stop).Source
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    Add-ProcessArgument $startInfo $AssemblyPath
    foreach ($argument in $ProgramArguments) {
        Add-ProcessArgument $startInfo $argument
    }

    Write-Host "`n==> $Label"
    $process = [System.Diagnostics.Process]::Start($startInfo)
    if ($null -eq $process) {
        throw "Failed to start $Label"
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $process.Kill()
        $process.WaitForExit()
        throw "$Label timed out after $TimeoutSeconds seconds"
    }
    $process.WaitForExit()
    $stdout = $stdoutTask.GetAwaiter().GetResult().Trim()
    $stderr = $stderrTask.GetAwaiter().GetResult().Trim()
    if (-not [string]::IsNullOrWhiteSpace($stdout)) {
        Write-Host $stdout
    }
    if (-not [string]::IsNullOrWhiteSpace($stderr)) {
        Write-Host $stderr
    }
    if ($process.ExitCode -ne 0) {
        throw "$Label failed with exit code $($process.ExitCode)"
    }
}

function Invoke-EngineSampleSmoke {
    param([Parameter(Mandatory)][string]$AssemblyPath)

    $dotnet = (Get-Command "dotnet" -ErrorAction Stop).Source
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $dotnet
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardInput = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in @($AssemblyPath)) {
        Add-ProcessArgument $startInfo $argument
    }

    Write-Host "`n==> EngineSample JSON-line protocol smoke"
    $process = [System.Diagnostics.Process]::Start($startInfo)
    if ($null -eq $process) {
        throw "Failed to start EngineSample"
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.StandardInput.WriteLine('{"type":"Shutdown"}')
    $process.StandardInput.Close()
    if (-not $process.WaitForExit(15000)) {
        $process.Kill()
        $process.WaitForExit()
        throw "EngineSample protocol smoke timed out"
    }
    $process.WaitForExit()
    $stdout = $stdoutTask.GetAwaiter().GetResult().Trim()
    $stderr = $stderrTask.GetAwaiter().GetResult().Trim()
    if ($process.ExitCode -ne 0) {
        throw "EngineSample failed with exit code $($process.ExitCode): $stderr"
    }
    if ([string]::IsNullOrWhiteSpace($stdout)) {
        throw "EngineSample returned no protocol response"
    }
    $response = $stdout | ConvertFrom-Json
    if ($response.type -ne "Shutdown") {
        throw "EngineSample returned an unexpected protocol response: $stdout"
    }
    Write-Host $stdout
}

function Invoke-ManagedChecks {
    if (-not $isWindowsPlatform) {
        throw "The managed/native ABI smoke requires Windows and engine_ffi.dll"
    }

    $managedBase = [System.IO.Path]::GetFullPath((Join-Path $repoRoot "target\ci-managed"))
    $artifactsRoot = Join-Path $managedBase ([Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $artifactsRoot | Out-Null

    $oldCargoTarget = $env:CARGO_TARGET_DIR
    $oldFfiLibrary = $env:ENGINE_FFI_LIBRARY
    $oldFfiHostPid = $env:ENGINE_FFI_HOST_PID
    try {
        $env:CARGO_TARGET_DIR = Join-Path $artifactsRoot "cargo-target"
        Invoke-Native "build isolated native engine_ffi DLL (Release)" "cargo" @(
            "build",
            "--locked",
            "--release",
            "-p",
            "engine-ffi"
        )

        $nativeLibrary = Join-Path $env:CARGO_TARGET_DIR "release\engine_ffi.dll"
        if (-not (Test-Path -LiteralPath $nativeLibrary -PathType Leaf)) {
            throw "Native ABI library was not produced: $nativeLibrary"
        }

        $projects = @(
            "scripts\csharp\Engine.API.Tests\Engine.API.Tests.csproj",
            "scripts\csharp\EngineSample\EngineSample.csproj",
            "scripts\csharp\GameLogic\Sandbox\Sandbox.csproj"
        )
        foreach ($relativeProject in $projects) {
            $project = Join-Path $repoRoot $relativeProject
            Invoke-Native "isolated Release build $relativeProject" "dotnet" @(
                "build",
                $project,
                "--configuration",
                "Release",
                "--artifacts-path",
                $artifactsRoot,
                "--nologo",
                "--warnaserror"
            )
        }

        $apiTests = Get-ManagedAssembly $artifactsRoot "Engine.API.Tests"
        $env:ENGINE_FFI_LIBRARY = $nativeLibrary
        Remove-Item Env:\ENGINE_FFI_HOST_PID -ErrorAction SilentlyContinue
        Invoke-DotnetAssembly "Engine.API native ABI and coroutine smoke" $apiTests
        Invoke-DotnetAssembly "Engine.API missing host PID rejection" $apiTests @(
            "--expect-missing-pid-rejection"
        )
        $env:ENGINE_FFI_HOST_PID = "0"
        Invoke-DotnetAssembly "Engine.API cross-process rejection" $apiTests @(
            "--expect-process-rejection"
        )
        Remove-Item Env:\ENGINE_FFI_LIBRARY -ErrorAction SilentlyContinue
        Remove-Item Env:\ENGINE_FFI_HOST_PID -ErrorAction SilentlyContinue
        Invoke-DotnetAssembly "Engine.API missing native library rejection" $apiTests @(
            "--expect-missing-library-rejection"
        )

        $engineSample = Get-ManagedAssembly $artifactsRoot "EngineSample"
        Invoke-EngineSampleSmoke $engineSample
        $sandbox = Get-ManagedAssembly $artifactsRoot "Sandbox"
        Invoke-DotnetAssembly "C# sandbox smoke" $sandbox
    }
    finally {
        $env:CARGO_TARGET_DIR = $oldCargoTarget
        $env:ENGINE_FFI_LIBRARY = $oldFfiLibrary
        $env:ENGINE_FFI_HOST_PID = $oldFfiHostPid

        $cleanupPath = [System.IO.Path]::GetFullPath($artifactsRoot)
        if (
            $cleanupPath.StartsWith($managedBase, [System.StringComparison]::OrdinalIgnoreCase) -and
            $cleanupPath -ne $managedBase -and
            (Test-Path -LiteralPath $cleanupPath)
        ) {
            Remove-Item -LiteralPath $cleanupPath -Recurse -Force
        }
    }
}

function Invoke-ShaderChecks {
    $shaderScript = Join-Path $PSScriptRoot "verify-shaders.ps1"
    & $shaderScript
    if (-not $?) {
        throw "Shader verification script failed"
    }
    if ($LASTEXITCODE -ne 0) {
        throw "Shader verification script failed with exit code $LASTEXITCODE"
    }
}

function Invoke-ReleaseBuild {
    Invoke-Native "cargo build --workspace --release --locked" "cargo" @(
        "build",
        "--workspace",
        "--release",
        "--locked"
    )
    Invoke-Native "cargo build Vulkan desktop sandbox --release --locked" "cargo" @(
        "build",
        "--release",
        "--locked",
        "-p",
        "sandbox",
        "--features",
        "backend-vulkan,tooling-editor,target-desktop,subsystem-scripting-csharp"
    )
}

Push-Location $repoRoot
try {
    switch ($Task) {
        "Format" { Invoke-Format }
        "Clippy" { Invoke-Clippy }
        "FeatureCheck" { Invoke-FeatureCheck }
        "Test" { Invoke-WorkspaceTests }
        "Fixture" { Invoke-FixtureTest }
        "Managed" { Invoke-ManagedChecks }
        "Shaders" { Invoke-ShaderChecks }
        "Release" { Invoke-ReleaseBuild }
        "Rust" {
            Invoke-Format
            Invoke-Clippy
            Invoke-FeatureCheck
            Invoke-WorkspaceTests
            Invoke-FixtureTest
        }
        "All" {
            Invoke-Format
            Invoke-Clippy
            Invoke-FeatureCheck
            Invoke-WorkspaceTests
            Invoke-FixtureTest
            Invoke-ManagedChecks
            Invoke-ShaderChecks
            Invoke-ReleaseBuild
        }
    }
}
finally {
    Pop-Location
}
