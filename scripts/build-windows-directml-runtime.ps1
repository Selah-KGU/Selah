param(
    [string]$SherpaVersion = "v1.12.39",
    [string]$WorkRoot = "",
    [string]$LibStageDir = "",
    [string]$RuntimeStageDir = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Resolve-AbsolutePath {
    param([string]$PathValue)

    if ([string]::IsNullOrWhiteSpace($PathValue)) {
        return $null
    }

    return [System.IO.Path]::GetFullPath($PathValue)
}

function Reset-Directory {
    param([string]$PathValue)

    if (Test-Path $PathValue) {
        Remove-Item $PathValue -Recurse -Force
    }

    New-Item -ItemType Directory -Path $PathValue -Force | Out-Null
}

function Copy-IfExists {
    param(
        [string]$Pattern,
        [string]$Destination
    )

    $items = Get-ChildItem -Path $Pattern -ErrorAction SilentlyContinue
    foreach ($item in $items) {
        Copy-Item $item.FullName -Destination $Destination -Force
    }
}

function Assert-FilesExist {
    param(
        [string]$BaseDir,
        [string[]]$RequiredFiles
    )

    $missing = @()
    foreach ($name in $RequiredFiles) {
        if (-not (Test-Path (Join-Path $BaseDir $name))) {
            $missing += $name
        }
    }

    if ($missing.Count -gt 0) {
        throw "Missing expected DirectML runtime files in ${BaseDir}: $($missing -join ', ')"
    }
}

$repoRoot = Resolve-AbsolutePath (Join-Path $PSScriptRoot "..")

if ([string]::IsNullOrWhiteSpace($WorkRoot)) {
    $WorkRoot = Join-Path $repoRoot ".cache\windows-directml"
}
if ([string]::IsNullOrWhiteSpace($LibStageDir)) {
    $LibStageDir = Join-Path $WorkRoot "lib"
}

$WorkRoot = Resolve-AbsolutePath $WorkRoot
$LibStageDir = Resolve-AbsolutePath $LibStageDir
if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    $RuntimeStageDir = Resolve-AbsolutePath $RuntimeStageDir
}

$sourceDir = Join-Path $WorkRoot "sherpa-onnx"
$buildDir = Join-Path $WorkRoot "build"
$installDir = Join-Path $WorkRoot "install"

Write-Host "Preparing sherpa-onnx DirectML runtime"
Write-Host "  version: $SherpaVersion"
Write-Host "  work root: $WorkRoot"
Write-Host "  lib stage: $LibStageDir"
if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    Write-Host "  runtime stage: $RuntimeStageDir"
}

Reset-Directory $WorkRoot
New-Item -ItemType Directory -Path $LibStageDir -Force | Out-Null
if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    New-Item -ItemType Directory -Path $RuntimeStageDir -Force | Out-Null
}

git clone --depth 1 --branch $SherpaVersion https://github.com/k2-fsa/sherpa-onnx.git $sourceDir

$cmakeConfigureArgs = @(
    "-S", $sourceDir,
    "-B", $buildDir,
    "-A", "x64",
    "-DCMAKE_BUILD_TYPE=Release",
    "-DCMAKE_INSTALL_PREFIX=$installDir",
    "-DBUILD_SHARED_LIBS=ON",
    "-DSHERPA_ONNX_ENABLE_C_API=ON",
    "-DSHERPA_ONNX_ENABLE_DIRECTML=ON",
    "-DSHERPA_ONNX_USE_STATIC_CRT=ON",
    "-DSHERPA_ONNX_ENABLE_BINARY=OFF",
    "-DSHERPA_ONNX_BUILD_C_API_EXAMPLES=OFF",
    "-DSHERPA_ONNX_ENABLE_PORTAUDIO=OFF",
    "-DSHERPA_ONNX_ENABLE_TTS=OFF",
    "-DSHERPA_ONNX_ENABLE_SPEAKER_DIARIZATION=OFF",
    "-DSHERPA_ONNX_ENABLE_WEBSOCKET=OFF",
    "-DSHERPA_ONNX_ENABLE_TESTS=OFF",
    "-DSHERPA_ONNX_USE_PRE_INSTALLED_ONNXRUNTIME_IF_AVAILABLE=OFF"
)

& cmake @cmakeConfigureArgs
& cmake --build $buildDir --config Release --target install -- /m:2

Reset-Directory $LibStageDir
if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    Reset-Directory $RuntimeStageDir
}

Copy-IfExists (Join-Path $installDir "lib\*.lib") $LibStageDir
Copy-IfExists (Join-Path $installDir "lib\*.dll") $LibStageDir
Copy-IfExists (Join-Path $installDir "bin\*.dll") $LibStageDir
if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    Copy-IfExists (Join-Path $LibStageDir "*.dll") $RuntimeStageDir
}

Assert-FilesExist $LibStageDir @(
    "sherpa-onnx-c-api.lib",
    "sherpa-onnx-c-api.dll",
    "onnxruntime.lib",
    "onnxruntime.dll",
    "DirectML.lib",
    "DirectML.dll"
)

if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    Assert-FilesExist $RuntimeStageDir @(
        "sherpa-onnx-c-api.dll",
        "onnxruntime.dll",
        "DirectML.dll"
    )
}

Write-Host ""
Write-Host "DirectML runtime is ready."
Write-Host "SHERPA_ONNX_LIB_DIR=$LibStageDir"
if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    Write-Host "WINDOWS_RUNTIME_DIR=$RuntimeStageDir"
}
