//! Host-only system notifications. Linux notifications remain unsupported.

pub(super) fn show(app: &tauri::AppHandle, title: &str, body: &str) -> Result<(), ()> {
    let mut notification = notify_rust::Notification::new();
    notification.summary(title).body(body).auto_icon();
    #[cfg(windows)]
    {
        let executable = tauri::utils::platform::current_exe().map_err(|_| ())?;
        if registered_application(&executable) {
            notification.app_id(&app.config().identifier);
        }
    }
    #[cfg(target_os = "macos")]
    notify_rust::set_application(if tauri::is_dev() {
        "com.apple.Terminal"
    } else {
        &app.config().identifier
    })
    .map_err(|_| ())?;
    notification.show().map(|_| ()).map_err(|_| ())
}

#[cfg(windows)]
fn registered_application(executable: &std::path::Path) -> bool {
    // Match the shipped notification adapter: unregistered Cargo executables use
    // the native development fallback instead of an absent Start-menu identity.
    executable.parent().is_some_and(|directory| {
        !["debug", "release"]
            .iter()
            .any(|profile| directory.ends_with(std::path::Path::new("target").join(profile)))
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn installed_notifications_keep_the_registered_identity() {
        assert!(registered_application(Path::new(
            r"C:\Users\fixture\AppData\Local\Colossus\Colossus Desktop.exe"
        )));
        for profile in ["debug", "release"] {
            assert!(!registered_application(Path::new(&format!(
                r"D:\Colossus\target\{profile}\colossus-desktop.exe"
            ))));
        }
    }
}
