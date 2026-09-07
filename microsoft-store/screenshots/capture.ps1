<#
.SYNOPSIS
  Produce the Store screenshots from a real CoreScout, end to end.

.DESCRIPTION
  Builds the service and the desktop front end, starts the service against a
  throwaway data directory, gives it a week of AI history through its own API,
  and photographs the running application.

  Everything on the resulting screenshots was computed by the shipping engine.
  The history it reasoned over is stated in seed.mjs and described in
  ../LISTING.md; nothing on screen is drawn by hand.

  The throwaway data directory means this never touches the CoreScout the
  developer is actually using, and it means the shots are reproducible: the
  same seed gives the same conclusions.

.EXAMPLE
  powershell -File microsoft-store\screenshots\capture.ps1
#>
[CmdletBinding()]
param(
    [string] $OutDir,

    # How long to let the service watch this machine before photographing it.
    # The mirror has to see a state recur before it can say it recognises one,
    # and that is a real wait, not a loading spinner. Six seconds produces a
    # truthful but unflattering "still working out what normal looks like".
    [int] $WarmUpSeconds = 150
)

$ErrorActionPreference = 'Stop'

# Not defaulted in the param block: Windows PowerShell 5.1 leaves $PSScriptRoot
# empty there, and this has to run on the machine that has 5.1.
if (-not $OutDir) { $OutDir = Join-Path $PSScriptRoot 'out' }
$repo = [string](Resolve-Path (Join-Path $PSScriptRoot '..\..'))

# A data directory of its own, removed afterwards. The screenshots must not
# depend on, or disturb, whatever this machine has actually learned.
$data = Join-Path ([System.IO.Path]::GetTempPath()) ("corescout-shots-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $data | Out-Null

$service = $null
try {
    Write-Host 'Building the service and the application...'
    & cargo build --release -p corescout-service
    if ($LASTEXITCODE -ne 0) { throw 'the service did not build' }

    Push-Location (Join-Path $repo 'apps\corescout-desktop')
    & npx vite build
    if ($LASTEXITCODE -ne 0) { Pop-Location; throw 'the front end did not build' }
    Pop-Location

    $env:CORESCOUT_DATA_DIR = $data
    $exe = Join-Path $repo 'target\release\corescout-service.exe'
    if (-not (Test-Path $exe)) { throw "no service binary at $exe" }
    $service = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru -WindowStyle Hidden

    # The service publishes its port and token once it is listening. Waiting for
    # the file is the same handshake the CLI and the desktop shell use.
    $endpoint = Join-Path $data 'endpoint.json'
    $deadline = (Get-Date).AddSeconds(30)
    while (-not (Test-Path $endpoint)) {
        if ((Get-Date) -gt $deadline) { throw "the service never published $endpoint" }
        Start-Sleep -Milliseconds 200
    }
    Write-Host "Watching this machine for $WarmUpSeconds seconds..."
    Start-Sleep -Seconds $WarmUpSeconds

    Write-Host 'Seeding a week of AI work...'
    & node (Join-Path $PSScriptRoot 'seed.mjs') $endpoint
    if ($LASTEXITCODE -ne 0) { throw 'the seed failed' }

    Write-Host 'Capturing...'
    & node (Join-Path $PSScriptRoot 'capture.mjs') $endpoint (Join-Path $repo 'apps\corescout-desktop\dist') $OutDir
    if ($LASTEXITCODE -ne 0) { throw 'the capture failed' }

    Write-Host "Screenshots written to $OutDir"
}
finally {
    if ($service -and -not $service.HasExited) { Stop-Process -Id $service.Id -Force }
    Remove-Item -Recurse -Force $data -ErrorAction SilentlyContinue
    Remove-Item Env:\CORESCOUT_DATA_DIR -ErrorAction SilentlyContinue
}
