<#
.SYNOPSIS
    Builds Overmark's release files: the MSI, the portable zip and their SHA-256 checksums.
.DESCRIPTION
    Into target\dist (or -OutDir):
      overmark-<version>-x64.msi          per user, no admin, or per machine (installer\overmark.wxs)
      overmark-<version>-portable.zip     the exe, the "portable" marker, the licences
      SHA256SUMS.txt                      the checksums of both, as `sha256sum -c` reads them

    Both hold overmark.exe, LICENSE.txt and THIRD-PARTY-NOTICES.txt. It fails if
    THIRD-PARTY-NOTICES.txt is out of date (scripts\third-party-notices.ps1 -Check). It builds the
    exe with `cargo build --release` (nothing to do if it's up to date) and packages the exe cargo
    reports building, wherever its target directory is (scripts\cargo-exe.ps1): never a file that
    happens to be at target\release from an earlier build.

    The exe is checked to import no C runtime DLL and nothing that isn't part of Windows (the C
    runtime is linked in: .cargo\config.toml), so it starts on a clean PC without an admin
    install of a redistributable, and to carry its icon (ID 1) and a version resource of the
    crate's version (crates\overmark\build.rs); the MSI's File table carries that version too. The MSI is checked, with Windows Installer's own API (opened
    read-only: nothing is installed), to: carry its text from locales\en\installer.ftl; install
    per user by default and per machine on ALLUSERS=1 (a dual-purpose package); write its
    registry values only under HKMU (the user's hive per user, the machine's per machine); write
    the Run value only when STARTWITHWINDOWS=1; upgrade any earlier install, the same version
    included; and start Overmark after an interactive per-user install only. The zip is checked
    to hold exactly its four files.

    Build-only tools: the Rust toolchain, cargo-about 0.9.2, and WiX 5.0.2
    (dotnet tool install --global wix --version 5.0.2). Nothing is signed yet.

    Adapted from Legible's scripts/package.ps1.
#>
[CmdletBinding()]
param([string]$OutDir)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $root 'target\dist' }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value

# The MSI's own text, from Fluent: the installer-* messages, one line of plain text each.
function Read-InstallerText([string]$path) {
    $messages = @{}
    $lines = @(Get-Content -Encoding utf8 $path)
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^(installer-[a-z0-9-]*)\s*=\s*(.*)$') {
            $next = if ($i + 1 -lt $lines.Count) { $lines[$i + 1] } else { '' }
            if ($Matches[2] -match '[{}]' -or -not $Matches[2].Trim() -or $next -match '^\s+\S') {
                throw "$path`: $($Matches[1]) must be one line of plain text."
            }
            $messages[$Matches[1]] = $Matches[2].Trim()
        }
    }
    $messages
}
$installerText = Read-InstallerText (Join-Path $root 'locales\en\installer.ftl')
foreach ($key in 'installer-downgrade', 'installer-shortcut-description') {
    if (-not $installerText.ContainsKey($key)) { throw "locales\en\installer.ftl has no $key." }
}
# MSI "formatted" text, where [name] is a property: brackets are escaped.
function Format-Msi([string]$text) { [regex]::Replace($text, '[\[\]]', { "[\$($args[0].Value)]" }) }
$downgrade = Format-Msi $installerText['installer-downgrade']
$shortcut = $installerText['installer-shortcut-description']

& (Join-Path $PSScriptRoot 'third-party-notices.ps1') -Check
if ($LASTEXITCODE) { throw 'THIRD-PARTY-NOTICES.txt is out of date.' }
. (Join-Path $PSScriptRoot 'cargo-exe.ps1')
$built = Get-CargoExe -Root $root -Bin 'overmark'
Write-Host "Packaging $built"

