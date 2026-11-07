use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalSttEngineType {
    Parakeet,
    Moonshine,
    MoonshineStreaming,
    SenseVoice,
    GigaAm,
    Canary,
    Cohere,
    Whisper,
    AppleSpeech,
    FluidAudio,
    CtcBooster,
}

#[derive(Clone, Debug, Serialize)]
pub struct LocalSttModelInfo {
    pub install_kind: &'static str,
    pub id: String,
    pub name: String,
    pub description: String,
    pub filename: String,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub size_mb: u64,
    pub is_directory: bool,
    pub is_downloaded: bool,
    pub is_downloading: bool,
    pub partial_size: u64,
    pub engine_type: LocalSttEngineType,
    pub speed_score: f32,
    pub accuracy_score: f32,
    pub privacy_label: String,
    pub supported_languages: Vec<String>,
    pub supports_language_selection: bool,
    pub supports_translation: bool,
    pub is_recommended: bool,
}

#[derive(Clone, Debug)]
pub struct LocalSttModelManifest {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub filename: &'static str,
    pub url: Option<&'static str>,
    pub sha256: Option<&'static str>,
    pub size_mb: u64,
    pub is_directory: bool,
    pub install_name: &'static str,
    pub engine_type: LocalSttEngineType,
    pub speed_score: f32,
    pub accuracy_score: f32,
    pub privacy_label: &'static str,
    pub supported_languages: &'static [&'static str],
    pub supports_language_selection: bool,
    pub supports_translation: bool,
    pub is_recommended: bool,
}

impl LocalSttModelManifest {
    pub fn is_supported(&self) -> bool {
        match self.engine_type {
            LocalSttEngineType::AppleSpeech => cfg!(target_os = "macos"),
            LocalSttEngineType::FluidAudio | LocalSttEngineType::CtcBooster => {
                super::fluid_supported()
            }
            LocalSttEngineType::Whisper => !cfg!(target_os = "android"),
            _ => true,
        }
    }
    pub fn final_path(&self, root: &Path) -> PathBuf {
        root.join(self.install_name)
    }

    pub fn partial_download_path(&self, root: &Path) -> PathBuf {
        root.join(format!("{}.partial", self.filename))
    }

    pub fn extracting_path(&self, root: &Path) -> PathBuf {
        root.join(format!("{}.extracting", self.install_name))
    }

    pub fn is_downloaded(&self, root: &Path) -> bool {
        if matches!(
            self.engine_type,
            LocalSttEngineType::FluidAudio | LocalSttEngineType::CtcBooster
        ) {
            return self.is_supported() && super::fluid_download::installed(self, root);
        }
        if self.engine_type == LocalSttEngineType::AppleSpeech {
            return cfg!(target_os = "macos");
        }
        if self.engine_type == LocalSttEngineType::Whisper && cfg!(target_os = "android") {
            return false;
        }
        let path = self.final_path(root);
        if self.is_directory {
            path.is_dir()
        } else {
            path.is_file()
        }
    }

    pub fn partial_size(&self, root: &Path) -> u64 {
        std::fs::metadata(self.partial_download_path(root))
            .map(|meta| meta.len())
            .unwrap_or(0)
    }

    pub fn to_info(&self, root: &Path, is_downloading: bool) -> LocalSttModelInfo {
        LocalSttModelInfo {
            install_kind: if self.engine_type == LocalSttEngineType::AppleSpeech {
                "system_managed"
            } else {
                "download"
            },
            id: self.id.to_string(),
            name: self.name.to_string(),
            description: self.description.to_string(),
            filename: self.filename.to_string(),
            url: self.url.map(str::to_string),
            sha256: self.sha256.map(str::to_string),
            size_mb: self.size_mb,
            is_directory: self.is_directory,
            is_downloaded: self.is_downloaded(root),
            is_downloading,
            partial_size: self.partial_size(root),
            engine_type: self.engine_type,
            speed_score: self.speed_score,
            accuracy_score: self.accuracy_score,
            privacy_label: self.privacy_label.to_string(),
            supported_languages: self
                .supported_languages
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            supports_language_selection: self.supports_language_selection,
            supports_translation: self.supports_translation,
            is_recommended: self.is_recommended,
        }
    }
}

