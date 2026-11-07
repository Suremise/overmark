//! Pictures of the toolbar, drawn as its window draws it (egui, then the CPU rasterizer): the
//! window itself is excluded from capture, so a screenshot can't show it.
//!
//! ```text
//! cargo run -p overmark --release --example toolbar_shot -- <folder>
//! ```
//!
//! Writes `toolbar-{expanded,frozen,collapsed}-{100,150}.png` into `<folder>`, over a grey
//! checkerboard so its transparent parts show.

use overmark::toolbar::{State, render_offscreen};
use overmark_core::backdrop::{Backdrop, Guides};
use overmark_core::settings::DEFAULT_PALETTE;
use overmark_core::tool::ToolKind;

/// A picture: its name, what the toolbar shows, and each (scale, width in pixels).
type Shot<'a> = (&'a str, &'a State, &'a [(f32, usize)]);

fn main() {
    let folder = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: toolbar_shot <folder>"),
    );
    std::fs::create_dir_all(&folder).unwrap();
    overmark_core::i18n::init("en");
    let base = State {
        backdrop: Backdrop::Live,
        guides: Guides::Off,
        tool: ToolKind::Pen,
        colour: DEFAULT_PALETTE[0],
        palette: DEFAULT_PALETTE.to_vec(),
        width: Some(4.0),
        can_undo: true,
        can_redo: false,
        collapsed: false,
        disappearing_ink: false,
        right_click_undoes: true,
        recording: false,
    };
    let frozen = State {
        backdrop: Backdrop::Frozen,
        guides: Guides::Grid,
        ..base.clone()
    };
    let collapsed = State {
        collapsed: true,
        ..base.clone()
    };
    // The longest names on a 1280-pixel monitor at 150 %: it wraps rather than losing its ends.
    let long = State {
        backdrop: Backdrop::Frozen,
        guides: Guides::Lines,
        tool: ToolKind::Highlighter,
        width: Some(128.0),
        ..base.clone()
    };
    let wide: &[(f32, usize)] = &[(1.0, 1040), (1.5, 1560)];
    let shots: [Shot; 4] = [
        ("expanded", &base, wide),
        ("frozen", &frozen, wide),
        ("collapsed", &collapsed, wide),
        ("narrow", &long, &[(1.5, 1280)]),
    ];
    for (name, state, sizes) in shots {
        for &(scale, w) in sizes {
            let h = (130.0 * scale) as usize;
            let rgba = render_offscreen(state, w, h, scale, 3, None);
            let mut img = image::RgbaImage::new(w as u32, h as u32);
            for (i, p) in img.pixels_mut().enumerate() {
                let (x, y) = (i % w, i / w);
                let check = if (x / 8 + y / 8) % 2 == 0 {
                    200.0
                } else {
                    160.0
                };
                let s = &rgba[i * 4..i * 4 + 4];
                let a = f32::from(s[3]) / 255.0;
                let c = |v: u8| (f32::from(v) + check * (1.0 - a)).round().min(255.0) as u8;
                *p = image::Rgba([c(s[0]), c(s[1]), c(s[2]), 255]);
            }
            let path = folder.join(format!("toolbar-{name}-{}.png", (scale * 100.0) as u32));
            img.save(&path).unwrap();
            println!("{}", path.display());
        }
    }
}
