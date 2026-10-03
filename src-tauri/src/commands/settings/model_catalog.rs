//! Provider availability plus public capabilities. Metadata hosts receive no keys or user data.
use super::*;
use std::collections::BTreeMap;

const METADATA_URL: &str = "https://models.dev/api.json?type=all";
const EXCEPTIONS_URL: &str =
    "https://raw.githubusercontent.com/Verenu/Verenu/master/resources/model-catalog.json";
const BUNDLED_EXCEPTIONS: &str = include_str!("../../../../resources/model-catalog.json");
const MAX_METADATA_BYTES: usize = 12 * 1024 * 1024;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCapability {
    label: String,
    tasks: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct ProviderModelCatalog {
    ids: Vec<String>,
    metadata: BTreeMap<String, ModelCapability>,
    warning: Option<String>,
}

/// Coalesce public requests across providers, with a shorter failure cooldown.
async fn public_json(url: &'static str) -> Result<serde_json::Value, String> {
    type Snapshot = (std::time::Instant, Result<serde_json::Value, String>);
    static CACHE: std::sync::OnceLock<tokio::sync::Mutex<BTreeMap<&'static str, Snapshot>>> =
        std::sync::OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().await;
    if let Some((at, result)) = cache.get(url) {
        let ttl = if result.is_ok() { 86400 } else { 900 };
        if at.elapsed().as_secs() < ttl {
            return result.clone();
        }
    }
    let result = async {
        let mut response = crate::api::client::get()
            .get(url)
            .timeout(std::time::Duration::from_secs(6))
            .send()
            .await
            .map_err(|_| "Model metadata could not be reached.".to_string())?
            .error_for_status()
            .map_err(|_| "Model metadata is unavailable.".to_string())?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "Incomplete model metadata.")?
        {
            if bytes.len() + chunk.len() > MAX_METADATA_BYTES {
                return Err("Model metadata exceeded the size limit.".to_string());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| "Invalid model metadata.".to_string())
    }
    .await;
    cache.insert(url, (std::time::Instant::now(), result.clone()));
    result
}

fn valid_capability(value: &serde_json::Value) -> Option<ModelCapability> {
    let info: ModelCapability = serde_json::from_value(value.clone()).ok()?;
    if info.label.trim().is_empty()
        || info.label.len() > 200
        || info.tasks.is_empty()
        || info.tasks.len() > 2
        || info
            .tasks
            .iter()
            .any(|task| !matches!(task.as_str(), "transcription" | "cleanup"))
    {
        return None;
    }
    Some(info)
}

fn exceptions(
    value: &serde_json::Value,
    provider: &str,
) -> Result<BTreeMap<String, ModelCapability>, String> {
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(1)
        || !value
            .get("providers")
            .is_some_and(serde_json::Value::is_object)
    {
        return Err("Unsupported model catalog version.".into());
    }
    let mut result = BTreeMap::new();
    if let Some(entries) = value
        .get("providers")
        .and_then(|v| v.get(provider))
        .and_then(serde_json::Value::as_object)
    {
        if entries.len() > 1000 {
            return Err("Model catalog exceeded the entry limit.".into());
        }
        for (id, value) in entries {
            if id.trim() != id
                || id.is_empty()
                || id.len() > 200
                || id.chars().any(char::is_control)
            {
                return Err("Invalid catalog model id.".into());
            }
            let info = valid_capability(value).ok_or("Invalid model capability.")?;
            if provider == store::ASSEMBLYAI
                && info.tasks.iter().any(|task| task != "transcription")
            {
                return Err("AssemblyAI only supports transcription.".into());
            }
            result.insert(id.clone(), info);
        }
    }
    Ok(result)
}

fn has(modalities: Option<&serde_json::Value>, kind: &str) -> bool {
    modalities
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(kind)))
}

