use super::*;

/// Two side-by-side 1920x1080 monitors, the second to the right of the first.
const LEFT: PixelRect = PixelRect::new(0, 0, 1920, 1080);
const RIGHT: PixelRect = PixelRect::new(1920, 0, 3840, 1080);

#[test]
fn a_box_on_one_monitor_keeps_its_place() {
    let p = place(PixelRect::new(100, 200, 740, 680), &[LEFT, RIGHT]).unwrap();
    assert_eq!(p.monitor, 0);
    assert_eq!(p.area, PixelRect::new(100, 200, 740, 680));

    let p = place(PixelRect::new(2000, 10, 2100, 110), &[LEFT, RIGHT]).unwrap();
    assert_eq!(p.monitor, 1);
    assert_eq!(p.area, PixelRect::new(2000, 10, 2100, 110));
}

#[test]
fn sides_are_rounded_down_to_even_keeping_the_top_left() {
    let p = place(PixelRect::new(11, 21, 112, 98), &[LEFT]).unwrap();
    // 101 x 77 becomes 100 x 76, inside the box.
    assert_eq!(p.area, PixelRect::new(11, 21, 111, 97));
    assert_eq!((p.area.width() % 2, p.area.height() % 2), (0, 0));
}

#[test]
fn a_box_across_two_monitors_is_refused() {
    let across = PixelRect::new(1800, 100, 2000, 300);
    assert_eq!(
        place(across, &[LEFT, RIGHT]),
        Err(PlaceError::SpansMonitors)
    );
    // Touching the edge isn't crossing it: right is exclusive.
    let touching = PixelRect::new(1800, 100, 1920, 300);
    assert_eq!(place(touching, &[LEFT, RIGHT]).unwrap().monitor, 0);
}

#[test]
fn a_box_partly_off_the_desktop_is_cut_to_its_monitor() {
    // Hanging off the left and top of the only monitor.
    let p = place(PixelRect::new(-50, -31, 201, 100), &[LEFT]).unwrap();
    assert_eq!(p.area, PixelRect::new(0, 0, 200, 100));
    // Off the right of a smaller second monitor, below the top one's reach.
    let small = PixelRect::new(1920, 0, 2944, 768);
    let p = place(PixelRect::new(2800, 700, 3000, 900), &[LEFT, small]).unwrap();
    assert_eq!(
        (p.monitor, p.area),
        (1, PixelRect::new(2800, 700, 2944, 768))
    );
}

#[test]
fn a_box_off_every_monitor_or_too_small_is_refused() {
    let off = PixelRect::new(5000, 0, 5100, 100);
    assert_eq!(place(off, &[LEFT, RIGHT]), Err(PlaceError::OffScreen));
    assert_eq!(
        place(PixelRect::default(), &[LEFT]),
        Err(PlaceError::OffScreen)
    );
    assert_eq!(
        place(PixelRect::new(0, 0, 100, 100), &[]),
        Err(PlaceError::OffScreen)
    );
    // 35 rounds down to 34: allowed. 33 rounds down to 32, which isn't.
    assert!(place(PixelRect::new(0, 0, 35, 35), &[LEFT]).is_ok());
    assert_eq!(
        place(PixelRect::new(0, 0, 33, 400), &[LEFT]),
        Err(PlaceError::TooSmall)
    );
    // Cut to the monitor, only 10 pixels remain.
    assert_eq!(
        place(PixelRect::new(1910, 0, 2000, 100), &[LEFT]),
        Err(PlaceError::TooSmall)
    );
}

#[test]
fn even_down_rounds_towards_zero_for_sizes() {
    assert_eq!(
        [0, 1, 2, 3, 1081, 1080].map(even_down),
        [0, 0, 2, 2, 1080, 1080]
    );
}

