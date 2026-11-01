<#
.SYNOPSIS
    Checks Get-CargoExe (which package.ps1 uses) returns the exe cargo built, never a stale one.
.DESCRIPTION
    Everything happens in a temporary folder; the checkout's own files and builds are never
    touched. A tiny Cargo project (one binary, no dependencies) is written there, with a stale
    stand-in planted where an exe would be by default (<fixture>\target\release\fixture-bin.exe),
    and CARGO_TARGET_DIR pointed at another folder there. It checks Get-CargoExe:
      - returns the freshly built exe in CARGO_TARGET_DIR, a real exe (MZ), never the stale one;
      - returns the same exe again when it's up to date (cargo builds nothing);
    and that the stale file is left as it was. The folder has a space in its name, as a user's
    profile might. The temporary folder is removed afterwards, however it ends.

        pwsh scripts/test-package.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'cargo-exe.ps1')

$temp = Join-Path ([IO.Path]::GetTempPath()) ("overmark package test " + [guid]::NewGuid())
$savedTarget = $env:CARGO_TARGET_DIR
try {
    # The fixture: a standalone package with one binary and no dependencies.
    $fixture = Join-Path $temp 'fixture'
    New-Item -ItemType Directory -Force -Path (Join-Path $fixture 'src') | Out-Null
    [IO.File]::WriteAllText((Join-Path $fixture 'Cargo.toml'),
        "[package]`nname = `"fixture-bin`"`nversion = `"0.1.0`"`nedition = `"2021`"`n`n[workspace]`n")
    [IO.File]::WriteAllText((Join-Path $fixture 'src\main.rs'), "fn main() {}`n")
    Push-Location $fixture
    try {
        & cargo generate-lockfile --offline
        if ($LASTEXITCODE -ne 0) { throw "cargo generate-lockfile failed ($LASTEXITCODE)" }
    }
    finally { Pop-Location }

    # A stale stand-in where an exe would be with the default target directory.
    $stale = Join-Path $fixture 'target\release\fixture-bin.exe'
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $stale) | Out-Null
    [IO.File]::WriteAllText($stale, 'stale, not an exe')

    $env:CARGO_TARGET_DIR = Join-Path $temp 'override target'
    $before = Get-Date
    $exe = Get-CargoExe -Root $fixture -Bin 'fixture-bin'

    $full = [IO.Path]::GetFullPath($exe)
    $wantedDir = [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
    if (-not $full.StartsWith($wantedDir, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Get-CargoExe returned $full, not an exe under CARGO_TARGET_DIR ($wantedDir)."
    }
    if ($full -ieq [IO.Path]::GetFullPath($stale)) { throw 'Get-CargoExe returned the stale exe.' }
    $bytes = [IO.File]::ReadAllBytes($full)
    if ($bytes.Length -lt 2 -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) { throw "$full isn't an exe." }
    if ((Get-Item -LiteralPath $full).LastWriteTime -lt $before) { throw "$full wasn't built by this run." }

    # Up to date: cargo builds nothing, and still reports the same exe.
    $again = Get-CargoExe -Root $fixture -Bin 'fixture-bin'
    if ([IO.Path]::GetFullPath($again) -ne $full) { throw "An up-to-date build returned $again, not $full." }

    if ([IO.File]::ReadAllText($stale) -ne 'stale, not an exe') { throw 'The stale stand-in was changed.' }
    Write-Host "Get-CargoExe returned the built $full (twice); the stale exe was ignored and left as it was."
}
finally {
    $env:CARGO_TARGET_DIR = $savedTarget
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $temp
}
