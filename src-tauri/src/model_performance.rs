//! Process-local timing metadata. Never retains audio, text, keys, or endpoints.
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, serde::Serialize)]
pub struct ModelPerformance {
    pub task: String,
    pub id: String,
    pub samples: u32,
    pub failures: u32,
    pub latency_ms: f64,
    pub updated_at_ms: u64,
}

static SAMPLES: OnceLock<Mutex<BTreeMap<String, ModelPerformance>>> = OnceLock::new();

pub fn record(task: &str, provider: &str, model: &str, elapsed_ms: f64, success: bool) {
    let Ok(mut samples) = SAMPLES.get_or_init(Default::default).lock() else {
        return;
    };
    let id = format!("{provider}/{model}");
    let key = format!("{task}/{id}");
    if samples.len() >= 256 && !samples.contains_key(&key) {
        return;
    }
    let sample = samples.entry(key).or_insert_with(|| ModelPerformance {
        task: task.to_string(),
        id,
        samples: 0,
        failures: 0,
        latency_ms: 0.0,
        updated_at_ms: 0,
    });
    sample.samples = sample.samples.saturating_add(1);
    sample.failures = sample.failures.saturating_add(u32::from(!success));
    sample.latency_ms = if sample.samples == 1 {
        elapsed_ms
    } else {
        sample.latency_ms * 0.75 + elapsed_ms * 0.25
    };
    sample.updated_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
}

pub fn snapshot() -> Vec<ModelPerformance> {
    SAMPLES
        .get_or_init(Default::default)
        .lock()
        .map(|samples| samples.values().cloned().collect())
        .unwrap_or_default()
}

pub fn prioritize(chain: &mut [(String, String)], task: &str, samples: &[ModelPerformance]) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let measurement = |candidate: &(String, String)| {
        samples.iter().find(|sample| {
            sample.task == task
                && sample.id.strip_prefix(candidate.0.as_str())
                    .and_then(|suffix| suffix.strip_prefix('/')) == Some(candidate.1.as_str())
                && sample.samples >= 3
                && now.saturating_sub(sample.updated_at_ms) < 7 * 86400000
        })
    };
    let unhealthy = |sample: &ModelPerformance| {
        u64::from(sample.failures) * 2 >= u64::from(sample.samples)
    };
    chain.sort_by_key(|candidate| measurement(candidate).is_some_and(unhealthy));
    for failed in [false, true] {
        let measured: Vec<_> = chain
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                measurement(candidate)
                    .filter(|sample| unhealthy(sample) == failed)
                    .map(|sample| (index, sample.latency_ms))
            })
            .collect();
        let mut sorted: Vec<_> = measured.iter()
            .map(|(index, latency)| (chain[*index].clone(), *latency))
            .collect();
        sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
        for ((index, _), (candidate, _)) in measured.into_iter().zip(sorted) {
            chain[index] = candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_preserves_unknown_slots_and_separates_unhealthy_models() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_millis() as u64;
        let sample = |id: &str, latency_ms, failures, samples, updated_at_ms| ModelPerformance {
            task: "cleanup".into(), id: id.into(), latency_ms, failures, samples, updated_at_ms,
        };
        let samples = [
            sample("groq/slow", 500.0, 1, 3, now),
            sample("groq/fast", 100.0, 0, 3, now),
            sample("groq/failed-fast", 10.0, 2, 4, now),
            sample("groq/failed-slow", 200.0, 3, 4, now),
            sample("groq/stale", 1.0, 0, 3, now - 7 * 86400000),
            sample("groq/few", 1.0, 0, 2, now),
            sample("groq/prefix", 1.0, 3, 3, now),
        ];
        let mut chain: Vec<_> = ["slow", "unknown", "failed-slow", "fast", "stale", "few", "failed-fast"]
            .into_iter().map(|id| ("groq".into(), id.into())).collect();
        chain.push(("gro".into(), "prefix".into()));
        prioritize(&mut chain, "cleanup", &samples);
        let ids: Vec<_> = chain.iter().map(|(_, id)| id.as_str()).collect();
        assert_eq!(ids, ["fast", "unknown", "slow", "stale", "few", "prefix", "failed-fast", "failed-slow"]);
        let unchanged = chain.clone();
        prioritize(&mut chain, "transcription", &samples);
        assert_eq!(chain, unchanged);
    }

    #[test]
    fn timings_keep_only_bounded_metadata() {
        record("cleanup", "local", "timing-test", 100.0, true);
        record("cleanup", "local", "timing-test", 200.0, false);
        let sample = snapshot()
            .into_iter()
            .find(|s| s.id == "local/timing-test")
            .unwrap();
        assert_eq!(sample.samples, 2);
        assert_eq!(sample.failures, 1);
        assert_eq!(sample.latency_ms, 125.0);
    }
}
