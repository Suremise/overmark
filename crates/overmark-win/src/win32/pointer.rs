//! Mouse, pen and touch through `WM_POINTER*` messages to our own window: no hooks. With
//! [`enable_mouse_in_pointer`], the mouse comes as pointer messages too, so one path handles
//! all three. Each message can carry several coalesced samples (a pen reports at 200+ Hz, a
//! mouse at up to 1 kHz); [`samples`] returns them all, oldest first, with the pen's pressure,
//! tilt and whether it's the eraser end.

use std::sync::OnceLock;

use windows::Win32::Foundation::WPARAM;
use windows::Win32::UI::Input::Pointer::{
    EnableMouseInPointer, GetPointerInfo, GetPointerInfoHistory, GetPointerPenInfoHistory,
    GetPointerType, POINTER_FLAG_FIRSTBUTTON, POINTER_FLAG_INCONTACT, POINTER_INFO,
    POINTER_PEN_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, POINTER_INPUT_TYPE,
    PT_MOUSE, PT_PEN, PT_TOUCH, PT_TOUCHPAD,
};

/// Makes the mouse send `WM_POINTER*` messages to this process's windows, as pens do.
pub fn enable_mouse_in_pointer() -> windows::core::Result<()> {
    // SAFETY: a process-wide switch with no pointers.
    unsafe { EnableMouseInPointer(true) }
}

/// What made a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    Mouse,
    Pen,
    /// A finger (or a touchpad): it draws like a mouse.
    Touch,
}

/// One input sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub id: u32,
    pub kind: PointerKind,
    /// Screen position, physical pixels.
    pub x: i32,
    pub y: i32,
    /// A pen's pressure, 0 to 1; `None` for the mouse and touch.
    pub pressure: Option<f32>,
    /// A pen's tilt, in degrees along each axis, if it reports one.
    pub tilt: Option<(f32, f32)>,
    /// The pen's eraser end (or its eraser button held): it erases whole items.
    pub eraser: bool,
    /// Drawing: the pen or finger touches, or the mouse's left button is down.
    pub in_contact: bool,
    /// When the hardware reported it (100 ns units, as `clock::now_100ns`).
    pub time_100ns: i64,
}

fn qpc_to_100ns(qpc: u64) -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    let f = *FREQ.get_or_init(|| {
        let mut f = 0i64;
        // SAFETY: writes one i64 we own.
        let _ = unsafe { windows::Win32::System::Performance::QueryPerformanceFrequency(&mut f) };
        f.max(1)
    });
    ((qpc as i128 * 10_000_000) / f as i128) as i64
}

fn kind_of(t: POINTER_INPUT_TYPE) -> PointerKind {
    match t {
        PT_PEN => PointerKind::Pen,
        PT_TOUCH | PT_TOUCHPAD => PointerKind::Touch,
        PT_MOUSE => PointerKind::Mouse,
        _ => PointerKind::Mouse,
    }
}

fn from_info(info: &POINTER_INFO, kind: PointerKind) -> Sample {
    let contact = info.pointerFlags.contains(POINTER_FLAG_INCONTACT);
    Sample {
        id: info.pointerId,
        kind,
        x: info.ptPixelLocation.x,
        y: info.ptPixelLocation.y,
        pressure: None,
        tilt: None,
        eraser: false,
        // A mouse draws with its left button only.
        in_contact: contact
            && (kind != PointerKind::Mouse || info.pointerFlags.contains(POINTER_FLAG_FIRSTBUTTON)),
        // A device without timestamps reports 0: now is the next best.
        time_100ns: match info.PerformanceCount {
            0 => crate::win32::clock::now_100ns(),
            qpc => qpc_to_100ns(qpc),
        },
    }
}

fn from_pen(p: &POINTER_PEN_INFO) -> Sample {
    let mut s = from_info(&p.pointerInfo, PointerKind::Pen);
    s.pressure = Some(p.pressure as f32 / 1024.0);
    if p.penMask & (PEN_MASK_TILT_X | PEN_MASK_TILT_Y) != 0 {
        s.tilt = Some((p.tiltX as f32, p.tiltY as f32));
    }
    s.eraser = p.penFlags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0;
    s
}

