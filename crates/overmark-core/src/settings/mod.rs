//! `settings.toml`: everything the Settings window sets.
//!
//! - **Human-readable, every key documented:** [`Settings::to_documented_toml`] writes a comment
//!   above each key, and [`Settings::save`] writes that. `docs/settings.example.toml` is the
//!   default file, and a test keeps it current.
//! - **Forgiving:** a missing key takes its default. An unknown key, a number out of range, a
//!   bad file-name pattern or a palette of the wrong size is a [`Warning`]: the rest loads, and
//!   the value is replaced by the nearest one allowed or the default.
//! - **Clear when it can't be used:** a file that isn't valid TOML, or whose values have the
//!   wrong type, is a [`SettingsError`] saying where, worded for people. The app shows it and
//!   runs on [`Settings::default`].
//!
//! Where the file lives (installed or portable) is the platform's business, not this module's:
//! it reads and writes the path it's given.

pub mod file_name;

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::colour::Colour;
use crate::tool::ToolKind;
use crate::tr;

/// The settings file's name.
pub const FILE_NAME: &str = "settings.toml";

/// The token at the start of [`CaptureSettings::save_folder`] that stands for the user's
/// Pictures folder, which the platform resolves.
pub const PICTURES_TOKEN: &str = "{pictures}";

/// Everything in `settings.toml`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub general: GeneralSettings,
    pub drawing: DrawingSettings,
    pub capture: CaptureSettings,
    pub advanced: AdvancedSettings,
    pub backdrops: BackdropSettings,
    pub guides: GuideSettings,
    pub toolbar: ToolbarSettings,
}

/// Settings, General.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    /// Whether Overmark starts when the user logs on to Windows.
    pub start_with_windows: bool,
    /// What left-clicking the tray icon does.
    pub tray_click: TrayClick,
    /// The hotkey that opens the layer in Live mode, such as `Win+Shift+D`. Empty: none.
    pub hotkey_draw: String,
    /// The hotkey that opens the layer Frozen. Empty: none.
    pub hotkey_draw_frozen: String,
    /// What Print Screen does, if anything.
    pub print_screen: PrintScreen,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        GeneralSettings {
            start_with_windows: false,
            tray_click: TrayClick::Live,
            hotkey_draw: "Win+Shift+D".to_owned(),
            hotkey_draw_frozen: "Win+Shift+F".to_owned(),
            print_screen: PrintScreen::Off,
        }
    }
}

/// What left-clicking the tray icon does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrayClick {
    /// Opens the layer in Live mode.
    #[default]
    Live,
    /// Opens the layer Frozen.
    Frozen,
    /// Opens Settings.
    Settings,
}

/// What the Print Screen key does: nothing (Windows keeps it), or open the layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrintScreen {
    #[default]
    Off,
    Live,
    Frozen,
}

/// Settings, Drawing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawingSettings {
    /// The tool picked when the layer opens.
    pub default_tool: ToolKind,
    /// The colours keys 1 to 9 pick: red, amber, yellow, green, blue, purple, black, white,
    /// then the custom one. Always [`PALETTE_SIZE`] long once loaded.
    pub palette: Vec<Colour>,
    /// Which of the palette the layer opens with (0 to 8).
    pub default_colour: usize,
    /// Each tool's starting width.
    pub widths: Widths,
    /// How dark the veil over the screen is while the layer is open: 0 (off) to 0.8.
    pub dim: f32,
    /// Whether strokes fade after a while.
    pub disappearing_ink: bool,
    /// How long a stroke stays before it fades, in seconds.
    pub disappearing_ink_seconds: f32,
    /// How long the laser pointer's trail lasts, in milliseconds.
    pub laser_trail_ms: u32,
    /// Whether a mouse's right-click on the drawing undoes, as Ctrl+Z does. On by default: a
    /// right-click there did nothing before, so it takes nothing away, and one by mistake is
    /// undone again with Ctrl+Y.
    pub right_click_undoes: bool,
}

/// The palette's length: eight fixed colours and one custom.
pub const PALETTE_SIZE: usize = 9;

