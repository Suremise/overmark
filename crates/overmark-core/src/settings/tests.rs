use super::*;

fn parse(text: &str) -> Loaded {
    Settings::parse(text).unwrap()
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn changed() -> Settings {
    let mut s = Settings::default();
    s.general.start_with_windows = true;
    s.general.tray_click = TrayClick::Settings;
    s.general.hotkey_draw = "Ctrl+Shift+D".into();
    s.general.hotkey_draw_frozen = String::new();
    s.general.hotkey_capture_region = "Ctrl+Shift+R".into();
    s.general.print_screen = PrintScreen::Frozen;
    s.drawing.default_tool = ToolKind::Highlighter;
    s.drawing.palette[8] = Colour {
        r: 1,
        g: 2,
        b: 3,
        a: 128,
    };
    s.drawing.default_colour = 4;
    s.drawing.widths.pen = 6.5;
    s.drawing.widths.highlighter = 30.0;
    s.drawing.widths.arrow = 5.5;
    s.drawing.widths.rectangle = 3.0;
    s.drawing.widths.ellipse = 2.5;
    s.drawing.widths.line = 7.0;
    s.drawing.widths.text = 31.5;
    s.drawing.widths.stamp = 40.0;
    s.drawing.widths.blur = 16.0;
    s.drawing.dim = 0.0;
    s.drawing.disappearing_ink = true;
    s.drawing.disappearing_ink_seconds = 1.5;
    s.drawing.laser_trail_ms = 0;
    s.drawing.right_click_undoes = false;
    s.capture.save_folder = r"D:\Shots\It's mine".into();
    s.capture.file_name_pattern = "shot {{{year}}} \"q\"".into();
    s.capture.after_capture = AfterCapture::Copy;
    s.capture.format = ImageFormat::Jpg;
    s.capture.jpg_quality = 75;
    s.capture.include_drawing = false;
    s.capture.toast = false;
    s.advanced.check_for_updates = true;
    s.advanced.diagnostic_logging = true;
    s.backdrops.whiteboard_colour = Colour::rgb(250, 248, 240);
    s.backdrops.blackboard_colour = Colour::rgb(10, 40, 30);
    s.backdrops.hotkey_whiteboard = "Win+Shift+W".into();
    s.backdrops.hotkey_blackboard = "Win+Alt+B".into();
    s.guides.spacing = 48;
    s.guides.show_on_shares = false;
    s.guides.include_in_saves = false;
    s.toolbar
        .positions
        .insert(r"\\.\DISPLAY1".into(), ToolbarPosition { x: 640, y: 8 });
    s.toolbar
        .positions
        .insert("monitor \"two\"".into(), ToolbarPosition { x: -1200, y: 0 });
    s
}

#[test]
fn the_defaults() {
    let s = Settings::default();
    assert_eq!(s.general.hotkey_draw, "Win+Shift+D");
    assert_eq!(s.general.hotkey_draw_frozen, "Win+Shift+F");
    assert_eq!(
        s.general.hotkey_capture_region,
        DEFAULT_CAPTURE_REGION_HOTKEY
    );
    assert_eq!(s.general.print_screen, PrintScreen::Off);
    assert_eq!(s.general.tray_click, TrayClick::Live);
    assert_eq!(s.drawing.palette.len(), PALETTE_SIZE);
    assert!((0.3..=0.4).contains(&s.drawing.dim));
    assert!(!s.drawing.disappearing_ink);
    assert!(s.drawing.right_click_undoes);
    // Copy and save, decided.
    assert_eq!(s.capture.after_capture, AfterCapture::CopyAndSave);
    assert!(s.capture.after_capture.copies() && s.capture.after_capture.saves());
    assert_eq!(s.capture.format, ImageFormat::Png);
    assert!(s.capture.include_drawing);
    // Off by default: no network.
    assert!(!s.advanced.check_for_updates);
    // Off by default: the log adds no diagnostic detail.
    assert!(!s.advanced.diagnostic_logging);
    assert_eq!(
        s.capture.resolve_save_folder(Path::new("/home/p/Pictures")),
        Path::new("/home/p/Pictures").join("Overmark")
    );
}

#[test]
fn an_empty_file_is_the_defaults() {
    let loaded = parse("");
    assert_eq!(loaded.settings, Settings::default());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn the_documented_file_round_trips() {
    for settings in [Settings::default(), changed()] {
        let text = settings.to_documented_toml();
        let loaded = parse(&text);
        assert_eq!(loaded.settings, settings, "{text}");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }
}

#[test]
fn save_and_load_round_trip_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    assert_eq!(
        Settings::load_or_default(&path).unwrap().settings,
        Settings::default()
    );
    changed().save(&path).unwrap();
    assert_eq!(Settings::load(&path).unwrap().settings, changed());
    Settings::default().save(&path).unwrap();
    assert_eq!(Settings::load(&path).unwrap().settings, Settings::default());
    // Nothing left behind but the file.
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, [FILE_NAME]);
}

