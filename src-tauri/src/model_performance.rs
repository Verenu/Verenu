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
                && sample.id == format!("{}/{}", candidate.0, candidate.1)
                && sample.samples >= 3
                && now.saturating_sub(sample.updated_at_ms) < 7 * 86400000
        })
    };
    let unhealthy = |candidate: &(String, String)| {
        measurement(candidate)
            .is_some_and(|sample| u64::from(sample.failures) * 2 >= u64::from(sample.samples))
    };
    chain.sort_by_key(&unhealthy);
    for failed in [false, true] {
        let measured: Vec<_> = chain
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                measurement(candidate)
                    .filter(|_| unhealthy(candidate) == failed)
                    .map(|sample| (index, candidate.clone(), sample.latency_ms))
            })
            .collect();
        let mut sorted = measured.clone();
        sorted.sort_by(|a, b| a.2.total_cmp(&b.2));
        for ((index, _, _), (_, candidate, _)) in measured.into_iter().zip(sorted) {
            chain[index] = candidate;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