pub fn built_in_model_manifests() -> Vec<LocalSttModelManifest> {
    vec![
        fluid_manifest("fluid-parakeet-ultra", "Parakeet Ultra · CoreML", "Mac Apple Silicon offline multilingual speech. FluidAudio CoreML engine.", 603, &["Multilingual"]),
        fluid_manifest("fluid-parakeet-110m", "Parakeet 110M · CoreML", "Small English offline speech on Mac Apple Silicon. FluidAudio TDT engine.", 217, &["English"]),
        fluid_manifest("fluid-parakeet-ja", "Parakeet Japanese · CoreML", "Japanese offline speech on Mac Apple Silicon. FluidAudio TDT engine.", 591, &["Japanese"]),
        LocalSttModelManifest { engine_type: LocalSttEngineType::CtcBooster,
            ..fluid_manifest("fluid-english-booster", "English vocabulary booster", "Optional acoustic Context vocabulary rescoring for FluidAudio, for explicitly English dictation. Separate from speech models.", 98, &["English"]) },
        LocalSttModelManifest {
            id: "apple-speech", name: "Apple Speech",
            description: "System-managed on-device recognition on macOS. Requires Speech permission and supported language assets.",
            filename: "", url: None, sha256: None, size_mb: 0, is_directory: false,
            install_name: "apple-speech", engine_type: LocalSttEngineType::AppleSpeech,
            speed_score: 3.0, accuracy_score: 3.0, privacy_label: "Runs on this device",
            supported_languages: &["System languages"], supports_language_selection: true,
            supports_translation: false, is_recommended: false,
        },
        LocalSttModelManifest {
            id: "whisper-small", name: "Whisper Small",
            description: "Multilingual offline speech with Context vocabulary hints. Portable whisper.cpp CPU engine.",
            filename: "ggml-small.bin",
            url: Some("https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.bin"),
            sha256: Some("1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"),
            size_mb: 465, is_directory: false, install_name: "ggml-small.bin",
            engine_type: LocalSttEngineType::Whisper,
            speed_score: 2.0, accuracy_score: 3.5,
            privacy_label: "Runs on this device", supported_languages: &["Multilingual"],
            supports_language_selection: true, supports_translation: false, is_recommended: false,
        },
        LocalSttModelManifest {
            id: "whisper-large-v3-turbo", name: "Whisper Large V3 Turbo",
            description: "Larger multilingual offline speech with Context vocabulary hints. Portable whisper.cpp CPU engine.",
            filename: "ggml-large-v3-turbo.bin",
            url: Some("https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo.bin"),
            sha256: Some("1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69"),
            size_mb: 1550, is_directory: false, install_name: "ggml-large-v3-turbo.bin",
            engine_type: LocalSttEngineType::Whisper,
            speed_score: 1.5, accuracy_score: 4.0,
            privacy_label: "Runs on this device", supported_languages: &["Multilingual"],
            supports_language_selection: true, supports_translation: false, is_recommended: false,
        },
        LocalSttModelManifest {
            id: "parakeet-v3",
            name: "Parakeet V3",
            description: "Fast and accurate. Supports 25 European languages.",
            filename: "parakeet-v3-int8.tar.gz",
            url: None,
            sha256: None,
            size_mb: 456,
            is_directory: true,
            install_name: "parakeet-tdt-0.6b-v3-int8",
            engine_type: LocalSttEngineType::Parakeet,
            speed_score: 4.25,
            accuracy_score: 4.0,
            privacy_label: "Runs on this device",
            supported_languages: &[
                "Bulgarian", "Croatian", "Czech", "Danish", "Dutch", "English", "Estonian",
                "Finnish", "French", "German", "Greek", "Hungarian", "Italian", "Latvian",
                "Lithuanian", "Maltese", "Polish", "Portuguese", "Romanian", "Slovak",
                "Slovenian", "Spanish", "Swedish", "Russian", "Ukrainian",
            ],
            supports_language_selection: false,
            supports_translation: false,
            is_recommended: true,
        },
        LocalSttModelManifest {
            id: "parakeet-v2",
            name: "Parakeet V2",
            description: "English-only alternative to Parakeet V3 with slightly higher English accuracy.",
            filename: "parakeet-v2-int8.tar.gz",
            url: None,
            sha256: None,
            size_mb: 451,
            is_directory: true,
            install_name: "parakeet-tdt-0.6b-v2-int8",
            engine_type: LocalSttEngineType::Parakeet,
            speed_score: 4.25,
            accuracy_score: 4.25,
            privacy_label: "Runs on this device",
            supported_languages: &["English"],
            supports_language_selection: false,
            supports_translation: false,
            is_recommended: false,
        },
        LocalSttModelManifest {
            id: "moonshine-base",
            name: "Moonshine Base",
            description: "Smaller English model for weaker machines and faster local tests.",
            filename: "moonshine-base.tar.gz",
            url: None,
            sha256: None,
            size_mb: 187,
            is_directory: true,
            install_name: "moonshine-base",
            engine_type: LocalSttEngineType::Moonshine,
            speed_score: 4.8,
            accuracy_score: 3.3,
            privacy_label: "Runs on this device",
            supported_languages: &["English"],
            supports_language_selection: false,
            supports_translation: false,
            is_recommended: false,
        },
        LocalSttModelManifest {
            id: "moonshine-tiny",
            name: "Moonshine Tiny",
            description: "Smallest and fastest English model. Best for low-power machines.",
            filename: "moonshine-tiny-streaming-en.tar.gz",
            url: None,
            sha256: None,
            size_mb: 31,
            is_directory: true,
            install_name: "moonshine-tiny-streaming-en",
            engine_type: LocalSttEngineType::MoonshineStreaming,
            speed_score: 4.75,
            accuracy_score: 2.75,
            privacy_label: "Runs on this device",
            supported_languages: &["English"],
            supports_language_selection: false,
            supports_translation: false,
            is_recommended: false,
        },
        LocalSttModelManifest {
            id: "moonshine-small",
            name: "Moonshine Small",
            description: "Fast English model with a good balance of speed and accuracy.",
            filename: "moonshine-small-streaming-en.tar.gz",
            url: None,
            sha256: None,
            size_mb: 99,
            is_directory: true,
            install_name: "moonshine-small-streaming-en",
            engine_type: LocalSttEngineType::MoonshineStreaming,
            speed_score: 4.5,
            accuracy_score: 3.25,
            privacy_label: "Runs on this device",
            supported_languages: &["English"],
            supports_language_selection: false,
            supports_translation: false,
            is_recommended: false,
        },
        LocalSttModelManifest {
            id: "moonshine-medium",
            name: "Moonshine Medium",
            description: "Higher quality English transcription, still fast.",
            filename: "moonshine-medium-streaming-en.tar.gz",
            url: None,
            sha256: None,
            size_mb: 192,
            is_directory: true,
            install_name: "moonshine-medium-streaming-en",
            engine_type: LocalSttEngineType::MoonshineStreaming,
            speed_score: 4.0,
            accuracy_score: 3.75,
            privacy_label: "Runs on this device",
            supported_languages: &["English"],
            supports_language_selection: false,
            supports_translation: false,
            is_recommended: false,
        },
        LocalSttModelManifest {
            id: "sense-voice",
            name: "SenseVoice",
            description: "Very fast multilingual model: Chinese, English, Japanese, Korean, Cantonese.",
            filename: "sense-voice-int8.tar.gz",
            url: None,
            sha256: None,
            size_mb: 152,
            is_directory: true,
            install_name: "sense-voice-int8",
            engine_type: LocalSttEngineType::SenseVoice,
            speed_score: 4.75,
            accuracy_score: 3.25,
            privacy_label: "Runs on this device",
            supported_languages: &["Chinese", "English", "Japanese", "Korean", "Cantonese"],
            supports_language_selection: true,
            supports_translation: false,
            is_recommended: false,
        },
        LocalSttModelManifest {
            id: "gigaam-v3",
            name: "GigaAM v3",
            description: "Dedicated Russian speech recognition. Fast and accurate.",
            filename: "giga-am-v3-int8.tar.gz",
            url: None,
            sha256: None,
            size_mb: 151,
            is_directory: true,
            install_name: "giga-am-v3-int8",
            engine_type: LocalSttEngineType::GigaAm,
            speed_score: 3.75,
            accuracy_score: 4.25,
            privacy_label: "Runs on this device",
            supported_languages: &["Russian"],
            supports_language_selection: false,
            supports_translation: false,
            is_recommended: false,
        },
        LocalSttModelManifest {
            id: "canary-180m-flash",
            name: "Canary 180M Flash",
            description: "Small, fast multilingual model: English, German, Spanish, French. Supports translation.",
            filename: "canary-180m-flash.tar.gz",
            url: None,
            sha256: None,
            size_mb: 146,
            is_directory: true,
            install_name: "canary-180m-flash",
            engine_type: LocalSttEngineType::Canary,
            speed_score: 4.25,
            accuracy_score: 3.75,
            privacy_label: "Runs on this device",
            supported_languages: &["English", "German", "Spanish", "French"],
            supports_language_selection: true,
            supports_translation: true,
            is_recommended: false,
        },
        LocalSttModelManifest {
            id: "canary-1b-v2",
            name: "Canary 1B v2",
            description: "Larger, more accurate multilingual model. 25 European languages. Supports translation.",
            filename: "canary-1b-v2.tar.gz",
            url: None,
            sha256: None,
            size_mb: 691,
            is_directory: true,
            install_name: "canary-1b-v2",
            engine_type: LocalSttEngineType::Canary,
            speed_score: 3.5,
            accuracy_score: 4.25,
            privacy_label: "Runs on this device",
            supported_languages: &[
                "Bulgarian", "Croatian", "Czech", "Danish", "Dutch", "English", "Estonian",
                "Finnish", "French", "German", "Greek", "Hungarian", "Italian", "Latvian",
                "Lithuanian", "Maltese", "Polish", "Portuguese", "Romanian", "Slovak",
                "Slovenian", "Spanish", "Swedish", "Russian", "Ukrainian",
            ],
            supports_language_selection: true,
            supports_translation: true,
            is_recommended: false,
        },
        LocalSttModelManifest {
            id: "cohere",
            name: "Cohere",
            description: "Largest and most accurate multilingual model. Covers European and East Asian languages, but slower.",
            filename: "cohere-int8.tar.gz",
            url: None,
            sha256: None,
            size_mb: 1708,
            is_directory: true,
            install_name: "cohere-int8",
            engine_type: LocalSttEngineType::Cohere,
            speed_score: 3.0,
            accuracy_score: 4.5,
            privacy_label: "Runs on this device",
            supported_languages: &[
                "English", "French", "German", "Italian", "Spanish", "Portuguese", "Greek",
                "Dutch", "Polish", "Chinese", "Japanese", "Korean", "Vietnamese", "Arabic",
            ],
            supports_language_selection: true,
            supports_translation: false,
            is_recommended: false,
        },
    ]
}