# The DLLs a 64-bit PE file imports, from its import directory.
function Get-ImportedDlls([string]$path) {
    $b = [IO.File]::ReadAllBytes($path)
    $pe = [BitConverter]::ToInt32($b, 0x3C)
    $sections = [BitConverter]::ToUInt16($b, $pe + 6)
    $optional = $pe + 24
    $table = $optional + [BitConverter]::ToUInt16($b, $pe + 20)
    function ToOffset([int]$rva) {
        for ($i = 0; $i -lt $sections; $i++) {
            $s = $table + $i * 40
            $va = [BitConverter]::ToInt32($b, $s + 12)
            $size = [Math]::Max([BitConverter]::ToInt32($b, $s + 8), [BitConverter]::ToInt32($b, $s + 16))
            if ($rva -ge $va -and $rva -lt $va + $size) { return $rva - $va + [BitConverter]::ToInt32($b, $s + 20) }
        }
        throw "RVA $rva isn't in a section of $path"
    }
    $entry = ToOffset ([BitConverter]::ToInt32($b, $optional + 120))
    $dlls = @()
    while (($name = [BitConverter]::ToInt32($b, $entry + 12)) -ne 0) {
        $at = ToOffset $name
        $end = [Array]::IndexOf($b, [byte]0, $at)
        $dlls += [Text.Encoding]::ASCII.GetString($b, $at, $end - $at).ToLowerInvariant()
        $entry += 20
    }
    $dlls
}

# What both packages hold.
$stage = Join-Path $OutDir 'stage'
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
$exe = Join-Path $stage 'overmark.exe'
Copy-Item -LiteralPath $built $exe
# It starts on a clean PC: no C runtime DLL (it's linked in), nothing that isn't Windows' own.
$system32 = Join-Path $env:SystemRoot 'System32'
$foreign = Get-ImportedDlls $exe | Where-Object {
    $_ -match '^(vcruntime|msvcp|ucrtbase|api-ms-win-crt-)' -or
    -not ($_ -match '^(api-ms-win-|ext-ms-)' -or (Test-Path (Join-Path $system32 $_)))
}
if ($foreign) { throw "overmark.exe imports $($foreign -join ', '): link the C runtime statically (.cargo\config.toml)." }
# It carries a version resource of the crate's version (crates\overmark\build.rs): Details shows
# it, and the installer compares it with the installed exe's.
$fileVersion = (@($version -split '[.+-]' | Select-Object -First 3) + '0') -join '.'
$info = (Get-Item $exe).VersionInfo
if ($info.FileVersionRaw -ne [version]$fileVersion -or $info.ProductVersion -ne $version -or $info.ProductName -ne 'Overmark') {
    throw "overmark.exe's version resource is '$($info.ProductName)' $($info.ProductVersion) ($($info.FileVersionRaw)), not Overmark $version ($fileVersion)."
}
# Its own icon (ID 1, the lowest, which Explorer, the taskbar and the shortcut show).
Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Icon]::ExtractIcon($exe, -1, 32)
if (-not $icon) { throw 'overmark.exe has no icon resource 1.' }
$icon.Dispose()
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $stage 'LICENSE.txt')
Copy-Item (Join-Path $root 'THIRD-PARTY-NOTICES.txt') $stage

