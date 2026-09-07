<#
.SYNOPSIS
    Build an MSIX package of CoreScout, for the Microsoft Store or for
    side-loading.

.DESCRIPTION
    Stages the four programs, the web assets, the tile images and a manifest
    into one folder, and calls MakeAppx on it.

    The identity values default to something that works for side-loading and
    that the Store will reject, which is deliberate: the real ones come from
    Partner Center when the app name is reserved, and a build that silently
    used a plausible-looking wrong publisher would fail at upload with a
    message about signatures rather than about names. See docs/STORE.md.

.PARAMETER IdentityName
    The Package/Identity/Name from Partner Center, e.g. 12345Publisher.CoreScout

.PARAMETER Publisher
    The Package/Identity/Publisher from Partner Center. The full subject,
    e.g. CN=ABCD1234-...

.PARAMETER PublisherDisplayName
    The publisher name shown to users.

.PARAMETER Sign
    Thumbprint of a certificate to sign with. Omit for a Store upload: the
    Store signs it. Required for side-loading.
#>
[CmdletBinding()]
param(
    [string] $IdentityName = "CoreScout",
    [string] $Publisher = "CN=CoreScout",
    [string] $PublisherDisplayName = "CoreScout",
    [string] $Sign,
    [string] $OutDir = "dist"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Find-SdkTool([string] $name) {
    $roots = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin",
        "${env:ProgramFiles}\Windows Kits\10\bin"
    ) | Where-Object { Test-Path $_ }
    $found = $roots |
        ForEach-Object { Get-ChildItem $_ -Recurse -Filter $name -ErrorAction SilentlyContinue } |
        Where-Object { $_.FullName -match "\\x64\\" } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $found) {
        throw "$name is not on this machine. Install the Windows SDK; docs/STORE.md says which parts."
    }
    $found.FullName
}

$version = (Select-String -Path "Cargo.toml" -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
# MSIX versions are four parts and the last must be zero for a Store upload.
$msixVersion = "$version.0"

Write-Host "Building CoreScout $msixVersion for MSIX..." -ForegroundColor Cyan
& (Join-Path $PSScriptRoot "release.ps1") -OutDir $OutDir
if ($LASTEXITCODE -ne 0) { throw "the build failed" }

$stage = Join-Path $root "target\msix"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage, (Join-Path $stage "Assets") | Out-Null

$desktop = Join-Path $root "apps\corescout-desktop"
$built = Join-Path $desktop "src-tauri\target\release"

# The window, and the three programs it depends on.
Copy-Item (Join-Path $built "corescout-desktop.exe") (Join-Path $stage "CoreScout.exe") -Force
foreach ($name in @("corescout.exe", "corescout-service.exe", "corescout-mcp.exe")) {
    Copy-Item (Join-Path $built $name) (Join-Path $stage $name) -Force
}

# Every tile the manifest names. A missing one fails MakeAppx with a message
# about the manifest rather than about the file, so they are copied by name.
$icons = Join-Path $desktop "src-tauri\icons"
foreach ($asset in @(
    "StoreLogo.png", "Square44x44Logo.png", "Square71x71Logo.png",
    "Square150x150Logo.png", "Square310x310Logo.png",
    "Wide310x150Logo.png", "SplashScreen.png"
)) {
    $from = Join-Path $icons $asset
    if (-not (Test-Path $from)) { throw "$asset is missing from $icons" }
    Copy-Item $from (Join-Path $stage "Assets\$asset") -Force
}

$manifest = Get-Content (Join-Path $root "packaging\msix\AppxManifest.xml") -Raw
$manifest = $manifest.
    Replace("CORESCOUT_IDENTITY_NAME", $IdentityName).
    Replace("CORESCOUT_PUBLISHER_DISPLAY", $PublisherDisplayName).
    Replace("CORESCOUT_PUBLISHER", $Publisher).
    Replace("CORESCOUT_VERSION", $msixVersion)
Set-Content -Path (Join-Path $stage "AppxManifest.xml") -Value $manifest -Encoding utf8

$out = Join-Path $root $OutDir
New-Item -ItemType Directory -Force -Path $out | Out-Null
$package = Join-Path $out "CoreScout.msix"
if (Test-Path $package) { Remove-Item $package -Force }

$makeappx = Find-SdkTool "makeappx.exe"
Write-Host "Packing with $makeappx" -ForegroundColor Cyan
& $makeappx pack /d $stage /p $package /o
if ($LASTEXITCODE -ne 0) { throw "MakeAppx failed" }

if ($Sign) {
    $signtool = Find-SdkTool "signtool.exe"
    & $signtool sign /fd SHA256 /sha1 $Sign /tr http://timestamp.digicert.com /td SHA256 $package
    if ($LASTEXITCODE -ne 0) { throw "signing failed" }
    Write-Host "Signed. This can be side-loaded." -ForegroundColor Green
} else {
    Write-Host ""
    Write-Host "Unsigned, which is what a Store upload wants: the Store signs it." -ForegroundColor Yellow
    Write-Host "To side-load instead, pass -Sign <thumbprint>." -ForegroundColor Yellow
}

$hash = (Get-FileHash $package -Algorithm SHA256).Hash.ToLower()
"$hash  CoreScout.msix" | Out-File (Join-Path $out "CoreScout.msix.sha256") -Encoding ascii
Write-Host ""
Write-Host ("{0}  {1:N0} KB" -f $package, ((Get-Item $package).Length / 1KB)) -ForegroundColor Green

if ($IdentityName -eq "CoreScout") {
    Write-Host ""
    Write-Host "The identity is the placeholder, so the Store will reject this." -ForegroundColor Yellow
    Write-Host "Reserve the name in Partner Center and pass -IdentityName and -Publisher." -ForegroundColor Yellow
    Write-Host "docs/STORE.md has the whole sequence." -ForegroundColor Yellow
}
