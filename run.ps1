#Requires -Version 5.1
<#
.SYNOPSIS
    Selah Windows dev helper  (mirrors run.sh for macOS)

.PARAMETER Command
    setup    Install dev prerequisites (first-time setup)
    dev      Start development server (default)
    directml Build the local Windows DirectML STT runtime cache
    build    Production build  (add --features llm-vulkan for Vulkan GPU)
    clean    Clean all build caches
    rebuild  Clean + build
    kill     Kill running Selah processes
    open     Open last built installer/exe

.EXAMPLE
    .\run.ps1 setup   # first time only
    .\run.ps1
    .\run.ps1 dev
    .\run.ps1 build
    .\run.ps1 clean
#>
param(
    [string]$Command = "dev"
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $ScriptDir

# Keep window open on error when launched by double-click (no parent terminal)
$IsDoubleClick = ($Host.Name -eq "ConsoleHost") -and (-not $env:WT_SESSION) -and (-not $env:TERM_PROGRAM)
trap {
    Write-Host ""
    Write-Host "ERROR: $_" -ForegroundColor Red
    if ($IsDoubleClick) { Read-Host "按 Enter 关闭" }
    exit 1
}

# Explorer double-click doesn't inherit the user's PATH from terminal profiles.
# Manually add Node.js and common tool paths so npm/npx are found.
foreach ($nodeDir in @(
    "$env:ProgramFiles\nodejs",
    "$env:APPDATA\npm",
    "$env:LOCALAPPDATA\Programs\nodejs"
)) {
    if ((Test-Path $nodeDir) -and ($env:PATH -notlike "*$nodeDir*")) {
        $env:PATH = "$nodeDir;$env:PATH"
    }
}

$AppName    = "selah-app"
$BundlePath = "src-tauri\target\release\$AppName.exe"

# ---------------------------------------------------------------------------
# Environment setup
# ---------------------------------------------------------------------------
function Enable-DirectMLIfAvailable {
    if (-not $env:SHERPA_ONNX_LIB_DIR) {
        return
    }

    $required = @(
        "sherpa-onnx-c-api.lib",
        "sherpa-onnx-c-api.dll",
        "onnxruntime.lib",
        "onnxruntime.dll",
        "onnxruntime_providers_shared.dll",
        "DirectML.lib",
        "DirectML.dll"
    )
    $missing = @(
        foreach ($name in $required) {
            if (-not (Test-Path (Join-Path $env:SHERPA_ONNX_LIB_DIR $name))) {
                $name
            }
        }
    )

    if ($missing.Count -eq 0) {
        $env:SELAH_ENABLE_STT_DIRECTML = "1"
        if ($env:PATH -notlike "*$env:SHERPA_ONNX_LIB_DIR*") {
            $env:PATH = "$env:SHERPA_ONNX_LIB_DIR;$env:PATH"
        }
        Write-Host "  SELAH_ENABLE_STT_DIRECTML=1"
    } elseif ($env:SELAH_ENABLE_STT_DIRECTML) {
        Write-Warning "DirectML was requested, but SHERPA_ONNX_LIB_DIR is incomplete: $($missing -join ', ')"
    } else {
        Write-Host "  DirectML STT runtime not enabled (missing $($missing -join ', '))"
    }
}

function Set-BuildEnv {
    # If SHERPA_ONNX_LIB_DIR is already set globally, respect it.
    # Otherwise look for the default local cache produced by build-windows-directml-runtime.ps1
    if (-not $env:SHERPA_ONNX_LIB_DIR) {
        $defaultLibDir = Join-Path $ScriptDir ".cache\windows-directml\lib"
        if (Test-Path $defaultLibDir) {
            $env:SHERPA_ONNX_LIB_DIR = $defaultLibDir
            Write-Host "  SHERPA_ONNX_LIB_DIR=$env:SHERPA_ONNX_LIB_DIR"
        } else {
            Write-Host "  SHERPA_ONNX_LIB_DIR not set (sherpa-onnx will auto-download prebuilt libs)"
        }
    }

    Enable-DirectMLIfAvailable

    # bindgen needs libclang — try common LLVM install paths if not set
    if (-not $env:LIBCLANG_PATH) {
        # Try LLVM system installs first, then Python libclang package as fallback
        $candidates = @(
            "C:\Program Files\LLVM\bin",
            "C:\Program Files (x86)\LLVM\bin"
        )
        # Auto-detect Python libclang package (pip install libclang)
        try {
            $pyClang = & python -c "import clang.cindex, os; print(os.path.dirname(clang.cindex.__file__) + r'\native')" 2>$null
            if ($pyClang -and (Test-Path (Join-Path $pyClang "libclang.dll"))) {
                $candidates += $pyClang
            }
        } catch {}
        foreach ($c in $candidates) {
            if (Test-Path (Join-Path $c "libclang.dll")) {
                $env:LIBCLANG_PATH = $c
                Write-Host "  LIBCLANG_PATH=$env:LIBCLANG_PATH"
                break
            }
        }
        if (-not $env:LIBCLANG_PATH) {
            Write-Warning "libclang.dll not found. Install LLVM from https://releases.llvm.org/ or run: pip install libclang"
        }
    }

    # llama.cpp C++ sources contain UTF-8 literals; on Chinese Windows (GBK code page 936)
    # cl.exe would fail with C2001 without this flag.
    if (-not $env:CXXFLAGS) {
        $env:CXXFLAGS = "/utf-8"
    }

    # cmake is required to build llama.cpp — check PATH, then portable install location
    if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
        $portableCmake = Join-Path $env:USERPROFILE ".local\cmake"
        $cmakeExe = Get-ChildItem $portableCmake -Filter "cmake.exe" -Recurse -Depth 4 -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($cmakeExe) {
            $env:PATH = "$($cmakeExe.DirectoryName);$env:PATH"
            Write-Host "  cmake=$($cmakeExe.FullName)"
        } else {
            Write-Warning "cmake not found. Download portable cmake to ~/.local/cmake or install from https://cmake.org/download/"
        }
    }
}