/// The default palette.
pub const DEFAULT_PALETTE: [Colour; PALETTE_SIZE] = [
    Colour::rgb(0xE5, 0x39, 0x35), // red
    Colour::rgb(0xFF, 0x8F, 0x00), // amber
    Colour::rgb(0xFD, 0xD8, 0x35), // yellow
    Colour::rgb(0x43, 0xA0, 0x47), // green
    Colour::rgb(0x1E, 0x88, 0xE5), // blue
    Colour::rgb(0x8E, 0x24, 0xAA), // purple
    Colour::rgb(0x21, 0x21, 0x21), // black
    Colour::rgb(0xFF, 0xFF, 0xFF), // white
    Colour::rgb(0x00, 0xAC, 0xC1), // custom: teal until it's changed
];

/// The default dim: close to Windows' snip veil, to be tuned by eye.
pub const DEFAULT_DIM: f32 = 0.35;

impl Default for DrawingSettings {
    fn default() -> Self {
        DrawingSettings {
            default_tool: ToolKind::Pen,
            palette: DEFAULT_PALETTE.to_vec(),
            default_colour: 0,
            widths: Widths::default(),
            dim: DEFAULT_DIM,
            disappearing_ink: false,
            disappearing_ink_seconds: 3.0,
            laser_trail_ms: 500,
            right_click_undoes: true,
        }
    }
}

/// Each tool's starting width, in pixels at 100% scale. For text it's the font size, for
/// stamps the badge's diameter, for blur the size of its blocks. The eraser and the laser have
/// none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Widths {
    pub pen: f32,
    pub highlighter: f32,
    pub arrow: f32,
    pub rectangle: f32,
    pub ellipse: f32,
    pub line: f32,
    pub text: f32,
    pub stamp: f32,
    pub blur: f32,
}

impl Default for Widths {
    fn default() -> Self {
        Widths {
            pen: 4.0,
            highlighter: 20.0,
            arrow: 4.0,
            rectangle: 4.0,
            ellipse: 4.0,
            line: 4.0,
            text: 24.0,
            stamp: 28.0,
            blur: 12.0,
        }
    }
}

impl Widths {
    /// The smallest and largest width allowed.
    pub const RANGE: (f32, f32) = (1.0, 200.0);
    /// One step's factor: a notch of the wheel (holding a tool's key), or an arrow key on the
    /// toolbar's width slider, makes the width this much wider (or narrower).
    pub const STEP: f32 = 1.1;

    /// `width` moved `steps` steps (negative: narrower; a wheel's notch is one), kept in
    /// [`Self::RANGE`].
    pub fn stepped(width: f32, steps: f32) -> f32 {
        let (lo, hi) = Self::RANGE;
        (width * Self::STEP.powf(steps)).clamp(lo, hi)
    }

    /// A tool's starting width, if it has one.
    pub fn get(&self, tool: ToolKind) -> Option<f32> {
        Some(match tool {
            ToolKind::Pen => self.pen,
            ToolKind::Highlighter => self.highlighter,
            ToolKind::Arrow => self.arrow,
            ToolKind::Rectangle => self.rectangle,
            ToolKind::Ellipse => self.ellipse,
            ToolKind::Line => self.line,
            ToolKind::Text => self.text,
            ToolKind::Stamp => self.stamp,
            ToolKind::Blur => self.blur,
            ToolKind::Eraser | ToolKind::Laser => return None,
        })
    }

    fn fields(&self) -> [(&'static str, f32); 9] {
        [
            ("pen", self.pen),
            ("highlighter", self.highlighter),
            ("arrow", self.arrow),
            ("rectangle", self.rectangle),
            ("ellipse", self.ellipse),
            ("line", self.line),
            ("text", self.text),
            ("stamp", self.stamp),
            ("blur", self.blur),
        ]
    }

    fn fields_mut(&mut self) -> [(&'static str, &mut f32); 9] {
        [
            ("pen", &mut self.pen),
            ("highlighter", &mut self.highlighter),
            ("arrow", &mut self.arrow),
            ("rectangle", &mut self.rectangle),
            ("ellipse", &mut self.ellipse),
            ("line", &mut self.line),
            ("text", &mut self.text),
            ("stamp", &mut self.stamp),
            ("blur", &mut self.blur),
        ]
    }
}

/// Settings, Capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureSettings {
    /// Where captures are saved. May start with [`PICTURES_TOKEN`]; see
    /// [`CaptureSettings::resolve_save_folder`].
    pub save_folder: String,
    /// The file name pattern ([`file_name`] has the codes).
    pub file_name_pattern: String,
    /// Whether a capture is copied, saved or both.
    pub after_capture: AfterCapture,
    /// The saved file's format.
    pub format: ImageFormat,
    /// JPG quality, 1 to 100.
    pub jpg_quality: u8,
    /// Whether a capture includes the drawing unless asked otherwise.
    pub include_drawing: bool,
    /// Whether a notice shows after each capture.
    pub toast: bool,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        CaptureSettings {
            save_folder: format!("{PICTURES_TOKEN}\\Overmark"),
            file_name_pattern: file_name::DEFAULT_PATTERN.to_owned(),
            after_capture: AfterCapture::CopyAndSave,
            format: ImageFormat::Png,
            jpg_quality: 90,
            include_drawing: true,
            toast: true,
        }
    }
}

