<#
.SYNOPSIS
    Sign and install the MSIX on this machine, to test what a customer gets.

.DESCRIPTION
    The Store signs and installs the real package. To exercise the same code
    path before submitting, this creates a self-signed certificate, trusts it
    for this machine, signs dist\CoreScout.msix and installs it.

    It is the only script in this repository that needs an elevated prompt, for
    two reasons that Windows will not budge on: trusting a certificate writes
    to the machine store, and installing a side-loaded package requires that
    trust to already exist. Nothing here is needed to build the package, and
    nothing here goes anywhere near a Store submission, which is signed by
    Microsoft.

    The certificate subject must match Package/Identity/Publisher exactly or
    the install fails with a signature error that does not mention names, so
    both come from the same parameter.

.PARAMETER Publisher
    The Package/Identity/Publisher. Must match what msix.ps1 was given.

.PARAMETER Uninstall
    Remove the package and the certificate instead of installing.

.PARAMETER KeepData
    With -Uninstall, leave the package's data behind, which is what a reinstall
    test wants to check.

.EXAMPLE
    # From an elevated PowerShell, in the repository root:
    powershell -ExecutionPolicy Bypass -File scripts\install-local.ps1

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\install-local.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [string] $Publisher = "CN=CoreScout",
    [switch] $Uninstall,
    [switch] $KeepData
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not (New-Object Security.Principal.WindowsPrincipal($identity)).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "This one needs an elevated PowerShell. Everything else in scripts\ does not."
}

$familyPrefix = "CoreScout"

if ($Uninstall) {
    $installed = Get-AppxPackage | Where-Object { $_.Name -like "*$familyPrefix*" }
    foreach ($package in $installed) {
        Write-Host "Removing $($package.PackageFullName)" -ForegroundColor Cyan
        if ($KeepData) {
            Remove-AppxPackage -Package $package.PackageFullName -PreserveApplicationData
        } else {
            Remove-AppxPackage -Package $package.PackageFullName
        }
    }
    if (-not $installed) { Write-Host "Nothing installed." }

    foreach ($store in @("Cert:\LocalMachine\TrustedPeople", "Cert:\CurrentUser\My")) {
        Get-ChildItem $store -ErrorAction SilentlyContinue |
            Where-Object { $_.Subject -eq $Publisher } |
            ForEach-Object {
                Write-Host "Removing certificate $($_.Thumbprint) from $store" -ForegroundColor Cyan
                Remove-Item $_.PSPath -Force
            }
    }
    Write-Host "Done." -ForegroundColor Green
    return
}

$package = Join-Path $root "dist\CoreScout.msix"
if (-not (Test-Path $package)) {
    throw "No package at $package. Run scripts\msix.ps1 first."
}

# One certificate, reused across runs, so repeated installs do not accumulate
# a store full of near-identical test certificates.
$cert = Get-ChildItem Cert:\CurrentUser\My |
    Where-Object { $_.Subject -eq $Publisher -and $_.NotAfter -gt (Get-Date) } |
    Sort-Object NotAfter -Descending |
    Select-Object -First 1

if (-not $cert) {
    Write-Host "Creating a self-signed certificate for $Publisher" -ForegroundColor Cyan
    $cert = New-SelfSignedCertificate `
        -Type Custom `
        -Subject $Publisher `
        -KeyUsage DigitalSignature `
        -FriendlyName "CoreScout side-loading (test only)" `
        -CertStoreLocation "Cert:\CurrentUser\My" `
        -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
}
Write-Host "Certificate $($cert.Thumbprint)" -ForegroundColor Cyan

# Trusting it. This is what makes the package installable and it is exactly
# why this script is quarantined behind an explicit elevated run.
$trusted = Get-ChildItem Cert:\LocalMachine\TrustedPeople -ErrorAction SilentlyContinue |
    Where-Object { $_.Thumbprint -eq $cert.Thumbprint }
if (-not $trusted) {
    $exported = Join-Path $env:TEMP "corescout-sideload.cer"
    Export-Certificate -Cert $cert -FilePath $exported | Out-Null
    Import-Certificate -FilePath $exported -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
    Remove-Item $exported -Force
    Write-Host "Trusted for this machine." -ForegroundColor Cyan
}

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
    if (-not $found) { throw "$name is not on this machine. Install the Windows SDK." }
    $found.FullName
}

# Signed in place would leave dist\ holding a package that is not what would be
# uploaded, so the signed copy is a separate file.
$signed = Join-Path $root "dist\CoreScout.sideload.msix"
Copy-Item $package $signed -Force
$signtool = Find-SdkTool "signtool.exe"
& $signtool sign /fd SHA256 /sha1 $cert.Thumbprint $signed
if ($LASTEXITCODE -ne 0) { throw "signing failed" }

Write-Host "Installing..." -ForegroundColor Cyan
Add-AppxPackage -Path $signed
Write-Host ""
Get-AppxPackage | Where-Object { $_.Name -like "*$familyPrefix*" } |
    Format-List Name, PackageFullName, PackageFamilyName, Version, InstallLocation
Write-Host "Installed. dist\CoreScout.msix is untouched and is what gets uploaded." -ForegroundColor Green