/// Every sample a `WM_POINTERDOWN`, `WM_POINTERUPDATE` or `WM_POINTERUP` carries, oldest first.
pub fn samples(wparam: WPARAM) -> Vec<Sample> {
    let id = (wparam.0 & 0xFFFF) as u32;
    // SAFETY: each call fills a buffer we own, of the length we pass.
    unsafe {
        let mut t = POINTER_INPUT_TYPE::default();
        let _ = GetPointerType(id, &mut t);
        let kind = kind_of(t);
        let mut info = POINTER_INFO::default();
        if GetPointerInfo(id, &mut info).is_err() {
            return Vec::new();
        }
        let mut count = info.historyCount.max(1);
        if kind == PointerKind::Pen {
            let mut pens = vec![POINTER_PEN_INFO::default(); count as usize];
            if GetPointerPenInfoHistory(id, &mut count, Some(pens.as_mut_ptr())).is_err() {
                return vec![from_info(&info, kind)];
            }
            pens.truncate(count as usize);
            pens.iter().rev().map(from_pen).collect()
        } else {
            let mut infos = vec![POINTER_INFO::default(); count as usize];
            if GetPointerInfoHistory(id, &mut count, Some(infos.as_mut_ptr())).is_err() {
                return vec![from_info(&info, kind)];
            }
            infos.truncate(count as usize);
            infos.iter().rev().map(|i| from_info(i, kind)).collect()
        }
    }
}

/// A sample packed into a message's `wparam` and `lparam`, for the overlay's test message: how
/// checks draw through the same path as `WM_POINTER*` without global input (Windows has no way
/// to make a real pointer sample but injecting input system-wide).
///
/// `wparam`: bits 0-9 pressure (0-1022; 1023 for none), bit 10 in contact, bit 11 the eraser
/// end, bits 12-13 the kind (0 mouse, 1 pen, 2 touch). `lparam`: x and y, 16 bits each, signed,
/// in screen pixels.
pub fn pack(s: &Sample) -> (usize, isize) {
    let pressure = s.pressure.map_or(1023, |p| {
        ((p.clamp(0.0, 1.0) * 1022.0).round() as usize).min(1022)
    });
    let kind = match s.kind {
        PointerKind::Mouse => 0,
        PointerKind::Pen => 1,
        PointerKind::Touch => 2,
    };
    let wp = pressure | usize::from(s.in_contact) << 10 | usize::from(s.eraser) << 11 | kind << 12;
    let lp = ((s.y as i16 as u16 as isize) << 16) | (s.x as i16 as u16 as isize);
    (wp, lp)
}

/// The pointer id [`unpack`] gives: a check's pointer, never one of Windows' (which are small
/// numbers), so the real mouse moving meanwhile is another pointer and can't end its stroke.
pub const CHECK_POINTER_ID: u32 = 0xC0DE_0001;

/// [`pack`]'s sample back, stamped `time_100ns`, from [`CHECK_POINTER_ID`].
pub fn unpack(wp: usize, lp: isize, time_100ns: i64) -> Sample {
    let p = wp & 0x3FF;
    Sample {
        id: CHECK_POINTER_ID,
        kind: match (wp >> 12) & 3 {
            1 => PointerKind::Pen,
            2 => PointerKind::Touch,
            _ => PointerKind::Mouse,
        },
        x: (lp & 0xFFFF) as u16 as i16 as i32,
        y: ((lp >> 16) & 0xFFFF) as u16 as i16 as i32,
        pressure: (p != 1023).then(|| p as f32 / 1022.0),
        tilt: None,
        eraser: wp & (1 << 11) != 0,
        in_contact: wp & (1 << 10) != 0,
        time_100ns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packed_sample_comes_back_the_same() {
        for s in [
            Sample {
                id: 1,
                kind: PointerKind::Pen,
                x: -1920,
                y: 1079,
                pressure: Some(0.5),
                tilt: None,
                eraser: true,
                in_contact: true,
                time_100ns: 7,
            },
            Sample {
                id: 1,
                kind: PointerKind::Mouse,
                x: 2559,
                y: 0,
                pressure: None,
                tilt: None,
                eraser: false,
                in_contact: false,
                time_100ns: 7,
            },
        ] {
            let (wp, lp) = pack(&s);
            let back = unpack(wp, lp, 7);
            assert_eq!((back.kind, back.x, back.y), (s.kind, s.x, s.y));
            assert_eq!((back.eraser, back.in_contact), (s.eraser, s.in_contact));
            match (back.pressure, s.pressure) {
                (Some(a), Some(b)) => assert!((a - b).abs() < 0.002),
                (a, b) => assert_eq!(a, b),
            }
        }
    }
}
