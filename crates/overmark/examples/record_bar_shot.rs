//! Pictures of the record bar, drawn as its window draws it (egui, then the CPU rasterizer):
//! the window itself is excluded from capture, so a screenshot can't show it.
//!
//! ```text
//! cargo run -p overmark --release --example record_bar_shot -- <folder>
//! ```
//!
//! Writes `record-bar-{ready,recording,paused,discard}-{100,150}.png` into `<folder>`, over a
//! grey checkerboard so its transparent parts show.

use std::time::Duration;

use overmark::record_bar::{Phase, View, WINDOW_POINTS, render_offscreen};

fn main() {
    let folder = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: record_bar_shot <folder>"),
    );
    std::fs::create_dir_all(&folder).unwrap();
    overmark_core::i18n::init("en");
    let view = |phase, secs, confirm_discard| View {
        phase,
        elapsed: Duration::from_secs(secs),
        confirm_discard,
    };
    let shots = [
        ("ready", view(Phase::Ready, 0, false)),
        ("recording", view(Phase::Recording, 83, false)),
        ("paused", view(Phase::Paused, 83, false)),
        ("discard", view(Phase::Recording, 83, true)),
    ];
    for (name, v) in shots {
        for scale in [1.0f32, 1.5] {
            let (w, h) = (
                (WINDOW_POINTS.0 * scale) as usize,
                (WINDOW_POINTS.1 * scale) as usize,
            );
            let rgba = render_offscreen(&v, w, h, scale);
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
            let path = folder.join(format!("record-bar-{name}-{}.png", (scale * 100.0) as u32));
            img.save(&path).unwrap();
            println!("{}", path.display());
        }
    }
}