#[test]
fn every_key_has_a_comment_above_it() {
    let text = changed().to_documented_toml();
    let lines: Vec<&str> = text.lines().collect();
    let known = toml::Table::try_from(changed()).unwrap();
    let mut leaves = Vec::new();
    for (table, value) in &known {
        for (key, value) in value.as_table().unwrap() {
            if let toml::Value::Table(inner) = value
                && key != "positions"
            {
                leaves.extend(inner.keys().map(|k| format!("{table}.{key}.{k}")));
            } else if key != "positions" {
                leaves.push(format!("{table}.{key}"));
            }
        }
    }
    assert!(leaves.len() > 25, "{leaves:?}");
    for leaf in leaves {
        let key = leaf.rsplit('.').next().unwrap();
        let at = lines
            .iter()
            .position(|l| l.starts_with(&format!("{key} = ")))
            .unwrap_or_else(|| panic!("{leaf} isn't written"));
        assert!(lines[at - 1].starts_with("# "), "{leaf} has no comment");
    }
}

#[test]
fn a_partial_file_takes_defaults_for_the_rest() {
    let loaded = parse("[drawing]\ndim = 0\n\n[capture]\nformat = \"jpg\"\n");
    let mut expected = Settings::default();
    expected.drawing.dim = 0.0;
    expected.capture.format = ImageFormat::Jpg;
    assert_eq!(loaded.settings, expected);
    assert!(loaded.warnings.is_empty());

    let loaded = parse("[drawing.widths]\npen = 9\n");
    assert_eq!(loaded.settings.drawing.widths.pen, 9.0);
    assert_eq!(loaded.settings.drawing.widths.highlighter, 20.0);
}

