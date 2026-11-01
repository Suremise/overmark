//! An ink surface presented through DirectComposition: a premultiplied flip-model swap chain
//! made for composition, the window's only content (`WS_EX_NOREDIRECTIONBITMAP`).
//!
//! The renderer still draws on the CPU, into the same premultiplied BGRA layout as
//! [`super::layered`] (core renders saves with the same code, so live and saved ink look the
//! same). Each present uploads the dirty rectangle into a GPU copy of the image, copies that
//! into the back buffer and presents it with the dirty rectangle.

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Texture2D,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_PRESENT_PARAMETERS, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2,
    IDXGISwapChain1,
};
use windows::core::Interface;

use super::Rect;
use super::d3d::Device;

/// A `width` × `height` swap chain shown as `hwnd`'s content.
pub struct CompSurface {
    device: Device,
    swap_chain: IDXGISwapChain1,
    ink: ID3D11Texture2D,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    width: i32,
    height: i32,
    presented_once: bool,
}

impl CompSurface {
    /// `hwnd` must have been made with `no_redirection_bitmap`. Fails if a side isn't positive
    /// or the image is too big (see [`super::byte_len`]).
    pub fn new(hwnd: HWND, width: i32, height: i32) -> windows::core::Result<CompSurface> {
        super::checked_len(width, height)?;
        let device = Device::new(None)?;
        // SAFETY: standard DXGI/DirectComposition setup; every object is kept in the result
        // for as long as the window shows it.
        unsafe {
            let dxgi: IDXGIDevice = device.device.cast()?;
            let adapter = dxgi.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width as u32,
                Height: height as u32,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: 0,
            };
            let swap_chain = factory.CreateSwapChainForComposition(&device.device, &desc, None)?;
            let ink_desc = D3D11_TEXTURE2D_DESC {
                Width: width as u32,
                Height: height as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: 0,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut ink = None;
            device
                .device
                .CreateTexture2D(&ink_desc, None, Some(&mut ink))?;
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap_chain)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;
            Ok(CompSurface {
                device,
                swap_chain,
                ink: ink.expect("ink texture"),
                _dcomp: dcomp,
                _target: target,
                _visual: visual,
                width,
                height,
                presented_once: false,
            })
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// Uploads `dirty` of `pixels` (rows of `width` premultiplied BGRA pixels; `None`: all of
    /// it) to the GPU, without presenting.
    pub fn upload(&mut self, pixels: &[u8], dirty: Option<Rect>) {
        let full = Rect::new(0, 0, self.width, self.height);
        let r = match dirty {
            Some(r) if self.presented_once => r.clamp_to(self.width, self.height),
            _ => full,
        };
        if r.is_empty() {
            return;
        }
        let stride = (self.width * 4) as usize;
        let bx = D3D11_BOX {
            left: r.left as u32,
            top: r.top as u32,
            front: 0,
            right: r.right as u32,
            bottom: r.bottom as u32,
            back: 1,
        };
        let offset = r.top as usize * stride + r.left as usize * 4;
        assert!(pixels.len() >= (self.height as usize) * stride);
        // SAFETY: `pixels` holds the whole image (asserted), so the box's rows starting at
        // `offset` with `stride` stay inside it.
        unsafe {
            self.device.context.UpdateSubresource(
                &self.ink,
                0,
                Some(&bx),
                pixels.as_ptr().add(offset) as *const _,
                stride as u32,
                0,
            );
        }
    }

    /// Presents the uploaded image, telling DWM only `dirty` changed. With `vsync`, waits for
    /// a free buffer as a real app would; without, returns at once (the bench's CPU cost).
    pub fn present(&mut self, dirty: Option<Rect>, vsync: bool) -> windows::core::Result<()> {
        // SAFETY: the back buffer is the swap chain's, the same size and format as `ink`.
        unsafe {
            let back: ID3D11Texture2D = self.swap_chain.GetBuffer(0)?;
            self.device.context.CopyResource(&back, &self.ink);
            let mut rect: RECT;
            let params = match dirty {
                Some(r) if self.presented_once => {
                    let r = r.clamp_to(self.width, self.height);
                    if r.is_empty() {
                        return Ok(());
                    }
                    rect = r.to_win32();
                    DXGI_PRESENT_PARAMETERS {
                        DirtyRectsCount: 1,
                        pDirtyRects: &mut rect,
                        pScrollRect: std::ptr::null_mut(),
                        pScrollOffset: std::ptr::null_mut(),
                    }
                }
                _ => DXGI_PRESENT_PARAMETERS::default(),
            };
            self.swap_chain
                .Present1(u32::from(vsync), DXGI_PRESENT(0), &params)
                .ok()?;
        }
        self.presented_once = true;
        Ok(())
    }

    /// Composes the layer with the screen behind it by each channel's minimum (a "darken"
    /// blend; DirectComposition's MIN_BLEND, Windows 8.1 and later) instead of alpha-over:
    /// the highlighter experiment (docs/ARCHITECTURE.md, The highlighter).
    pub fn set_min_blend(&mut self, on: bool) -> windows::core::Result<()> {
        use windows::Win32::Graphics::DirectComposition::{
            DCOMPOSITION_COMPOSITE_MODE_MIN_BLEND, DCOMPOSITION_COMPOSITE_MODE_SOURCE_OVER,
        };
        // SAFETY: our own visual and device.
        unsafe {
            self._visual.SetCompositeMode(if on {
                DCOMPOSITION_COMPOSITE_MODE_MIN_BLEND
            } else {
                DCOMPOSITION_COMPOSITE_MODE_SOURCE_OVER
            })?;
            self._dcomp.Commit()
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }
}
