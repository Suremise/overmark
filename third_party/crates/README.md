# Crates' licence files

Some crates in `overmark.exe` are published without their licence file, so cargo-about gives
their licence's template and the crates hold no copyright notice of their own. Where the
project's repository has the licence file, its copy is kept here, and
`scripts/third-party-notices.ps1` takes those crates' copyright notices from it, offline. A row
names each crate with the version its licence file was checked against: when a crate's version
changes, the script stops until the row is checked and updated.

| Crates | Version | From | Licence |
|---|---|---|---|
| ecolor, eframe, egui, egui_glow, egui-winit, emath, epaint | 0.37.0 | <https://github.com/emilk/egui>, `LICENSE-MIT` at tag 0.37.0 (841a328), the same at eframe's published revision 3c6eb82 | MIT (`egui-LICENSE-MIT`) |