#[test]
fn unknown_keys_are_warnings() {
    let loaded = parse(
        "theme = \"dark\"\n[drawing]\ncolour = 3\n[drawing.widths]\nmarker = 2\n[future]\nx = 1\n\
         [toolbar.positions]\n'\\\\.\\DISPLAY9' = { x = 1, y = 2 }\n",
    );
    let mut keys: Vec<String> = loaded
        .warnings
        .iter()
        .map(|w| match w {
            Warning::UnknownKey(k) => k.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        ["drawing.colour", "drawing.widths.marker", "future", "theme"]
    );
    assert_eq!(
        loaded.settings.toolbar.positions[r"\\.\DISPLAY9"],
        ToolbarPosition { x: 1, y: 2 }
    );
    assert!(
        loaded.warnings[0]
            .to_string()
            .contains("isn't an Overmark setting")
    );
}

/// Any monitor id is a valid key, but a field in a position other than x and y is unknown.
#[test]
fn unknown_fields_in_a_toolbar_position_are_warnings() {
    let loaded = parse(
        "[toolbar.positions]\nmain = { x = 1, y = 2, future = 3 }\n'\\\\.\\DISPLAY2' = { x = 5, y = 6 }\n\n\
         [toolbar.positions.other]\nx = 7\ny = 8\nz = 9\n",
    );
    assert_eq!(
        loaded.warnings,
        [
            Warning::UnknownKey("toolbar.positions.main.future".into()),
            Warning::UnknownKey("toolbar.positions.other.z".into()),
        ]
    );
    assert_eq!(loaded.settings.toolbar.positions.len(), 3);

    let loaded = parse("[toolbar.positions]\nmain = { x = 1, y = 2 }\n'a b' = { x = 0, y = 0 }\n");
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
}

#[test]
fn out_of_range_values_are_clamped_with_a_warning() {
    let loaded = parse(
        "[drawing]\ndim = 2.5\ndisappearing_ink_seconds = 0\nlaser_trail_ms = 99999\n\
         default_colour = 12\n[drawing.widths]\npen = 0\n[capture]\njpg_quality = 0\n",
    );
    let d = &loaded.settings.drawing;
    assert_eq!(d.dim, 0.8);
    assert_eq!(d.disappearing_ink_seconds, 0.5);
    assert_eq!(d.laser_trail_ms, 5000);
    assert_eq!(d.default_colour, 8);
    assert_eq!(d.widths.pen, 1.0);
    assert_eq!(loaded.settings.capture.jpg_quality, 1);
    let keys: Vec<_> = loaded
        .warnings
        .iter()
        .map(|w| match w {
            Warning::OutOfRange { key, .. } => key.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        keys,
        [
            "drawing.dim",
            "drawing.disappearing_ink_seconds",
            "drawing.widths.pen",
            "drawing.laser_trail_ms",
            "drawing.default_colour",
            "capture.jpg_quality"
        ]
    );
    let text = loaded.warnings[0].to_string();
    assert!(
        text.contains("drawing.dim = 2.5") && text.contains("0.8"),
        "{text}"
    );
}

/// A fresh install keeps Win+Shift+D (Live) and Win+Shift+F (Frozen);
/// only the new backdrops' optional hotkeys start unset. Set ones survive a round trip.
#[test]
fn backdrop_hotkeys_default_as_decided_and_round_trip() {
    let s = Settings::default();
    assert_eq!(s.general.hotkey_draw, "Win+Shift+D");
    assert_eq!(s.general.hotkey_draw_frozen, "Win+Shift+F");
    assert_eq!(s.backdrops.hotkey_whiteboard, "");
    assert_eq!(s.backdrops.hotkey_blackboard, "");
    let mut set = Settings::default();
    set.backdrops.hotkey_whiteboard = "Win+Shift+W".into();
    set.backdrops.hotkey_blackboard = "Win+Shift+B".into();
    let loaded = parse(&set.to_documented_toml());
    assert_eq!(loaded.settings.backdrops, set.backdrops);
}

/// A whiteboard or blackboard hides the screen, so its colour loads opaque, with a warning.
#[test]
fn translucent_backdrop_colours_load_opaque() {
    let loaded = parse(
        "[backdrops]\nwhiteboard_colour = \"#FF000080\"\nblackboard_colour = \"#10203040\"\n",
    );
    assert_eq!(
        loaded.settings.backdrops.whiteboard_colour,
        Colour::rgb(255, 0, 0)
    );
    assert_eq!(
        loaded.settings.backdrops.blackboard_colour,
        Colour::rgb(0x10, 0x20, 0x30)
    );
    assert_eq!(
        loaded.warnings,
        [
            Warning::TranslucentBackdrop("backdrops.whiteboard_colour".into()),
            Warning::TranslucentBackdrop("backdrops.blackboard_colour".into()),
        ]
    );
    assert!(loaded.warnings[0].to_string().contains("opaque"));
    let opaque = parse("[backdrops]\nwhiteboard_colour = \"#FAFAFA\"\n");
    assert!(opaque.warnings.is_empty());
}

#[test]
fn a_bad_pattern_or_palette_falls_back_with_a_warning() {
    let loaded = parse(
        "[drawing]\npalette = [\"#000000\", \"#111111\"]\n[capture]\nfile_name_pattern = \"{hours}\"\n",
    );
    let s = &loaded.settings;
    assert_eq!(s.drawing.palette.len(), PALETTE_SIZE);
    assert_eq!(s.drawing.palette[1], Colour::rgb(0x11, 0x11, 0x11));
    assert_eq!(s.drawing.palette[2], DEFAULT_PALETTE[2]);
    assert_eq!(s.capture.file_name_pattern, file_name::DEFAULT_PATTERN);
    assert_eq!(loaded.warnings.len(), 2);
    assert_eq!(loaded.warnings[0], Warning::PaletteSize(2));
    let text = loaded.warnings[1].to_string();
    assert!(text.contains("{hours}"), "{text}");

    let ten = format!(
        "[drawing]\npalette = [{}]\n",
        ["\"#ABCDEF\""; 10].join(", ")
    );
    let loaded = parse(&ten);
    assert_eq!(
        loaded.settings.drawing.palette,
        vec![Colour::rgb(0xAB, 0xCD, 0xEF); 9]
    );
    assert_eq!(loaded.warnings, [Warning::PaletteSize(10)]);
}

#[test]
fn a_corrupt_file_is_a_clear_error_with_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    fs::write(
        &path,
        "[general]\nstart_with_windows = true\n[drawing\ndim = 0.2\n",
    )
    .unwrap();
    let error = Settings::load(&path).unwrap_err();
    match &error {
        SettingsError::Corrupt {
            path: Some(p),
            line: Some(3),
            ..
        } => assert_eq!(p, &path),
        other => panic!("{other:?}"),
    }
    let text = error.to_string();
    assert!(
        text.contains("can't be read, so the defaults are used"),
        "{text}"
    );
    assert!(text.contains("line 3, column"), "{text}");
    assert!(text.contains(&path.display().to_string()), "{text}");
}

#[test]
fn a_wrong_type_or_value_is_an_error_too() {
    for text in [
        "[drawing]\ndim = \"dark\"\n",
        "[drawing]\ndefault_tool = \"crayon\"\n",
        "[drawing]\npalette = [\"red\"]\n",
        "[capture]\nformat = \"gif\"\n",
        "[toolbar.positions]\nmain = { x = 1 }\n",
        "general = 5\n",
    ] {
        let error = Settings::parse(text).unwrap_err();
        assert!(
            matches!(error, SettingsError::Corrupt { line: Some(_), .. }),
            "{text}: {error:?}"
        );
    }
    let text = Settings::parse("[drawing]\npalette = [\"red\"]\n")
        .unwrap_err()
        .to_string();
    assert!(text.contains("isn't a colour"), "{text}");
}

#[test]
fn a_byte_order_mark_is_fine() {
    let loaded = parse("\u{feff}[general]\nstart_with_windows = true\n");
    assert!(loaded.settings.general.start_with_windows);
}

#[test]
fn reading_a_missing_or_unreadable_file() {
    let dir = tempfile::tempdir().unwrap();
    // A folder where the file should be can't be read as one.
    let error = Settings::load_or_default(dir.path()).unwrap_err();
    assert!(matches!(error, SettingsError::Read { .. }), "{error:?}");
    let error = Settings::load(&dir.path().join("none.toml")).unwrap_err();
    assert!(matches!(error, SettingsError::Read { .. }), "{error:?}");
    assert!(!error.to_string().starts_with("error-"));
}

#[test]
fn saving_where_it_cant_write_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing folder").join(FILE_NAME);
    let error = Settings::default().save(&path).unwrap_err();
    assert!(matches!(error, SettingsError::Write { .. }), "{error:?}");
    assert!(error.to_string().contains("couldn't be saved"));
}

