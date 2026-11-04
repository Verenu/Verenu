use std::future::Future;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub(crate) struct Usage {
    pub stage: String,
    pub provider: String,
    pub model: String,
    pub input: Option<u64>,
    pub output: Option<u64>,
}

pub(crate) type Collector = Arc<Mutex<Vec<Usage>>>;

tokio::task_local! {
    static CURRENT: (Collector, String, String, String);
}

pub(crate) async fn scope<F: Future>(collector: Option<Collector>, stage: &str, provider: &str, model: &str, future: F) -> F::Output {
    match collector {
        Some(collector) => CURRENT.scope((collector, stage.to_owned(), provider.to_owned(), model.to_owned()), future).await,
        None => future.await,
    }
}

pub(crate) fn record(input: Option<u64>, output: Option<u64>) {
    if input.is_none() && output.is_none() { return; }
    let _ = CURRENT.try_with(|(collector, stage, provider, model)| {
        if let Ok(mut entries) = collector.lock() {
            entries.push(Usage { stage: stage.clone(), provider: provider.clone(), model: model.clone(), input, output });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn concurrent_requests_keep_usage_separate() {
        let a: Collector = Default::default();
        let b: Collector = Default::default();
        tokio::join!(
            scope(Some(a.clone()), "cleanup", "first", "model-a", async { tokio::task::yield_now().await; record(Some(12), Some(3)); }),
            scope(Some(b.clone()), "transcription", "second", "model-b", async { record(Some(7), None); tokio::task::yield_now().await; }),
        );
        let a = a.lock().unwrap();
        let b = b.lock().unwrap();
        assert_eq!((a[0].provider.as_str(), a[0].input, a[0].output), ("first", Some(12), Some(3)));
        assert_eq!((b[0].provider.as_str(), b[0].input, b[0].output), ("second", Some(7), None));
    }
}
