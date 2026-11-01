## What and why

<!-- What this changes, and the problem it solves. Link the issue: "Fixes #12". -->

## How to check it

<!-- What a reviewer can run or try to see it working. For a change to how something looks, before and after screenshots. -->

## Checklist

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo nextest run --workspace` (or `cargo test --workspace`) and `cargo deny check` pass
- [ ] **New dependencies pass §3.1:** MIT or equally permissive, no copyleft, no OFL or Ubuntu-licensed fonts or assets (see [CONTRIBUTING.md](https://github.com/Suremise/overmark/blob/main/CONTRIBUTING.md#dependencies-the-licence-rule)), or there are none
- [ ] `overmark-core` stays platform-free: Windows code is in `overmark-win`
- [ ] No network calls, no telemetry, and no keyboard or mouse hooks
- [ ] Tests use temporary folders only, wait for conditions rather than time, and a fix has a test that fails without it
- [ ] Golden images regenerated and checked, if how something looks changed
- [ ] New user-visible text is in `locales/en/`, not in the code
- [ ] Docs updated where they're affected (the user guide, `docs/ARCHITECTURE.md`), and an `Unreleased` entry in `CHANGELOG.md` for anything a user would notice
