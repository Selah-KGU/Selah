param(
    [string]$SherpaVersion = "v1.12.39",
    [string]$OnnxRuntimeDirectMLVersion = "1.24.4",
    [string]$DirectMLVersion = "1.15.4",
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

function Convert-ToCMakePath {
    param([string]$PathValue)

    return $PathValue.Replace("\", "/")
}

function Save-NuGetPackage {
    param(
        [string]$PackageId,
        [string]$Version,
        [string]$DestinationDir
    )

    New-Item -ItemType Directory -Path $DestinationDir -Force | Out-Null
    $lowerId = $PackageId.ToLowerInvariant()
    # NuGet packages are zip archives. Keep a .zip extension for local
    # FetchContent URLs because CMake does not infer .nupkg extraction.
    $destination = Join-Path $DestinationDir "$lowerId.$Version.zip"
    $url = "https://globalcdn.nuget.org/packages/$lowerId.$Version.nupkg"
    Write-Host "Downloading $PackageId $Version"
    Invoke-WebRequest -Uri $url -OutFile $destination
    $hash = (Get-FileHash -Algorithm SHA256 -Path $destination).Hash.ToLowerInvariant()
    return [pscustomobject]@{
        Path = (Resolve-AbsolutePath $destination)
        Hash = $hash
    }
}

function Update-DirectMLCMakePackageVersions {
    param(
        [string]$SourceDir,
        [string]$OnnxRuntimePackagePath,
        [string]$OnnxRuntimePackageHash,
        [string]$DirectMLPackagePath,
        [string]$DirectMLPackageHash
    )

    $cmakePath = Join-Path $SourceDir "cmake\onnxruntime-win-x64-directml.cmake"
    $content = Get-Content -LiteralPath $cmakePath -Raw
    $onnxRuntimePath = Convert-ToCMakePath $OnnxRuntimePackagePath
    $directMLPath = Convert-ToCMakePath $DirectMLPackagePath

    $content = $content -replace 'set\(onnxruntime_URL\s+"[^"]+"\)', "set(onnxruntime_URL  `"$onnxRuntimePath`")"
    $content = $content -replace 'set\(onnxruntime_URL2\s+"[^"]+"\)', 'set(onnxruntime_URL2 "")'
    $content = $content -replace 'set\(onnxruntime_HASH\s+"SHA256=[0-9a-fA-F]+"\)', "set(onnxruntime_HASH `"SHA256=$OnnxRuntimePackageHash`")"
    $content = $content -replace 'microsoft\.ml\.onnxruntime\.directml\.[0-9.]+\.nupkg', ([System.IO.Path]::GetFileName($OnnxRuntimePackagePath))

    $content = $content -replace 'set\(directml_URL\s+"[^"]+"\)', "set(directml_URL `"$directMLPath`")"
    $content = $content -replace 'set\(directml_HASH\s+"SHA256=[0-9a-fA-F]+"\)', "set(directml_HASH `"SHA256=$DirectMLPackageHash`")"
    $content = $content -replace 'Microsoft\.AI\.DirectML\.[0-9.]+\.nupkg', ([System.IO.Path]::GetFileName($DirectMLPackagePath))

    Set-Content -LiteralPath $cmakePath -Value $content -NoNewline
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
Write-Host "  onnxruntime directml: $OnnxRuntimeDirectMLVersion"
Write-Host "  directml: $DirectMLVersion"
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

$downloadDir = Join-Path $WorkRoot "downloads"
$onnxRuntimePackage = Save-NuGetPackage `
    -PackageId "Microsoft.ML.OnnxRuntime.DirectML" `
    -Version $OnnxRuntimeDirectMLVersion `
    -DestinationDir $downloadDir
$directMLPackage = Save-NuGetPackage `
    -PackageId "Microsoft.AI.DirectML" `
    -Version $DirectMLVersion `
    -DestinationDir $downloadDir

function Invoke-GitCloneWithRetry {
    param(
        [string]$Branch,
        [string]$Url,
        [string]$Destination,
        [int]$MaxAttempts = 5
    )

    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        if (Test-Path $Destination) {
            Remove-Item $Destination -Recurse -Force
        }

        Write-Host "git clone attempt $attempt of $MaxAttempts"
        & git -c http.postBuffer=524288000 -c http.lowSpeedLimit=1000 -c http.lowSpeedTime=60 `
            clone --depth 1 --branch $Branch $Url $Destination
        if ($LASTEXITCODE -eq 0) {
            return
        }

        Write-Warning "git clone failed with exit code $LASTEXITCODE"
        if ($attempt -lt $MaxAttempts) {
            $delay = [Math]::Min(30, [Math]::Pow(2, $attempt))
            Write-Host "Retrying in $delay seconds..."
            Start-Sleep -Seconds $delay
        }
    }

    throw "git clone $Url (branch $Branch) failed after $MaxAttempts attempts"
}

Invoke-GitCloneWithRetry -Branch $SherpaVersion -Url "https://github.com/k2-fsa/sherpa-onnx.git" -Destination $sourceDir

Update-DirectMLCMakePackageVersions `
    -SourceDir $sourceDir `
    -OnnxRuntimePackagePath $onnxRuntimePackage.Path `
    -OnnxRuntimePackageHash $onnxRuntimePackage.Hash `
    -DirectMLPackagePath $directMLPackage.Path `
    -DirectMLPackageHash $directMLPackage.Hash

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
    Set-Content -LiteralPath (Join-Path $RuntimeStageDir ".gitkeep") -Value ""
}

Copy-IfExists (Join-Path $installDir "lib\*.lib") $LibStageDir
Copy-IfExists (Join-Path $installDir "lib\*.dll") $LibStageDir
Copy-IfExists (Join-Path $installDir "bin\*.dll") $LibStageDir
Copy-IfExists (Join-Path $buildDir "_deps\onnxruntime-src\runtimes\win-x64\native\*.dll") $LibStageDir
if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    Copy-IfExists (Join-Path $LibStageDir "*.dll") $RuntimeStageDir
}

Assert-FilesExist $LibStageDir @(
    "sherpa-onnx-c-api.lib",
    "sherpa-onnx-c-api.dll",
    "onnxruntime.lib",
    "onnxruntime.dll",
    "onnxruntime_providers_shared.dll",
    "DirectML.lib",
    "DirectML.dll"
)

if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    Assert-FilesExist $RuntimeStageDir @(
        "sherpa-onnx-c-api.dll",
        "onnxruntime.dll",
        "onnxruntime_providers_shared.dll",
        "DirectML.dll"
    )
}

Write-Host ""
Write-Host "DirectML runtime is ready."
Write-Host "SHERPA_ONNX_LIB_DIR=$LibStageDir"
if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    Write-Host "WINDOWS_RUNTIME_DIR=$RuntimeStageDir"
}
