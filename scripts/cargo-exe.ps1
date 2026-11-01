<#
.SYNOPSIS
    Defines Get-CargoExe: builds a binary with cargo and returns the path of the exe cargo made.
.DESCRIPTION
    Dot-source it (`. scripts/cargo-exe.ps1`). The path comes from cargo's own build output
    (`--message-format=json`, the compiler-artifact's `executable`), so it's the exe this build
    made wherever cargo's target directory is (CARGO_TARGET_DIR, build.target-dir), never a file
    that happens to be at target\release from an earlier build. When the exe is up to date, cargo
    builds nothing and still reports it.
#>

function Get-CargoExe {
    param(
        [Parameter(Mandatory)][string]$Root,
        [Parameter(Mandatory)][string]$Bin
    )
    Push-Location $Root
    try {
        # Diagnostics go to stderr rendered as usual; stdout is one JSON message per line.
        $lines = & cargo build --release --locked --bin $Bin --message-format=json-render-diagnostics
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    }
    finally { Pop-Location }
    $exes = @($lines |
            Where-Object { $_ -is [string] -and $_.StartsWith('{') } |
            ForEach-Object { $_ | ConvertFrom-Json } |
            Where-Object {
                $_.reason -eq 'compiler-artifact' -and $_.target.name -eq $Bin -and
                $_.target.kind -contains 'bin' -and $_.executable
            } |
            ForEach-Object { $_.executable })
    if ($exes.Count -ne 1) { throw "cargo reported $($exes.Count) executables for $Bin, not one." }
    if (-not (Test-Path -LiteralPath $exes[0])) { throw "cargo reported $($exes[0]), which doesn't exist." }
    $exes[0]
}
