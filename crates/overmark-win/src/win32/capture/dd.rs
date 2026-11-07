//! DXGI Desktop Duplication: one monitor's desktop image, as composed by DWM. It never shows a
//! border, and windows excluded from capture (`WDA_EXCLUDEFROMCAPTURE`) aren't in it.

use overmark_core::record::pointer::{Shape, ShapeKind};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_POINTER_SHAPE_INFO,
    DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR, DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR,
    DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME, IDXGIOutputDuplication, IDXGIResource,
};
use windows::core::Interface;

use super::{Frame, Rotation, texture_area, unrotate};
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
    /// [`Duplication::next_change`]'s staging texture, kept from one frame to the next.
    staging: Option<(ID3D11Texture2D, Rect)>,
    /// Whether [`Duplication::next_change`] has had its first image.
    had_image: bool,
}

/// What changed on the screen since the last [`Duplication::next_change`].
#[derive(Debug, Default)]
pub struct Change {
    /// The image in the area: the buffer given was refilled.
    pub image: bool,
    /// Where the pointer is now, if it moved or was shown or hidden.
    pub pointer: Option<PointerPosition>,
    /// The pointer's new shape, if it changed.
    pub shape: Option<Shape>,
}

/// Where the pointer's shape is drawn: its top-left, relative to the monitor's top-left (the
/// position Desktop Duplication gives, as Microsoft's sample draws it), and whether it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerPosition {
    pub x: i32,
    pub y: i32,
    pub visible: bool,
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
            staging: None,
            had_image: false,
        })
    }

    /// Waits up to `timeout_ms` for the screen to change, then reads what did: the image in
    /// `area` (relative to the monitor's top-left, in its desktop's orientation) into `out` as
    /// tightly packed BGRA rows, and the pointer. A timeout is no change, not an error; any
    /// other error (`DXGI_ERROR_ACCESS_LOST` when the screen locks or the mode changes) means
    /// this duplication is finished.
    pub fn next_change(
        &mut self,
        timeout_ms: u32,
        area: Rect,
        out: &mut Vec<u8>,
    ) -> windows::core::Result<Change> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: the out-parameters are ours; the frame acquired is released below.
        match unsafe {
            self.dup
                .AcquireNextFrame(timeout_ms, &mut info, &mut resource)
        } {
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(Change::default()),
            Err(e) => return Err(e),
            Ok(()) => {}
        }
        let result = self.read_change(&info, resource, area, out);
        // SAFETY: releases the frame acquired above, whatever reading it gave.
        let released = unsafe { self.dup.ReleaseFrame() };
        let change = result?;
        released?;
        Ok(change)
    }

    fn read_change(
        &mut self,
        info: &DXGI_OUTDUPL_FRAME_INFO,
        resource: Option<IDXGIResource>,
        area: Rect,
        out: &mut Vec<u8>,
    ) -> windows::core::Result<Change> {
        let mut change = Change::default();
        if info.LastPresentTime != 0 || !self.had_image {
            let texture: ID3D11Texture2D = resource.expect("frame resource").cast()?;
            let in_texture = texture_area(area, self.rotation, self.width, self.height);
            let raw = self.read_area(&texture, in_texture)?;
            let turned = unrotate(
                &Frame {
                    width: in_texture.width(),
                    height: in_texture.height(),
                    bgra: raw,
                },
                self.rotation,
            );
            *out = turned.bgra;
            self.had_image = true;
            change.image = true;
        }
        if info.LastMouseUpdateTime != 0 {
            let p = info.PointerPosition;
            change.pointer = Some(PointerPosition {
                x: p.Position.x,
                y: p.Position.y,
                visible: p.Visible.as_bool(),
            });
        }
        if info.PointerShapeBufferSize > 0 {
            change.shape = self.pointer_shape(info.PointerShapeBufferSize)?;
        }
        Ok(change)
    }

    /// `area` of `texture` (in the texture's own, unturned, pixels) through a staging texture
    /// kept while the area's size stays the same.
    fn read_area(
        &mut self,
        texture: &ID3D11Texture2D,
        area: Rect,
    ) -> windows::core::Result<Vec<u8>> {
        let (w, h) = (area.width() as u32, area.height() as u32);
        let staging = match &self.staging {
            Some((t, r)) if (r.width(), r.height()) == (area.width(), area.height()) => t.clone(),
            _ => {
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                // SAFETY: a plain query of the frame's texture.
                unsafe { texture.GetDesc(&mut desc) };
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: w,
                    Height: h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut t = None;
                // SAFETY: a staging texture made on our own device.
                unsafe {
                    self.device
                        .device
                        .CreateTexture2D(&staging_desc, None, Some(&mut t))?
                };
                let t = t.expect("staging texture");
                self.staging = Some((t.clone(), area));
                t
            }
        };
        let bx = D3D11_BOX {
            left: area.left as u32,
            top: area.top as u32,
            front: 0,
            right: area.right as u32,
            bottom: area.bottom as u32,
            back: 1,
        };
        let context = &self.device.context;
        // SAFETY: the box is inside the texture (texture_area keeps it there); the staging
        // texture is the box's size; the mapped rows are read within their pitch, then
        // unmapped.
        unsafe {
            context.CopySubresourceRegion(&staging, 0, 0, 0, 0, texture, 0, Some(&bx));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                out.extend_from_slice(std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    w as usize * 4,
                ));
            }
            context.Unmap(&staging, 0);
            Ok(out)
        }
    }

    /// The pointer's new shape, `size` bytes of it; `None` for a type this doesn't know.
    fn pointer_shape(&mut self, size: u32) -> windows::core::Result<Option<Shape>> {
        let mut data = vec![0u8; size as usize];
        let mut required = 0u32;
        let mut info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
        // SAFETY: the buffer is `size` bytes, the size the frame's info gave.
        unsafe {
            self.dup.GetFramePointerShape(
                size,
                data.as_mut_ptr().cast(),
                &mut required,
                &mut info,
            )?
        };
        let kind = match info.Type as i32 {
            t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 => ShapeKind::Colour,
            t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR.0 => ShapeKind::MaskedColour,
            t if t == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 => ShapeKind::Monochrome,
            _ => return Ok(None),
        };
        // A monochrome shape's height counts both masks.
        let height = match kind {
            ShapeKind::Monochrome => info.Height / 2,
            _ => info.Height,
        };
        Ok(Some(Shape {
            kind,
            width: info.Width,
            height,
            pitch: info.Pitch,
            data,
        }))
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
