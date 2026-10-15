param(
    [ValidateSet("x64", "arm64")]
    [string]$Architecture = "arm64",
    [string]$SherpaVersion = "v1.13.8",
    [string]$WorkRoot = "",
    [string]$LibStageDir = "",
    [string]$RuntimeStageDir = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$assets = @{
    x64 = @{
        Name = "sherpa-onnx-v1.13.8-win-x64-shared-MT-Release-lib.tar.bz2"
        Sha256 = "b8eedf41bd6d3779218887b48367bb7a3ece5aaa7667f01f69ee823a12b0a9e7"
    }
    arm64 = @{
        Name = "sherpa-onnx-v1.13.8-win-arm64-shared-MT-Release-lib.tar.bz2"
        Sha256 = "22cd2b2b5e35c1132abc74a91ee77f683645437382617af6a4bd5818b9c509f4"
    }
}

if ($SherpaVersion -ne "v1.13.8") {
    throw "Unsupported sherpa-onnx version '$SherpaVersion'. Update the pinned asset names and SHA-256 hashes before changing versions."
}

function Resolve-AbsolutePath {
    param([Parameter(Mandatory)][string]$PathValue)
    return [System.IO.Path]::GetFullPath($PathValue)
}

function Reset-Directory {
    param([Parameter(Mandatory)][string]$PathValue)

    if (Test-Path -LiteralPath $PathValue) {
        Remove-Item -LiteralPath $PathValue -Recurse -Force
    }
    New-Item -ItemType Directory -Path $PathValue -Force | Out-Null
}

function Assert-FilesExist {
    param(
        [Parameter(Mandatory)][string]$BaseDir,
        [Parameter(Mandatory)][string[]]$RequiredFiles
    )

    $missing = $RequiredFiles | Where-Object {
        -not (Test-Path -LiteralPath (Join-Path $BaseDir $_))
    }
    if ($missing) {
        throw "Incomplete sherpa-onnx runtime in ${BaseDir}. Missing: $($missing -join ', ')"
    }
}

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(ValueFromRemainingArguments = $true)][object[]]$Arguments
    )

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath failed with exit code $LASTEXITCODE."
    }
}

$repoRoot = Resolve-AbsolutePath (Join-Path $PSScriptRoot "..")
if ([string]::IsNullOrWhiteSpace($WorkRoot)) {
    $WorkRoot = Join-Path $repoRoot ".tools\sherpa-$Architecture-cpu-shared"
}
if ([string]::IsNullOrWhiteSpace($LibStageDir)) {
    $LibStageDir = Join-Path $WorkRoot "install\lib"
}

$WorkRoot = Resolve-AbsolutePath $WorkRoot
$LibStageDir = Resolve-AbsolutePath $LibStageDir
if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    $RuntimeStageDir = Resolve-AbsolutePath $RuntimeStageDir
}

$asset = $assets[$Architecture]
$archivePath = Join-Path $WorkRoot $asset.Name
$extractDir = Join-Path $WorkRoot "extract"
$stampPath = Join-Path $LibStageDir ".selah-sherpa-runtime.json"
$expectedStamp = [ordered]@{
    version = $SherpaVersion
    architecture = $Architecture
    asset = $asset.Name
    sha256 = $asset.Sha256
}
$requiredLibFiles = @(
    "sherpa-onnx-c-api.dll",
    "sherpa-onnx-c-api.lib",
    "onnxruntime.dll",
    "onnxruntime.lib"
)
$requiredRuntimeFiles = @(
    "sherpa-onnx-c-api.dll",
    "onnxruntime.dll"
)

$runtimeIsCurrent = $false
if (Test-Path -LiteralPath $stampPath) {
    try {
        $stamp = Get-Content -LiteralPath $stampPath -Raw | ConvertFrom-Json
        $runtimeIsCurrent = (
            $stamp.version -eq $expectedStamp.version -and
            $stamp.architecture -eq $expectedStamp.architecture -and
            $stamp.asset -eq $expectedStamp.asset -and
            $stamp.sha256 -eq $expectedStamp.sha256
        )
        foreach ($name in $requiredLibFiles) {
            if (-not (Test-Path -LiteralPath (Join-Path $LibStageDir $name))) {
                $runtimeIsCurrent = $false
            }
        }
    } catch {
        $runtimeIsCurrent = $false
    }
}

if (-not $runtimeIsCurrent) {
    New-Item -ItemType Directory -Path $WorkRoot -Force | Out-Null

    $archiveIsCurrent = $false
    if (Test-Path -LiteralPath $archivePath) {
        $archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        $archiveIsCurrent = $archiveHash -eq $asset.Sha256
    }

    if (-not $archiveIsCurrent) {
        $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
        if (-not $curl) {
            throw "curl.exe is required to download the pinned sherpa-onnx Windows runtime."
        }
        $url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/$SherpaVersion/$($asset.Name)"
        $partialPath = "$archivePath.part"
        Remove-Item -LiteralPath $partialPath -Force -ErrorAction SilentlyContinue
        Write-Host "Downloading sherpa-onnx $SherpaVersion Windows $Architecture CPU runtime..."
        Invoke-CheckedCommand $curl.Source -L --fail --retry 3 --output $partialPath $url
        $downloadHash = (Get-FileHash -LiteralPath $partialPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($downloadHash -ne $asset.Sha256) {
            Remove-Item -LiteralPath $partialPath -Force -ErrorAction SilentlyContinue
            throw "SHA-256 mismatch for $($asset.Name). Expected $($asset.Sha256), got $downloadHash."
        }
        Move-Item -LiteralPath $partialPath -Destination $archivePath -Force
    }

    $tar = Get-Command tar.exe -ErrorAction SilentlyContinue
    if (-not $tar) {
        throw "tar.exe is required to extract the sherpa-onnx Windows runtime."
    }

    Reset-Directory $extractDir
    Invoke-CheckedCommand $tar.Source -xf $archivePath -C $extractDir
    $sourceLibDir = Get-ChildItem -LiteralPath $extractDir -Directory |
        ForEach-Object { Join-Path $_.FullName "lib" } |
        Where-Object { Test-Path -LiteralPath $_ } |
        Select-Object -First 1
    if (-not $sourceLibDir) {
        throw "The sherpa-onnx archive does not contain the expected lib directory."
    }

    Assert-FilesExist -BaseDir $sourceLibDir -RequiredFiles $requiredLibFiles
    Reset-Directory $LibStageDir
    Get-ChildItem -LiteralPath $sourceLibDir -File |
        Copy-Item -Destination $LibStageDir -Force
    ($expectedStamp | ConvertTo-Json) | Set-Content -LiteralPath $stampPath -Encoding UTF8
}

Assert-FilesExist -BaseDir $LibStageDir -RequiredFiles $requiredLibFiles

if (-not [string]::IsNullOrWhiteSpace($RuntimeStageDir)) {
    Reset-Directory $RuntimeStageDir
    foreach ($name in $requiredRuntimeFiles) {
        Copy-Item -LiteralPath (Join-Path $LibStageDir $name) -Destination $RuntimeStageDir -Force
    }
    Set-Content -LiteralPath (Join-Path $RuntimeStageDir ".gitkeep") -Value "" -Encoding ASCII
    Assert-FilesExist -BaseDir $RuntimeStageDir -RequiredFiles $requiredRuntimeFiles
}

Write-Host "sherpa-onnx Windows $Architecture CPU runtime is ready."
Write-Host "SHERPA_ONNX_LIB_DIR=$LibStageDir"