/// Audio modalities do not make Realtime compatible with audio/transcriptions.
fn capability(
    provider: &str,
    id: &str,
    label: &str,
    input: Option<&serde_json::Value>,
    output: Option<&serde_json::Value>,
) -> Option<ModelCapability> {
    let lower = id.to_lowercase();
    // Dedicated STT output differs from audio-input chat, which uses another endpoint.
    if provider == store::OPENROUTER && has(input, "audio") && has(output, "transcription") {
        return Some(ModelCapability {
            label: label.chars().take(200).collect(),
            tasks: vec!["transcription".into()],
        });
    }
    if [
        "embed",
        "rerank",
        "moderation",
        "guard",
        "realtime",
        "tts",
        "image",
        "dall-e",
        "sora",
        "veo",
        "imagen",
        "lyria",
    ]
    .iter()
    .any(|term| lower.contains(term))
        || !has(output, "text")
    {
        return None;
    }
    let mut tasks = Vec::new();
    let multipart_stt = lower.contains("whisper") || lower.contains("transcribe");
    if has(input, "text")
        && !(multipart_stt && !matches!(provider, store::GOOGLE | store::OPENROUTER))
    {
        tasks.push("cleanup".into());
    }
    if has(input, "audio")
        && (provider == store::GOOGLE || (provider != store::OPENROUTER && multipart_stt))
    {
        tasks.push("transcription".into());
    }
    if tasks.is_empty() {
        return None;
    }
    Some(ModelCapability {
        label: label.chars().take(200).collect(),
        tasks,
    })
}

fn catalog_metadata(
    value: &serde_json::Value,
    provider: &str,
    ids: &[String],
) -> BTreeMap<String, ModelCapability> {
    let mut result = BTreeMap::new();
    for id in ids {
        let Some(model) = value
            .get(provider)
            .and_then(|v| v.get("models"))
            .and_then(|v| v.get(id))
        else {
            continue;
        };
        if model.pointer("/modalities/input").is_some()
            && model.pointer("/modalities/output").is_some()
        {
            let label = model
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(id);
            let info = capability(
                provider,
                id,
                label,
                model.pointer("/modalities/input"),
                model.pointer("/modalities/output"),
            )
            .unwrap_or(ModelCapability {
                label: label.chars().take(200).collect(),
                tasks: Vec::new(),
            });
            result.insert(id.clone(), info);
        }
    }
    result
}

