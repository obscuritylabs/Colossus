//! Target-only Windows Graphics Capture. Desktop DC cropping is forbidden.
use super::{Error, failed, identity::Window};
use crate::TrustedComputerContext;
use image::ImageEncoder as _;
use std::time::{Duration, Instant};
use windows::{
    Graphics::{
        Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession},
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{Direct3D::D3D_DRIVER_TYPE_HARDWARE, Direct3D11::*, Dxgi::IDXGIDevice},
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx},
            WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
            },
        },
    },
    core::{Interface as _, factory},
};

pub(crate) fn png(
    window: &Window,
    context: &TrustedComputerContext,
    max_bytes: u32,
    max_pixels: u32,
) -> Result<Vec<u8>, Error> {
    window.revalidate(context)?;
    if u64::from(window.rect.width) * u64::from(window.rect.height) > u64::from(max_pixels) {
        return Err(Error::InvalidInput);
    }
    // SAFETY: disposable no-window worker owns its MTA and all COM objects.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(failed)?;
    if !GraphicsCaptureSession::IsSupported().map_err(failed)? {
        return Err(Error::Unavailable);
    }
    let interop: IGraphicsCaptureItemInterop =
        factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(failed)?;
    // SAFETY: exact revalidated admitted window; never a monitor or desktop capture item.
    let item: GraphicsCaptureItem =
        unsafe { interop.CreateForWindow(HWND(window.hwnd())) }.map_err(failed)?;
    let size = item.Size().map_err(failed)?;
    if size.Width != window.rect.width as i32 || size.Height != window.rect.height as i32 {
        return Err(Error::Stale);
    }
    let mut device = None;
    let mut context3d = None;
    // SAFETY: fixed BGRA hardware device, bounded owned outputs, no external pointers.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context3d),
        )
    }
    .map_err(failed)?;
    let device = device.ok_or(Error::Failed)?;
    let context3d = context3d.ok_or(Error::Failed)?;
    let dxgi: IDXGIDevice = device.cast().map_err(failed)?;
    // SAFETY: retained D3D11 device projected to its WinRT surface-device interface.
    let direct: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .map_err(failed)?
        .cast()
        .map_err(failed)?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &direct,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        size,
    )
    .map_err(failed)?;
    let session = pool.CreateCaptureSession(&item).map_err(failed)?;
    // Preserve the Windows capture border. The helper separately displays Pause/Stop.
    session.SetIsCursorCaptureEnabled(true).map_err(failed)?;
    session.StartCapture().map_err(failed)?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let frame = loop {
        if let Ok(frame) = pool.TryGetNextFrame() {
            break frame;
        }
        if Instant::now() >= deadline {
            return Err(Error::Unavailable);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let content = frame.ContentSize().map_err(failed)?;
    if content != size {
        return Err(Error::Stale);
    }
    window.revalidate(context)?;
    let access: IDirect3DDxgiInterfaceAccess =
        frame.Surface().map_err(failed)?.cast().map_err(failed)?;
    // SAFETY: retained frame surface exposes its actual texture through the documented interop.
    let texture: ID3D11Texture2D = unsafe { access.GetInterface() }.map_err(failed)?;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: retained texture and correctly sized stack descriptor.
    unsafe { texture.GetDesc(&mut desc) };
    if desc.Width < window.rect.width || desc.Height < window.rect.height {
        return Err(Error::Stale);
    }
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    let mut staging = None;
    // SAFETY: validated descriptor comes from the same device's captured texture.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }.map_err(failed)?;
    let staging = staging.ok_or(Error::Failed)?;
    // SAFETY: same-size/format resources from one device, retained through copy/map.
    unsafe { context3d.CopyResource(&staging, &texture) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: staging texture permits read mapping; output remains valid until Unmap.
    unsafe { context3d.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.map_err(failed)?;
    let width = window.rect.width as usize;
    let height = window.rect.height as usize;
    let mut rgba = vec![0u8; width * height * 4];
    if mapped.pData.is_null() || (mapped.RowPitch as usize) < width * 4 {
        // SAFETY: successful Map must be balanced even on invalid native geometry.
        unsafe { context3d.Unmap(&staging, 0) };
        return Err(Error::Failed);
    }
    for y in 0..height {
        // SAFETY: mapped descriptor proves every copied row lies in retained texture memory.
        let row = unsafe {
            std::slice::from_raw_parts(
                mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                width * 4,
            )
        };
        for (source, dest) in row
            .chunks_exact(4)
            .zip(rgba[y * width * 4..(y + 1) * width * 4].chunks_exact_mut(4))
        {
            dest.copy_from_slice(&[source[2], source[1], source[0], 255]);
        }
    }
    // SAFETY: last access to mapped memory completed above.
    unsafe { context3d.Unmap(&staging, 0) };
    frame.Close().map_err(failed)?;
    session.Close().map_err(failed)?;
    pool.Close().map_err(failed)?;
    window.revalidate(context)?;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &rgba,
            window.rect.width,
            window.rect.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(failed)?;
    if png.len() > max_bytes as usize {
        return Err(Error::InvalidInput);
    }
    Ok(png)
}
