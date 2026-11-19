//! Exercise the build script's actual channel rule without selecting native bytes.

#[path = "../build/windows_browser_release_policy.rs"]
mod release_policy;

#[test]
fn release_edition_accepts_published_and_unsigned_validation_channels() {
    for channel in ["stable", "developer_preview", "validation_only"] {
        assert!(release_policy::admitted("release", Some(channel), false));
    }
}

#[test]
fn an_explicit_known_channel_is_required() {
    for channel in [None, Some(""), Some("unknown"), Some("Stable")] {
        assert!(!release_policy::admitted("release", channel, false));
    }
}

#[test]
fn debug_or_unknown_profiles_cannot_select_the_release_edition() {
    for profile in ["debug", "dev", "", "Release"] {
        for channel in ["stable", "developer_preview", "validation_only"] {
            assert!(!release_policy::admitted(profile, Some(channel), false));
        }
    }
}

#[test]
fn development_browser_features_cannot_be_combined_with_the_release_edition() {
    for channel in ["stable", "developer_preview", "validation_only"] {
        assert!(!release_policy::admitted("release", Some(channel), true));
    }
}
