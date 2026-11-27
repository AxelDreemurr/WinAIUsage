<#
.SYNOPSIS
  Builds the WinAIUsage MSIX package for the Microsoft Store (no MSIX Packaging Tool).

.DESCRIPTION
  1. Builds the release exe with `tauri build --no-bundle` (skip with -SkipBuild).
  2. Lays out the package: exe under VFS\ProgramFilesX64\WinAIUsage, logos generated
     from src-tauri\icons\icon.png into Assets\ (scale and targetsize variants, plus the
     widget picker's WidgetIcon.png / WidgetScreenshot.png).
  3. Copies packaging\AppxManifest.xml with Version taken from tauri.conf.json (x.y.z.0),
     and packaging\Strings\<lang>\Resources.resw for its ms-resource: strings.
  4. Runs makepri + makeappx from the Windows SDK.

  packaging\AppxManifest.xml is NOT versioned (it holds the Store identity). Create it
  locally from the Store's Product identity (Partner Center > Product management).

  The Store signs the package on submission, so signing is only needed to install it
  locally. For that, pass -CertThumbprint of a code-signing cert in CurrentUser\My whose
  subject equals the manifest Publisher, and trust that cert on the machine.

.EXAMPLE
  npm run build:msix
.EXAMPLE
  npm run build:msix -- -SkipBuild -CertThumbprint 0123ABCD...
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$CertThumbprint
)

$ErrorActionPreference = 'Stop'

$root        = Split-Path -Parent $PSScriptRoot
$tauriDir    = Join-Path $root 'src-tauri'
$manifestSrc = Join-Path $root 'packaging\AppxManifest.xml'
$iconSrc     = Join-Path $tauriDir 'icons\icon.png'
$exeSrc      = Join-Path $tauriDir 'target\release\winaiusage.exe'
$workDir     = Join-Path $tauriDir 'target\msix'
$layout      = Join-Path $workDir 'layout'
$assetsDir   = Join-Path $layout 'Assets'
$outDir      = Join-Path $tauriDir 'target\release\bundle\msix'

function Find-SdkTool([string]$Name) {
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $tool = Get-ChildItem $kits -Directory -Filter '10.*' -ErrorAction SilentlyContinue |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName "x64\$Name" } |
        Where-Object { Test-Path $_ } |
        Select-Object -First 1
    if (-not $tool) { throw "$Name not found. Install the Windows SDK (Windows Kits\10)." }
    return $tool
}

# Runs an SDK tool; its (very verbose) output is shown only when it fails.
function Invoke-Tool([string]$Exe, [string[]]$ToolArgs) {
    $output = & $Exe @ToolArgs 2>&1
    if ($LASTEXITCODE -ne 0) {
        $output | Write-Host
        throw "$(Split-Path -Leaf $Exe) failed with exit code $LASTEXITCODE"
    }
}

if (-not (Test-Path $manifestSrc)) {
    throw "Missing $manifestSrc (local only, not in git). Create it with the Store identity first."
}

# -- Version: tauri.conf.json "x.y.z" -> MSIX "x.y.z.0" (Store requires revision 0) --
$conf = Get-Content (Join-Path $tauriDir 'tauri.conf.json') -Raw | ConvertFrom-Json
$parts = @(($conf.version -replace '[-+].*$', '').Split('.') | ForEach-Object { [int]$_ })
while ($parts.Count -lt 3) { $parts += 0 }
$msixVersion = '{0}.{1}.{2}.0' -f $parts[0], $parts[1], $parts[2]
Write-Host "MSIX version: $msixVersion"

# -- Build --------------------------------------------------------------------
if (-not $SkipBuild) {
    Push-Location $root
    try {
        & npx tauri build --no-bundle
        if ($LASTEXITCODE -ne 0) { throw "tauri build failed with exit code $LASTEXITCODE" }
    } finally { Pop-Location }
}
if (-not (Test-Path $exeSrc)) { throw "Release exe not found: $exeSrc" }

# -- Layout -------------------------------------------------------------------
if (Test-Path $workDir) { Remove-Item $workDir -Recurse -Force }
New-Item -ItemType Directory -Force -Path $layout, $assetsDir, $outDir | Out-Null

$manifestText = [IO.File]::ReadAllText($manifestSrc)
$manifestText = [regex]::Replace($manifestText, '(<Identity\b[^>]*?\bVersion=")[^"]*(")', "`${1}$msixVersion`${2}")
$manifestDst = Join-Path $layout 'AppxManifest.xml'
[IO.File]::WriteAllText($manifestDst, $manifestText, (New-Object Text.UTF8Encoding $false))

# Localized manifest strings (ms-resource:...), indexed into resources.pri below.
Copy-Item (Join-Path $root 'packaging\Strings') (Join-Path $layout 'Strings') -Recurse

