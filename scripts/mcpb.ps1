<#
.SYNOPSIS
    Build CoreScout as an MCP Bundle (.mcpb), for Claude Desktop and for
    Anthropic's desktop extension submission.

.DESCRIPTION
    An MCPB is a zip with a manifest.json at its root. This one carries both
    programs it needs: the bridge an AI talks to, and the service the bridge
    forwards to. The bridge starts a missing service from its own folder, so a
    bundle that ships both works on a machine where CoreScout was never
    installed. That is the case a reviewer will be in.

    The version in the manifest is checked against the workspace version
    rather than trusted, because a bundle whose manifest disagrees with the
    binaries inside it is the kind of thing nobody notices until a user does.

.PARAMETER BinDir
    Folder holding corescout-mcp.exe and corescout-service.exe. Found from the
    usual build locations when omitted.

.PARAMETER OutDir
    Where to write CoreScout.mcpb.
#>
[CmdletBinding()]
param(
    [string] $BinDir,
    [string] $OutDir = "dist"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# --- find the binaries -------------------------------------------------------

function Find-BinDir {
    $candidates = @()
    if ($env:CARGO_TARGET_DIR) { $candidates += (Join-Path $env:CARGO_TARGET_DIR "release") }
    $candidates += @(
        (Join-Path $root "target\release"),
        (Join-Path $root "apps\corescout-desktop\src-tauri\target\release"),
        (Join-Path $env:USERPROFILE ".cargo\cargo-target\release")
    )
    foreach ($c in $candidates) {
        if ((Test-Path (Join-Path $c "corescout-mcp.exe")) -and
            (Test-Path (Join-Path $c "corescout-service.exe"))) { return $c }
    }
    throw "corescout-mcp.exe and corescout-service.exe were not found. Build them first: cargo build --release -p corescout-mcp -p corescout-service"
}

if (-not $BinDir) { $BinDir = Find-BinDir }
Write-Host "Binaries: $BinDir"

# --- check the manifest agrees with the workspace ---------------------------

$manifestPath = Join-Path $root "packaging\mcpb\manifest.json"
$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json

$workspaceVersion = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if ($manifest.version -ne $workspaceVersion) {
    throw "packaging\mcpb\manifest.json says version $($manifest.version); the workspace says $workspaceVersion."
}

# Every tool the manifest advertises must be one the server actually has, and
# the reverse. A listing that promises a tool Claude cannot call is rejected.
$toolsRs = Get-Content (Join-Path $root "crates\mcp-server\src\tools.rs") -Raw
$actual = [regex]::Matches($toolsRs, 'name:\s*"(corescout_[a-z_]+)"') |
    ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
$declared = $manifest.tools | ForEach-Object { $_.name } | Sort-Object -Unique
$missing = $actual | Where-Object { $declared -notcontains $_ }
$extra = $declared | Where-Object { $actual -notcontains $_ }
if ($missing) { throw "manifest.json does not list: $($missing -join ', ')" }
if ($extra) { throw "manifest.json lists tools that do not exist: $($extra -join ', ')" }
Write-Host "Tools: $($declared.Count), matching crates\mcp-server"

# --- stage -------------------------------------------------------------------

$stage = Join-Path $root "$OutDir\mcpb"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Path (Join-Path $stage "server") -Force | Out-Null

Copy-Item $manifestPath (Join-Path $stage "manifest.json")
Copy-Item (Join-Path $root "README.md") (Join-Path $stage "README.md")
Copy-Item (Join-Path $root "packaging\mcpb\icon.png") (Join-Path $stage "icon.png")
Copy-Item (Join-Path $BinDir "corescout-mcp.exe") (Join-Path $stage "server\corescout-mcp.exe")
Copy-Item (Join-Path $BinDir "corescout-service.exe") (Join-Path $stage "server\corescout-service.exe")

# The privacy policy is a submission requirement, and it is checked here rather
# than by a reviewer: a missing one is an immediate rejection.
$readme = Get-Content (Join-Path $stage "README.md") -Raw
if ($readme -notmatch '(?m)^##\s+Privacy Policy\s*$') {
    throw "README.md needs a `"## Privacy Policy`" section. Anthropic rejects local connectors without one."
}
if (-not $manifest.privacy_policies -or $manifest.privacy_policies.Count -eq 0) {
    throw "manifest.json needs a privacy_policies array of https URLs."
}
foreach ($url in $manifest.privacy_policies) {
    if ($url -notmatch '^https://') { throw "privacy policy must be https: $url" }
}

# --- verify the thing runs before it is packed -------------------------------

# A bundle whose server does not answer initialize is one the reviewer finds
# out about, not us. Ask it for its tool list over a pipe.
$handshake = @(
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"mcpb.ps1","version":"1.0.0"}}}'
    '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
) -join "`n"
$reply = ($handshake | & (Join-Path $stage "server\corescout-mcp.exe") --no-start 2>$null) -join "`n"
$listed = ([regex]::Matches($reply, '"(corescout_[a-z_]+)"') |
    ForEach-Object { $_.Groups[1].Value }) | Sort-Object -Unique
if ($listed.Count -ne $actual.Count) {
    throw "the staged server listed $($listed.Count) tools; the table has $($actual.Count)."
}
if ($reply -notmatch '"readOnlyHint"' -or $reply -notmatch '"destructiveHint"' -or $reply -notmatch '"title"') {
    throw "the staged server's tools are missing title/readOnlyHint/destructiveHint annotations, which the directory requires."
}
Write-Host "Handshake: $($listed.Count) tools listed, annotations present"

# --- pack --------------------------------------------------------------------

$out = Join-Path $root "$OutDir\CoreScout.mcpb"
if (Test-Path $out) { Remove-Item $out -Force }
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath "$out.zip" -CompressionLevel Optimal
Move-Item "$out.zip" $out

$size = [math]::Round((Get-Item $out).Length / 1MB, 1)
$hash = (Get-FileHash $out -Algorithm SHA256).Hash
Write-Host ""
Write-Host "$out  ($size MB)"
Write-Host "SHA-256: $hash"
Write-Host ""
Write-Host "Submit at https://clau.de/desktop-extention-submission"
