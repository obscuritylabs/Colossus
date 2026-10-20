//! Actual live Release-host cache evidence beside long managed ancestry.
use std::{os::windows::ffi::OsStrExt as _, path::PathBuf};

pub(super) struct Layout {
    pub managed_root: PathBuf,
    pub engine_root: PathBuf,
}

fn native_length(path: &std::path::Path) -> usize {
    let native = path.as_os_str().encode_wide().collect::<Vec<_>>();
    native
        .strip_prefix(&[92, 92, 63, 92])
        .unwrap_or(&native)
        .len()
}

impl Layout {
    pub(super) fn assert_short_private_profile_without_logs(&self) {
        assert!(
            self.managed_root.is_dir(),
            "managed instance remains retained"
        );
        assert!(
            native_length(
                &self
                    .managed_root
                    .join("session-00000000000000000000000000000000")
                    .join("profile")
            ) > 260,
            "managed ancestry would exceed Windows MAX_PATH"
        );
        let entries = std::fs::read_dir(&self.engine_root)
            .expect("owned native state remains readable")
            .take(2)
            .collect::<Result<Vec<_>, _>>()
            .expect("owned native state entries remain readable");
        assert_eq!(entries.len(), 1, "one actual owned browser session");
        let entry = &entries[0];
        assert!(
            entry.file_name().to_string_lossy().starts_with("session-")
                && entry.file_type().expect("session type").is_dir(),
            "native session must exist before checking its profile"
        );
        let profile = entry.path().join("profile");
        assert!(profile.is_dir(), "actual native profile exists");
        assert!(
            native_length(&profile) < 240 && !profile.starts_with(&self.managed_root),
            "native profile uses the distinct short private allocation"
        );
        for name in ["startup.log", "debug.log", "chrome_debug.log"] {
            assert!(
                !profile
                    .join(name)
                    .try_exists()
                    .expect("logging evidence readable"),
                "non-diagnostic Release host must not create file logs"
            );
        }
    }
}