#[test]
fn the_save_folder_resolves() {
    let pictures = Path::new("P");
    let folder = |s: &str| {
        CaptureSettings {
            save_folder: s.into(),
            ..CaptureSettings::default()
        }
        .resolve_save_folder(pictures)
    };
    assert_eq!(folder(r"{pictures}\Overmark"), pictures.join("Overmark"));
    assert_eq!(folder("{pictures}/A/B"), pictures.join("A").join("B"));
    assert_eq!(folder("{pictures}"), pictures);
    assert_eq!(folder(""), pictures.join("Overmark"));
    assert_eq!(folder(r"D:\Shots"), PathBuf::from(r"D:\Shots"));
}

#[test]
fn replace_retries_while_refused() {
    let mut tries = 0;
    let mut pauses = Vec::new();
    let result = replace_with(
        Path::new("a"),
        Path::new("b"),
        |_, _| {
            tries += 1;
            if tries < 3 {
                Err(io::ErrorKind::PermissionDenied.into())
            } else {
                Ok(())
            }
        },
        |d| pauses.push(d.as_millis()),
    );
    assert!(result.is_ok());
    assert_eq!(pauses, [10, 20]);

    let mut tries = 0;
    let result = replace_with(
        Path::new("a"),
        Path::new("b"),
        |_, _| {
            tries += 1;
            Err(io::ErrorKind::PermissionDenied.into())
        },
        |_| {},
    );
    assert!(result.is_err());
    assert_eq!(tries, REPLACE_TRIES);
}

/// `docs/settings.example.toml` is the default file, as Overmark writes it on first run.
/// To update it after changing the settings: `UPDATE_EXAMPLE=1 cargo test -p overmark-core`.
#[test]
fn the_example_file_is_current() {
    let path = workspace().join("docs/settings.example.toml");
    let expected = Settings::default().to_documented_toml();
    if std::env::var_os("UPDATE_EXAMPLE").is_some() {
        fs::write(&path, &expected).unwrap();
    }
    let actual = fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        actual == expected,
        "docs/settings.example.toml is out of date: run UPDATE_EXAMPLE=1 cargo test -p overmark-core"
    );
}

#[test]
fn a_width_steps_a_tenth_at_a_time_inside_its_range() {
    assert!((Widths::stepped(10.0, 1.0) - 11.0).abs() < 1e-4);
    assert!((Widths::stepped(11.0, -1.0) - 10.0).abs() < 1e-4);
    // Three notches at once, as a fast wheel gives.
    assert!((Widths::stepped(10.0, 3.0) - 13.31).abs() < 1e-3);
    let (lo, hi) = Widths::RANGE;
    assert_eq!(Widths::stepped(hi, 1.0), hi);
    assert_eq!(Widths::stepped(lo, -1.0), lo);
    assert_eq!(Widths::stepped(5000.0, 0.0), hi);
}

