# Testing Overmark

How the tests are run, what to run when, and the rules every test keeps. For contributors,
people and coding agents alike. The aim is what's necessary without going overboard.

## Running the tests

The suite runs under [cargo-nextest](https://nexte.st), a build-only tool (never shipped):

```powershell
cargo install cargo-nextest --version 0.9.149 --locked   # once

cargo nextest run --workspace                            # every test but the ignored
cargo test --workspace --doc                             # the doc tests (nextest doesn't run them)
```

nextest runs every test as its own process, all the test binaries side by side. Its settings are
in `.config/nextest.toml`. **`cargo test --workspace` still works**, for anyone without nextest:
the same tests, slower.

**Just the tests you changed:** name the package and the binary (a file under `tests/`) or the
unit tests' module:

```powershell
cargo nextest run -p overmark-core -E 'test(/history::/)'
cargo nextest run -p overmark-win -E 'binary(hotkeys)'
```

**On a busy PC** (several builds at once), cap the threads: `-j 4`, and
`CARGO_BUILD_JOBS=4` for the build.

**The full suite should stay under 60 seconds** (the build aside). A test that needs longer, or
repeats something many times to catch a rare race, is marked `#[ignore = "stress: …"]` with a
quick version in the default run, and run by hand with `--run-ignored only`.

**The core is platform-free:** `cargo test -p overmark-core` passes on Linux too (CI's Linux job
runs it).

## Tests that take turns

Tests in different binaries run at the same moment. A test that touches something the OS shares
between processes goes in a **test group** with `max-threads = 1` in `.config/nextest.toml`,
never in a lock of its own (nextest runs each test as its own process, so an in-process `Mutex`
doesn't make tests take turns). The groups:

- `clipboard`: the real clipboard;
- `autostart`: the current user's Run key.

Put such a test's binary in its group with an override (the file says how). Everything else
shares what it uses safely: temp folders, `APPDATA` per test, names of its own.

## What to run when

- **As you work:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and the tests of the crates and files you changed (as above). Not the full suite.
- **Before review, once:** the full suite (`cargo nextest run --workspace`, and the doc tests if
  you changed a doc comment's example), and `cargo deny check` if a dependency changed.
- **Before merging:** the full suite again only if main moved since that run.
- **Reviewers** rely on the author's run record (the commit, each command, its result and where
  the log is), and rerun only what they doubt, plus the tests of anything they probe.

## Golden images

`crates/overmark-core/tests/golden.rs` renders known strokes and compares them with the PNGs in
`crates/overmark-core/tests/golden/`, allowing tiny differences between platforms (the file's
docs give the tolerance). A mismatch writes the render to `target/golden-failures/`. After a
deliberate change to how strokes look, regenerate them, look at each changed PNG and commit it:

```powershell
$env:UPDATE_GOLDEN=1; cargo test -p overmark-core --test golden; Remove-Item Env:UPDATE_GOLDEN
```

**Rendering speed:** `cargo bench -p overmark-core --bench redraw` times a long stroke on a 4K
layer (docs/ARCHITECTURE.md, Drawing and rendering).

## Settings from every release

Upgrading never loses a setting. `crates/overmark-core/src/settings/released/` holds each
release's `settings.toml` as that release wrote it: `<version>-default.toml` (its defaults) and
`<version>-changed.toml` (every setting away from its default). A test loads each with today's
code and checks there are no warnings and every value the file set is kept; keys added since
take their defaults. Another checks each changed file, and the tests' `changed()` that writes
the next one, sets every key away from its default: **a new setting goes in `changed()`** with
a value of its own. **Each release adds its own pair** while it's being readied, written by
that release's code, then lists them in the test's `RELEASED`:

```powershell
$env:UPDATE_RELEASED='<version>'; cargo test -p overmark-core released; Remove-Item Env:UPDATE_RELEASED
```

The files are never changed afterwards: a failure means a release's file no longer loads as it
did, so fix the loading (an alias for a renamed key, say), not the file.

## A flaky test

Fix it at its cause: a test that waits for what it expects, or a race in the product. **The proof
is the affected test file 50 times under parallel load plus one full run**, never 50 full runs:

```powershell
# 50 runs of one binary while another binary's run gives the load; then the full suite once.
cargo nextest run -p overmark-win -E 'binary(hotkeys)' --stress-count 50
cargo nextest run --workspace
```

Run another binary beside it, or the full suite in a second terminal, for the load. Log both runs
where a reviewer can read them.

## Rules every test keeps

- **Wait for conditions, never for time.** Poll what you expect to see, with a generous bound for
  a hung run, rather than sleep and then check. A timeout bounds a hung run and is never the fix.
- **Clean up, even on failure.** Temp folders and anything else a test makes are removed in
  `Drop`, so a failed assertion still cleans up.
- **Pass under load,** not only alone: every test runs beside all the others.
- **Nothing outside temp folders:** no test writes to the real `%APPDATA%\Overmark`, the real Run
  key or the real clipboard outside its test group.
- **No network.** Overmark makes no network calls, and nor do its tests.
- **No global keyboard or mouse input, ever,** in tests, probes or scripts: no SendKeys,
  SendInput, keybd_event or mouse_event, wscript or AutoHotkey, no keyboard or mouse hooks, and
  no clicking at screen coordinates. It goes to whatever is in the foreground, which is the
  user's work. Drive only your own windows: `WM_POINTER*` or `WM_HOTKEY` messages posted to your
  own window's handle, UI Automation patterns on elements found by your own process ID or
  window handle, or egui's AccessKit tree in your own process. If you can't, don't drive the
  real UI: prove it with tests, and say so in the summary.
- **Never activate a real window,** your own included, in tests or probes: don't send or post
  `WM_ACTIVATE` to one, and don't call `SetActiveWindow` or `SetForegroundWindow` on it.
  `DefWindowProc`'s handling of `WM_ACTIVATE` sets the focus and activates the window for real,
  even with the minimised flag set, so it takes the user's foreground. To test activation
  handling, call the window procedure or handler directly (as
  `overlay::tests::an_ink_layer_says_when_it_is_activated` does); for z-order, use helpers that
  don't activate, such as `window::to_top`.

## Disk space

Cargo never removes an old test exe: each round of edits builds new ones, and the incremental
caches grow beside them, so a checkout's `target\debug` reaches tens of GB within days (one of
Replica's worktrees reached 65 GB). Overmark keeps it down from the start, in two ways:

- **Less debug info in dev and test builds** (`Cargo.toml`): `debug = "line-tables-only"` for
  the workspace's crates, so panics, test failures and backtraces still name the file and line,
  and none for the dependencies. Release builds are unchanged. Measured on Replica and Legible,
  it makes a full test build's `target\debug` less than half the size and the build quicker
  (Legible, 27 Oct 2026: 530 MB to 332 MB, 33 s to 20 s from empty). A debugger shows no local
  variables in a dev build; to get them back for a session, set `CARGO_PROFILE_DEV_DEBUG=full`
  (and rebuild).
- **`pwsh scripts/clean-build.ps1`** removes the workspace crates' dev and test builds (their
  exes, rlibs, every stale test exe and the incremental caches) and keeps the compiled
  dependencies and `target\release`, so the next build stays quick. `-All` removes everything
  cargo built (a full rebuild follows). Both print `target`'s size before and after. Neither
  ever touches `target\ui-shots` (screenshots awaiting approval), `target\dist`, golden images
  or anything else outside the checkout's `target` (a cargo build folder set elsewhere with
  `CARGO_TARGET_DIR` or `build.build-dir` is left alone), and both stop before cleaning anything
  if `target` is, or holds, a junction or symbolic link (cargo never makes one).
  `pwsh scripts/test-clean-build.ps1` checks all of it in a throwaway workspace, and CI's
  Windows job runs that check. Builders run the script once their work is merged (AGENTS.md);
  the release steps run it once a dist build is checked, keeping `target\release`.