$exeRel = ([xml]$manifestText).Package.Applications.Application.Executable
$exeDst = Join-Path $layout $exeRel
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $exeDst) | Out-Null
Copy-Item $exeSrc $exeDst

# -- Logos --------------------------------------------------------------------
Add-Type -AssemblyName System.Drawing
$srcImage = [System.Drawing.Image]::FromFile($iconSrc)

# Draws the icon centered on a transparent WxH canvas, sized to $Fill of the shorter side.
function Save-Logo([string]$FileName, [int]$W, [int]$H, [double]$Fill) {
    $bmp = New-Object System.Drawing.Bitmap($W, $H, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    try {
        $g.InterpolationMode  = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.SmoothingMode      = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
        $g.PixelOffsetMode    = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
        $g.Clear([System.Drawing.Color]::Transparent)
        $size = [int][math]::Round([math]::Min($W, $H) * $Fill)
        $dest = New-Object System.Drawing.Rectangle([int](($W - $size) / 2), [int](($H - $size) / 2), $size, $size)
        $attrs = New-Object System.Drawing.Imaging.ImageAttributes
        $attrs.SetWrapMode([System.Drawing.Drawing2D.WrapMode]::TileFlipXY) # no edge bleeding
        $g.DrawImage($srcImage, $dest, 0, 0, $srcImage.Width, $srcImage.Height, [System.Drawing.GraphicsUnit]::Pixel, $attrs)
        $bmp.Save((Join-Path $assetsDir $FileName), [System.Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $g.Dispose(); $bmp.Dispose()
    }
}

try {
    $logos = @(
        @{ Name = 'Square44x44Logo';   W = 44;  H = 44;  Fill = 1.0 },
        @{ Name = 'Square71x71Logo';   W = 71;  H = 71;  Fill = 0.6 },
        @{ Name = 'Square150x150Logo'; W = 150; H = 150; Fill = 0.6 },
        @{ Name = 'Square310x310Logo'; W = 310; H = 310; Fill = 0.6 },
        @{ Name = 'Wide310x150Logo';   W = 310; H = 150; Fill = 0.6 },
        @{ Name = 'StoreLogo';         W = 50;  H = 50;  Fill = 1.0 }
    )
    foreach ($logo in $logos) {
        foreach ($scale in 100, 125, 150, 200, 400) {
            $w = [int][math]::Round($logo.W * $scale / 100)
            $h = [int][math]::Round($logo.H * $scale / 100)
            Save-Logo "$($logo.Name).scale-$scale.png" $w $h $logo.Fill
        }
    }
    # Widget picker icon and preview (manifest: com.microsoft.windows.widgets).
    Save-Logo 'WidgetIcon.png' 64 64 1.0
    Save-Logo 'WidgetScreenshot.png' 300 304 0.5
    # Taskbar / Start / Explorer icons.
    foreach ($t in 16, 24, 32, 48, 256) {
        Save-Logo "Square44x44Logo.targetsize-$t.png" $t $t 1.0
        Save-Logo "Square44x44Logo.targetsize-${t}_altform-unplated.png" $t $t 1.0
        Save-Logo "Square44x44Logo.targetsize-${t}_altform-lightunplated.png" $t $t 1.0
    }
} finally {
    $srcImage.Dispose()
}

# -- resources.pri ------------------------------------------------------------
$makepri = Find-SdkTool 'makepri.exe'
$priConfig = Join-Path $workDir 'priconfig.xml'
Invoke-Tool $makepri @('createconfig', '/cf', $priConfig, '/dq', 'en-US', '/pv', '10.0.0', '/o')
# Drop <packaging> (auto resource packs are for bundles): keep every scale in one resources.pri.
$priXml = [xml](Get-Content $priConfig -Raw)
$priXml.SelectNodes('//packaging') | ForEach-Object { [void]$_.ParentNode.RemoveChild($_) }
$priXml.Save($priConfig)
Invoke-Tool $makepri @('new', '/pr', $layout, '/cf', $priConfig, '/mn', $manifestDst, '/of', (Join-Path $layout 'resources.pri'), '/o')

# -- Pack (+ optional sign) ---------------------------------------------------
$makeappx = Find-SdkTool 'makeappx.exe'
$msix = Join-Path $outDir "WinAIUsage_${msixVersion}_x64.msix"
Invoke-Tool $makeappx @('pack', '/d', $layout, '/p', $msix, '/o')

if ($CertThumbprint) {
    $signtool = Find-SdkTool 'signtool.exe'
    Invoke-Tool $signtool @('sign', '/fd', 'SHA256', '/sha1', $CertThumbprint, $msix)
}

Write-Host ""
Write-Host "MSIX ready: $msix" -ForegroundColor Green