impl CaptureSettings {
    /// The save folder as a path: a leading [`PICTURES_TOKEN`] becomes `pictures` (the platform
    /// gives the user's Pictures folder), and the rest is joined to it; anything else is taken
    /// as a path. An empty setting is the default.
    pub fn resolve_save_folder(&self, pictures: &Path) -> PathBuf {
        let folder = self.save_folder.trim();
        let folder = if folder.is_empty() {
            Self::default().save_folder
        } else {
            folder.to_owned()
        };
        match folder.strip_prefix(PICTURES_TOKEN) {
            Some(rest) => rest
                .split(['\\', '/'])
                .filter(|part| !part.is_empty())
                .fold(pictures.to_path_buf(), |path, part| path.join(part)),
            None => PathBuf::from(folder),
        }
    }
}

/// What happens to a capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AfterCapture {
    /// Copied to the clipboard and saved (the default).
    #[default]
    CopyAndSave,
    Copy,
    Save,
}

impl AfterCapture {
    pub fn copies(self) -> bool {
        matches!(self, AfterCapture::CopyAndSave | AfterCapture::Copy)
    }

    pub fn saves(self) -> bool {
        matches!(self, AfterCapture::CopyAndSave | AfterCapture::Save)
    }
}

/// A saved capture's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageFormat {
    #[default]
    Png,
    Jpg,
}

impl ImageFormat {
    /// The file extension, without its dot.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpg => "jpg",
        }
    }
}

/// Settings, About and advanced.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvancedSettings {
    /// Whether Overmark looks for a newer release on GitHub. Off by default: no request unless
    /// the user asks for one.
    pub check_for_updates: bool,
}

/// The solid backdrops: their colours and optional hotkeys.
/// Live and Frozen open with `general.hotkey_draw` and `hotkey_draw_frozen`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackdropSettings {
    /// The whiteboard's colour.
    pub whiteboard_colour: Colour,
    /// The blackboard's colour.
    pub blackboard_colour: Colour,
    /// A global hotkey that opens the layer on the whiteboard. Empty (the default): none.
    pub hotkey_whiteboard: String,
    /// A global hotkey that opens the layer on the blackboard. Empty (the default): none.
    pub hotkey_blackboard: String,
}

/// The whiteboard's default colour.
pub const DEFAULT_WHITEBOARD: Colour = Colour::rgb(0xFF, 0xFF, 0xFF);
/// The blackboard's default colour: a dark grey, easier on the eyes than black.
pub const DEFAULT_BLACKBOARD: Colour = Colour::rgb(0x26, 0x2A, 0x2E);

impl Default for BackdropSettings {
    fn default() -> Self {
        BackdropSettings {
            whiteboard_colour: DEFAULT_WHITEBOARD,
            blackboard_colour: DEFAULT_BLACKBOARD,
            hotkey_whiteboard: String::new(),
            hotkey_blackboard: String::new(),
        }
    }
}

/// The guides over the backdrop (grid, dots, ruled lines).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuideSettings {
    /// The distance between grid lines, dots or ruled lines, in pixels at 100% scale.
    pub spacing: u32,
    /// Whether people on a screen share see the guides (on by default).
    pub show_on_shares: bool,
    /// Whether saves and copies include the guides (on by default).
    pub include_in_saves: bool,
}

impl Default for GuideSettings {
    fn default() -> Self {
        GuideSettings {
            spacing: 32,
            show_on_shares: true,
            include_in_saves: true,
        }
    }
}

