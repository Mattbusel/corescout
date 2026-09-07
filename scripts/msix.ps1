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
    # Defaulted to what Partner Center assigned to this listing, so an
    # ordinary build produces an uploadable package. They are public values:
    # they are in the Store URL and in every copy of the shipped package.
    [string] $IdentityName = "Tensorust.CoreScout",
    [string] $Publisher = "CN=98B77C5C-8582-4364-B50E-0922AE25FBF6",
    [string] $PublisherDisplayName = "Tensorust",
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
#
# The command line tool is staged as corescout-cli.exe. It cannot be staged as
# corescout.exe: Windows filenames are case-insensitive, so that and the
# window's CoreScout.exe are the same file, and the second copy silently
# replaces the first. The manifest gives it the execution alias "corescout",
# so what a person types is unchanged.
$payload = @{
    "corescout-desktop.exe" = "CoreScout.exe"
    "corescout.exe"         = "corescout-cli.exe"
    "corescout-service.exe" = "corescout-service.exe"
    "corescout-mcp.exe"     = "corescout-mcp.exe"
}
foreach ($from in $payload.Keys) {
    $source = Join-Path $built $from
    if (-not (Test-Path $source)) { throw "$from was not built; expected it in $built" }
    Copy-Item $source (Join-Path $stage $payload[$from]) -Force
}

# And prove it, because the failure this guards against is silent: MakeAppx
# packs whatever is in the folder and the manifest resolves names
# case-insensitively, so a collision produces a package that builds, installs,
# and launches the wrong program.
$staged = Get-ChildItem $stage -File | Select-Object -ExpandProperty Name
$collisions = $staged | Group-Object { $_.ToLowerInvariant() } | Where-Object { $_.Count -gt 1 }
if ($collisions) { throw "two payload files differ only by case: $($collisions.Name -join ', ')" }
if ($staged.Count -ne $payload.Count) {
    throw "expected $($payload.Count) programs in $stage, found $($staged.Count): $($staged -join ', ')"
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

if ($IdentityName -notmatch '^[^.]+\.[^.]+') {
    Write-Host ""
    Write-Host "The identity does not look like a Partner Center one, so the Store will reject this." -ForegroundColor Yellow
    Write-Host "microsoft-store/IDENTITY.md has the values for this listing." -ForegroundColor Yellow
} else {
    Write-Host ""
    Write-Host "Identity: $IdentityName / $Publisher" -ForegroundColor Green
    Write-Host "Upload this to Partner Center. It is unsigned on purpose." -ForegroundColor Green
}
