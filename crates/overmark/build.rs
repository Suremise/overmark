//! Compiles Overmark's Windows resources into the exe: the app icon (`assets/icon/overmark.ico`,
//! made by `assets/export.py`) as ID 1, so Explorer, the taskbar and the Start menu show it, and
//! the version info: Cargo's version, and LICENSE's copyright line and holder, so the file's
//! Details show them and the installer can compare the exe it replaces (`scripts/package.ps1`
//! checks both).
//!
//! Copied from Legible's `crates/legible/build.rs` and adapted: no manifest (Overmark sets its
//! DPI awareness at run time) and no file icon.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// The icon's resource ID: the lowest, so Explorer shows it for the exe.
const APP_ICON: u16 = 1;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by Cargo"))
        .join("..")
        .join("..");
    let icon = root.join("assets").join("icon").join("overmark.ico");
    let license = root.join("LICENSE");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed={}", license.display());

    // LICENSE's own "Copyright (c) <year> <holder>" line, and its holder for CompanyName.
    let copyright = fs::read_to_string(&license)
        .expect("LICENSE")
        .lines()
        .find(|l| l.starts_with("Copyright"))
        .expect("LICENSE has a Copyright line")
        .trim()
        .to_owned();
    let holder = copyright
        .split_whitespace()
        .skip_while(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        !holder.is_empty(),
        "LICENSE's Copyright line names its holder after the year"
    );

    let version = env::var("CARGO_PKG_VERSION").expect("set by Cargo");
    let numbers = ["MAJOR", "MINOR", "PATCH"]
        .map(|part| env::var(format!("CARGO_PKG_VERSION_{part}")).expect("set by Cargo"))
        .join(",");
    let rc = format!(
        r#"#pragma code_page(65001)
{APP_ICON} ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numbers},0
PRODUCTVERSION {numbers},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "{holder}"
      VALUE "FileDescription", "Overmark"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "overmark"
      VALUE "LegalCopyright", "{copyright}"
      VALUE "OriginalFilename", "overmark.exe"
      VALUE "ProductName", "Overmark"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = rc_path(&icon),
        // rc.exe's strings write a quote as "".
        copyright = copyright.replace('"', "\"\""),
        holder = holder.replace('"', "\"\""),
    );
    let out = PathBuf::from(env::var("OUT_DIR").expect("set by Cargo")).join("overmark.rc");
    fs::write(&out, rc).expect("write overmark.rc");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .expect("compile Overmark's resources (needs the Windows SDK's rc.exe)");
}

/// A path as an .rc string: backslashes doubled.
fn rc_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}