# ---------------------------------------------------------------------------
function Stop-Selah {
    Write-Host "Killing running Selah processes..."
    Get-Process | Where-Object { $_.Name -match '^selah-app$|^cargo$' } |
        Stop-Process -Force -ErrorAction SilentlyContinue
    # Kill the Vite dev server (node processes on port 5173)
    $conns = Get-NetTCPConnection -LocalPort 5173 -ErrorAction SilentlyContinue
    if ($conns) {
        $conns | Select-Object -ExpandProperty OwningProcess -Unique |
            ForEach-Object { Stop-Process -Id $_ -Force -ErrorAction SilentlyContinue }
    }
    Start-Sleep -Milliseconds 500
    Write-Host "Done."
}

# ---------------------------------------------------------------------------
function Start-Dev {
    Stop-Selah
    Set-BuildEnv
    Write-Host "Starting dev server..."
    & npm run tauri dev
}

# ---------------------------------------------------------------------------
function Start-Build {
    Stop-Selah
    Set-BuildEnv
    if (Test-Path "dist") { Remove-Item "dist" -Recurse -Force }
    Write-Host "Building $AppName..."
    & npx tauri build
    if (Test-Path $BundlePath) {
        Write-Host "Build complete: $BundlePath"
        Start-Process $BundlePath
    } else {
        # Release bundle might be an NSIS installer
        $installer = Get-ChildItem "src-tauri\target\release\bundle\nsis\*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($installer) {
            Write-Host "Build complete: $($installer.FullName)"
            Start-Process $installer.FullName
        } else {
            Write-Error "Build output not found."
        }
    }
}

