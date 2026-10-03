//! Bounded, best-effort persistence of already-redacted session logs.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use fs2::FileExt;

const FLUSH_INTERVAL: Duration = Duration::from_millis(250);
const SPACE_INTERVAL: Duration = Duration::from_secs(5);
const RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MIN_FREE: u64 = 256 * 1024 * 1024;
const RESUME_FREE: u64 = 512 * 1024 * 1024;
const PART_BYTES: u64 = 8 * 1024 * 1024;
const TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 16 * 1024;

enum Message {
    Line(String),
    Flush(SyncSender<()>),
}

struct Sink {
    tx: SyncSender<Message>,
    dropped: Arc<AtomicUsize>,
}

static SINK: OnceLock<Sink> = OnceLock::new();

pub(super) fn init(directory: PathBuf) {
    SINK.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel(1024);
        let dropped = Arc::new(AtomicUsize::new(0));
        let count = dropped.clone();
        // Filesystem work never runs on the dictation or native hotkey thread.
        let _ = std::thread::Builder::new()
            .name("verenu-log-writer".into())
            .spawn(move || run(directory, rx, count));
        Sink { tx, dropped }
    });
}

pub(super) fn append(line: &str) {
    if let Some(sink) = SINK.get() {
        let end = line.floor_char_boundary(line.len().min(MAX_LINE_BYTES));
        let mut bounded = line[..end].to_owned();
        if end < line.len() {
            bounded.push_str(" [truncated]");
        }
        if sink.tx.try_send(Message::Line(bounded)).is_err() {
            sink.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(super) fn flush() {
    if let Some(sink) = SINK.get() {
        let (tx, rx) = mpsc::sync_channel(1);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut message = Message::Flush(tx);
        loop {
            match sink.tx.try_send(message) {
                Ok(()) => {
                    let _ = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
                    break;
                }
                Err(mpsc::TrySendError::Full(returned)) if Instant::now() < deadline => {
                    message = returned;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    }
}

struct Writer {
    directory: PathBuf,
    session: String,
    part: u32,
    file: Option<File>,
    written: u64,
    paused: bool,
    space_checked: Option<Instant>,
    cleaned: Instant,
}

impl Writer {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            session: format!(
                "session-{}-{}",
                chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
                uuid::Uuid::new_v4()
            ),
            part: 0,
            file: None,
            written: 0,
            paused: false,
            space_checked: None,
            cleaned: Instant::now(),
        }
    }

    fn write(&mut self, bytes: &[u8], free: u64) -> io::Result<bool> {
        self.paused = if self.paused {
            free < RESUME_FREE
        } else {
            free < MIN_FREE
        };
        if self.paused {
            return Ok(false);
        }
        if self.file.is_none() || self.written + bytes.len() as u64 > PART_BYTES {
            self.file = None;
            // Reserve one part so the directory stays bounded as this file grows.
            prune(&self.directory, SystemTime::now(), TOTAL_BYTES - PART_BYTES)?;
            self.cleaned = Instant::now();
            let path = self
                .directory
                .join(format!("{}-{:04}.log", self.session, self.part));
            self.part += 1;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options.open(path)?;
            FileExt::try_lock_exclusive(&file)?;
            self.file = Some(file);
            self.written = 0;
        }
        let file = self.file.as_mut().unwrap();
        file.write_all(bytes)?;
        file.flush()?;
        self.written += bytes.len() as u64;
        Ok(true)
    }
}

fn run(directory: PathBuf, rx: Receiver<Message>, dropped: Arc<AtomicUsize>) {
    let mut writer = Writer::new(directory);
    let mut batch = Vec::with_capacity(64 * 1024);
    let mut free = 0;
    let mut retry_after = Instant::now();
    let mut directory_ready = false;
    let mut deadline = Instant::now() + FLUSH_INTERVAL;
    loop {
        let message = if batch.is_empty() {
            rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        };
        let mut ack = None;
        match message {
            Ok(Message::Line(line)) => {
                if batch.is_empty() {
                    deadline = Instant::now() + FLUSH_INTERVAL;
                }
                batch.extend_from_slice(line.as_bytes());
                batch.push(b'\n');
                if batch.len() < 64 * 1024 && Instant::now() < deadline {
                    continue;
                }
            }
            Ok(Message::Flush(tx)) => ack = Some(tx),
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !batch.is_empty() && Instant::now() >= retry_after {
            let records = batch.iter().filter(|&&b| b == b'\n').count();
            let mut omitted = 0;
            let result = (|| -> io::Result<bool> {
                if !directory_ready {
                    fs::create_dir_all(&writer.directory)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&writer.directory, fs::Permissions::from_mode(0o700))?;
                    }
                    prune(
                        &writer.directory,
                        SystemTime::now(),
                        TOTAL_BYTES - PART_BYTES,
                    )?;
                    writer.cleaned = Instant::now();
                    directory_ready = true;
                }
                if writer
                    .space_checked
                    .is_none_or(|at| at.elapsed() >= SPACE_INTERVAL)
                {
                    // Treat failed disk queries conservatively and retry later.
                    free = fs2::available_space(&writer.directory).unwrap_or(0);
                    writer.space_checked = Some(Instant::now());
                }
                if writer.cleaned.elapsed() >= Duration::from_secs(3600) {
                    prune(
                        &writer.directory,
                        SystemTime::now(),
                        TOTAL_BYTES - PART_BYTES,
                    )?;
                    writer.cleaned = Instant::now();
                }
                omitted = dropped.swap(0, Ordering::Relaxed);
                if omitted > 0 {
                    batch.extend_from_slice(
                        format!(
                            "[log-writer] {omitted} records omitted during backlog or disk pause\n"
                        )
                        .as_bytes(),
                    );
                }
                writer.write(&batch, free)
            })();
            if !matches!(result, Ok(true)) {
                dropped.fetch_add(records + omitted, Ordering::Relaxed);
                // Reopen after I/O errors. Never spin, accumulate retries, or log recursively.
                if result.is_err() {
                    writer.file = None;
                    writer.space_checked = None;
                    directory_ready = false;
                    retry_after = Instant::now() + SPACE_INTERVAL;
                }
            }
            batch.clear();
        } else if !batch.is_empty() {
            dropped.fetch_add(
                batch.iter().filter(|&&b| b == b'\n').count(),
                Ordering::Relaxed,
            );
            batch.clear();
        }
        if let Some(tx) = ack {
            let _ = tx.send(());
        }
        deadline = Instant::now() + FLUSH_INTERVAL;
    }
}

fn prune(directory: &Path, now: SystemTime, budget: u64) -> io::Result<()> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("session-") || !name.ends_with(".log") || !entry.file_type()?.is_file()
        {
            continue;
        }
        let metadata = entry.metadata()?;
        files.push((metadata.modified()?, metadata.len(), entry.path()));
    }
    files.sort_by_key(|entry| entry.0);
    let mut total: u64 = files.iter().map(|entry| entry.1).sum();
    for (modified, size, path) in files {
        if now.duration_since(modified).unwrap_or_default() <= RETENTION && total <= budget {
            continue;
        }
        // Another app/dev session may still own this file. Leave it alone.
        if let Ok(file) = OpenOptions::new().write(true).open(&path) {
            if FileExt::try_lock_exclusive(&file).is_ok() {
                // Windows refuses to remove an open file. Drop the lock handle
                // before deletion so the same retention pass works everywhere.
                let unlocked = FileExt::unlock(&file).is_ok();
                drop(file);
                if unlocked && fs::remove_file(&path).is_ok() {
                    total = total.saturating_sub(size);
                }
            }
        }
    }
    if total > budget {
        return Err(io::Error::other("log budget occupied by active sessions"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("verenu-log-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn sessions_are_separate_and_parts_rotate() {
        let dir = Directory::new();
        let mut first = Writer::new(dir.0.clone());
        assert!(first.write(b"first\n", RESUME_FREE).unwrap());
        first.written = PART_BYTES;
        assert!(first.write(b"rotated\n", RESUME_FREE).unwrap());
        let mut second = Writer::new(dir.0.clone());
        assert!(second.write(b"second\n", RESUME_FREE).unwrap());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 3);
        let logs: String = fs::read_dir(&dir.0)
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        assert!(
            logs.contains("first\n") && logs.contains("rotated\n") && logs.contains("second\n")
        );
    }

    #[test]
    fn low_space_pauses_with_hysteresis_and_resumes() {
        let dir = Directory::new();
        let mut writer = Writer::new(dir.0.clone());
        assert!(!writer.write(b"discarded\n", MIN_FREE - 1).unwrap());
        assert!(!writer.write(b"discarded\n", MIN_FREE + 1).unwrap());
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
        assert!(writer.write(b"recovered\n", RESUME_FREE).unwrap());
        assert!(!writer.write(b"discarded\n", 0).unwrap());
        assert!(writer.write(b"resumed\n", RESUME_FREE).unwrap());
        assert_eq!(
            fs::read_to_string(
                fs::read_dir(&dir.0)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path()
            )
            .unwrap(),
            "recovered\nresumed\n"
        );
    }

    #[test]
    fn retention_and_budget_preserve_unrelated_and_active_files() {
        let dir = Directory::new();
        fs::write(dir.0.join("session-old.log"), b"old").unwrap();
        fs::write(dir.0.join("notes.txt"), b"keep").unwrap();
        let active = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.0.join("session-active.log"))
            .unwrap();
        FileExt::try_lock_exclusive(&active).unwrap();
        active.set_len(10).unwrap();
        prune(
            &dir.0,
            SystemTime::now() + RETENTION + Duration::from_secs(1),
            20,
        )
        .unwrap();
        assert!(!dir.0.join("session-old.log").exists());
        assert!(dir.0.join("notes.txt").exists());
        assert!(dir.0.join("session-active.log").exists());
        assert!(prune(&dir.0, SystemTime::now(), 0).is_err());
        drop(active);
        prune(&dir.0, SystemTime::now(), 0).unwrap();
        assert!(!dir.0.join("session-active.log").exists());
    }

    #[test]
    fn worker_writes_promptly_and_flush_acknowledges_disk_write() {
        let dir = Directory::new();
        let (tx, rx) = mpsc::sync_channel(8);
        let path = dir.0.clone();
        let worker = std::thread::spawn(move || run(path, rx, Arc::new(AtomicUsize::new(0))));
        tx.send(Message::Line("safe metadata".into())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let path = loop {
            if let Some(entry) = fs::read_dir(&dir.0).unwrap().next() {
                let path = entry.unwrap().path();
                if fs::read_to_string(&path).unwrap() == "safe metadata\n" {
                    break path;
                }
            }
            assert!(Instant::now() < deadline, "batch was not written promptly");
            std::thread::sleep(Duration::from_millis(10));
        };
        tx.send(Message::Line("shutdown metadata".into())).unwrap();
        let (ack, done) = mpsc::sync_channel(1);
        tx.send(Message::Flush(ack)).unwrap();
        done.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "safe metadata\nshutdown metadata\n"
        );
        drop(tx);
        worker.join().unwrap();
    }
}