#[test]
fn bitrate_grows_with_area_rate_and_quality_within_its_bounds() {
    use FrameRate::*;
    use Quality::*;
    // 1920x1080 at 15 fps, Balanced: 2,073,600 px * 15 * 0.07.
    assert_eq!(bitrate(Balanced, 1920, 1080, Fps15), 2_177_280);
    // Four times the area, four times the bits.
    assert_eq!(
        bitrate(Balanced, 1280, 720, Fps15) * 4,
        bitrate(Balanced, 2560, 1440, Fps15)
    );
    assert_eq!(
        bitrate(Balanced, 1920, 1080, Fps30),
        2 * bitrate(Balanced, 1920, 1080, Fps15)
    );
    let ranked = Quality::ALL.map(|q| bitrate(q, 1920, 1080, Fps24));
    assert!(ranked[0] < ranked[1] && ranked[1] < ranked[2], "{ranked:?}");
    // A tiny box gets the floor; a huge one the ceiling.
    assert_eq!(bitrate(Small, 16, 16, Fps15), MIN_BITRATE);
    assert_eq!(bitrate(Sharp, 7680, 4320, Fps30), MAX_BITRATE);
    assert_eq!(bitrate(Sharp, 0, -5, Fps30), MIN_BITRATE);
    assert_eq!(peak_bitrate(1_000_000), 2_000_000);
    assert_eq!(peak_bitrate(u32::MAX), u32::MAX);
}

#[test]
fn frame_times_are_exact_at_every_rate() {
    for rate in FrameRate::ALL {
        let fps = u64::from(rate.fps());
        // A whole second of frames ends exactly at one second.
        assert_eq!(frame_time(fps, rate), UNITS_PER_SEC);
        let total: i64 = (0..fps).map(|n| frame_duration(n, rate)).sum();
        assert_eq!(total, UNITS_PER_SEC, "{rate:?}");
    }
    // 15 fps: 666,666.67 units a frame, never drifting.
    assert_eq!(frame_time(1, FrameRate::Fps15), 666_666);
    assert_eq!(frame_time(2, FrameRate::Fps15), 1_333_333);
    assert_eq!(
        frame_time(15 * 3600, FrameRate::Fps15),
        3600 * UNITS_PER_SEC
    );
}

#[test]
fn frames_started_and_at_stop() {
    let r = FrameRate::Fps15;
    assert_eq!(frames_started(-1, r), 0);
    assert_eq!(frames_started(0, r), 1);
    assert_eq!(frames_started(frame_time(1, r) - 1, r), 1);
    assert_eq!(frames_started(frame_time(1, r), r), 2);
    // Stopping at exactly two seconds: frames 0..30, the last ending at 2 s.
    assert_eq!(frames_at_stop(2 * UNITS_PER_SEC, r), 30);
    assert_eq!(frames_at_stop(2 * UNITS_PER_SEC + 1, r), 31);
    // Even an instant recording has a frame.
    assert_eq!(frames_at_stop(0, r), 1);
    assert_eq!(frames_at_stop(-5, r), 1);
}

#[test]
fn the_clock_counts_real_time() {
    let c = Clock::new(1_000);
    assert_eq!(c.active(1_000), 0);
    assert_eq!(c.active(6_000), 5_000);
    // Before the start (a clock read on another thread a moment early): never negative.
    assert_eq!(c.active(500), 0);
}

#[test]
fn a_pause_is_taken_off_later_times() {
    let mut c = Clock::new(0);
    c.pause(3_000);
    assert!(c.is_paused());
    // Still while paused.
    assert_eq!(c.active(3_000), 3_000);
    assert_eq!(c.active(9_000), 3_000);
    c.resume(10_000);
    assert!(!c.is_paused());
    // The 7,000 paused aren't recorded: no gap.
    assert_eq!(c.active(10_000), 3_000);
    assert_eq!(c.active(12_000), 5_000);
    // A second pause adds to the first.
    c.pause(13_000);
    c.resume(20_000);
    assert_eq!(c.active(21_000), 7_000);
}

#[test]
fn pausing_twice_or_resuming_unpaused_changes_nothing() {
    let mut c = Clock::new(0);
    c.resume(5_000);
    assert_eq!(c.active(6_000), 6_000);
    c.pause(6_000);
    c.pause(8_000);
    c.resume(10_000);
    assert_eq!(c.active(10_000), 6_000);
}

#[test]
fn settings_default_to_15_fps_balanced_with_the_pointer() {
    let s = RecordSettings::default();
    assert_eq!(
        (s.frame_rate, s.quality, s.pointer),
        (FrameRate::Fps15, Quality::Balanced, true)
    );
    assert_eq!(FrameRate::ALL.map(FrameRate::fps), [15, 24, 30]);
}

