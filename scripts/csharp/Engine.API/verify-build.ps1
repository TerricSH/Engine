[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release"
)

$ErrorActionPreference = "Stop"
$project = Join-Path $PSScriptRoot "Engine.API.csproj"
$tests = Join-Path (Split-Path $PSScriptRoot -Parent) "Engine.API.Tests\Engine.API.Tests.csproj"

dotnet build $project --configuration $Configuration --nologo --warnaserror
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$repository = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
$nativeCandidates = @(
    (Join-Path $repository "target\$($Configuration.ToLowerInvariant())\engine_ffi.dll"),
    (Join-Path $repository "target\debug\engine_ffi.dll")
)
$nativeLibrary = $nativeCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if ($nativeLibrary) {
    $env:ENGINE_FFI_LIBRARY = $nativeLibrary
}

dotnet run --project $tests --configuration $Configuration --no-restore
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

if ($nativeLibrary) {
    Remove-Item Env:ENGINE_FFI_HOST_PID -ErrorAction SilentlyContinue
    dotnet run --project $tests --configuration $Configuration --no-restore -- --expect-missing-pid-rejection
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    $env:ENGINE_FFI_HOST_PID = "0"
    dotnet run --project $tests --configuration $Configuration --no-restore -- --expect-process-rejection
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Remove-Item Env:ENGINE_FFI_LIBRARY -ErrorAction SilentlyContinue
Remove-Item Env:ENGINE_FFI_HOST_PID -ErrorAction SilentlyContinue
dotnet run --project $tests --configuration $Configuration --no-restore -- --expect-missing-library-rejection
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
