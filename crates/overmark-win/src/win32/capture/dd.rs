//! DXGI Desktop Duplication: one monitor's desktop image, as composed by DWM. It never shows a
//! border, and windows excluded from capture (`WDA_EXCLUDEFROMCAPTURE`) aren't in it.

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIOutputDuplication, IDXGIResource,
};
use windows::core::Interface;

use super::{Frame, Rotation, unrotate};
use crate::win32::Rect;
use crate::win32::clock;
use crate::win32::d3d::{Device, output_for};
use crate::win32::dpi::Monitor;

/// A duplication of one monitor.
pub struct Duplication {
    device: Device,
    dup: IDXGIOutputDuplication,
    /// The desktop's size (the monitor rectangle's), which frames are turned to.
    width: i32,
    height: i32,
    rotation: Rotation,
    /// The performance counter's frequency, for `LastPresentTime`.
    qpc_per_sec: i64,
}

/// A grabbed frame and when DWM presented it (100 ns units, as [`clock::now_100ns`]).
pub struct Grab {
    pub frame: Frame,
    pub presented_100ns: i64,
}

impl Duplication {
    pub fn new(monitor: &Monitor) -> windows::core::Result<Duplication> {
        let (adapter, output) = output_for(monitor.handle)?;
        let device = Device::new(Some(&adapter))?;
        // SAFETY: duplicates the output on the device made on its own adapter.
        let dup = unsafe { output.DuplicateOutput(&device.device)? };
        // SAFETY: a plain query of the duplication we own.
        let desc = unsafe { dup.GetDesc() };
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_MODE_ROTATION_ROTATE90, DXGI_MODE_ROTATION_ROTATE180, DXGI_MODE_ROTATION_ROTATE270,
        };
        let rotation = match desc.Rotation {
            DXGI_MODE_ROTATION_ROTATE90 => Rotation::Rotate90,
            DXGI_MODE_ROTATION_ROTATE180 => Rotation::Rotate180,
            DXGI_MODE_ROTATION_ROTATE270 => Rotation::Rotate270,
            _ => Rotation::None,
        };
        let mut f = 0i64;
        // SAFETY: writes one i64 we own.
        unsafe { windows::Win32::System::Performance::QueryPerformanceFrequency(&mut f)? };
        Ok(Duplication {
            device,
            dup,
            width: monitor.rect.width(),
            height: monitor.rect.height(),
            rotation,
            qpc_per_sec: f,
        })
    }

    /// The screen once a change made at `after_100ns` is surely in it: frames are taken until
    /// none comes for `settle_ms`, or one comes that was presented `settle_ms` after the change
    /// (a playing video never stops presenting); the last one is returned. The first frame
    /// after a present can still be a composition from before it (measured in the ink-layer
    /// spike), so a save or a freeze uses this rather than [`Duplication::grab_after`].
    pub fn grab_settled(
        &mut self,
        after_100ns: i64,
        settle_ms: u32,
        timeout_ms: u32,
    ) -> windows::core::Result<Grab> {
        let mut last = self.grab_after(after_100ns, timeout_ms)?;
        let settled_at = after_100ns + i64::from(settle_ms) * 10_000;
        while last.presented_100ns < settled_at {
            match self.grab_after(last.presented_100ns, settle_ms) {
                Ok(g) => last = g,
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => break,
                Err(e) => return Err(e),
            }
        }
        Ok(last)
    }

    /// The next frame DWM presents after `after_100ns` (or the current image, for a first grab
    /// with `after_100ns` 0), waiting up to `timeout_ms` in all.
    pub fn grab_after(&mut self, after_100ns: i64, timeout_ms: u32) -> windows::core::Result<Grab> {
        let deadline = clock::now_secs() + f64::from(timeout_ms) / 1000.0;
        loop {
            let left = ((deadline - clock::now_secs()) * 1000.0).max(0.0) as u32;
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            // SAFETY: the out-parameters are ours; each acquired frame is released below.
            let acquired = unsafe { self.dup.AcquireNextFrame(left, &mut info, &mut resource) };
            match acquired {
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    return Err(windows::core::Error::new(
                        DXGI_ERROR_WAIT_TIMEOUT,
                        "no new frame from Desktop Duplication in time",
                    ));
                }
                Err(e) => return Err(e),
                Ok(()) => {}
            }
            let presented =
                (info.LastPresentTime as i128 * 10_000_000 / self.qpc_per_sec as i128) as i64;
            let fresh = info.LastPresentTime != 0 && presented > after_100ns;
            let result = if fresh || after_100ns == 0 {
                let texture: ID3D11Texture2D = resource.expect("frame resource").cast()?;
                // The texture's own size: unturned, so a portrait monitor's is landscape.
                let mut desc =
                    windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC::default();
                // SAFETY: a plain query of the frame's texture.
                unsafe { texture.GetDesc(&mut desc) };
                let (tw, th) = (desc.Width as i32, desc.Height as i32);
                let bgra = self.device.read_back(&texture, Rect::new(0, 0, tw, th))?;
                let frame = unrotate(
                    &Frame {
                        width: tw,
                        height: th,
                        bgra,
                    },
                    self.rotation,
                );
                if (frame.width, frame.height) != (self.width, self.height) {
                    // SAFETY: releases the frame acquired above before giving up.
                    unsafe { self.dup.ReleaseFrame()? };
                    return Err(windows::core::Error::new(
                        windows::Win32::Foundation::E_UNEXPECTED,
                        format!(
                            "Desktop Duplication gave {tw}x{th} turned {:?}, not the monitor's {}x{}",
                            self.rotation, self.width, self.height
                        ),
                    ));
                }
                Some(Grab {
                    frame,
                    presented_100ns: presented,
                })
            } else {
                None
            };
            // SAFETY: releases the frame acquired above.
            unsafe { self.dup.ReleaseFrame()? };
            if let Some(g) = result {
                return Ok(g);
            }
            if clock::now_secs() >= deadline {
                return Err(windows::core::Error::new(
                    DXGI_ERROR_WAIT_TIMEOUT,
                    "no new frame from Desktop Duplication in time",
                ));
            }
        }
    }
}