# The MSI.
$msi = Join-Path $OutDir "overmark-$version-x64.msi"
& wix build (Join-Path $root 'installer\overmark.wxs') -arch x64 -d "Version=$version" -d "Stage=$stage" `
    -d "DowngradeMessage=$downgrade" -d "ShortcutDescription=$shortcut" -o $msi
if ($LASTEXITCODE -ne 0) { throw "wix build failed ($LASTEXITCODE)" }
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($msi, '.wixpdb'))

# Its tables, read with Windows Installer's own API (opened read-only: nothing is installed).
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @([string]$msi, [int]0))
# Each row of a query, as an array of its fields' text.
function Get-MsiRows([string]$query) {
    $view = $database.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $database, @($query))
    $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
    while ($record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)) {
        $count = $record.GetType().InvokeMember('FieldCount', 'GetProperty', $null, $record, $null)
        , @(1..$count | ForEach-Object { $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, $_) })
    }
    $view.GetType().InvokeMember('Close', 'InvokeMethod', $null, $view, $null) | Out-Null
}
function Get-Property([string]$name) {
    @(Get-MsiRows "SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$name'") | ForEach-Object { $_[0] }
}
try {
    # The Fluent text, as it is.
    $inMsi = @{
        # MajorUpgrade's DowngradeErrorMessage becomes a launch condition (NOT WIX_DOWNGRADE_DETECTED).
        'installer-downgrade'            = @(Get-MsiRows "SELECT ``Description`` FROM ``LaunchCondition``" | ForEach-Object { $_[0] })
        'installer-shortcut-description' = @(Get-MsiRows "SELECT ``Description`` FROM ``Shortcut``" | ForEach-Object { $_[0] })
    }
    if ($inMsi['installer-downgrade'] -notcontains $downgrade) {
        throw "The MSI's downgrade message isn't installer.ftl's '$downgrade'."
    }
    if (@($inMsi['installer-shortcut-description']) -notcontains $shortcut) {
        throw "The MSI's shortcut description is '$($inMsi['installer-shortcut-description'])', not installer.ftl's '$shortcut'."
    }

    # Per user by default, per machine on ALLUSERS=1: a dual-purpose package.
    if ((Get-Property 'ALLUSERS') -ne '2' -or (Get-Property 'MSIINSTALLPERUSER') -ne '1') {
        throw "The MSI isn't per user by default (ALLUSERS '$(Get-Property 'ALLUSERS')', MSIINSTALLPERUSER '$(Get-Property 'MSIINSTALLPERUSER')')."
    }
    # It installs under ProgramFiles64Folder, which is %LOCALAPPDATA%\Programs per user.
    $folder = @(Get-MsiRows "SELECT ``Directory_Parent``, ``DefaultDir`` FROM ``Directory`` WHERE ``Directory`` = 'INSTALLFOLDER'")
    if ($folder.Count -ne 1 -or $folder[0][0] -ne 'ProgramFiles64Folder' -or $folder[0][1] -notmatch 'Overmark$') {
        throw "The MSI's INSTALLFOLDER is '$($folder | ForEach-Object { $_ -join ' ' })', not ProgramFiles64Folder\Overmark."
    }

    # Every registry value under HKMU (root -1): HKCU per user, HKLM per machine.
    $registry = @(Get-MsiRows "SELECT ``Root``, ``Key``, ``Name``, ``Value``, ``Component_`` FROM ``Registry``")
    if ($registry | Where-Object { $_[0] -ne '-1' }) { throw 'The MSI writes registry values outside HKMU.' }
    # The Run value: named as the app's own setting names it, only in its own component, and
    # that component only on STARTWITHWINDOWS=1.
    $run = @($registry | Where-Object { $_[1] -eq 'Software\Microsoft\Windows\CurrentVersion\Run' })
    if ($run.Count -ne 1 -or $run[0][2] -ne 'Overmark' -or $run[0][3] -ne '"[INSTALLFOLDER]overmark.exe"' -or $run[0][4] -ne 'StartWithWindows') {
        throw "The MSI's Run value is '$($run | ForEach-Object { $_ -join ', ' })', not Overmark = `"[INSTALLFOLDER]overmark.exe`" in StartWithWindows."
    }
    $condition = @(Get-MsiRows "SELECT ``Condition`` FROM ``Component`` WHERE ``Component`` = 'StartWithWindows'")
    if ($condition.Count -ne 1 -or $condition[0][0] -ne 'STARTWITHWINDOWS = 1') {
        throw "The MSI's Start with Windows is on '$($condition | ForEach-Object { $_ })', not STARTWITHWINDOWS = 1."
    }

    # A same-version MSI upgrades too (AllowSameVersionUpgrades: the upgrade row's VersionMax is
    # this version, inclusive), else a rebuild installs beside the old one.
    $upgrade = @(Get-MsiRows "SELECT ``VersionMax``, ``Attributes`` FROM ``Upgrade`` WHERE ``ActionProperty`` = 'WIX_UPGRADE_DETECTED'")
    $inclusive = 0x200  # msidbUpgradeAttributesVersionMaxInclusive
    if ($upgrade.Count -ne 1 -or $upgrade[0][0] -ne $version -or -not ([int]$upgrade[0][1] -band $inclusive)) {
        throw "The MSI doesn't upgrade an install of its own version (AllowSameVersionUpgrades): its Upgrade row is '$($upgrade | ForEach-Object { $_ -join ', ' })'."
    }
    # Every earlier install is removed before this one installs (Schedule="afterInstallInitialize").
    $sequence = @{}
    Get-MsiRows "SELECT ``Action``, ``Sequence`` FROM ``InstallExecuteSequence``" | ForEach-Object { $sequence[$_[0]] = [int]$_[1] }
    $removeAt = $sequence['RemoveExistingProducts']
    $between = $sequence.Values | Where-Object { $_ -gt $sequence['InstallInitialize'] -and $_ -lt $removeAt }
    if (-not $removeAt -or $removeAt -le $sequence['InstallInitialize'] -or $between) {
        throw "The MSI's RemoveExistingProducts is at $removeAt, not just after InstallInitialize ($($sequence['InstallInitialize']))."
    }
    # Overmark starts after an interactive per-user install only (UILevel 5 is the full UI; /qn
    # and /qb are 2 and 3; a per-machine install is Privileged), never on an uninstall.
    $launch = @(Get-MsiRows "SELECT ``Condition`` FROM ``InstallExecuteSequence`` WHERE ``Action`` = 'LaunchOvermark'")
    if ($launch.Count -ne 1 -or $launch[0][0] -ne 'UILevel > 3 AND NOT (REMOVE~="ALL") AND NOT Privileged') {
        throw "The MSI starts Overmark on '$($launch | ForEach-Object { $_ })', not after an interactive per-user install only."
    }
    # Its files: the exe, the licence and the notices.
    $files = @(Get-MsiRows "SELECT ``FileName`` FROM ``File``" | ForEach-Object { ($_[0] -split '\|')[-1] } | Sort-Object)
    $wantedFiles = @('LICENSE.txt', 'THIRD-PARTY-NOTICES.txt', 'overmark.exe') | Sort-Object
    if (($files -join ',') -ne ($wantedFiles -join ',')) { throw "The MSI holds $($files -join ', '), not $($wantedFiles -join ', ')." }
    # The exe's row carries its version, so an upgrade replaces it (a file without one is only
    # replaced when it's unchanged on disk).
    $exeRow = @(Get-MsiRows "SELECT ``Version`` FROM ``File`` WHERE ``File`` = 'OvermarkExe'")
    if ($exeRow.Count -ne 1 -or $exeRow[0][0] -ne $fileVersion) {
        throw "The MSI's overmark.exe has version '$($exeRow | ForEach-Object { $_ })', not $fileVersion."
    }
}
finally {
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($database)
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
}

