# Contributing to Overmark

Thanks for helping. Bug reports, fixes, translations and ideas are all welcome. Please read the
[Code of Conduct](CODE_OF_CONDUCT.md) first, and report security problems privately, as
[SECURITY.md](SECURITY.md) says, not in a public issue.

For anything bigger than a small fix, open an issue first so we can agree on the shape before you
spend time on it. Overmark should stay fast (the layer opens in under 100 ms), never in the way,
and friendly to locked-down company laptops: no admin rights, no network, no hooks.

## Building and checking

You need Windows 10 or 11, [Rust](https://rustup.rs) and Visual Studio's C++ build tools with the
Windows SDK. The toolchain is pinned in `rust-toolchain.toml`, and rustup installs it on first
use.

```
cargo build
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace        # the tests, in parallel (cargo install cargo-nextest)
cargo test --workspace --doc         # the doc tests, which nextest doesn't run
cargo deny check                     # licences, advisories and sources (cargo install cargo-deny)
```

`cargo test --workspace` runs the same tests without nextest, more slowly.
[docs/TESTING.md](docs/TESTING.md) says what to run when, how to run only the tests you changed,
and the rules every test keeps.

A pull request must pass all of these. CI runs them on Windows for every push to `main` and every
pull request, and runs `overmark-core`'s tests on Linux too, to keep the core platform-free.

**Disk space.** Cargo never removes old test exes, so `pwsh scripts/clean-build.ps1` now and then
frees what piles up in `target\` while keeping the compiled dependencies.

### Golden images

How strokes, shapes, text and the backdrops look is checked against PNGs in
`crates/overmark-core/tests/golden/`, with a small tolerance for maths that differs between
platforms. A mismatch writes the render to `target/golden-failures/`. After a deliberate change to
how something looks, regenerate them, look at every changed PNG, and commit them with the change:

```powershell
$env:UPDATE_GOLDEN=1; cargo test -p overmark-core --test golden; Remove-Item Env:UPDATE_GOLDEN
```

`docs/settings.example.toml` is regenerated the same way, with `UPDATE_EXAMPLE=1 cargo test -p
overmark-core`, after a change to the settings.

## How the code is laid out

```
crates/
  overmark-core/  platform-free: the stroke model and the perfect-freehand port, the tools,
                  rendering with tiny-skia, undo and redo, the eraser's hit-testing, text,
                  backdrops and guides, saves' compositing, the settings model, the strings
  overmark-win/   Windows only: the overlay windows, capture, hotkeys, pointer input,
                  display affinity, the tray, the clipboard, notifications, autostart, paths
  overmark/       the exe: wires the two together, with the toolbar and the Settings window
locales/en/       the strings, in Fluent
assets/           the icons, drawn by assets/export.py
docs/             ARCHITECTURE.md, TESTING.md, the settings example, the user guide
installer/        the MSI's source
third_party/      the licences of code ported into Overmark
scripts/          build-only scripts
```

**`overmark-core` must build and test on any OS**: nothing in it calls Windows, so a Mac or Linux
port later only adds a platform crate. Windows code goes in `overmark-win`.
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains each part and the decisions behind it.

## Strings and languages

Every word the user sees comes from the [Fluent](https://projectfluent.org) files in
`locales/<language>/`. Logs stay in English.

- **Adding a string:** add `key = Text` to the right file in `locales/en/` (`toolbar.ftl`,
  `settings.ftl`, `errors.ftl`, ...). Keys are lowercase and hyphenated, prefixed by where they
  show (`tray-`, `tool-`, `toolbar-`, `toast-`, `settings-`, `hotkey-`, `error-`, ...). Add a
  comment naming each `$argument`. In Rust, use `tr("key")`, or `tr!("key", name = value)` with
  arguments, and pass the key as a literal: a test checks that every key the code uses exists in
  English.
- **No words in the code:** a test (`no_text_is_written_into_ui_calls`) fails if a label,
  button, tooltip, accessible name, window title, tray notice, capture notice or console message
  is given a string literal with words in it, on the call's own line. Put the words in
  `locales/en/` instead (a window's title through `win32::window::title`). It reads lines, so it
  misses a literal on the next line or one that's all lower case: keep to the rule anyway.
  An id that only looks like words can take `// not ui text` on its line.
- **Adding a language:** copy `locales/en/` to `locales/<tag>/` (such as `locales/de/`) and
  translate the values, leaving the keys. Missing keys fall back to English one by one. Then add
  it to `LOCALES` in `crates/overmark-core/src/i18n/embedded.rs`: the tests fail until every file
  on disk is listed there.

## Dependencies: the licence rule

Everything Overmark ships must be **MIT or equally permissive**: MIT, Apache-2.0 (with or without
the LLVM exception), BSD-2-Clause, BSD-3-Clause, ISC, Zlib, 0BSD, BSL-1.0, CC0-1.0, or
Unicode-3.0 / Unicode-DFS-2016, or a dual licence that includes one of them. **No copyleft and no
extra conditions**: no GPL, LGPL, AGPL, MPL, EPL, CDDL or MS-RL, and **no fonts or assets under
the SIL Open Font License or the Ubuntu Font Licence**. Overmark uses Windows' own fonts at run
time and bundles none, and egui's `default_fonts` stays off. That covers everything in the exe,
the MSI and the zip: crates (and theirs), fonts, icons, images and data.

- **Rust crates:** `cargo deny check` enforces the rule for every crate in the tree (the allowed
  list is in `deny.toml`). Before adding one, check its licence and its dependencies'; prefer a
  well-used, maintained crate, with only the features you need; and say in the pull request what
  it's for and why it's worth it.
- **Ported code** (such as perfect-freehand) keeps its licence beside it in
  `third_party/ported/` and is credited in THIRD-PARTY-NOTICES.
- **Art:** icons are original, drawn by `assets/export.py`.

Build-only tools (the toolchain, WiX, CI actions, Pillow) don't ship, so the rule doesn't apply to
them, but record a new one in docs/ARCHITECTURE.md.

## Tests

- **Tests use temporary folders only.** A test never reads or writes your real settings, hotkeys,
  clipboard or captures folder, and cleans up after itself even when it fails.
- **Tests wait for conditions, never for time,** and pass under load, beside every other test.
- **No network:** Overmark makes no network calls of its own, and nor do its tests.
- **No global keyboard or mouse input,** ever: a test drives only its own windows.
- A fix comes with a test that fails without it.

## Commits and pull requests

- **A commit message's first line says what changed, in plain English** ("The eraser finds thin
  strokes on a fast swipe"), under about 72 characters, with the issue or pull request number at
  the end when there is one ("... (#12)"). Add a body when the why isn't obvious.
- Keep a pull request to one change. Update the docs it affects (the user guide for anything a
  user does, [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how it works), and add an
  `Unreleased` entry in [CHANGELOG.md](CHANGELOG.md) for anything a user would notice.
- A change to how something looks comes with before and after screenshots, and the golden images
  regenerated.
- By contributing, you agree that your contribution is under Overmark's [MIT licence](LICENSE).