/// Each release's settings files, as that release wrote them: its defaults, and one with every
/// setting changed ([`changed`]). Upgrading never loses a setting, so each loads with no warnings
/// and keeps every value it set. Each release adds its own pair here (docs/TESTING.md):
/// `UPDATE_RELEASED=<version> cargo test -p overmark-core released` writes them from the code as
/// it is, then add their two lines below.
const RELEASED: &[(&str, &str)] = &[
    ("0.1.0-default", include_str!("released/0.1.0-default.toml")),
    ("0.1.0-changed", include_str!("released/0.1.0-changed.toml")),
    ("0.2.0-default", include_str!("released/0.2.0-default.toml")),
    ("0.2.0-changed", include_str!("released/0.2.0-changed.toml")),
];

/// Every key `old` sets is in `new` with the same value. (`new` is the file as this version
/// saves it, so a float reads as it's written: 0.35, not the f32's 0.3499999940395355.)
fn assert_keeps(path: &str, old: &toml::Table, new: &toml::Table, file: &str) {
    for (key, value) in old {
        let at = format!("{path}{key}");
        match (value, new.get(key)) {
            (toml::Value::Table(o), Some(toml::Value::Table(n))) => {
                assert_keeps(&format!("{at}."), o, n, file);
            }
            (o, n) => assert_eq!(Some(o), n, "{file}: {at}"),
        }
    }
}

/// The keys that are the same in `a` and `b`, by dotted path.
fn same_keys(path: &str, a: &toml::Table, b: &toml::Table) -> Vec<String> {
    let mut same = Vec::new();
    for (key, value) in a {
        let at = format!("{path}{key}");
        match (value, b.get(key)) {
            (toml::Value::Table(x), Some(toml::Value::Table(y))) => {
                same.extend(same_keys(&format!("{at}."), x, y));
            }
            (x, y) if Some(x) == y => same.push(at),
            _ => {}
        }
    }
    same
}

/// Each release's changed file sets every key away from its default, so a value lost on
/// loading shows; and so does today's [`changed`], which writes the next release's.
#[test]
fn each_changed_file_changes_every_setting() {
    let mut pairs: Vec<(String, toml::Table, toml::Table)> = RELEASED
        .chunks(2)
        .map(|pair| {
            let [(name, default), (_, changed)] = pair else {
                panic!("RELEASED lists each release's default file, then its changed one")
            };
            (
                name.to_string(),
                default.parse().unwrap(),
                changed.parse().unwrap(),
            )
        })
        .collect();
    pairs.push((
        "today's".into(),
        Settings::default().to_documented_toml().parse().unwrap(),
        changed().to_documented_toml().parse().unwrap(),
    ));
    for (name, default, changed) in pairs {
        let same = same_keys("", &default, &changed);
        assert!(same.is_empty(), "{name}: the changed file keeps {same:?}");
    }
}

#[test]
fn every_released_versions_settings_load_and_keep_their_values() {
    if let Some(version) = std::env::var_os("UPDATE_RELEASED") {
        let version = version.to_string_lossy();
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/settings/released");
        for (kind, settings) in [("default", Settings::default()), ("changed", changed())] {
            let path = folder.join(format!("{version}-{kind}.toml"));
            fs::write(&path, settings.to_documented_toml()).unwrap();
        }
    }
    for (name, text) in RELEASED {
        let loaded = parse(text);
        assert!(loaded.warnings.is_empty(), "{name}: {:?}", loaded.warnings);
        let old: toml::Table = text.parse().unwrap();
        let new: toml::Table = loaded.settings.to_documented_toml().parse().unwrap();
        assert_keeps("", &old, &new, name);
    }
    // No released version had diagnostic logging: each file, changed ones too, reads it as off.
    for (name, text) in RELEASED {
        assert!(!text.contains("diagnostic_logging"), "{name}");
        assert!(!parse(text).settings.advanced.diagnostic_logging, "{name}");
    }
    // 0.1.0 had no right-click undo or Capture a region hotkey: either file takes the defaults.
    for (name, text) in &RELEASED[..2] {
        let s = parse(text).settings;
        assert!(s.drawing.right_click_undoes, "{name}");
        assert_eq!(
            s.general.hotkey_capture_region, DEFAULT_CAPTURE_REGION_HOTKEY,
            "{name}"
        );
    }
}
