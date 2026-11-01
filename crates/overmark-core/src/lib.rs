//! Overmark's platform-free core: the stroke model, tools, geometry, rendering, undo history,
//! hit-testing and the settings model. Nothing here may depend on Windows (or any OS): it
//! builds and tests on every platform, so a later port only adds a platform crate.

pub mod backdrop;
pub mod colour;
pub mod composite;
pub mod drawing;
pub mod freehand;
pub mod geometry;
pub mod hit;
pub mod hotkey;
pub mod hotkey_status;
pub mod i18n;
pub mod laser;
pub mod pen;
pub mod render;
pub mod session;
pub mod settings;
pub mod text;
pub mod tool;
pub mod update;

/// The app's name, as shown to people and used for its folders.
pub const APP_NAME: &str = "Overmark";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_overmark() {
        assert_eq!(APP_NAME, "Overmark");
    }
}
