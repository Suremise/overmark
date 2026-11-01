<#
.SYNOPSIS
    Writes THIRD-PARTY-NOTICES.txt: every crate in overmark.exe, and the source ported into it.
.DESCRIPTION
    Part 1 is the crates of the Windows build (cargo-about, with about.toml and about.hbs),
    without dev or build dependencies, then the copyright notices of the crates whose licence
    text there is its template (see Get-CopyrightNotices; for a crate published without its
    licence file, the project's own, kept in third_party/crates with its README.md). Part 2 is the source ported into
    Overmark's own crates (third_party/ported/: the table in its README.md, and each project's
    licence file beside it). Part 3 is Microsoft's C runtime, linked in statically: not a crate,
    so cargo-about doesn't see it, and under Microsoft's terms, the one recorded exception to
    the licence rule (docs/ARCHITECTURE.md, Decisions).

    The file is committed: About shows it (it's embedded), and both packages ship it.
    -Check writes it to a temp file and fails if the committed one differs (package.ps1 runs that).
    Build-only: it needs cargo-about 0.9.2 (cargo install cargo-about --version 0.9.2 --locked
    --features cli) and makes no network requests.

    Copied from Legible's scripts/third-party-notices.ps1 and adapted: the copyright collection is
    Legible's as it is; Overmark has no MPL crates and no JavaScript, and adds its ported source.
#>
[CmdletBinding()]
param([switch]$Check)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# A licence text whose copyright line is still its template's placeholder, whatever the licence:
# "<year> <copyright holders>" (MIT), "<year> <owner>" (BSD), "[yyyy] [name of copyright owner]"
# (Apache's appendix), and the like. An email address in angle brackets isn't one.
$TemplateCopyright = '(?im)^\s*Copyright\b.*(<(?![^>]*@)[^>]+>|\[[^\]]+\])'

# The crates (name and version) under each licence block of cargo-about's text whose copyright
# line is a placeholder.
function Get-TemplatedCrates([string]$crateText) {
    $crates = [System.Collections.Generic.List[string]]::new()
    foreach ($block in ($crateText -split "(?m)^-{80}$")) {
        if ($block -notmatch $TemplateCopyright) { continue }
        foreach ($m in [regex]::Matches($block, '(?m)^  - (\S+) (\S+)(?: |$)')) {
            $crates.Add("$($m.Groups[1].Value) $($m.Groups[2].Value)")
        }
    }
    , @($crates | Sort-Object -Unique -CaseSensitive)
}

# Where a crate ships no filled-in licence file, cargo-about gives the licence's template, and
# the crate's copyright notices are left out. This collects them from the crate's own files, as
# built (its licence files' "Copyright" lines, any other notice file whole, such as a COPYING
# or NOTICE, and its SPDX-FileCopyrightText headers), for every crate under such a template,
# into a section of their own: offline, and in a fixed order, so -Check stays reproducible.
function Get-CopyrightNotices([string]$crateText, [string]$root) {
    $rule = '-' * 80
    $crates = Get-TemplatedCrates $crateText
    if ($crates.Count -eq 0) { return '' }

    Push-Location $root
    # cargo writes UTF-8; read it as such whatever the console's code page (authors' names
    # aren't all ASCII), so the output is the same however the script is started.
    $encoding = [Console]::OutputEncoding
    [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
    try {
        $metadata = & cargo metadata --format-version 1 --offline --locked | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed ($LASTEXITCODE)" }
    }
    finally {
        [Console]::OutputEncoding = $encoding
        Pop-Location
    }
    $dirs = @{}
    $authors = @{}
    foreach ($p in $metadata.packages) {
        $dirs["$($p.name) $($p.version)"] = Split-Path -Parent $p.manifest_path
        $authors["$($p.name) $($p.version)"] = @($p.authors)
    }

    # Crates published without their licence file, whose project's own is kept in
    # third_party/crates (the table in its README.md), by name and version.
    $supplementDir = Join-Path $root 'third_party/crates'
    $supplements = @{}
    $supplementNames = @{}
    $rows = Get-Content -Encoding utf8 (Join-Path $supplementDir 'README.md') |
        Where-Object { $_ -match '^\| (?!Crates|---)' }
    foreach ($row in $rows) {
        if ($row -notmatch '^\| ([^|]+)\| ([^|]+)\| ([^|]+)\| [^|]*\(`([^`]+)`\) \|$') {
            throw "third_party/crates/README.md: can't read the row: $row"
        }
        $names = $Matches[1].Split(',') | ForEach-Object { $_.Trim() }
        $rowVersion = $Matches[2].Trim()
        $from = ($Matches[3].Trim() -replace '[<>`]', '')
        $file = Join-Path $supplementDir $Matches[4]
        if (-not (Test-Path $file)) { throw "third_party/crates/$($Matches[4]) is missing." }
        foreach ($n in $names) {
            $supplements["$n $rowVersion"] = @{ File = $file; From = $from }
            $supplementNames[$n] = $rowVersion
        }
    }
    foreach ($crate in $crates) {
        $name, $v = $crate.Split(' ')
        if ($supplementNames.ContainsKey($name) -and $supplementNames[$name] -ne $v) {
            throw "$name is $v, but third_party/crates/README.md checked its licence file against $($supplementNames[$name]): check it again and update the row."
        }
    }

    $lines = @(
        $rule,
        'Copyright notices of the crates above whose licence text is a template',
        '',
        'These crates'' licence text above is its template ("<year> <owner>" and the like): their',
        'copyright notices are in their own files, collected here from the crates as built (their',
        'licence and notice files, and the SPDX-FileCopyrightText headers of their files), or, for',
        'a crate published without its licence file, from its project''s own (third_party/crates).',
        ''
    )
    foreach ($crate in $crates) {
        $dir = $dirs[$crate]
        if (-not $dir) { throw "no source for $crate in cargo metadata" }
        $found = [System.Collections.Generic.SortedSet[string]]::new([StringComparer]::Ordinal)
        # Standard licence texts give their "Copyright" lines; any other notice file (such as a
        # LICENSE-LIBM-MIT for code taken from elsewhere) is given whole, below.
        $licenceFiles = @(Get-ChildItem -Path $dir -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE)' } | Sort-Object Name)
        $licenceDir = Join-Path $dir 'LICENSES'
        if (Test-Path $licenceDir) { $licenceFiles += Get-ChildItem -Path $licenceDir -File | Sort-Object Name }
        $whole = [System.Collections.Generic.List[object]]::new()
        foreach ($f in $licenceFiles) {
            $standard = $f.DirectoryName -eq $licenceDir -or $f.Name -match '^LICEN[CS]E(\.(md|txt))?$|^LICEN[CS]E[-_](MIT|APACHE)'
            if (-not $standard) { $whole.Add($f); continue }
            foreach ($line in (Get-Content -Encoding utf8 $f.FullName)) {
                $t = $line.Trim()
                # A notice, with its year or (c); not a template's placeholder.
                if ($t -match '^Copyright\s+(\(c\)|©|\d)' -and $t -notmatch '[<\[{]') { [void]$found.Add($t) }
            }
        }
        $sources = Get-ChildItem -Path $dir -Recurse -File -Include *.rs, *.toml, *.md
        foreach ($m in ($sources | Select-String -Pattern 'SPDX-FileCopyrightText:\s*(.+?)\s*$' -Encoding utf8)) {
            $holder = $m.Matches[0].Groups[1].Value
            if ($holder -notmatch '^(Copyright|©)') { $holder = "Copyright $holder" }
            [void]$found.Add($holder)
        }
        $lines += $crate
        if ($found.Count -gt 0) {
            $lines += @($found | ForEach-Object { "  $_" })
        }
        foreach ($f in $whole) {
            $lines += "  $($f.Name):"
            $lines += @((Get-Content -Encoding utf8 $f.FullName) | ForEach-Object { "    $_".TrimEnd() })
        }
        if ($found.Count -eq 0 -and $whole.Count -eq 0 -and $supplements.ContainsKey($crate)) {
            # Published without its licence file: the project's own, kept in third_party/crates.
            $s = $supplements[$crate]
            foreach ($line in (Get-Content -Encoding utf8 $s.File)) {
                $t = $line.Trim()
                if ($t -match '^Copyright\s+(\(c\)|©|\d)' -and $t -notmatch '[\[{]') { [void]$found.Add($t) }
            }
            if ($found.Count -eq 0) { throw "third_party/crates/$(Split-Path -Leaf $s.File) has no copyright notice." }
            $lines += @($found | ForEach-Object { "  $_" })
            $lines += "  (from $($s.From); the crate is published without it)"
        }
        elseif ($found.Count -eq 0 -and $whole.Count -eq 0) {
            $names = @($authors[$crate] | Where-Object { $_ })
            if ($names.Count -gt 0) {
                $lines += "  No copyright notice in its files. Its authors, as its Cargo.toml names them: $($names -join ', ')"
            }
            else {
                $lines += '  No copyright notice in its files, and its Cargo.toml names no authors.'
            }
        }
        $lines += ''
    }
    return ($lines -join "`n")
}

$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'THIRD-PARTY-NOTICES.txt'
$work = Join-Path ([IO.Path]::GetTempPath()) ("notices-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $crates = Join-Path $work 'crates.txt'
    Push-Location $root
    try {
        & cargo about generate --workspace --offline --fail about.hbs -o $crates
        if ($LASTEXITCODE -ne 0) { throw "cargo about failed ($LASTEXITCODE)" }
    }
    finally { Pop-Location }

    $version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
    $rule = '=' * 80
    $thin = '-' * 80
    $header = @(
        "THIRD-PARTY NOTICES FOR OVERMARK $version",
        '',
        'Overmark is under the MIT licence (LICENSE, beside this file). It is built from the',
        'third-party Rust crates in part 1, carries the source ported into its own crates in',
        'part 2, and links in Microsoft''s C runtime (part 3). Their licences and notices follow.',
        'Overmark bundles no fonts: text uses the system''s own.',
        '',
        $rule
    ) -join "`n"

    # Part 2: each ported project (the table in third_party/ported/README.md), with its licence.
    $portedDir = Join-Path $root 'third_party/ported'
    $ported = @($rule, 'PART 2: PORTED SOURCE', '',
        'Code from these projects is ported into Overmark''s own crates (third_party/ported/README.md).')
    $rows = Get-Content -Encoding utf8 (Join-Path $portedDir 'README.md') |
        Where-Object { $_ -match '^\| (?!What|---)([^|]+)\| `([^`]+)` \| ([^|]+)\| ([^|]+)\|$' }
    if (@($rows).Count -eq 0) { throw 'third_party/ported/README.md lists no ported project.' }
    foreach ($row in $rows) {
        $null = $row -match '^\| ([^|]+)\| `([^`]+)` \| ([^|]+)\| ([^|]+)\|$'
        $what = $Matches[1].Trim()
        $where = $Matches[2]
        $from = $Matches[3].Trim() -replace '[<>]', ''
        $licence = $Matches[4].Trim()
        if ($licence -notmatch '\(`([^`]+)`\)') { throw "third_party/ported/README.md: no licence file named for $what." }
        $licenceFile = Join-Path $portedDir $Matches[1]
        if (-not (Test-Path $licenceFile)) { throw "third_party/ported/$($Matches[1]) is missing." }
        $ported += ''
        $ported += $thin
        $ported += $what
        $ported += $from
        $ported += "Ported to Rust in $where."
        $ported += ''
        $ported += ((Get-Content -Raw -Encoding utf8 $licenceFile) -replace "`r`n", "`n").TrimEnd()
    }

    # Part 3: Microsoft's C runtime, linked in statically (.cargo/config.toml).
    $crt = @('', $rule, 'PART 3: MICROSOFT C RUNTIME', '',
        'overmark.exe links in parts of Microsoft''s C runtime (the static libraries of the Visual C++',
        'build tools and the Windows SDK: libcmt, libvcruntime and libucrt), unmodified, so it needs',
        'no Visual C++ redistributable installed. They are Copyright (c) Microsoft Corporation, and',
        'are distributed under Microsoft''s terms for "Distributable Code" in the Microsoft Visual',
        'Studio licence, not under an open-source licence: an explicit exception to Overmark''s',
        'rule that everything it ships is MIT or equally permissive. See',
        'https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files') -join "`n"

    $crateText = (Get-Content -Raw -Encoding utf8 $crates) -replace "`r`n", "`n"
    $copyrights = Get-CopyrightNotices $crateText $root
    $text = ($header, $crateText.TrimEnd(), '', $copyrights, ($ported -join "`n"), $crt) -join "`n"
    $text = ($text -replace "`r`n", "`n").TrimEnd() + "`n"

    # Every crate whose licence text above is a template has its own notices below it (a
    # crate under a BSD or MIT template with no notice of its own would ship without one).
    $templated = Get-TemplatedCrates $crateText
    $missing = @($templated | Where-Object {
            $copyrights -notmatch "(?m)^$([regex]::Escape($_))`n  \S"
        })
    if ($missing) { throw "No copyright notices collected for $($missing -join ', '), whose licence text is a template." }

    if ($Check) {
        $committed = if (Test-Path $out) { (Get-Content -Raw -Encoding utf8 $out) -replace "`r`n", "`n" } else { '' }
        if ($committed -ne $text) {
            Write-Error 'THIRD-PARTY-NOTICES.txt is out of date: run scripts/third-party-notices.ps1 and commit it.'
            exit 1
        }
        Write-Host 'THIRD-PARTY-NOTICES.txt is up to date.'
    }
    else {
        [IO.File]::WriteAllText($out, $text, [Text.UTF8Encoding]::new($false))
        Write-Host "Wrote $out ($((Get-Item $out).Length) bytes)"
    }
}
finally {
    Remove-Item -Recurse -Force $work
}
