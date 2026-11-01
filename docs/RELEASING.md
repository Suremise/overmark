# Releasing Overmark

A release is two files, their checksums and the third-party notices, built by `release.yml` on a
version tag and published by hand after the manual checklist below:

| File | What |
|---|---|
| `overmark-<version>-x64.msi` | Installs **per user, with no admin rights**, to `%LOCALAPPDATA%\Programs\Overmark`; or **per machine** for IT with `ALLUSERS=1` (below) |
| `overmark-<version>-portable.zip` | The same files and an empty `portable` marker: unzip anywhere, and everything is kept beside the exe |
| `SHA256SUMS.txt` | The SHA-256 of both, as `sha256sum -c` reads them; users compare their download with it |
| `THIRD-PARTY-NOTICES.txt` | Every crate's licence and the ported source's, also inside both packages |

Both packages hold `overmark.exe`, `LICENSE.txt` and `THIRD-PARTY-NOTICES.txt`. Nothing is signed
yet, so Windows SmartScreen warns about both (see the checklist).

## Steps

1. **Update what changed.**
   - Regenerate the notices: `pwsh scripts/third-party-notices.ps1` (needs cargo-about 0.9.2,
     below). Commit `THIRD-PARTY-NOTICES.txt` if it changed: About shows it, and the release
     fails if it's out of date. Code ported from another project goes in `third_party/ported/`
     (its licence file and a row in its README), and the notices pick it up.
   - Move `CHANGELOG.md`'s Unreleased notes under a dated section for the version
     (`## [<version>] - <yyyy-mm-dd>`), and leave an empty `## [Unreleased]` above it. The
     release's notes are that section, so write it for the people downloading Overmark, and
     say that it isn't signed yet and how to check the download.
2. **Set the version** in `Cargo.toml` (`[workspace.package] version`), and run `cargo check` so
   `Cargo.lock` follows. Regenerate the notices again (they name the version). Add the
   release's settings files to the tests (docs/TESTING.md, Settings from every release).
