use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdin, ChildStdout},
    sync::{mpsc, oneshot, Mutex},
};

const BLOCK_SAMPLES: usize = 3200;
const QUEUE_BLOCKS: usize = 180;
const IO_TIMEOUT: Duration = Duration::from_secs(30);

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl Worker {
    fn start(dir: &std::path::Path) -> anyhow::Result<Self> {
        use std::process::Stdio;
        let mut child = tokio::process::Command::new(dir.join(".venv/bin/python"))
            .arg(dir.join("realtime_worker.py"))
            .env("VERENU_REALTIME_MODEL_DIR", dir.join("realtime-model"))
            .env("HF_HUB_OFFLINE", "1")
            .env("HF_HOME", dir.join("hf-cache"))
            .env("TORCH_HOME", dir.join("torch-cache"))
            .env("TRITON_CACHE_DIR", dir.join("triton-cache"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("worker stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("worker stdout unavailable"))?;
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    async fn exchange(
        &mut self,
        code: u32,
        samples: &[f32],
        max_output: usize,
    ) -> anyhow::Result<Vec<f32>> {
        let operation = async {
            self.stdin.write_all(&code.to_le_bytes()).await?;
            let mut bytes = Vec::with_capacity(samples.len() * 4);
            for sample in samples {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            self.stdin.write_all(&bytes).await?;
            self.stdin.flush().await?;
            let count = self.stdout.read_u32_le().await? as usize;
            anyhow::ensure!(count <= max_output, "real-time worker output too large");
            let mut output = vec![0; count * 4];
            self.stdout.read_exact(&mut output).await?;
            let decoded: Vec<f32> = output
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
                .map(f32::from_le_bytes)
                .collect();
            anyhow::ensure!(
                decoded.iter().all(|sample| sample.is_finite()),
                "invalid real-time output"
            );
            Ok::<_, anyhow::Error>(decoded)
        };
        tokio::time::timeout(IO_TIMEOUT, operation).await?
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[derive(Default)]
struct Pool {
    worker: Option<Worker>,
    last_used: Option<Instant>,
}

fn pool() -> &'static Mutex<Pool> {
    static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(Pool::default()))
}

pub fn evict_if_idle(policy: &str, pressure: bool) {
    let Ok(mut pool) = pool().try_lock() else {
        return;
    };
    let limit = match policy {
        "unload_immediately" => Some(Duration::ZERO),
        "unload_after_15m" => Some(Duration::from_secs(900)),
        "keep_loaded" => None,
        _ => Some(Duration::from_secs(300)),
    };
    if should_evict(limit, pool.last_used.map(|last| last.elapsed()), pressure) {
        pool.worker = None;
        pool.last_used = None;
    }
}

fn should_evict(limit: Option<Duration>, idle: Option<Duration>, pressure: bool) -> bool {
    pressure || limit.zip(idle).is_some_and(|(limit, idle)| idle >= limit)
}

pub fn is_loaded() -> bool {
    pool()
        .try_lock()
        .map(|pool| pool.worker.is_some())
        .unwrap_or(true)
}

pub async fn unload() {
    let mut pool = pool().lock().await;
    pool.worker = None;
    pool.last_used = None;
}

enum Message {
    Samples(Vec<f32>),
    Finish,
}

#[derive(Clone)]
pub struct AudioSender {
    tx: mpsc::Sender<Message>,
    failed: Arc<AtomicBool>,
    pending: Arc<std::sync::Mutex<Vec<f32>>>,
}

impl AudioSender {
    pub fn send(&self, samples: &[f32]) {
        if self.failed.load(Ordering::Relaxed) {
            return;
        }
        let Ok(mut pending) = self.pending.lock() else {
            self.failed.store(true, Ordering::Relaxed);
            return;
        };
        pending.extend_from_slice(samples);
        while pending.len() >= BLOCK_SAMPLES {
            let rest = pending.split_off(BLOCK_SAMPLES);
            let block = std::mem::replace(&mut *pending, rest);
            if self.tx.try_send(Message::Samples(block)).is_err() {
                self.failed.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
}

pub struct Recording {
    sender: AudioSender,
    result: oneshot::Receiver<anyhow::Result<Vec<f32>>>,
}

impl Recording {
    pub fn sender(&self) -> AudioSender {
        self.sender.clone()
    }

    pub async fn finish(self, expected: usize) -> anyhow::Result<Vec<f32>> {
        anyhow::ensure!(
            !self.sender.failed.load(Ordering::Relaxed),
            "real-time queue overflow"
        );
        let pending = std::mem::take(
            &mut *self
                .sender
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("real-time queue poisoned"))?,
        );
        if !pending.is_empty() {
            self.sender.tx.send(Message::Samples(pending)).await?;
        }
        self.sender.tx.send(Message::Finish).await?;
        let output = tokio::time::timeout(IO_TIMEOUT, self.result).await???;
        anyhow::ensure!(output.len() == expected, "real-time output length mismatch");
        Ok(output)
    }
}

pub fn start(
    dir: PathBuf,
    immediate: bool,
    runtime_use: super::nvidia_reuse::RuntimeUse,
) -> Recording {
    let (tx, mut rx) = mpsc::channel(QUEUE_BLOCKS);
    let (mut result_tx, result) = oneshot::channel();
    let failed = Arc::new(AtomicBool::new(false));
    let task_failed = Arc::clone(&failed);
    tauri::async_runtime::spawn(async move {
        let _runtime_use = runtime_use;
        let result = async {
            let mut pool = tokio::select! {
                guard = pool().lock() => guard,
                _ = result_tx.closed() => anyhow::bail!("recording cancelled"),
            };
            if pool.worker.is_none() {
                pool.worker = Some(Worker::start(&dir)?);
            }
            let worker = pool.worker.as_mut().expect("worker initialized");
            let outcome = async {
                anyhow::ensure!(
                    worker.exchange(0, &[], 0).await?.is_empty(),
                    "invalid reset response"
                );
                let mut output = Vec::new();
                loop {
                    anyhow::ensure!(!task_failed.load(Ordering::Relaxed), "real-time queue overflow");
                    match rx.recv().await {
                        Some(Message::Samples(samples)) => {
                            output.extend(
                                worker
                                    .exchange(samples.len() as u32, &samples, BLOCK_SAMPLES + 640)
                                    .await?,
                            );
                        }
                        Some(Message::Finish) => {
                            anyhow::ensure!(
                                !task_failed.load(Ordering::Relaxed),
                                "real-time queue overflow"
                            );
                            output.extend(worker.exchange(u32::MAX, &[], 1920).await?);
                            break Ok(output);
                        }
                        None => break Err(anyhow::anyhow!("recording cancelled")),
                    }
                }
            };
            let outcome = tokio::select! {
                value = outcome => value,
                _ = result_tx.closed() => Err(anyhow::anyhow!("recording cancelled")),
            };
            if outcome.is_err() || immediate {
                pool.worker = None;
            }
            pool.last_used = Some(Instant::now());
            outcome
        }
        .await;
        let _ = result_tx.send(result);
    });
    Recording {
        sender: AudioSender {
            tx,
            failed,
            pending: Arc::new(std::sync::Mutex::new(Vec::new())),
        },
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_uses_idle_policy_or_actual_pressure() {
        let five = Some(Duration::from_secs(300));
        assert!(!should_evict(five, Some(Duration::from_secs(299)), false));
        assert!(should_evict(five, Some(Duration::from_secs(300)), false));
        assert!(!should_evict(None, Some(Duration::from_secs(900)), false));
        assert!(should_evict(None, Some(Duration::ZERO), true));
        assert!(!should_evict(five, None, false));
    }

    #[tokio::test]
    async fn pressure_does_not_evict_a_leased_worker() {
        let mut lease = pool().lock().await;
        lease.last_used = Some(Instant::now());
        evict_if_idle("unload_immediately", true);
        assert!(lease.last_used.is_some());
        drop(lease);
        evict_if_idle("unload_immediately", true);
        assert!(pool().lock().await.last_used.is_none());
    }

    #[test]
    fn capture_batches_without_blocking_and_marks_overflow() {
        let (tx, mut rx) = mpsc::channel(1);
        let failed = Arc::new(AtomicBool::new(false));
        let sender = AudioSender {
            tx,
            failed: Arc::clone(&failed),
            pending: Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        sender.send(&vec![0.25; BLOCK_SAMPLES - 1]);
        assert!(rx.try_recv().is_err());
        sender.send(&[0.5]);
        match rx.try_recv().unwrap() {
            Message::Samples(block) => {
                assert_eq!(block.len(), BLOCK_SAMPLES);
                assert_eq!(block[BLOCK_SAMPLES - 1], 0.5);
            }
            Message::Finish => panic!("unexpected finish"),
        }
        sender.send(&vec![0.25; BLOCK_SAMPLES * 2]);
        assert!(failed.load(Ordering::Relaxed));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn worker_protocol_checks_output_and_resets() {
        use std::os::unix::fs::symlink;
        let dir =
            std::env::temp_dir().join(format!("verenu-realtime-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join(".venv/bin")).unwrap();
        symlink("/usr/bin/python3", dir.join(".venv/bin/python")).unwrap();
        std::fs::write(
            dir.join("realtime_worker.py"),
            r#"
import struct, sys
while True:
    header = sys.stdin.buffer.read(4)
    if not header: break
    count = struct.unpack('<I', header)[0]
    if count in (0, 0xffffffff):
        sys.stdout.buffer.write(struct.pack('<I', 0))
    else:
        data = sys.stdin.buffer.read(count * 4)
        sys.stdout.buffer.write(struct.pack('<I', count) + data)
    sys.stdout.buffer.flush()
"#,
        )
        .unwrap();
        let mut worker = Worker::start(&dir).unwrap();
        assert!(worker.exchange(0, &[], 0).await.unwrap().is_empty());
        assert_eq!(
            worker.exchange(2, &[0.25, -0.5], 2).await.unwrap(),
            vec![0.25, -0.5]
        );
        assert!(worker.exchange(u32::MAX, &[], 0).await.unwrap().is_empty());
        drop(worker);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
