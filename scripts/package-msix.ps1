param(
  [Parameter(Mandatory = $true)]
  [string]$Version,
  [Parameter(Mandatory = $true)]
  [string]$InputExe,
  [Parameter(Mandatory = $true)]
  [string]$OutputDir,
  [Parameter(Mandatory = $true)]
  [string]$IdentityName,
  [Parameter(Mandatory = $true)]
  [string]$Publisher,
  [Parameter(Mandatory = $true)]
  [string]$PublisherDisplayName,
  [string]$CertificatePath,
  [string]$CertificatePassword
)

$ErrorActionPreference = "Stop"

function Find-WindowsSdkTool {
  param([Parameter(Mandatory = $true)][string]$Name)

  $direct = Get-Command $Name -ErrorAction SilentlyContinue
  if ($direct) { return $direct.Source }

  $kitsRoot = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
  if (Test-Path $kitsRoot) {
    $candidate = Get-ChildItem $kitsRoot -Recurse -Filter $Name -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match "\\x64\\" } |
      Sort-Object FullName -Descending |
      Select-Object -First 1
    if ($candidate) { return $candidate.FullName }
  }

  throw "Could not find $Name. Install the Windows 10/11 SDK on the runner."
}

function Escape-Xml {
  param([string]$Value)
  return [System.Security.SecurityElement]::Escape($Value)
}

function To-MsixVersion {
  param([string]$Value)
  $main = ($Value -split "[-+]")[0]
  $parts = @($main -split "\.")
  while ($parts.Count -lt 4) { $parts += "0" }
  if ($parts.Count -gt 4) { $parts = $parts[0..3] }
  return ($parts -join ".")
}

function Resize-Png {
  param(
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Destination,
    [Parameter(Mandatory = $true)][int]$Width,
    [Parameter(Mandatory = $true)][int]$Height
  )

  Add-Type -AssemblyName System.Drawing
  $sourceImage = [System.Drawing.Image]::FromFile($Source)
  try {
    $target = New-Object System.Drawing.Bitmap $Width, $Height
    try {
      $graphics = [System.Drawing.Graphics]::FromImage($target)
      try {
        $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
        $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $graphics.Clear([System.Drawing.Color]::Transparent)
        $graphics.DrawImage($sourceImage, 0, 0, $Width, $Height)
      } finally {
        $graphics.Dispose()
      }
      $target.Save($Destination, [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
      $target.Dispose()
    }
  } finally {
    $sourceImage.Dispose()
  }
}

$makeAppx = Find-WindowsSdkTool "makeappx.exe"
$signtool = Find-WindowsSdkTool "signtool.exe"
$packageVersion = To-MsixVersion $Version
$root = Resolve-Path (Join-Path $PSScriptRoot "..")
$layoutDir = Join-Path $OutputDir "msix-layout"
$assetsDir = Join-Path $layoutDir "Assets"
$msixPath = Join-Path $OutputDir "Selah_${Version}_x64.msix"

if (-not (Test-Path $InputExe)) {
  throw "Input executable not found: $InputExe"
}

Remove-Item $layoutDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $layoutDir, $assetsDir | Out-Null

Copy-Item $InputExe (Join-Path $layoutDir "Selah.exe") -Force

$runtimeDir = Join-Path $root "src-tauri\windows-runtime"
if (Test-Path $runtimeDir) {
  Copy-Item (Join-Path $runtimeDir "*") $layoutDir -Recurse -Force
}

$iconDir = Join-Path $root "src-tauri\icons"
Copy-Item (Join-Path $iconDir "Square150x150Logo.png") (Join-Path $assetsDir "Square150x150Logo.png") -Force
Copy-Item (Join-Path $iconDir "Square310x310Logo.png") (Join-Path $assetsDir "Square310x310Logo.png") -Force
Resize-Png (Join-Path $iconDir "icon.png") (Join-Path $assetsDir "Square44x44Logo.png") 44 44
Resize-Png (Join-Path $iconDir "icon.png") (Join-Path $assetsDir "StoreLogo.png") 50 50

$manifest = @"
<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap rescap">
  <Identity
    Name="$(Escape-Xml $IdentityName)"
    Publisher="$(Escape-Xml $Publisher)"
    Version="$(Escape-Xml $packageVersion)"
    ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>Selah</DisplayName>
    <PublisherDisplayName>$(Escape-Xml $PublisherDisplayName)</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.22621.0" />
  </Dependencies>
  <Resources>
    <Resource Language="ja-jp" />
    <Resource Language="en-us" />
  </Resources>
  <Applications>
    <Application Id="Selah" Executable="Selah.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements
        DisplayName="Selah"
        Description="Selah"
        BackgroundColor="transparent"
        Square150x150Logo="Assets\Square150x150Logo.png"
        Square44x44Logo="Assets\Square44x44Logo.png">
        <uap:DefaultTile Square310x310Logo="Assets\Square310x310Logo.png" />
      </uap:VisualElements>
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>
</Package>
"@

$manifest | Set-Content -Path (Join-Path $layoutDir "AppxManifest.xml") -Encoding UTF8

Remove-Item $msixPath -Force -ErrorAction SilentlyContinue
& $makeAppx pack /d $layoutDir /p $msixPath /o
if ($LASTEXITCODE -ne 0) {
  throw "makeappx failed with exit code $LASTEXITCODE"
}

if ($CertificatePath) {
  if (-not (Test-Path $CertificatePath)) {
    throw "Certificate not found: $CertificatePath"
  }

  $signArgs = @("sign", "/fd", "SHA256", "/a", "/f", $CertificatePath)
  if ($CertificatePassword) {
    $signArgs += @("/p", $CertificatePassword)
  }
  $signArgs += $msixPath
  & $signtool @signArgs
  if ($LASTEXITCODE -ne 0) {
    throw "signtool failed with exit code $LASTEXITCODE"
  }
}

Write-Host "MSIX created: $msixPath"
