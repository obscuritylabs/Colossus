//! Target-only Windows Graphics Capture. Desktop DC cropping is forbidden.
use super::{Error, identity::Window};
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
    diagnostic("begin", None);
    window.revalidate(context)?;
    if u64::from(window.rect.width) * u64::from(window.rect.height) > u64::from(max_pixels) {
        return Err(Error::InvalidInput);
    }
    // SAFETY: disposable no-window worker owns its MTA and all COM objects.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(failed_at("co_initialize"))?;
    if !GraphicsCaptureSession::IsSupported().map_err(failed_at("is_supported"))? {
        return Err(Error::Unavailable);
    }
    let interop: IGraphicsCaptureItemInterop =
        factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(failed_at("capture_item_factory"))?;
    // SAFETY: exact revalidated admitted window; never a monitor or desktop capture item.
    let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(HWND(window.hwnd())) }
        .map_err(failed_at("create_for_window"))?;
    let size = item.Size().map_err(failed_at("item_size"))?;
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
    .map_err(failed_at("d3d_create_device"))?;
    let device = device.ok_or(Error::Failed)?;
    let context3d = context3d.ok_or(Error::Failed)?;
    let dxgi: IDXGIDevice = device.cast().map_err(failed_at("dxgi_device"))?;
    // SAFETY: retained D3D11 device projected to its WinRT surface-device interface.
    let direct: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .map_err(failed_at("create_winrt_device"))?
        .cast()
        .map_err(failed_at("winrt_device_interface"))?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &direct,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        size,
    )
    .map_err(failed_at("create_frame_pool"))?;
    let session = pool
        .CreateCaptureSession(&item)
        .map_err(failed_at("create_capture_session"))?;
    // Preserve the Windows capture border. The helper separately displays Pause/Stop.
    session
        .SetIsCursorCaptureEnabled(true)
        .map_err(failed_at("cursor_capture"))?;
    session.StartCapture().map_err(failed_at("start_capture"))?;
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
    let content = frame
        .ContentSize()
        .map_err(failed_at("frame_content_size"))?;
    if content != size {
        return Err(Error::Stale);
    }
    window.revalidate(context)?;
    let access: IDirect3DDxgiInterfaceAccess = frame
        .Surface()
        .map_err(failed_at("frame_surface"))?
        .cast()
        .map_err(failed_at("surface_access"))?;
    // SAFETY: retained frame surface exposes its actual texture through the documented interop.
    let texture: ID3D11Texture2D =
        unsafe { access.GetInterface() }.map_err(failed_at("surface_texture"))?;
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
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut staging)) }
        .map_err(failed_at("create_staging_texture"))?;
    let staging = staging.ok_or(Error::Failed)?;
    // SAFETY: same-size/format resources from one device, retained through copy/map.
    unsafe { context3d.CopyResource(&staging, &texture) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: staging texture permits read mapping; output remains valid until Unmap.
    unsafe { context3d.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
        .map_err(failed_at("map_staging_texture"))?;
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
    frame.Close().map_err(failed_at("close_frame"))?;
    session.Close().map_err(failed_at("close_session"))?;
    pool.Close().map_err(failed_at("close_pool"))?;
    window.revalidate(context)?;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &rgba,
            window.rect.width,
            window.rect.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| {
            diagnostic("encode_png", None);
            Error::Failed
        })?;
    if png.len() > max_bytes as usize {
        return Err(Error::InvalidInput);
    }
    Ok(png)
}

fn diagnostic(stage: &'static str, code: Option<i32>) {
    // Only synthetic acceptance builds inherit this stderr. Never include native
    // error messages, target identity, captions, input, paths or captured bytes.
    #[cfg(feature = "native-test-driver")]
    match code {
        Some(code) => eprintln!(
            "COMPUTER_NATIVE_CAPTURE stage={stage} hresult={:08x}",
            code as u32
        ),
        None => eprintln!("COMPUTER_NATIVE_CAPTURE stage={stage}"),
    }
    #[cfg(not(feature = "native-test-driver"))]
    let _ = (stage, code);
}

fn failed_at(stage: &'static str) -> impl FnOnce(windows::core::Error) -> Error {
    move |error| {
        diagnostic(stage, Some(error.code().0));
        Error::Failed
    }
}

#[cfg(all(test, feature = "native-test-driver"))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "explicit no-window native WinRT/D3D initialization probe; no capture or input"]
    fn capture_runtime_initialization_without_window_or_capture() {
        std::thread::spawn(|| {
            // SAFETY: disposable test thread owns its apartment, no UI/capture.
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                .ok()
                .map_err(failed_at("probe_co_initialize"))?;
            let supported =
                GraphicsCaptureSession::IsSupported().map_err(failed_at("probe_is_supported"))?;
            assert!(supported, "WGC reports unsupported");
            let _: IGraphicsCaptureItemInterop =
                factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                    .map_err(failed_at("probe_capture_item_factory"))?;
            let mut device = None;
            // SAFETY: only device initialization; no target, window, surface,
            // frame pool, capture session or native input is created.
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
                    None,
                )
            }
            .map_err(failed_at("probe_d3d_create_device"))?;
            let device = device.ok_or(Error::Failed)?;
            let dxgi: IDXGIDevice = device.cast().map_err(failed_at("probe_dxgi_device"))?;
            // SAFETY: retained ordinary device only, no capture session.
            let _: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
                .map_err(failed_at("probe_create_winrt_device"))?
                .cast()
                .map_err(failed_at("probe_winrt_device_interface"))?;
            Ok::<(), Error>(())
        })
        .join()
        .unwrap()
        .unwrap();
    }

    #[test]
    #[ignore = "explicit no-window native helper-environment initialization regression; no capture or input"]
    fn capture_runtime_initializes_with_only_verified_helper_environment() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.arg("--ignored").arg("--exact")
            .arg("platform::capture::tests::capture_runtime_initialization_without_window_or_capture")
            .arg("--nocapture")
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit());
        super::super::configure_environment(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("owned no-window initialization probe exceeded its deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            status.success(),
            "native helper environment failed no-window WinRT/D3D initialization"
        );
    }
}
