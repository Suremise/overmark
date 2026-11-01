//! D3D11 devices, on the GPU that drives a given monitor, and reading textures back.

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::core::Interface;

use super::Rect;

/// A D3D11 device and its immediate context.
#[derive(Clone)]
pub struct Device {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
}

/// The adapter and output (a DXGI monitor) for `monitor`.
pub fn output_for(monitor: HMONITOR) -> windows::core::Result<(IDXGIAdapter1, IDXGIOutput1)> {
    // SAFETY: plain DXGI enumeration; every out-parameter is ours.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut a = 0;
        while let Ok(adapter) = factory.EnumAdapters1(a) {
            let mut o = 0;
            while let Ok(output) = adapter.EnumOutputs(o) {
                let output: IDXGIOutput = output;
                if output.GetDesc()?.Monitor == monitor {
                    return Ok((adapter, output.cast()?));
                }
                o += 1;
            }
            a += 1;
        }
    }
    Err(windows::core::Error::new(
        windows::Win32::Foundation::E_FAIL,
        "no DXGI output for that monitor",
    ))
}

impl Device {
    /// A device on `adapter` (the one driving the monitor: Desktop Duplication needs that),
    /// or on the default adapter.
    pub fn new(adapter: Option<&IDXGIAdapter1>) -> windows::core::Result<Device> {
        let mut device = None;
        let mut context = None;
        let adapter: Option<windows::Win32::Graphics::Dxgi::IDXGIAdapter> =
            adapter.map(|a| a.cast()).transpose()?;
        // SAFETY: standard device creation; the out-parameters are ours.
        unsafe {
            D3D11CreateDevice(
                adapter.as_ref(),
                if adapter.is_some() {
                    D3D_DRIVER_TYPE_UNKNOWN
                } else {
                    windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE
                },
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        Ok(Device {
            device: device.expect("device"),
            context: context.expect("context"),
        })
    }

    /// Reads `area` of a 32-bit texture back to the CPU: tightly packed rows, 4 bytes a pixel,
    /// in the texture's own channel order.
    pub fn read_back(
        &self,
        texture: &ID3D11Texture2D,
        area: Rect,
    ) -> windows::core::Result<Vec<u8>> {
        // SAFETY: a staging texture of the area's size, copied into and mapped for reading;
        // the mapped rows are read within their pitch and the map is released.
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let area = area.clamp_to(desc.Width as i32, desc.Height as i32);
            let (w, h) = (area.width() as u32, area.height() as u32);
            if w == 0 || h == 0 {
                return Ok(Vec::new());
            }
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
            let mut staging = None;
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
            let staging = staging.expect("staging texture");
            let bx = D3D11_BOX {
                left: area.left as u32,
                top: area.top as u32,
                front: 0,
                right: area.right as u32,
                bottom: area.bottom as u32,
                back: 1,
            };
            self.context
                .CopySubresourceRegion(&staging, 0, 0, 0, 0, texture, 0, Some(&bx));
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    w as usize * 4,
                );
                out.extend_from_slice(row);
            }
            self.context.Unmap(&staging, 0);
            Ok(out)
        }
    }
}