pub fn manifest_by_id(model_id: &str) -> Option<LocalSttModelManifest> {
    built_in_model_manifests()
        .into_iter()
        .find(|manifest| manifest.id == model_id)
}

fn fluid_manifest(
    id: &'static str,
    name: &'static str,
    description: &'static str,
    size_mb: u64,
    languages: &'static [&'static str],
) -> LocalSttModelManifest {
    LocalSttModelManifest {
        id,
        name,
        description,
        filename: id,
        install_name: id,
        url: Some("https://huggingface.co/FluidInference"),
        sha256: None,
        size_mb,
        is_directory: true,
        engine_type: LocalSttEngineType::FluidAudio,
        speed_score: 3.0,
        accuracy_score: 4.0,
        privacy_label: "Runs on this device",
        supported_languages: languages,
        supports_language_selection: id == "fluid-parakeet-ultra",
        supports_translation: false,
        is_recommended: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_type_serializes_to_expected_snake_case() {
        let cases = [
            (LocalSttEngineType::Parakeet, "\"parakeet\""),
            (LocalSttEngineType::Moonshine, "\"moonshine\""),
            (
                LocalSttEngineType::MoonshineStreaming,
                "\"moonshine_streaming\"",
            ),
            (LocalSttEngineType::SenseVoice, "\"sense_voice\""),
            (LocalSttEngineType::GigaAm, "\"giga_am\""),
            (LocalSttEngineType::Canary, "\"canary\""),
            (LocalSttEngineType::Cohere, "\"cohere\""),
            (LocalSttEngineType::Whisper, "\"whisper\""),
            (LocalSttEngineType::AppleSpeech, "\"apple_speech\""),
            (LocalSttEngineType::FluidAudio, "\"fluid_audio\""),
            (LocalSttEngineType::CtcBooster, "\"ctc_booster\""),
        ];
        for (variant, expected) in cases {
            assert_eq!(serde_json::to_string(&variant).unwrap(), expected);
        }
    }

    #[test]
    fn every_manifest_has_a_unique_id_and_install_name() {
        let manifests = built_in_model_manifests();
        let mut ids = std::collections::HashSet::new();
        let mut install_names = std::collections::HashSet::new();
        for manifest in &manifests {
            assert!(ids.insert(manifest.id), "duplicate id: {}", manifest.id);
            assert!(
                install_names.insert(manifest.install_name),
                "duplicate install_name: {}",
                manifest.install_name
            );
        }
    }
}