impl GuideSettings {
    /// The smallest and largest spacing allowed.
    pub const SPACING_RANGE: (u64, u64) = (8, 400);
}

/// Where the toolbar was left, per monitor.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolbarSettings {
    /// Keyed by the platform's id for a monitor (on Windows, its device name).
    pub positions: BTreeMap<String, ToolbarPosition>,
}

/// The toolbar's top-left corner, in pixels from the monitor's top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolbarPosition {
    pub x: i32,
    pub y: i32,
}

/// Settings that loaded, and anything worth warning about.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub settings: Settings,
    pub warnings: Vec<Warning>,
}

/// Something in the file that was ignored or changed. The rest of the file loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum Warning {
    /// A key Overmark doesn't know, by its dotted path (`drawing.colour`).
    UnknownKey(String),
    /// A number outside what's allowed; `used` is the nearest allowed.
    OutOfRange {
        key: String,
        value: String,
        used: String,
    },
    /// The file name pattern can't be used, so the default is.
    BadPattern(file_name::PatternError),
    /// The palette didn't have [`PALETTE_SIZE`] colours.
    PaletteSize(usize),
    /// A backdrop colour wasn't opaque (`backdrops.whiteboard_colour`, say), so its alpha was
    /// dropped: a board hides the screen.
    TranslucentBackdrop(String),
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&match self {
            Warning::UnknownKey(key) => tr!("warning-settings-unknown-key", key = key.as_str()),
            Warning::OutOfRange { key, value, used } => tr!(
                "warning-settings-out-of-range",
                key = key.as_str(),
                value = value.as_str(),
                used = used.as_str()
            ),
            Warning::BadPattern(error) => {
                tr!("warning-settings-bad-pattern", detail = error.to_string())
            }
            Warning::TranslucentBackdrop(key) => {
                tr!("warning-settings-translucent-backdrop", key = key.as_str())
            }
            Warning::PaletteSize(count) => {
                tr!(
                    "warning-settings-palette-size",
                    count = *count,
                    size = PALETTE_SIZE
                )
            }
        })
    }
}

/// Settings that can't be used. The app shows it and runs on the defaults.
#[derive(Debug)]
pub enum SettingsError {
    /// The file couldn't be read.
    Read { path: PathBuf, source: io::Error },
    /// The file couldn't be written.
    Write { path: PathBuf, source: io::Error },
    /// The file isn't valid TOML, or a value has the wrong type.
    Corrupt {
        path: Option<PathBuf>,
        line: Option<usize>,
        column: Option<usize>,
        message: String,
    },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            SettingsError::Read { path, source } => tr!(
                "error-settings-unreadable",
                path = path.display().to_string(),
                detail = source.to_string()
            ),
            SettingsError::Write { path, source } => tr!(
                "error-settings-not-saved",
                path = path.display().to_string(),
                detail = source.to_string()
            ),
            SettingsError::Corrupt {
                path,
                line,
                column,
                message,
            } => {
                let path = path
                    .as_ref()
                    .map_or(FILE_NAME.to_owned(), |p| p.display().to_string());
                let detail = match (line, column) {
                    (Some(line), Some(column)) => tr!(
                        "error-settings-at",
                        line = *line,
                        column = *column,
                        detail = message.as_str()
                    ),
                    _ => message.clone(),
                };
                tr!("error-settings-unreadable", path = path, detail = detail)
            }
        };
        f.write_str(&text)
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SettingsError::Read { source, .. } | SettingsError::Write { source, .. } => {
                Some(source)
            }
            SettingsError::Corrupt { .. } => None,
        }
    }
}

