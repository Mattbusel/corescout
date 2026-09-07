<#
.SYNOPSIS
  Regenerate the generated blocks in LISTING.md from the code.

.DESCRIPTION
  Prices live in crates/licence/src/plans.rs, which is what the application's
  upgrade screen and the website read. This copies them into the Store listing
  so there is no fourth place for a price to be wrong.

  The blocks are delimited by

      <!-- generated:plans -->  ...  <!-- /generated:plans -->

  and everything between them is replaced. Everything outside is written by
  hand, because listing copy is not a data structure.

  It also writes pricing/plans.json, which is the machine-readable version used
  when filling in Partner Center.

.PARAMETER Check
  Do not write. Exit non-zero if the generated blocks are out of date, which is
  what CI runs.

.EXAMPLE
  powershell -File microsoft-store\build-listing.ps1
  powershell -File microsoft-store\build-listing.ps1 -Check
#>
[CmdletBinding()]
param(
    [switch] $Check
)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$repo = [string](Resolve-Path (Join-Path $here '..'))

Push-Location $repo
try {
    $table = & cargo run -q -p corescout-licensor -- plans
    if ($LASTEXITCODE -ne 0) { throw 'could not read the plans' }
    $json = & cargo run -q -p corescout-licensor -- plans --json
    if ($LASTEXITCODE -ne 0) { throw 'could not read the plans as JSON' }
}
finally {
    Pop-Location
}

$block = @()
$block += '<!-- generated:plans -->'
$block += ''
$block += 'Generated from `crates/licence/src/plans.rs` by `build-listing.ps1`. Do not edit by hand.'
$block += ''
$block += '```'
$block += ($table -join "`n").TrimEnd()
$block += '```'
$block += ''
$block += '<!-- /generated:plans -->'
$replacement = $block -join "`r`n"

$listingPath = Join-Path $here 'LISTING.md'
$listing = Get-Content $listingPath -Raw
$pattern = '(?s)<!-- generated:plans -->.*?<!-- /generated:plans -->'
if ($listing -notmatch $pattern) { throw "LISTING.md has no generated:plans block" }
$updated = [regex]::Replace($listing, $pattern, { $replacement })

$jsonPath = Join-Path $here 'pricing\plans.json'
New-Item -ItemType Directory -Force -Path (Split-Path $jsonPath) | Out-Null
$jsonText = ($json -join "`n").TrimEnd()

if ($Check) {
    $stale = @()
    if ($updated -ne $listing) { $stale += 'LISTING.md' }
    if (-not (Test-Path $jsonPath) -or ((Get-Content $jsonPath -Raw).TrimEnd() -ne $jsonText)) {
        $stale += 'pricing/plans.json'
    }
    if ($stale.Count -gt 0) {
        Write-Error ("out of date, run build-listing.ps1: " + ($stale -join ', '))
        exit 1
    }
    Write-Host 'The listing matches the code.' -ForegroundColor Green
    exit 0
}

Set-Content -Path $listingPath -Value $updated -Encoding utf8 -NoNewline
Set-Content -Path $jsonPath -Value $jsonText -Encoding utf8
Write-Host "Wrote LISTING.md and pricing\plans.json" -ForegroundColor Green
