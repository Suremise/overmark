# Security

Overmark sits on top of everything on your screen, captures it, and listens for global hotkeys,
so security reports matter a lot. Thank you for taking the time.

## Reporting a vulnerability

**Please report it privately, not in a public issue.**

Use GitHub's private vulnerability reporting: on
[github.com/Suremise/overmark](https://github.com/Suremise/overmark), open the **Security** tab and
choose **Report a vulnerability**, or go straight to
[the report form](https://github.com/Suremise/overmark/security/advisories/new). Only the
maintainers can see it.

Please include:

- what the problem is, and what someone could do with it;
- the steps that show it, the Overmark version (Settings > About) and the Windows version;
- anything you've found about a fix.

We aim to answer within a week, and to keep you told as we work on it. We'll credit you in the
advisory and the changelog unless you'd rather we didn't. Please give us a reasonable time to fix
it before you make it public.

## Supported versions

Fixes go into the latest release only.

## What's most sensitive

Reports in these areas matter most. Each is a promise Overmark makes:

- **No keyboard or mouse hooks.** Global hotkeys use Windows' `RegisterHotKey` only. Overmark
  never installs a low-level keyboard or mouse hook, and never sees keys pressed in other apps. A
  way for Overmark to observe input outside its own windows and hotkeys is a vulnerability.
- **No network.** Overmark makes no network connections unless "Check for updates" is turned on,
  and then only to read the latest release on GitHub; it never downloads or installs anything by
  itself. Any other connection, or a download, is a vulnerability.
- **Captures stay local.** A capture goes only to the clipboard and the folder the user chose,
  with a file name made safe for Windows. Writing a capture, or any file, anywhere else, or
  uploading one, is a vulnerability.
- **What screen sharing sees.** The toolbar and the dim are excluded from capture; the ink is
  meant to be seen. The **frozen frame is a copy of the screen**: it must never be written to
  disk or kept after the layer closes unless the user saves a capture.
- **Portable mode.** A portable copy keeps everything in its own folder and writes nothing to the
  registry except "Start with Windows" when ticked. If its folder can't be written to, it says so
  and keeps settings in memory: it never falls back to writing somewhere else.
- **Settings files.** A `settings.toml`, however it was written, can't make Overmark run
  anything, reach the network, or write outside its own folder and the captures folder.

Overmark has no account, no telemetry, and no background service: it runs as you, from the tray,
only while it's running.