impl Settings {
    /// Parses settings text.
    pub fn parse(text: &str) -> Result<Loaded, SettingsError> {
        // Settings files may start with a byte-order mark if edited in Notepad.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let settings: Settings = toml::from_str(text).map_err(|e| {
            let (line, column) = e.span().map(|s| line_col(text, s.start)).unzip();
            SettingsError::Corrupt {
                path: None,
                line,
                column,
                message: e.message().trim().to_owned(),
            }
        })?;
        let mut warnings = Vec::new();
        // The text parsed as Settings, so it's a valid table, and Settings always serialises.
        let raw: toml::Table = toml::from_str(text).expect("parsed above");
        let known = toml::Table::try_from(&settings).expect("settings serialise");
        unknown_keys(&raw, &known, "", &mut warnings);
        let settings = settings.checked(&mut warnings);
        Ok(Loaded { settings, warnings })
    }

    /// Loads a settings file.
    pub fn load(path: &Path) -> Result<Loaded, SettingsError> {
        let text = fs::read_to_string(path).map_err(|source| SettingsError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text).map_err(|e| match e {
            SettingsError::Corrupt {
                line,
                column,
                message,
                ..
            } => SettingsError::Corrupt {
                path: Some(path.to_path_buf()),
                line,
                column,
                message,
            },
            other => other,
        })
    }

    /// Loads a settings file, or the defaults if there isn't one yet.
    pub fn load_or_default(path: &Path) -> Result<Loaded, SettingsError> {
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Loaded {
                settings: Settings::default(),
                warnings: Vec::new(),
            }),
            _ => Self::load(path),
        }
    }

    /// Writes the documented file, replacing the old one only once the new one is written.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let error = |source| SettingsError::Write {
            path: path.to_path_buf(),
            source,
        };
        let tmp = write_temp(path, self.to_documented_toml().as_bytes()).map_err(error)?;
        replace(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            error(e)
        })
    }

    /// Brings every value into what's allowed, with a warning for each one changed.
    fn checked(mut self, warnings: &mut Vec<Warning>) -> Self {
        let mut clamp_f32 = |key: &str, value: &mut f32, (min, max): (f32, f32)| {
            let used = if value.is_nan() {
                min
            } else {
                value.clamp(min, max)
            };
            if used != *value {
                warnings.push(Warning::OutOfRange {
                    key: key.to_owned(),
                    value: value.to_string(),
                    used: used.to_string(),
                });
                *value = used;
            }
        };
        let d = &mut self.drawing;
        clamp_f32("drawing.dim", &mut d.dim, DIM_RANGE);
        clamp_f32(
            "drawing.disappearing_ink_seconds",
            &mut d.disappearing_ink_seconds,
            INK_SECONDS_RANGE,
        );
        for (name, width) in d.widths.fields_mut() {
            clamp_f32(&format!("drawing.widths.{name}"), width, Widths::RANGE);
        }

        let mut clamp_int = |key: &str, value: u64, (min, max): (u64, u64)| -> u64 {
            let used = value.clamp(min, max);
            if used != value {
                warnings.push(Warning::OutOfRange {
                    key: key.to_owned(),
                    value: value.to_string(),
                    used: used.to_string(),
                });
            }
            used
        };
        d.laser_trail_ms = clamp_int(
            "drawing.laser_trail_ms",
            d.laser_trail_ms.into(),
            LASER_TRAIL_RANGE,
        ) as u32;
        d.default_colour = clamp_int(
            "drawing.default_colour",
            d.default_colour as u64,
            (0, PALETTE_SIZE as u64 - 1),
        ) as usize;
        let c = &mut self.capture;
        c.jpg_quality = clamp_int("capture.jpg_quality", c.jpg_quality.into(), (1, 100)) as u8;
        self.guides.spacing = clamp_int(
            "guides.spacing",
            self.guides.spacing.into(),
            GuideSettings::SPACING_RANGE,
        ) as u32;

        for (key, colour) in [
            (
                "backdrops.whiteboard_colour",
                &mut self.backdrops.whiteboard_colour,
            ),
            (
                "backdrops.blackboard_colour",
                &mut self.backdrops.blackboard_colour,
            ),
        ] {
            if colour.a != 255 {
                warnings.push(Warning::TranslucentBackdrop(key.to_owned()));
                colour.a = 255;
            }
        }
        if d.palette.len() != PALETTE_SIZE {
            warnings.push(Warning::PaletteSize(d.palette.len()));
            d.palette.truncate(PALETTE_SIZE);
            let have = d.palette.len();
            d.palette.extend_from_slice(&DEFAULT_PALETTE[have..]);
        }
        if let Err(error) = file_name::check_pattern(&c.file_name_pattern) {
            warnings.push(Warning::BadPattern(error));
            c.file_name_pattern = file_name::DEFAULT_PATTERN.to_owned();
        }
        self
    }

    /// The settings as TOML with a comment above every key: what [`Settings::save`] writes.
    pub fn to_documented_toml(&self) -> String {
        let mut w = Writer::default();
        w.comment(
            "Overmark settings. Every key is optional: a missing key takes the default given",
        );
        w.comment("in its comment. Overmark rewrites this file when you change a setting in its");
        w.comment("Settings window, so comments you add here are not kept.");

        let g = &self.general;
        w.blank();
        w.table("[general]");
        w.key(
            "Start Overmark when you log on to Windows (default false).",
            "start_with_windows",
            &g.start_with_windows.to_string(),
        );
        w.key(
            "What left-clicking the tray icon does: \"live\" (draw; the default), \"frozen\" (draw on a\nfrozen screen) or \"settings\" (open Settings).",
            "tray_click",
            &enum_value(&g.tray_click),
        );
        w.key(
            "The hotkey that opens the drawing layer, Live (default \"Win+Shift+D\"). Keys are joined with\n\"+\": Ctrl, Alt, Shift and Win, then one key. \"\" turns it off.",
            "hotkey_draw",
            &str_value(&g.hotkey_draw),
        );
        w.key(
            "The hotkey that opens the layer on a frozen screen (default \"Win+Shift+F\"). \"\" turns it off.",
            "hotkey_draw_frozen",
            &str_value(&g.hotkey_draw_frozen),
        );
        w.key(
            "What Print Screen does: \"off\" (Windows keeps it; the default), \"live\" or \"frozen\". Windows 11\nsends Print Screen to the Snipping Tool unless you turn that off in its Settings.",
            "print_screen",
            &enum_value(&g.print_screen),
        );

        let d = &self.drawing;
        w.blank();
        w.table("[drawing]");
        w.key(
            "The tool picked when the layer opens (default \"pen\"): \"pen\", \"highlighter\", \"arrow\",\n\"rectangle\", \"ellipse\", \"line\", \"text\", \"stamp\", \"blur\", \"eraser\" or \"laser\".",
            "default_tool",
            &enum_value(&d.default_tool),
        );
        let palette: Vec<String> = d
            .palette
            .iter()
            .map(|c| str_value(&c.to_string()))
            .collect();
        w.key(
            "The colours keys 1 to 9 pick, as \"#RRGGBB\" (or \"#RRGGBBAA\"): red, amber, yellow, green, blue,\npurple, black, white, then one of your own. Nine colours; change any of them.",
            "palette",
            &format!("[{}]", palette.join(", ")),
        );
        w.key(
            "Which palette colour the layer opens with, 0 (the first) to 8 (default 0).",
            "default_colour",
            &d.default_colour.to_string(),
        );
        w.key(
            &format!(
                "How dark the screen is veiled while the layer is open, 0 (off) to {} (default {}).",
                DIM_RANGE.1, DEFAULT_DIM
            ),
            "dim",
            &float_value(d.dim),
        );
        w.key(
            "Make strokes fade a few seconds after you draw them, for calls (default false).",
            "disappearing_ink",
            &d.disappearing_ink.to_string(),
        );
        w.key(
            &format!(
                "How many seconds a stroke stays before it fades, {} to {} (default 3).",
                INK_SECONDS_RANGE.0, INK_SECONDS_RANGE.1
            ),
            "disappearing_ink_seconds",
            &float_value(d.disappearing_ink_seconds),
        );
        w.key(
            &format!(
                "How long the laser pointer's trail lasts, in milliseconds, {} to {} (default 500).",
                LASER_TRAIL_RANGE.0, LASER_TRAIL_RANGE.1
            ),
            "laser_trail_ms",
            &d.laser_trail_ms.to_string(),
        );
        w.key(
            "A right-click on the drawing undoes, as Ctrl+Z does (default true). The mouse's right
button only: a pen's barrel button and touch never undo.",
            "right_click_undoes",
            &d.right_click_undoes.to_string(),
        );

        w.blank();
        w.comment(&format!(
            "Each tool's starting width in pixels at 100% scale, {} to {}. For text it's the font size,\nfor stamps their size, for blur the size of its blocks.",
            Widths::RANGE.0,
            Widths::RANGE.1
        ));
        w.table("[drawing.widths]");
        for ((name, value), (_, default)) in d
            .widths
            .fields()
            .into_iter()
            .zip(Widths::default().fields())
        {
            w.key(
                &format!(
                    "{} (default {}).",
                    match name {
                        "text" => "Text: the font size",
                        "stamp" => "Numbered stamps: their size",
                        "blur" => "Blur: the size of its blocks",
                        "pen" => "Pen",
                        "highlighter" => "Highlighter",
                        "arrow" => "Arrow",
                        "rectangle" => "Rectangle",
                        "ellipse" => "Ellipse",
                        _ => "Line",
                    },
                    float_value(default)
                ),
                name,
                &float_value(value),
            );
        }

        let c = &self.capture;
        w.blank();
        w.table("[capture]");
        w.key(
            &format!(
                "Where captures are saved. {PICTURES_TOKEN} is your Pictures folder (default '{PICTURES_TOKEN}\\Overmark')."
            ),
            "save_folder",
            &str_value(&c.save_folder),
        );
        w.key(
            "The file name, without its extension. Codes in braces are replaced by the capture's time:\n{year} {yy} {month} {day} {hour} {minute} {second} {millisecond}. The default gives\n\"2026-10-21 09.42.17\". A name that's taken gets \" (2)\" and so on.",
            "file_name_pattern",
            &str_value(&c.file_name_pattern),
        );
        w.key(
            "What a capture does: \"copy-and-save\" (the default), \"copy\" (to the clipboard only) or \"save\".",
            "after_capture",
            &enum_value(&c.after_capture),
        );
        w.key(
            "The saved file's format: \"png\" (the default) or \"jpg\".",
            "format",
            &enum_value(&c.format),
        );
        w.key(
            "JPG quality, 1 to 100 (default 90).",
            "jpg_quality",
            &c.jpg_quality.to_string(),
        );
        w.key(
            "Include your drawing in captures; a separate key saves without it (default true).",
            "include_drawing",
            &c.include_drawing.to_string(),
        );
        w.key(
            "Show a small notice after each capture saying where it went (default true).",
            "toast",
            &c.toast.to_string(),
        );

        w.blank();
        w.table("[advanced]");
        w.key(
            "Look for a newer version on GitHub. Overmark only reads the latest release and never\ndownloads or installs anything by itself (default false: no network at all).",
            "check_for_updates",
            &self.advanced.check_for_updates.to_string(),
        );

        let b = &self.backdrops;
        w.blank();
        w.comment(
            "What's under the ink: W cycles Live, Frozen, the whiteboard and the blackboard.",
        );
        w.table("[backdrops]");
        w.key(
            &format!("The whiteboard's colour, as \"#RRGGBB\" (default \"{DEFAULT_WHITEBOARD}\")."),
            "whiteboard_colour",
            &str_value(&b.whiteboard_colour.to_string()),
        );
        w.key(
            &format!("The blackboard's colour (default \"{DEFAULT_BLACKBOARD}\", a dark grey)."),
            "blackboard_colour",
            &str_value(&b.blackboard_colour.to_string()),
        );
        w.key(
            "A hotkey that opens the layer on the whiteboard, as general's hotkeys are written\n(default \"\": none). Live and Frozen use hotkey_draw and hotkey_draw_frozen.",
            "hotkey_whiteboard",
            &str_value(&b.hotkey_whiteboard),
        );
        w.key(
            "A hotkey that opens the layer on the blackboard (default \"\": none).",
            "hotkey_blackboard",
            &str_value(&b.hotkey_blackboard),
        );

        let g = &self.guides;
        w.blank();
        w.comment("Guides over the backdrop: G cycles off, a grid, dots and ruled lines.");
        w.table("[guides]");
        w.key(
            &format!(
                "The distance between grid lines, dots or ruled lines, in pixels at 100% scale, {} to {}\n(default 32).",
                GuideSettings::SPACING_RANGE.0,
                GuideSettings::SPACING_RANGE.1
            ),
            "spacing",
            &g.spacing.to_string(),
        );
        w.key(
            "Show the guides to people watching a screen share (default true).",
            "show_on_shares",
            &g.show_on_shares.to_string(),
        );
        w.key(
            "Include the guides in saves and copies (default true).",
            "include_in_saves",
            &g.include_in_saves.to_string(),
        );

        w.blank();
        w.comment(
            "Where you left the toolbar on each monitor, in pixels from the monitor's top-left",
        );
        w.comment("corner. Overmark keeps these up to date.");
        w.table("[toolbar.positions]");
        for (monitor, position) in &self.toolbar.positions {
            let _ = writeln!(
                w.out,
                "{} = {{ x = {}, y = {} }}",
                str_value(monitor),
                position.x,
                position.y
            );
        }
        w.out
    }
}