# ---------------------------------------------------------------------------
function Install-Prerequisites {
    Write-Host "Installing dev prerequisites..."

    # 1. libclang via Python (needed for llama.cpp bindgen)
    & python -c "import clang" 2>$null
    if (-not $?) {
        Write-Host "  pip install libclang..."
        & pip install libclang --quiet
    } else {
        Write-Host "  libclang: already installed"
    }

    # 2. Portable cmake (needed to compile llama.cpp from source)
    $portableCmake = Join-Path $env:USERPROFILE ".local\cmake"
    $cmakeExe = Get-ChildItem $portableCmake -Filter "cmake.exe" -Recurse -Depth 4 -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cmakeExe) {
        Write-Host "  cmake: already at $($cmakeExe.FullName)"
    } else {
        Write-Host "  Downloading portable cmake..."
        $zipPath = "$env:TEMP\cmake-win64.zip"
        Invoke-WebRequest -Uri "https://github.com/Kitware/CMake/releases/download/v3.31.6/cmake-3.31.6-windows-x86_64.zip" -OutFile $zipPath
        New-Item -ItemType Directory -Path $portableCmake -Force | Out-Null
        Expand-Archive -Path $zipPath -DestinationPath $portableCmake -Force
        Remove-Item $zipPath -Force
        $cmakeExe = Get-ChildItem $portableCmake -Filter "cmake.exe" -Recurse -Depth 4 | Select-Object -First 1
        Write-Host "  cmake: installed at $($cmakeExe.FullName)"
    }

    # 3. npm install
    Write-Host "  npm install..."
    & npm install --silent

    Write-Host "Setup complete. Run '.\run.ps1 dev' to start."
}

# ---------------------------------------------------------------------------
function Build-DirectMLRuntime {
    Set-BuildEnv
    $workRoot = Join-Path $env:TEMP "selah-windows-directml-build"
    $libStageDir = Join-Path $ScriptDir ".cache\windows-directml\lib"
    $runtimeStageDir = Join-Path $ScriptDir "src-tauri\windows-runtime"
    & "$ScriptDir\scripts\build-windows-directml-runtime.ps1" `
        -WorkRoot $workRoot `
        -LibStageDir $libStageDir `
        -RuntimeStageDir $runtimeStageDir
    $env:SHERPA_ONNX_LIB_DIR = $libStageDir
    Enable-DirectMLIfAvailable
    Write-Host ""
    Write-Host "DirectML runtime cache is ready. Run '.\run.ps1 dev' or '.\run.ps1 build' again."
}

# ---------------------------------------------------------------------------
function Clear-Cache {
    Write-Host "Cleaning caches..."
    $targets = @("dist", "node_modules\.vite", "node_modules\.cache",
                 "src-tauri\target\debug\bundle", "src-tauri\target\release\bundle",
                 "src-tauri\gen\schemas")
    foreach ($t in $targets) {
        $full = Join-Path $ScriptDir $t
        if (Test-Path $full) {
            Remove-Item $full -Recurse -Force
            Write-Host "  Removed $t"
        }
    }
    Write-Host "Clean complete."
}

# ---------------------------------------------------------------------------
function Open-LastBuild {
    $installer = Get-ChildItem "src-tauri\target\release\bundle\nsis\*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($installer) {
        Start-Process $installer.FullName
    } elseif (Test-Path $BundlePath) {
        Start-Process $BundlePath
    } else {
        Write-Error "No build found. Run '.\run.ps1 build' first."
    }
}

# ---------------------------------------------------------------------------
switch ($Command.ToLower()) {
    "setup"   { Install-Prerequisites }
    "directml" { Build-DirectMLRuntime }
    "dev"     { Start-Dev }
    "build"   { Start-Build }
    "clean"   { Clear-Cache }
    "rebuild" { Clear-Cache; Start-Build }
    "kill"    { Stop-Selah }
    "open"    { Open-LastBuild }
    default {
        Write-Host "Usage: .\run.ps1 [setup|directml|dev|build|clean|rebuild|kill|open]"
        Write-Host ""
        Write-Host "  setup    Install dev prerequisites (first time only)"
        Write-Host "  directml Build local Windows DirectML STT runtime cache"
        Write-Host "  dev      Start development server (default)"
        Write-Host "  build    Production build"
        Write-Host "  clean    Clean all build caches"
        Write-Host "  rebuild  Clean + build"
        Write-Host "  kill     Kill running Selah processes"
        Write-Host "  open     Open last built exe/installer"
    }
}
