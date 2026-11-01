//! The look the toolbar and the Settings window share: egui with Windows' own
//! fonts, Overmark's light and dark palettes, the component library and line icons.
//!
//! Copied and adapted from Replica's `crates/replica/src/ui/` (its docs/ARCHITECTURE.md, "Shared
//! with other apps"): `fonts.rs` and `icons.rs` as they are, `theme.rs` with Overmark's copper
//! accent, and `widgets.rs` less what only Replica uses.
//!
//! **Using it:** each egui window calls [`install`] once, in its creator (or a test harness's
//! context setup). It loads the fonts, sets both themes' styles, and leaves egui following
//! Windows' light or dark mode, which then switches live:
//!
//! ```ignore
//! eframe::run_native("Overmark Settings", options, Box::new(|cc| {
//!     overmark::ui::install(&cc.egui_ctx)?; // NoTextFont: word it from Fluent and show it
//!     Ok(Box::new(SettingsApp::new()))
//! }))
//! ```
//!
//! Then draw with [`widgets`] (cards, setting rows, buttons, toggles, segmented controls…),
//! [`icons::Icon`] and [`theme::of`] for colours; call [`theme::paint_focus`] at the end of a
//! frame for the keyboard focus ring.
//!
//! **Cost while no window is open:** the resident app calls [`install`] once at start-up, for
//! the toolbar's egui, kept warm (docs/ARCHITECTURE.md, The toolbar): idle memory, no
//! CPU.

pub mod fonts;
pub mod icons;
pub mod theme;
pub mod widgets;

use eframe::egui;

/// Gives `ctx` Windows' fonts and Overmark's styles for both themes, following Windows' light or
/// dark mode. Fails only if no text font was found (the window can't show any text then).
pub fn install(ctx: &egui::Context) -> Result<(), fonts::NoTextFont> {
    fonts::install(ctx)?;
    theme::install(ctx);
    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::System);
    Ok(())
}