#[test]
fn every_error_has_a_message() {
    let errors = [
        RecordError::Place(PlaceError::SpansMonitors),
        RecordError::Place(PlaceError::OffScreen),
        RecordError::Place(PlaceError::TooSmall),
        RecordError::MediaFoundationMissing,
        RecordError::EncoderRefused {
            width: 8000,
            height: 6000,
            detail: "no".into(),
        },
        RecordError::CaptureUnavailable("x".into()),
        RecordError::CaptureLost("x".into()),
        RecordError::Encoding("x".into()),
        RecordError::File {
            path: "C:\\v.mp4".into(),
            detail: "x".into(),
        },
    ];
    for e in errors {
        let m = e.message();
        // A missing key comes back as the key itself.
        assert!(!m.starts_with("record-error"), "{e:?}: {m}");
        assert!(!m.is_empty());
    }
    assert!(
        RecordError::Place(PlaceError::TooSmall)
            .message()
            .contains("34")
    );
}

const MONITOR: PixelRect = PixelRect::new(0, 0, 1920, 1080);
const BAR: (i32, i32) = (300, 40);

#[test]
fn the_bar_sits_just_above_the_box_when_it_fits() {
    let area = PixelRect::new(400, 300, 1000, 700);
    assert_eq!(bar_origin(area, BAR, MONITOR, 6), (400, 300 - 6 - 40));
    // Exactly enough room above: still above.
    let area = PixelRect::new(400, 46, 1000, 700);
    assert_eq!(bar_origin(area, BAR, MONITOR, 6), (400, 0));
}

#[test]
fn the_bar_goes_just_inside_the_top_edge_without_room_above() {
    // At the monitor's top edge.
    let area = PixelRect::new(400, 10, 1000, 700);
    assert_eq!(bar_origin(area, BAR, MONITOR, 6), (400, 16));
    // Full screen.
    assert_eq!(bar_origin(MONITOR, BAR, MONITOR, 6), (0, 6));
    // On a second monitor to the right, full screen there.
    let right = PixelRect::new(1920, 0, 3840, 1080);
    assert_eq!(bar_origin(right, BAR, right, 6), (1920, 6));
}

#[test]
fn a_box_narrower_than_the_bar_has_it_overhang_from_its_left_edge() {
    let area = PixelRect::new(500, 500, 600, 560);
    let (x, y) = bar_origin(area, BAR, MONITOR, 6);
    assert_eq!((x, y), (500, 454));
    assert!(x + BAR.0 > area.right, "it overhangs the box");
}

#[test]
fn the_bar_stays_on_the_monitor() {
    // A narrow box at the right edge: moved left only as far as it must.
    let area = PixelRect::new(1800, 500, 1900, 600);
    assert_eq!(bar_origin(area, BAR, MONITOR, 6), (1920 - 300, 454));
    // A short box at the top whose inside can't hold the bar: on the monitor all the same.
    let short = PixelRect::new(0, 0, 400, 20);
    let tiny_monitor = PixelRect::new(0, 0, 800, 30);
    assert_eq!(bar_origin(short, BAR, tiny_monitor, 6), (0, 0));
}

#[test]
fn frame_rates_snap_to_the_nearest() {
    use FrameRate::*;
    assert_eq!(
        [0, 15, 19, 20, 24, 27, 30, 60, u32::MAX].map(FrameRate::nearest),
        [
            Fps15, Fps15, Fps15, Fps24, Fps24, Fps24, Fps30, Fps30, Fps30
        ]
    );
}

#[test]
fn the_timer_reads_minutes_and_seconds_then_hours() {
    use std::time::Duration;
    let t = |ms: u64| timer_text(Duration::from_millis(ms));
    assert_eq!(t(0), "0:00");
    assert_eq!(t(7_999), "0:07");
    assert_eq!(t(754_000), "12:34");
    assert_eq!(t(3_599_999), "59:59");
    assert_eq!(t(3_723_000), "1:02:03");
}

#[test]
fn the_bar_goes_back_on_top_at_once_then_every_half_second() {
    let mut r = Reassert::new(REASSERT_MS);
    assert_eq!(r.wait_ms(1_000), 0);
    assert!(r.due(1_000), "at once");
    assert!(!r.due(1_000));
    assert!(!r.due(1_499));
    assert_eq!(r.wait_ms(1_200), 300);
    assert!(r.due(1_500));
    // Counted from when it was done, not when it was due: a late turn doesn't bunch them up.
    assert!(r.due(2_700));
    assert!(!r.due(3_100));
    assert_eq!(r.wait_ms(3_100), 100);
    // A clock that went back (another thread's reading) waits a whole period, never panics.
    assert!(!r.due(2_000));
    assert_eq!(r.wait_ms(10_000), 0);
}