#[tauri::command]
pub async fn get_provider_model_catalog(
    app: AppHandle,
    provider: String,
) -> Result<ProviderModelCatalog, String> {
    if !store::PROVIDERS.contains(&provider.as_str()) || provider == store::LOCAL {
        return Err("Unknown cloud provider.".into());
    }
    let mut ids = list_provider_models(app, provider.clone()).await?;
    let (public, remote) = tokio::join!(public_json(METADATA_URL), public_json(EXCEPTIONS_URL));
    let mut warning = public.as_ref().err().cloned();
    let mut metadata = public
        .as_ref()
        .map(|value| catalog_metadata(value, &provider, &ids))
        .unwrap_or_default();
    let overrides = remote
        .as_ref()
        .ok()
        .and_then(|v| exceptions(v, &provider).ok());
    let overrides = match overrides {
        Some(overrides) => overrides,
        None => {
            if matches!(provider.as_str(), store::ASSEMBLYAI | store::GOOGLE) {
                warning = Some(
                    "Using bundled model exceptions; the online catalog could not be loaded."
                        .into(),
                );
            }
            exceptions(
                &serde_json::from_str(BUNDLED_EXCEPTIONS).expect("bundled catalog JSON"),
                &provider,
            )?
        }
    };
    // Only providers without discovery, or Google's separate Interactions API,
    // may add ids through the exceptions catalog.
    if provider == store::ASSEMBLYAI && !overrides.is_empty() {
        ids.clear();
    }
    for (id, info) in overrides {
        if !matches!(provider.as_str(), store::ASSEMBLYAI | store::GOOGLE) {
            continue;
        }
        if !ids.contains(&id) {
            // New endpoint families require application support. This build
            // knows Google's dedicated Interactions transcriber specifically.
            if provider == store::GOOGLE && id != "gemini-3.5-transcribe" {
                continue;
            }
            ids.push(id.clone());
        }
        metadata.insert(id, info);
    }
    if matches!(provider.as_str(), store::OPENROUTER | store::XAI) {
        let client = crate::api::client::get();
        let request = if provider == store::OPENROUTER {
            client
                .get("https://openrouter.ai/api/v1/models")
                .query(&[("output_modalities", "all")])
        } else {
            client
                .get("https://api.x.ai/v1/language-models")
                .bearer_auth(crate::data::credentials::get(&provider))
        };
        if let Ok(response) = request
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            if response.status().is_success() {
                if let Ok(value) = response.json::<serde_json::Value>().await {
                    let entries = value
                        .get(if provider == store::XAI {
                            "models"
                        } else {
                            "data"
                        })
                        .and_then(serde_json::Value::as_array);
                    for model in entries.into_iter().flatten() {
                        let Some(id) = model.get("id").and_then(serde_json::Value::as_str) else {
                            continue;
                        };
                        if !ids.iter().any(|listed| listed == id) {
                            continue;
                        }
                        let shape = model.get("architecture").unwrap_or(model);
                        if !shape
                            .get("input_modalities")
                            .is_some_and(serde_json::Value::is_array)
                            || !shape
                                .get("output_modalities")
                                .is_some_and(serde_json::Value::is_array)
                        {
                            continue;
                        }
                        let label = model
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(id);
                        let info = capability(
                            &provider,
                            id,
                            model
                                .get("name")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or(id),
                            shape.get("input_modalities"),
                            shape.get("output_modalities"),
                        )
                        .unwrap_or(ModelCapability {
                            label: label.chars().take(200).collect(),
                            tasks: Vec::new(),
                        });
                        metadata.insert(id.into(), info);
                    }
                }
            }
        }
    }
    Ok(ProviderModelCatalog {
        ids,
        metadata,
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn multimodal_generation_excludes_realtime_and_embeddings() {
        let input = json!(["text", "audio", "image"]);
        let output = json!(["text"]);
        let stt_output = json!(["transcription"]);
        assert_eq!(
            capability(
                "openrouter",
                "org/speech",
                "Speech",
                Some(&input),
                Some(&stt_output)
            )
            .unwrap()
            .tasks,
            vec!["transcription"]
        );
        assert_eq!(
            capability(
                "openrouter",
                "org/audio-chat",
                "Chat",
                Some(&input),
                Some(&output)
            )
            .unwrap()
            .tasks,
            vec!["cleanup"]
        );
        assert_eq!(
            capability("google", "gemini-new", "New", Some(&input), Some(&output))
                .unwrap()
                .tasks,
            vec!["cleanup", "transcription"]
        );
        assert!(capability(
            "openai",
            "gpt-realtime",
            "Realtime",
            Some(&input),
            Some(&output)
        )
        .is_none());
        assert!(capability(
            "google",
            "gemini-embedding-2",
            "Embedding",
            Some(&input),
            Some(&output)
        )
        .is_none());
        assert_eq!(
            capability("openai", "gpt-audio", "Audio", Some(&input), Some(&output))
                .unwrap()
                .tasks,
            vec!["cleanup"]
        );
    }
    #[test]
    fn metadata_only_classifies_listed_ids_and_preserves_qualified_ids() {
        let value = json!({"openrouter":{"models":{"org/model:free":{"name":"New model","modalities":{"input":["text"],"output":["text"]}},"unavailable":{"name":"Other","modalities":{"input":["text"],"output":["text"]}}}}});
        let result = catalog_metadata(&value, "openrouter", &["org/model:free".into()]);
        assert_eq!(result.len(), 1);
        assert_eq!(result["org/model:free"].tasks, vec!["cleanup"]);
    }
    #[test]
    fn exceptions_are_versioned_and_reject_unknown_tasks_and_fields() {
        assert!(exceptions(&json!({"version":2}), "assemblyai").is_err());
        for extra in [
            json!({"label":"New","tasks":["arbitrary"]}),
            json!({"label":"New","tasks":["transcription"],"url":"https://evil.example"}),
        ] {
            assert!(exceptions(
                &json!({"version":1,"providers":{"assemblyai":{"new":extra}}}),
                "assemblyai"
            )
            .is_err());
        }
        assert_eq!(
            exceptions(
                &serde_json::from_str(BUNDLED_EXCEPTIONS).unwrap(),
                "assemblyai"
            )
            .unwrap()
            .len(),
            2
        );
        assert!(exceptions(&json!({"version":1,"providers":{"assemblyai":{"wrong-endpoint":{"label":"Wrong","tasks":["cleanup"]}}}}), "assemblyai").is_err());
    }
}
