# Overmark: notes for agents

Overmark draws on the screen, then captures it: a small resident Windows app in Rust. These
notes are for every agent working on this repository, whichever CLI runs it: Codex reads this
file, and Claude Code reads it through `CLAUDE.md`.

- How the code fits together, the decisions behind it, the crate check and the licence rule:
  [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **Testing: what to run when, flaky tests, and the rules every test keeps:
  [docs/TESTING.md](docs/TESTING.md).** In short: as you work, fmt, clippy and the tests of what
  you changed; the full suite (`cargo nextest run --workspace`) once before review; a flaky
  test's proof is its file 50 times under load plus one full run, never 50 full runs; tests wait
  for conditions, never for time; the full suite stays under 60 s.

**The rules every change keeps:** platform-free code in `overmark-core`, Windows code in
`overmark-win`; user-visible strings through Fluent (`locales/en/`); everything shipped MIT or
equally permissive (crates, fonts, icons, data), with system fonts only, and `cargo deny check`
passing; no network and no telemetry; global hotkeys through `RegisterHotKey` only, never a
keyboard or mouse hook; tests use temp folders only.

**Disk space** ([docs/TESTING.md](docs/TESTING.md#disk-space)): cargo never removes old test
exes, so a checkout's `target` grows by gigabytes over a few changes.
- **Builders: once your work is merged** (after the last merge of a run of changes), run
  `pwsh scripts/clean-build.ps1` in your checkout. It keeps the compiled dependencies, so the
  next change's first build stays quick.
- **Reviewers: after each verdict,** delete your own probe folders under `%TEMP%`.

**No global keyboard or mouse input, ever** ([docs/TESTING.md](docs/TESTING.md#rules-every-test-keeps)),
in tests, probes or scripts: no SendKeys, SendInput, keybd_event or mouse_event, wscript or
AutoHotkey, no keyboard or mouse hooks, and no clicking at screen coordinates. Synthetic input
goes to whatever is in the foreground, which is the user's work (on another project, a probe's
typed path once arrived as a message the user sent).
- **Drive only your own windows:** post `WM_POINTER*` or `WM_HOTKEY` messages to your own
  window's handle; use UI Automation patterns (ValuePattern, InvokePattern and the like) on
  elements found by your own process ID or window handle; or drive egui through its AccessKit
  tree in your own process.
- **If you can't do it that way,** don't drive the real UI: prove it with tests, and say so in
  your summary of the change.

Probes, logs and scratch files go in your scratchpad or a temp folder, never in the repository or
beside a checkout. Personal, machine-specific notes belong in `CLAUDE.local.md` (Claude Code reads
it; keep it out of git), not here.

**The repository is public** at [github.com/Suremise/overmark](https://github.com/Suremise/overmark).
Push, tag or publish only with the owner's go-ahead, given at the time in your own session.

**Merging into main:** each piece of work lands on main as **one squashed commit**
(`git merge --squash <branch>`, then one commit), never a merge commit. Its message is for the
public: a short summary line, then what changed for a user or contributor and why. No card or
issue numbers (GitHub links `#n` to its issues), no people's names, no review rounds, and no
wording in the message text about the tools or agents that made it. That rule covers the text
only: the `Co-Authored-By:` trailer at the end stays on every commit, as honest attribution in
GitHub's standard form, so never strip it or rebuild commits to remove it before a push. Commit
as the repository's usual identity (its no-reply address). The round-by-round commits stay on your branch; once merged, reset your branch
to the new main so the next piece of work starts from it.

**Code, comments and docs are public too:** they cite no card or issue numbers, no people's names
(beyond the copyright and author lines), no private design brief or its sections, and no review
rounds. Give the reason itself ("never a hook: global hotkeys go through `RegisterHotKey`"), not
where it was decided.
