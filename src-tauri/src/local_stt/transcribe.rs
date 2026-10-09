use super::manager::LocalTranscriptionManager;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

pub const LOCAL_TRANSCRIPTION_TIMEOUT: Duration = Duration::from_secs(120);

struct CancelOnDrop(Arc<std::sync::atomic::AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn transcribe(
    manager: LocalTranscriptionManager,
    app: AppHandle,
    model_id: String,
    samples: Arc<Vec<f32>>,
    sample_rate: u32,
    language: String,
    vocabulary: super::vocabulary::Vocabulary,
) -> anyhow::Result<String> {
    let permit = if let Some(permit) = manager.transcription_permit() {
        permit
    } else {
        anyhow::bail!("local transcription is still stopping; try again shortly");
    };
    let manager_for_task = manager.clone();
    let cancellation = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _cancel_on_drop = CancelOnDrop(Arc::clone(&cancellation));
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        manager_for_task.transcribe_blocking(
            &app,
            &model_id,
            &samples,
            sample_rate,
            &language,
            &vocabulary,
            &cancellation,
        )
    });
    match tokio::time::timeout(LOCAL_TRANSCRIPTION_TIMEOUT, task).await {
        Ok(result) => result?,
        Err(_) => anyhow::bail!(
            "local transcription timed out after {} seconds; the native inference is still winding down",
            LOCAL_TRANSCRIPTION_TIMEOUT.as_secs()
        ),
    }
}
