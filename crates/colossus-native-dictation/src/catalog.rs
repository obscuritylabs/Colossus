use serde::{Deserialize, Serialize};

/// Reviewed English models from one immutable Whisper conversion revision.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelId {
    /// Small, portable English model included in Desktop installations.
    #[default]
    TinyEnglish,
    /// Larger English model installed separately.
    BaseEnglish,
}

impl ModelId {
    /// Fixed model display name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::TinyEnglish => "Tiny English",
            Self::BaseEnglish => "Base English",
        }
    }
    /// Fixed asset filename.
    #[must_use]
    pub fn filename(self) -> &'static str {
        match self {
            Self::TinyEnglish => "ggml-tiny.en.bin",
            Self::BaseEnglish => "ggml-base.en.bin",
        }
    }
    /// Expected size before accepting or loading an asset.
    #[must_use]
    pub fn bytes(self) -> u64 {
        match self {
            Self::TinyEnglish => 77_704_715,
            Self::BaseEnglish => 147_964_211,
        }
    }
    /// Reviewed SHA-256 of the pinned asset.
    #[must_use]
    pub fn digest(self) -> &'static str {
        match self {
            Self::TinyEnglish => "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
            Self::BaseEnglish => "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        }
    }
    /// Pinned download origin; callers cannot provide an alternate URL or digest.
    #[must_use]
    pub fn url(self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/{}",
            self.filename()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaging_provenance_matches_the_native_download_and_verification_catalog() {
        let pin: serde_json::Value =
            serde_json::from_str(include_str!("../../../release/dictation/models.json")).unwrap();
        for model in [ModelId::TinyEnglish, ModelId::BaseEnglish] {
            let id = serde_json::to_value(model).unwrap();
            let item = &pin["models"][id.as_str().unwrap()];
            assert_eq!(item["filename"], model.filename());
            assert_eq!(item["bytes"], model.bytes());
            assert_eq!(item["sha256"], model.digest());
            assert_eq!(item["bundled"], model == ModelId::TinyEnglish);
            assert_eq!(
                model.url(),
                format!(
                    "{}/resolve/{}/{}",
                    pin["upstream"].as_str().unwrap(),
                    pin["revision"].as_str().unwrap(),
                    model.filename()
                )
            );
        }
    }
}
