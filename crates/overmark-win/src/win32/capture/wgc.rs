//! Windows Graphics Capture of a monitor, as an observer: it sees the screen as a Teams or
//! Zoom share does. Each frame's time and a few probe pixels are kept; the last full frame too,
//! if asked.

use std::sync::{Arc, Mutex};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{IInspectable, Interface};

use super::Frame;
use crate::win32::Rect;
use crate::win32::d3d::Device;
use crate::win32::dpi::Monitor;

/// One observed frame: when DWM composed it (100 ns units, as
/// [`crate::win32::clock::now_100ns`]) and its probe pixels (BGRA), in the probes' order.
#[derive(Debug, Clone)]
pub struct Observed {
    pub time_100ns: i64,
    pub probes: Vec<[u8; 4]>,
}

#[derive(Default)]
struct Shared {
    frames: Vec<Observed>,
    last_full: Option<Frame>,
}

/// A running capture of one monitor; stops when dropped.
pub struct Observer {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    shared: Arc<Mutex<Shared>>,
    /// Whether Windows took `border: false` (the property exists from Windows 10 build 20348).
    pub border_setting_applied: bool,
}

impl Observer {
    /// Starts capturing `monitor`. `probes` are monitor-relative pixels read from every frame;
    /// with `keep_full`, the last whole frame is kept as well. `border` asks for Windows'
    /// yellow capture border (true) or none (false).
    pub fn start(
        monitor: &Monitor,
        probes: Vec<(i32, i32)>,
        keep_full: bool,
        border: bool,
    ) -> windows::core::Result<Observer> {
        let device = Device::new(None)?;
        // SAFETY: WinRT activation and interop calls with arguments we own.
        let (item, d3d) = unsafe {
            let interop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
            let item: GraphicsCaptureItem = interop.CreateForMonitor(monitor.handle)?;
            let dxgi: IDXGIDevice = device.device.cast()?;
            let inspectable: IInspectable = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?;
            let d3d: IDirect3DDevice = inspectable.cast()?;
            (item, d3d)
        };
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            item.Size()?,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        let border_setting_applied = session.SetIsBorderRequired(border).is_ok()
            && session.IsBorderRequired().ok() == Some(border);
        let _ = session.SetIsCursorCaptureEnabled(false);
        let shared = Arc::new(Mutex::new(Shared::default()));
        let sink = shared.clone();
        let reader = Mutex::new(device);
        pool.FrameArrived(&TypedEventHandler::new(
            move |pool: windows::core::Ref<Direct3D11CaptureFramePool>, _| {
                let Some(pool) = pool.as_ref() else {
                    return Ok(());
                };
                let frame = pool.TryGetNextFrame()?;
                let time = frame.SystemRelativeTime()?.Duration;
                let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
                // SAFETY: the frame's surface is a D3D11 texture on our device.
                let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
                let device = reader.lock().unwrap();
                let mut values = Vec::with_capacity(probes.len());
                for &(x, y) in &probes {
                    let px = device.read_back(&texture, Rect::new(x, y, x + 1, y + 1))?;
                    values.push(if px.len() == 4 {
                        [px[0], px[1], px[2], px[3]]
                    } else {
                        [0; 4]
                    });
                }
                let full = if keep_full {
                    let size = frame.ContentSize()?;
                    Some(Frame {
                        width: size.Width,
                        height: size.Height,
                        bgra: device
                            .read_back(&texture, Rect::new(0, 0, size.Width, size.Height))?,
                    })
                } else {
                    None
                };
                drop(device);
                let mut s = sink.lock().unwrap();
                s.frames.push(Observed {
                    time_100ns: time,
                    probes: values,
                });
                if full.is_some() {
                    s.last_full = full;
                }
                Ok(())
            },
        ))?;
        session.StartCapture()?;
        Ok(Observer {
            session,
            pool,
            shared,
            border_setting_applied,
        })
    }

    /// The frames observed so far, oldest first, taken out of the observer.
    pub fn take_frames(&self) -> Vec<Observed> {
        std::mem::take(&mut self.shared.lock().unwrap().frames)
    }

    /// How many frames have been observed and not taken.
    pub fn frame_count(&self) -> usize {
        self.shared.lock().unwrap().frames.len()
    }