/// The dim's range: 0 (off) to 0.8.
pub const DIM_RANGE: (f32, f32) = (0.0, 0.8);
/// How long disappearing ink stays, in seconds.
pub const INK_SECONDS_RANGE: (f32, f32) = (0.5, 60.0);
/// The laser's trail, in milliseconds.
pub const LASER_TRAIL_RANGE: (u64, u64) = (0, 5000);

/// Writes `text` to a new file beside `path` whose name no other file has (`<name>.<pid>-<n>.tmp`,
/// created exclusively), and returns its path.
fn write_temp(path: &Path, text: &[u8]) -> io::Result<PathBuf> {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let mut tmp = path.as_os_str().to_owned();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        tmp.push(format!(".{}-{n}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let written = file.write_all(text).and_then(|()| file.sync_all());
        drop(file);
        return match written {
            Ok(()) => Ok(tmp),
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                Err(e)
            }
        };
    }
}

/// How many times [`replace`] tries the rename.
const REPLACE_TRIES: usize = 10;

/// Renames `tmp` over `path`. Windows refuses a replacing rename for a moment (Access is
/// denied) while something such as an antivirus scan holds the file, so a refusal is tried
/// again after a short, growing pause, about a second in all.
fn replace(tmp: &Path, path: &Path) -> io::Result<()> {
    replace_with(
        tmp,
        path,
        |from, to| fs::rename(from, to),
        std::thread::sleep,
    )
}