3. **Build locally and check** (optional, but it's what the workflow does):
   `pwsh scripts/package.ps1` writes `target\dist\`. It checks, and fails on, each of: the
   notices up to date; the exe's imports (no C runtime DLL); the exe's own icon (resource 1)
   and its version resource, the crate's version (`crates/overmark/build.rs`), which the MSI's
   File table carries too, so an upgrade replaces the exe; the MSI's text from
   `locales/en/installer.ftl`; per user by default and per machine on `ALLUSERS=1`; its
   registry values only under HKMU; the Run value only on `STARTWITHWINDOWS=1`; its upgrade
   settings; starting Overmark after an interactive per-user install only; the MSI's three
   files; and the zip's four, the `portable` marker empty. It packages the exe cargo reports
   building, wherever cargo's target directory is (`scripts/cargo-exe.ps1`), never a leftover
   file at `target\release`; `pwsh scripts/test-package.ps1` checks that, with an overridden
   target directory and a stale exe at the default path, all in a temporary Cargo fixture (it
   never touches the checkout). **Don't install the MSI on a machine
   that matters:** look inside the zip, and decompile the MSI
   (`wix msi decompile target\dist\overmark-<version>-x64.msi -o x.wxs`). Once the build is
   checked, run `pwsh scripts/clean-build.ps1`: it keeps `target\release` and `target\dist`, and
   removes only the workspace crates' dev and test builds, so the next dist build stays quick
   and the checkout doesn't grow by gigabytes (docs/TESTING.md, Disk space).
4. **Commit, tag and push** (only with the maintainer's say-so): `git tag v<version>` and push
   the tag. `release.yml` checks the tag matches the version, runs `cargo deny check` and the
   tests, builds the exe, the MSI, the zip and the checksums, and **drafts** a GitHub release
   with them and the notices, its notes the version's section of `CHANGELOG.md` (it fails if
   there isn't one). CI (`ci.yml`) has already run fmt, clippy, the tests, `cargo deny` and
   `overmark-core` on Linux for the push.
5. **Run the manual checklist** below on the draft's files, on a test machine or account.
6. **Publish the draft** once the checklist passes.

### Build-only tools

| Tool | Install |
|---|---|
| Rust | `rust-toolchain.toml` (rustup) |
| cargo-deny | `cargo install cargo-deny --locked` |
| cargo-about 0.9.2 | `cargo install cargo-about --version 0.9.2 --locked --features cli` |
| cargo-nextest 0.9.149 | `cargo install cargo-nextest --version 0.9.149 --locked`: runs the tests (docs/TESTING.md) |
| WiX Toolset 5.0.2 | `dotnet tool install --global wix --version 5.0.2` (needs the .NET 8 SDK) |
| Python 3.10+ and Pillow | the icons (`assets/export.py`; assets/README.md) |
| MSVC build tools and Windows SDK | Visual Studio's C++ build tools: the linker, the C runtime's static libraries, and the SDK's `rc.exe` for the exe's icon and version (`embed-resource`, a build dependency, runs it) |

None of these tools ships, **but one thing from the MSVC build tools does: Microsoft's C
runtime is linked into the exe** (`.cargo/config.toml`, `+crt-static`), so it needs no Visual C++
redistributable, which would need an admin install. It's under Microsoft's terms ("Distributable
Code" in the Visual Studio licence), not an MIT-style licence: **the one deliberate exception to
the licence rule** (docs/ARCHITECTURE.md, Decisions), shipped unmodified. THIRD-PARTY-NOTICES
names it in its part 3, which `scripts/third-party-notices.ps1` writes, and the README's licence
line has a footnote. `package.ps1` checks the exe imports no C runtime DLL and nothing that isn't
part of Windows.

**WiX 5, not cargo-wix or WiX 6+:** `cargo-wix` drives WiX 3, which needs the .NET Framework 3.5
(an admin install); WiX 6 and later come with the Open Source Maintenance Fee terms. WiX 5 is
MS-RL and only builds the MSI: none of it ships in it. The MSI uses **no WiX extension and no
WixUI**, whose dialogs, bitmaps and custom-action DLLs (MS-RL) would be embedded in it, against
the licence rule; Windows shows its own progress window instead. So **there's no licence page in the
installer** (MIT asks no one to accept anything): `LICENSE.txt` is installed
beside the exe, and the README and About show it.

## The MSI

`installer/overmark.wxs`:

- **One MSI, per user or per machine** (`Scope="perUserOrMachine"`: `ALLUSERS=2` with
  `MSIINSTALLPERUSER=1`).
  - **Per user, no admin** (the default: double-click, or `msiexec /i overmark-<version>-x64.msi`):
    to `%LOCALAPPDATA%\Programs\Overmark`, a Start menu shortcut for that user.
  - **Per machine, for IT** (Intune, SCCM), from an elevated prompt or as SYSTEM:

    ```
    msiexec /i overmark-<version>-x64.msi ALLUSERS=1 /qn
    ```

    to `%ProgramFiles%\Overmark`, with the shortcut in every user's Start menu. `/qn` is
    silent; `/qb` shows progress; add `/l*v install.log` for a log. Uninstall with
    `msiexec /x overmark-<version>-x64.msi /qn` (or the product code).
  - Components keep their key paths under `HKMU\Software\Overmark\Installer`: HKCU per user,
    HKLM per machine.
- **Start with Windows**, optional: `STARTWITHWINDOWS=1` writes the Run value `Overmark` =
  `"<install folder>\overmark.exe"`, under HKCU per user (the same value the app's own setting
  writes and removes) or HKLM per machine (for every user). Uninstalling removes it.

  ```
  msiexec /i overmark-<version>-x64.msi STARTWITHWINDOWS=1
  msiexec /i overmark-<version>-x64.msi ALLUSERS=1 STARTWITHWINDOWS=1 /qn
  ```

- **Its own text** (the downgrade message and the shortcut's description) is the `installer-*`
  messages in `locales/en/installer.ftl`; `package.ps1` passes it in and checks the built MSI
  carries it.
- **The app icon** (`assets/icon/overmark.ico`) is on the shortcut and in Apps & features.
- **After an interactive per-user install or upgrade Overmark starts**, so its tray icon shows
  it worked. A silent or basic-UI one (`/qn`, `/qb`: `UILevel` 3 or less) starts nothing, and
  nor does a per-machine one, which runs elevated: the tray app must run as the user.
- **Upgrades** are major upgrades (same `UpgradeCode`), **the same version included**
  (`AllowSameVersionUpgrades`): every build has a new `ProductCode`, so without it a rebuilt MSI
  of the same version installs beside the old one. The old installs are removed first
  (`Schedule="afterInstallInitialize"`). WiX's ICE61 warning about same-version upgrades is
  expected.
- **Uninstalling** leaves the settings and logs in `%APPDATA%\Overmark`.

## The portable copy

The zip's empty `portable` marker makes Overmark keep its `settings.toml` and `logs\` beside the
exe. It writes nothing to the registry, except the HKCU Run value when the user ticks
Start with Windows. If the folder can't be written to, it says so and keeps settings in memory.

## Manual checklist

Copy this for each release, fill it in and keep it with the release (in its notes, or attached
to them).

## The README's clip

`assets/overmark-demo.gif` is rendered, not recorded: `examples/demo_clip.rs` draws a made-up
desktop and puts Overmark's own toolbar, strokes, veil and capture notice over it, frame by
frame, so no real screen is in it. Regenerate it when the toolbar or the drawing changes how
they look, with [ffmpeg](https://ffmpeg.org) on the PATH (a build-only tool):

```
cargo run -p overmark --release --example demo_clip
```

It writes the frames to `%TEMP%\overmark-demo-clip\frames`, then `assets/overmark-demo.gif` (960
px wide, for the README), and beside the frames `overmark-demo.mp4` (1920 px, H.264, no audio)
and `overmark-demo-poster.png` (a still of the Frozen part) for the website, which aren't
committed. Look through the GIF before committing it.

**Release:** v______ **Date:** ________ **Machine and Windows version:** ______________________
**Tester:** ________

| Done | Check | Result and notes |
|:---:|---|---|
| [ ] | **100%:** on a 100% display, the layer, the toolbar, the tray icon and Settings are sharp and laid out; strokes are crisp | |
| [ ] | **150%:** the same on a 150% display | |
| [ ] | **Mixed DPI:** two displays at different scaling (100% and 150%): draw on each, move the pointer between them; strokes are crisp on both, the toolbar follows the pointer's monitor | |
| [ ] | **Two monitors:** one layer per monitor; **Ctrl+S saves the monitor the pointer is on**, at its full resolution; region save works across them | |
| [ ] | **A pen tablet:** pressure changes the width; the pen's eraser end erases; a mouse still works fully | |
| [ ] | **A Teams or Zoom screen share:** the audience sees the ink (and the frozen frame), but **not the toolbar or the dim**; guides show unless "Show guides on screen shares" is off | |
| [ ] | **Win+Shift+D** opens Live and **Win+Shift+F** opens Frozen, each in well under 100 ms; F toggles with strokes kept; Esc closes and the strokes go | |
| [ ] | **Print Screen** as a binding: with Windows 11's "Use the Print screen key to open screen capture" off, Print Screen opens the layer; Settings explains the setting | |
| [ ] | **A taken hotkey:** with another app holding Win+Shift+D, Settings and a tray notice say so | |
| [ ] | **Portable from a USB stick:** unzip to a stick and run; `settings.toml` and `logs\` appear beside the exe; nothing in `%APPDATA%\Overmark`; nothing in the registry (unless Start with Windows is ticked) | |
| [ ] | **Portable from a read-only folder:** Overmark says it can't save its settings and keeps them in memory; it writes nowhere else | |
| [ ] | **MSI per user without admin:** double-click as a standard user; no UAC prompt; installed to `%LOCALAPPDATA%\Programs\Overmark`; Overmark starts (tray icon); the Start menu shortcut works, with the app icon | |
| [ ] | **MSI upgrade:** install the previous version, then this one (and this one over itself): Apps & features shows **one** Overmark, and settings are kept | |
| [ ] | **MSI uninstall, per user:** from Settings > Apps; the files and shortcut go; settings in `%APPDATA%\Overmark` stay | |
| [ ] | **MSI per machine:** `msiexec /i overmark-<version>-x64.msi ALLUSERS=1 /qn` from an elevated prompt installs silently to `%ProgramFiles%\Overmark`, for every user (the shortcut in another account's Start menu), and starts nothing; `msiexec /x … /qn` removes it | |
| [ ] | **Start with Windows from the MSI:** `STARTWITHWINDOWS=1`: Overmark starts at the next sign-in; the app's own Start with Windows setting shows it ticked and can untick it | |
| [ ] | **SmartScreen's unsigned warning:** download and run both files; Windows shows "Windows protected your PC"; **More info**, then **Run anyway**, as the README says; note what a user has to click | |
| [ ] | **A clean Windows VM** (no Visual C++ redistributable): the portable copy starts, and so does the MSI's | |
| [ ] | **Checksums:** `SHA256SUMS.txt` matches both downloaded files (`sha256sum -c SHA256SUMS.txt --ignore-missing`, or `Get-FileHash`) | |
| [ ] | **No network:** with Check for updates off, Resource Monitor (or a firewall log) shows Overmark makes no connection through a session of drawing and saving | |
| [ ] | **Idle:** in the tray, no CPU use and under 30 MB of memory | |

Adapted from Legible's docs/RELEASING.md.