    /// The last whole frame (with `keep_full`).
    pub fn last_full(&self) -> Option<Frame> {
        self.shared.lock().unwrap().last_full.clone()
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// What [`wait_for_frame`] found.
#[derive(Debug, Clone)]
pub struct Waited {
    /// The first fresh frame that matched, or else the last fresh frame seen (for a report).
    pub frame: Option<Observed>,
    pub matched: bool,
}

/// Waits for the screen to show something: polls `frames` (an observer's
/// [`Observer::take_frames`]) until a frame composed at or after `since_100ns` (a moment taken
/// just before the change that's expected) satisfies `want`, or `timeout_ms` passes. Frames from
/// before `since_100ns` never count, so a stale scene can't pass. `idle` runs between polls (pump
/// messages, wait a little). A wait for a condition, never for a set time (docs/TESTING.md).
pub fn wait_for_frame(
    mut frames: impl FnMut() -> Vec<Observed>,
    since_100ns: i64,
    timeout_ms: u32,
    mut want: impl FnMut(&Observed) -> bool,
    mut idle: impl FnMut(),
) -> Waited {
    let deadline = crate::win32::clock::now_secs() + f64::from(timeout_ms) / 1000.0;
    let mut last = None;
    loop {
        for f in frames().into_iter().filter(|f| f.time_100ns >= since_100ns) {
            if want(&f) {
                return Waited {
                    frame: Some(f),
                    matched: true,
                };
            }
            last = Some(f);
        }
        if crate::win32::clock::now_secs() >= deadline {
            return Waited {
                frame: last,
                matched: false,
            };
        }
        idle();
    }
}

/// Waits for a capture barrier, then gives the frame that shows it: the first frame whose probe
/// `barrier` shows `colour`. The caller changes a marker of its own to `colour` once the scene
/// it wants to judge is ready, so that frame was composed after the scene was ready, and every
/// frame before it, however it looks, is ignored. Judge the scene by the returned frame's other
/// probes. `None` if no frame showed the barrier within `timeout_ms`.
pub fn frame_after_barrier(
    mut frames: impl FnMut() -> Vec<Observed>,
    barrier: usize,
    colour: [u8; 4],
    timeout_ms: u32,
    mut idle: impl FnMut(),
) -> Option<Observed> {
    let deadline = crate::win32::clock::now_secs() + f64::from(timeout_ms) / 1000.0;
    loop {
        if let Some(f) = frames().into_iter().find(|f| {
            f.probes
                .get(barrier)
                .is_some_and(|p| super::close(*p, colour, 8))
        }) {
            return Some(f);
        }
        if crate::win32::clock::now_secs() >= deadline {
            return None;
        }
        idle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLEAN: [u8; 4] = [50, 100, 200, 255];
    const LEAKED_DIM: [u8; 4] = [32, 65, 130, 255];
    const OLD_MARK: [u8; 4] = [0, 0, 255, 255];
    const NEW_MARK: [u8; 4] = [0, 255, 0, 255];

    /// The counterexample, for an open and for F: after the request, a clean frame
    /// comes before the scene is ready, then only wrong frames. Judged after the barrier, the
    /// wrong scene must fail, and the clean pre-ready frame must not be the one judged.
    #[test]
    fn a_clean_frame_before_the_scene_is_ready_cant_pass() {
        for transition in ["open", "F"] {
            let mut queue = vec![vec![
                // Requested at 100; this frame is from before the overlay showed.
                Observed {
                    time_100ns: 110,
                    probes: vec![CLEAN, OLD_MARK],
                },
                // Ready at 200: the barrier shows, and the scene is wrong.
                Observed {
                    time_100ns: 210,
                    probes: vec![LEAKED_DIM, NEW_MARK],
                },
                Observed {
                    time_100ns: 220,
                    probes: vec![LEAKED_DIM, NEW_MARK],
                },
            ]];
            let judged =
                frame_after_barrier(|| queue.pop().unwrap_or_default(), 1, NEW_MARK, 1000, || {})
                    .expect("the barrier frame");
            assert_eq!(judged.time_100ns, 210, "{transition}");
            assert!(
                !super::super::close(judged.probes[0], CLEAN, 3),
                "{transition}: a leaked scene judged clean"
            );
        }
    }

    #[test]
    fn the_barrier_frame_is_the_one_judged_and_none_without_it() {
        let mut queue = vec![
            vec![Observed {
                time_100ns: 300,
                probes: vec![CLEAN, NEW_MARK],
            }],
            vec![Observed {
                time_100ns: 250,
                probes: vec![LEAKED_DIM, OLD_MARK],
            }],
        ];
        let judged =
            frame_after_barrier(|| queue.pop().unwrap_or_default(), 1, NEW_MARK, 1000, || {});
        assert_eq!(judged.unwrap().time_100ns, 300);
        let none = frame_after_barrier(
            || {
                vec![Observed {
                    time_100ns: 1,
                    probes: vec![CLEAN, OLD_MARK],
                }]
            },
            1,
            NEW_MARK,
            30,
            || {},
        );
        assert!(none.is_none());
    }

    fn frame(t: i64, v: u8) -> Observed {
        Observed {
            time_100ns: t,
            probes: vec![[v; 4]],
        }
    }

    /// A source whose frames come late: nothing for a while, then a stale frame that would
    /// match, then fresh frames that don't, then one that does.
    #[test]
    fn waits_for_a_late_fresh_frame_and_ignores_stale_ones() {
        let mut calls = 0;
        let source = || {
            calls += 1;
            match calls {
                1..=20 => vec![],
                21 => vec![frame(5, 9)],
                22..=30 => vec![frame(100 + calls, 1)],
                31 => vec![frame(200, 9), frame(201, 1)],
                _ => vec![],
            }
        };
        let w = wait_for_frame(source, 10, 10_000, |f| f.probes[0][0] == 9, || {});
        assert!(w.matched);
        assert_eq!(w.frame.unwrap().time_100ns, 200);
    }

    #[test]
    fn gives_up_at_the_deadline_with_the_last_fresh_frame() {
        let mut t = 0;
        let source = || {
            t += 1;
            vec![frame(t, 1)]
        };
        let w = wait_for_frame(
            source,
            0,
            50,
            |f| f.probes[0][0] == 9,
            || std::thread::sleep(std::time::Duration::from_millis(1)),
        );
        assert!(!w.matched);
        assert!(w.frame.is_some_and(|f| f.probes[0][0] == 1));
    }

    #[test]
    fn a_stale_match_alone_never_passes() {
        let w = wait_for_frame(|| vec![frame(1, 9)], 10, 30, |f| f.probes[0][0] == 9, || {});
        assert!(!w.matched && w.frame.is_none());
    }
}