/// [`replace`] with its rename and its pause given, so tests can refuse and count.
fn replace_with(
    tmp: &Path,
    path: &Path,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    mut pause: impl FnMut(std::time::Duration),
) -> io::Result<()> {
    // 10, 20, 40, 80, then 160 ms: 950 ms over the nine pauses.
    let mut wait = std::time::Duration::from_millis(10);
    for _ in 1..REPLACE_TRIES {
        match rename(tmp, path) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                pause(wait);
                wait = (wait * 2).min(std::time::Duration::from_millis(160));
            }
            done => return done,
        }
    }
    rename(tmp, path)
}

#[derive(Default)]
struct Writer {
    out: String,
}

impl Writer {
    fn comment(&mut self, text: &str) {
        for line in text.lines() {
            let _ = writeln!(self.out, "# {line}");
        }
    }

    fn blank(&mut self) {
        self.out.push('\n');
    }

    fn table(&mut self, header: &str) {
        let _ = writeln!(self.out, "{header}");
    }

    fn key(&mut self, doc: &str, key: &str, value: &str) {
        self.comment(doc);
        let _ = writeln!(self.out, "{key} = {value}");
    }
}

/// A TOML string: a literal 'string' when that needs no escapes (readable Windows paths),
/// otherwise a basic "string".
fn str_value(s: &str) -> String {
    if s.contains('\\') && !s.contains(['\'', '\n', '\r']) && !s.chars().any(char::is_control) {
        return format!("'{s}'");
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A unit enum's TOML value, as serde names it.
fn enum_value<T: Serialize>(value: &T) -> String {
    let value = toml::Value::try_from(value).expect("a unit enum serialises");
    str_value(value.as_str().expect("a unit enum is a string"))
}

/// A float as TOML: always with a decimal point, so it reads as the number it is.
fn float_value(value: f32) -> String {
    format!("{value:?}")
}

/// Keys in `raw` that the typed settings don't have, as warnings.
fn unknown_keys(raw: &toml::Table, known: &toml::Table, prefix: &str, out: &mut Vec<Warning>) {
    for (key, value) in raw {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        // Toolbar positions are keyed by monitor: every monitor in the file is in `known` too,
        // so any monitor id passes and only a field inside a position can be unknown.
        match known.get(key) {
            None => out.push(Warning::UnknownKey(path)),
            Some(k) => {
                if let (toml::Value::Table(r), toml::Value::Table(k)) = (value, k) {
                    unknown_keys(r, k, &path, out);
                }
            }
        }
    }
}

/// A byte offset as a 1-based line and column.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

#[cfg(test)]
mod tests;
