use crate::DictationError;
use cpal::traits::{DeviceTrait as _, HostTrait as _};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

/// Bounded microphone metadata; identity is opaque and carries no device path.
#[derive(Clone, Debug, Serialize)]
pub struct Microphone {
    /// Opaque native identity, persisted across launches when the OS identity is stable.
    pub id: String,
    /// Human-readable local input name.
    pub name: String,
}

fn identity(device: &cpal::Device) -> Result<String, DictationError> {
    let id = device
        .id()
        .map_err(|_| DictationError::CaptureUnavailable)?;
    Ok(format!("{:x}", Sha256::digest(id.to_string().as_bytes())))
}

/// Enumerate at most 64 locally available input devices without opening a stream.
/// # Errors
/// Reports an unavailable audio host; unavailable individual devices are omitted.
pub fn microphones() -> Result<Vec<Microphone>, DictationError> {
    let devices = cpal::default_host()
        .input_devices()
        .map_err(|_| DictationError::CaptureUnavailable)?;
    Ok(devices
        .take(64)
        .filter_map(|device| {
            let id = identity(&device).ok()?;
            let description = device.description().ok()?;
            let name = description
                .name()
                .chars()
                .filter(|character| !character.is_control())
                .take(80)
                .collect();
            Some(Microphone { id, name })
        })
        .collect())
}

pub(crate) fn selected(microphone: Option<&str>) -> Result<cpal::Device, DictationError> {
    let host = cpal::default_host();
    match microphone {
        None => host
            .default_input_device()
            .ok_or(DictationError::MicrophoneMissing),
        Some(selected) => host
            .input_devices()
            .map_err(|_| DictationError::CaptureUnavailable)?
            .take(64)
            .find(|device| identity(device).is_ok_and(|id| id == selected))
            .ok_or(DictationError::MicrophoneMissing),
    }
}
