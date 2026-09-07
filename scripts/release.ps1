<#
.SYNOPSIS
    Build everything CoreScout ships, and name it what the download page says.

.DESCRIPTION
    Tauri names its output after the version, which is right for a bundle
    directory and wrong for a download link people paste to each other. This
    produces `CoreScoutSetup.exe` and `CoreScout.msi` alongside the versioned
    originals, writes a checksum for each, and leaves both so a release can
    carry either name.

    Nothing here signs anything. Signing needs a certificate this repository
    does not contain and should not; see docs/WINDOWS.md for what to do when
    you have one.
#>
[CmdletBinding()]
param(
    [string] $OutDir = "dist"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "Building the command line, the service and the bridge..." -ForegroundColor Cyan
cargo build --release -p corescout-service -p corescout-cli -p corescout-mcp
if ($LASTEXITCODE -ne 0) { throw "the Rust build failed" }

$desktop = Join-Path $root "apps\corescout-desktop"
$binaries = Join-Path $desktop "src-tauri\binaries"
New-Item -ItemType Directory -Force -Path $binaries | Out-Null

# Tauri looks for sidecars named with the target triple and installs them
# without it. Copying here rather than committing binaries keeps the repository
# free of build output.
$triple = "x86_64-pc-windows-msvc"
foreach ($name in @("corescout-service", "corescout-mcp", "corescout")) {
    Copy-Item (Join-Path $root "target\release\$name.exe") `
              (Join-Path $binaries "$name-$triple.exe") -Force
}

Write-Host "Building the application and its installers..." -ForegroundColor Cyan
Push-Location $desktop
try {
    npm ci --silent
    npm run build
    if ($LASTEXITCODE -ne 0) { throw "the frontend build failed" }
    npx tauri build
    if ($LASTEXITCODE -ne 0) { throw "the bundle failed" }
} finally {
    Pop-Location
}

$bundle = Join-Path $desktop "src-tauri\target\release\bundle"
$out = Join-Path $root $OutDir
New-Item -ItemType Directory -Force -Path $out | Out-Null

$setup = Get-ChildItem (Join-Path $bundle "nsis") -Filter "*-setup.exe" | Select-Object -First 1
$msi = Get-ChildItem (Join-Path $bundle "msi") -Filter "*.msi" | Select-Object -First 1
if (-not $setup) { throw "no installer was produced" }

Copy-Item $setup.FullName (Join-Path $out "CoreScoutSetup.exe") -Force
Copy-Item $setup.FullName (Join-Path $out $setup.Name) -Force
if ($msi) {
    Copy-Item $msi.FullName (Join-Path $out "CoreScout.msi") -Force
    Copy-Item $msi.FullName (Join-Path $out $msi.Name) -Force
}

# A checksum next to every artefact, so someone who downloaded it over a link
# they were sent can tell whether it is the file this build produced.
Get-ChildItem $out -File | Where-Object { $_.Extension -in ".exe", ".msi" } | ForEach-Object {
    $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    "$hash  $($_.Name)" | Out-File -FilePath (Join-Path $out "$($_.Name).sha256") -Encoding ascii
    Write-Host ("{0,-44} {1,10:N0} KB  {2}" -f $_.Name, ($_.Length / 1KB), $hash.Substring(0, 16))
}

Write-Host ""
Write-Host "Ready in $out" -ForegroundColor Green
Write-Host "These installers are not signed. Windows will warn about that." -ForegroundColor Yellow
