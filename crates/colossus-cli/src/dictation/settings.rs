use colossus_native_dictation::{
    DictationSettings, DownloadCancellation, ModelId, download_model, microphones,
};

pub(super) fn configure(
    command: &str,
    cancellation: &DownloadCancellation,
) -> Result<String, String> {
    let settings = DictationSettings::discover().map_err(|error| error.to_string())?;
    super::native::install_bundled(&settings)?;
    let mut preferences = settings.load().map_err(|error| error.to_string())?;
    match command {
        "settings" | "status" => {
            let installed = settings.selected_model(&preferences).is_ok();
            return Ok(format!(
                "Dictation: {} · {} ({}) · microphone {} · spoken punctuation {}.\nF4 starts/pauses/resumes; Shift+F4 stops; Enter finishes and sends.\n/dictate on|off · microphones · microphone default|NUMBER · model tiny|base · install tiny|base · punctuation on|off",
                if preferences.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                preferences.model.name(),
                if installed {
                    "installed"
                } else {
                    "install required"
                },
                if preferences.microphone.is_some() {
                    "selected input"
                } else {
                    "OS default"
                },
                if preferences.spoken_punctuation {
                    "on"
                } else {
                    "off"
                }
            ));
        }
        "on" => preferences.enabled = true,
        "off" => preferences.enabled = false,
        "punctuation on" => preferences.spoken_punctuation = true,
        "punctuation off" => preferences.spoken_punctuation = false,
        "microphones" => {
            let inputs = microphones().map_err(|error| error.to_string())?;
            return Ok(format!(
                "Microphones (local to this terminal):\n0. OS default\n{}\nSelect with /dictate microphone NUMBER.",
                inputs
                    .iter()
                    .enumerate()
                    .map(|(index, input)| format!("{}. {}", index + 1, input.name))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        "microphone default" | "microphone 0" => preferences.microphone = None,
        "model tiny" | "model base" => {
            preferences.model = if command.ends_with("tiny") {
                ModelId::TinyEnglish
            } else {
                ModelId::BaseEnglish
            };
            preferences.model_path = None;
            settings.selected_model(&preferences).map_err(|_| {
                "Install that model first with /dictate install tiny|base.".to_owned()
            })?;
        }
        "install tiny" | "install base" => {
            let model = if command.ends_with("tiny") {
                ModelId::TinyEnglish
            } else {
                ModelId::BaseEnglish
            };
            download_model(&settings, model, cancellation).map_err(|error| error.to_string())?;
            return Ok(format!(
                "{} installed. Select it with /dictate model {}.",
                model.name(),
                if model == ModelId::TinyEnglish {
                    "tiny"
                } else {
                    "base"
                }
            ));
        }
        command if command.starts_with("microphone ") => {
            let number: usize = command
                .trim_start_matches("microphone ")
                .parse()
                .map_err(|_| "Use /dictate microphones to list inputs.")?;
            let inputs = microphones().map_err(|error| error.to_string())?;
            preferences.microphone = Some(
                inputs
                    .get(number.checked_sub(1).ok_or("Invalid microphone number.")?)
                    .ok_or("Microphone is no longer available.")?
                    .id
                    .clone(),
            );
        }
        _ => {
            return Err("Use /dictate settings for controls and model/microphone commands.".into());
        }
    }
    settings
        .save(&preferences)
        .map_err(|error| error.to_string())?;
    Ok("Dictation preferences saved locally. Recording requires F4 or /dictate start; audio stays on this machine.".into())
}