# The portable zip: the same files and the empty marker that keeps everything beside the exe.
$portable = Join-Path $OutDir 'portable'
if (Test-Path $portable) { Remove-Item -Recurse -Force $portable }
Copy-Item -Recurse $stage $portable
New-Item -ItemType File -Path (Join-Path $portable 'portable') | Out-Null
$zip = Join-Path $OutDir "overmark-$version-portable.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $portable '*') -DestinationPath $zip
Remove-Item -Recurse -Force $portable
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($zip)
try {
    $inZip = @($archive.Entries | ForEach-Object { $_.FullName } | Sort-Object)
    $marker = @($archive.Entries | Where-Object { $_.FullName -eq 'portable' })
    if ($marker.Count -ne 1 -or $marker[0].Length -ne 0) { throw "The zip's portable marker isn't an empty file." }
}
finally { $archive.Dispose() }
$wanted = @('LICENSE.txt', 'THIRD-PARTY-NOTICES.txt', 'overmark.exe', 'portable') | Sort-Object
if (($inZip -join ',') -ne ($wanted -join ',')) { throw "The zip holds $($inZip -join ', '), not $($wanted -join ', ')." }

# The checksums, as `sha256sum -c SHA256SUMS.txt` reads them.
$sums = foreach ($file in $msi, $zip) {
    '{0}  {1}' -f (Get-FileHash -Algorithm SHA256 $file).Hash.ToLowerInvariant(), (Split-Path -Leaf $file)
}
[IO.File]::WriteAllText((Join-Path $OutDir 'SHA256SUMS.txt'), (($sums -join "`n") + "`n"))
Remove-Item -Recurse -Force $stage

Write-Host "Built in ${OutDir}:"
Get-ChildItem $OutDir -File | ForEach-Object { Write-Host ("  {0} ({1:N0} bytes)" -f $_.Name, $_.Length) }
