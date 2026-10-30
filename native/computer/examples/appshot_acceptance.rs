//! Explicit ordinary-user capture of one nonce-named synthetic foreground window.
#[cfg(windows)]
fn main() {
    if let Some(code) = colossus_native_computer::run_appshot_worker_if_requested() {
        std::process::exit(if code == std::process::ExitCode::SUCCESS {
            0
        } else {
            1
        });
    }
    if let Err(error) = run() {
        eprintln!("App Shot acceptance failed: {error}");
        std::process::exit(1);
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("App Shot acceptance needs an ordinary-user Windows desktop");
    std::process::exit(78);
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: this standalone acceptance process owns its DPI setup before UI work.
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    use sha2::{Digest as _, Sha256};
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };
    let mut args = std::env::args().skip(1);
    let mut title = None;
    let mut output = None;
    let mut fixture_image = None;
    let mut fixture_sha256 = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--fixture-title" => title = args.next(),
            "--output" => output = args.next(),
            "--fixture-image" => fixture_image = args.next(),
            "--fixture-image-sha256" => fixture_sha256 = args.next(),
            _ => return Err("unknown acceptance argument".into()),
        }
    }
    let title = title
        .filter(|s| {
            s.len() == 58
                && s.starts_with("Colossus computer fixture ")
                && s.as_bytes()[26..].iter().all(u8::is_ascii_hexdigit)
        })
        .ok_or("exact nonce fixture title required")?;
    let output = PathBuf::from(output.ok_or("owned private output directory required")?);
    if !output.is_absolute() || !output.is_dir() {
        return Err("existing private absolute output directory required".into());
    }
    let fixture_image = PathBuf::from(fixture_image.ok_or("reviewed fixture image required")?);
    if !fixture_image.is_absolute() {
        return Err("absolute reviewed fixture image required".into());
    }
    let fixture_image = colossus_windows_native::BoundPath::open_immutable_file(&fixture_image)?;
    let fixture_sha256 = fixture_sha256.ok_or("reviewed fixture image SHA-256 required")?;
    let wait_until = Instant::now() + Duration::from_secs(45);
    let (ticket, admitted) = loop {
        // The title narrows metadata discovery only. Exact immutable image,
        // digest and frozen process/window admission precede every pixel read.
        if let Ok(ticket) = colossus_native_computer::prepare_foreground_appshot()
            && ticket.window_title() == title
        {
            let admitted = ticket
                .admit_test_fixture(&fixture_image, &fixture_sha256)
                .map_err(|_| "foreground target is not the reviewed synthetic fixture")?;
            break (ticket, admitted);
        }
        if Instant::now() >= wait_until {
            return Err("synthetic fixture was not foreground".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let started = Instant::now();
    let capture = ticket
        .capture_test_fixture(true, &admitted)
        .map_err(|_| "foreground capture or acknowledged helper cleanup failed")?;
    if capture.window_title != title || capture.png.len() > 6 * 1024 * 1024 {
        return Err("capture bounds differ".into());
    }
    let image = image::load_from_memory(&capture.png)?;
    if image.width() != capture.width || image.height() != capture.height {
        return Err("decoded target dimensions differ".into());
    }
    let text = capture
        .text
        .as_deref()
        .ok_or("synthetic visible accessibility labels absent")?;
    if !text.contains("Waiting for action") || text.len() > 32 * 1024 {
        return Err("synthetic text observation differs".into());
    }
    colossus_windows_native::create_private_file(&output.join("appshot.png"), &capture.png)?;
    let image_only = colossus_native_computer::prepare_foreground_appshot()
        .map_err(|_| "foreground target unavailable")?;
    if image_only.window_title() != title {
        return Err("foreground changed before image-only capture".into());
    }
    let image_only = image_only
        .capture_test_fixture(false, &admitted)
        .map_err(|_| "image-only capture failed")?;
    if image_only.text.is_some() {
        return Err("disabled accessibility read returned text".into());
    }
    let receipt = serde_json::json!({
        "schema_version":1,"passed":true,"fixture_title":title,"png_sha256":format!("{:x}", Sha256::digest(&capture.png)),
        "width":capture.width,"height":capture.height,"png_bytes":capture.png.len(),"text_bytes":text.len(),
        "text_present":true,"image_only_text_absent":true,"job_zero_before_each_release":true,"elapsed_ms":started.elapsed().as_millis(),
        "fixture_image_sha256":fixture_sha256,"fixture_image_admitted_before_read":true,"same_admitted_window_for_image_only":true
    });
    colossus_windows_native::create_private_file(
        &output.join("appshot-acceptance.json"),
        &serde_json::to_vec_pretty(&receipt)?,
    )?;
    println!("APP_SHOT_ACCEPTANCE_PASSED");
    Ok(())
}
