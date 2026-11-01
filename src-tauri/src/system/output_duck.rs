//! Linux output-volume primitives. Levels are kept per channel so that fading
//! never changes the user's channel balance or the separate mute property.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static RECOVERY_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct SinkLevel {
    pub name: String,
    pub channels: Vec<u32>,
    pub muted: bool,
}

fn run_pactl(args: &[&str]) -> Result<Vec<u8>> {
    let mut child = Command::new("pactl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("pactl is unavailable")?;
    let mut stdout = child.stdout.take().context("missing pactl output")?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).map(|_| output)
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let output = reader
        .join()
        .map_err(|_| anyhow::anyhow!("pactl reader failed"))??;
    if !status.context("pactl timed out")?.success() {
        bail!("pactl failed");
    }
    Ok(output)
}

fn sink_level(value: &Value, name: &str) -> Option<SinkLevel> {
    let sink = value
        .as_array()?
        .iter()
        .find(|sink| sink.get("name").and_then(Value::as_str) == Some(name))?;
    let order = sink.get("channel_map")?.as_str()?;
    let volume = sink.get("volume")?;
    let channels = order
        .split(',')
        .map(|channel| {
            volume
                .get(channel.trim())?
                .get("value")?
                .as_u64()?
                .try_into()
                .ok()
        })
        .collect::<Option<Vec<u32>>>()?;
    let muted = sink.get("mute")?.as_bool()?;
    (!channels.is_empty()).then(|| SinkLevel {
        name: name.to_owned(),
        channels,
        muted,
    })
}

pub(super) fn default_sink_name() -> Result<String> {
    let output = run_pactl(&["get-default-sink"])?;
    let name = std::str::from_utf8(&output)?.trim();
    if name.is_empty() {
        bail!("no default output");
    }
    Ok(name.to_owned())
}

pub(super) fn read_sink(name: &str) -> Result<SinkLevel> {
    let output = run_pactl(&["--format=json", "list", "sinks"])?;
    let sinks = serde_json::from_slice(&output)?;
    sink_level(&sinks, name).context("output device is unavailable")
}

pub(super) fn write_sink(level: &SinkLevel) -> Result<()> {
    let channels: Vec<String> = level.channels.iter().map(u32::to_string).collect();
    let mut args = vec!["set-sink-volume", level.name.as_str()];
    args.extend(channels.iter().map(String::as_str));
    run_pactl(&args)?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingRestore {
    original: SinkLevel,
    before: Vec<u32>,
    after: Vec<u32>,
}

/// The next write is journaled before it reaches the audio server. Both the
/// old and new values are recoverable if the guardian dies during the write.
fn write_journal(path: &Path, entries: &[PendingRestore]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, entries)?;
    file.flush()?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)?;
    if let Some(directory) = path.parent() {
        std::fs::File::open(directory)?.sync_all()?;
    }
    Ok(())
}

fn state_directory() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .context("no state directory for output recovery")?;
    let directory = base.join("verenu/output-duck");
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

fn belongs_to_guardian(entry: &PendingRestore, current: &[u32]) -> bool {
    current == entry.before || current == entry.after
}

fn recover_entry(entry: &PendingRestore) -> Result<bool> {
    let current = read_sink(&entry.original.name)?;
    if belongs_to_guardian(entry, &current.channels) {
        let mut restore = entry.original.clone();
        restore.muted = current.muted;
        write_sink(&restore)?;
        Ok(read_sink(&entry.original.name)?.channels == entry.original.channels)
    } else {
        // A volume change made after our last write belongs to the user.
        Ok(true)
    }
}

fn recover_journal(path: &Path, entries: &mut Vec<PendingRestore>) -> Result<()> {
    entries.retain(|entry| !recover_entry(entry).unwrap_or(false));
    write_journal(path, entries)
}

fn recover_inactive(
    path: &Path,
    entries: &mut Vec<PendingRestore>,
    active_name: Option<&str>,
) -> Result<()> {
    let previous_count = entries.len();
    entries.retain(|entry| {
        Some(entry.original.name.as_str()) == active_name || !recover_entry(entry).unwrap_or(false)
    });
    if entries.len() != previous_count {
        write_journal(path, entries)?;
    }
    Ok(())
}

fn interpolate(original: &[u32], target: &[u32], progress: f64, smooth: bool) -> Vec<u32> {
    let p = progress.clamp(0.0, 1.0);
    let p = if smooth { p * p * (3.0 - 2.0 * p) } else { p };
    original
        .iter()
        .zip(target)
        .map(|(&from, &to)| (from as f64 + (to as f64 - from as f64) * p).round() as u32)
        .collect()
}

struct ActiveFade {
    original: SinkLevel,
    last_written: Vec<u32>,
    from: Vec<u32>,
    target: Vec<u32>,
    started: Instant,
    duration: Duration,
}

impl ActiveFade {
    fn new(original: SinkLevel, fraction: f64, duration: Duration) -> Self {
        // PulseAudio's normal (100%) volume is 65536. Never raise a quiet sink.
        let maximum = original.channels.iter().copied().max().unwrap_or(0);
        let scale = if maximum == 0 {
            1.0
        } else {
            (65536.0 * fraction / f64::from(maximum)).min(1.0)
        };
        let target = original
            .channels
            .iter()
            .map(|&v| (v as f64 * scale).round() as u32)
            .collect();
        Self {
            last_written: original.channels.clone(),
            from: original.channels.clone(),
            original,
            target,
            started: Instant::now(),
            duration,
        }
    }

    fn restore(&mut self, duration: Duration) {
        self.from.clone_from(&self.last_written);
        self.target.clone_from(&self.original.channels);
        self.started = Instant::now();
        self.duration = duration;
    }

    fn finished(&self) -> bool {
        self.duration.is_zero() || self.started.elapsed() >= self.duration
    }

    fn next(&self, smooth: bool) -> Vec<u32> {
        let progress = if self.duration.is_zero() {
            1.0
        } else {
            self.started.elapsed().as_secs_f64() / self.duration.as_secs_f64()
        };
        interpolate(&self.from, &self.target, progress, smooth)
    }
}

/// Write only while the device still holds our last value. The caller owns the
/// journal lock and must keep the record if the device becomes unavailable.
fn fade_step(
    active: &mut ActiveFade,
    journal: &Path,
    entries: &mut Vec<PendingRestore>,
    smooth: bool,
) -> Result<bool> {
    let current = read_sink(&active.original.name)?;
    if current.channels != active.last_written {
        entries.retain(|entry| entry.original.name != active.original.name);
        write_journal(journal, entries)?;
        return Ok(false);
    }
    let next = active.next(smooth);
    if next != active.last_written {
        entries.retain(|entry| entry.original.name != active.original.name);
        entries.push(PendingRestore {
            original: active.original.clone(),
            before: active.last_written.clone(),
            after: next.clone(),
        });
        write_journal(journal, entries)?;
        let mut desired = current;
        desired.channels = next.clone();
        write_sink(&desired)?;
        active.last_written = next;
    }
    Ok(true)
}

fn restore_active(
    active: &mut ActiveFade,
    journal: &Path,
    entries: &mut Vec<PendingRestore>,
    duration: Duration,
    smooth: bool,
) -> Result<()> {
    active.restore(duration);
    loop {
        if journal.with_file_name("interrupt-restore").exists() {
            active.restore(Duration::ZERO);
        }
        if !fade_step(active, journal, entries, smooth)? {
            return Ok(());
        }
        if active.last_written == active.target {
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    entries.retain(|entry| entry.original.name != active.original.name);
    write_journal(journal, entries)
}

#[derive(Clone, Copy)]
pub(crate) struct DuckSettings {
    pub target: u8,
    pub down: Duration,
    pub up: Duration,
    pub smooth_down: bool,
    pub smooth_up: bool,
}

pub(crate) struct DuckGuard(Option<std::process::ChildStdin>);

impl DuckGuard {
    pub(crate) fn start(settings: DuckSettings) -> Result<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .arg("--verenu-output-guardian")
            .arg(settings.target.to_string())
            .arg(settings.down.as_millis().to_string())
            .arg(settings.up.as_millis().to_string())
            .arg(if settings.smooth_down {
                "smooth"
            } else {
                "linear"
            })
            .arg(if settings.smooth_up {
                "smooth"
            } else {
                "linear"
            })
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("could not start output guardian")?;
        let input = child.stdin.take().context("missing output guardian pipe")?;
        std::thread::spawn(move || {
            let _ = child.wait();
            start_recovery();
        });
        Ok(Self(Some(input)))
    }
}

impl Drop for DuckGuard {
    fn drop(&mut self) {
        drop(self.0.take());
    }
}

pub(crate) fn start_recovery() {
    let Ok(directory) = state_directory() else {
        return;
    };
    if !directory.join("journal.json").exists() {
        return;
    }
    if RECOVERY_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(move || {
        loop {
            if let Ok(mut child) = std::env::current_exe().and_then(|exe| {
                Command::new(exe)
                    .arg("--verenu-output-recover")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            }) {
                let _ = child.wait();
            }
            let pending = std::fs::read(directory.join("journal.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Vec<PendingRestore>>(&bytes).ok())
                .is_some_and(|entries| !entries.is_empty());
            if !pending {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        RECOVERY_RUNNING.store(false, Ordering::Release);
    });
}

fn recover_stale() -> Result<()> {
    use fs2::FileExt;
    let directory = state_directory()?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("lock"))?;
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
    }
    let journal = directory.join("journal.json");
    let bytes = match std::fs::read(&journal) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let mut entries = serde_json::from_slice(&bytes).context("invalid output recovery journal")?;
    recover_journal(&journal, &mut entries)
}

pub(crate) fn internal_entry() -> bool {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("--verenu-output-recover") {
        let _ = recover_stale();
        return true;
    }
    if command.as_deref() != Some("--verenu-output-guardian") {
        return false;
    }
    let parsed = (|| -> Option<DuckSettings> {
        let target: u8 = args.next()?.parse().ok()?;
        let down: u64 = args.next()?.parse().ok()?;
        let up: u64 = args.next()?.parse().ok()?;
        let down_curve = args.next()?;
        let up_curve = args.next()?;
        if target > 100
            || down > 5000
            || up > 5000
            || args.next().is_some()
            || !["linear", "smooth"].contains(&down_curve.as_str())
            || !["linear", "smooth"].contains(&up_curve.as_str())
        {
            return None;
        }
        Some(DuckSettings {
            target,
            down: Duration::from_millis(down),
            up: Duration::from_millis(up),
            smooth_down: down_curve == "smooth",
            smooth_up: up_curve == "smooth",
        })
    })();
    if let Some(settings) = parsed {
        let _ = run_guardian(settings);
    }
    true
}

fn run_guardian(settings: DuckSettings) -> Result<()> {
    use fs2::FileExt;
    let directory = state_directory()?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("lock"))?;
    let (stop_tx, stop_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        let _ = stop_tx.send(());
    });
    while lock.try_lock_exclusive().is_err() {
        // Wake an earlier guardian from a long fade-up so this recording can
        // take ownership promptly after the original level is restored.
        let _ = std::fs::write(directory.join("interrupt-restore"), b"");
        if stop_rx.recv_timeout(Duration::from_millis(40)).is_ok() {
            return Ok(());
        }
    }
    let _ = std::fs::remove_file(directory.join("interrupt-restore"));
    let journal = directory.join("journal.json");
    let mut entries: Vec<PendingRestore> = match std::fs::read(&journal) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("invalid output recovery journal")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    recover_journal(&journal, &mut entries)?;

    let mut active: Option<ActiveFade> = None;
    let mut skipped: Option<String> = None;
    let mut last_recovery = Instant::now();
    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        if last_recovery.elapsed() >= Duration::from_secs(2) {
            recover_inactive(
                &journal,
                &mut entries,
                active.as_ref().map(|fade| fade.original.name.as_str()),
            )?;
            last_recovery = Instant::now();
        }
        let name = match default_sink_name() {
            Ok(name) => name,
            Err(_) => {
                if let Some(mut old) = active.take() {
                    let _ = restore_active(
                        &mut old,
                        &journal,
                        &mut entries,
                        Duration::ZERO,
                        settings.smooth_up,
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        if active
            .as_ref()
            .is_some_and(|fade| fade.original.name != name)
        {
            if let Some(mut old) = active.take() {
                let _ = restore_active(
                    &mut old,
                    &journal,
                    &mut entries,
                    Duration::ZERO,
                    settings.smooth_up,
                );
            }
            skipped = None;
        }
        if active.is_none()
            && skipped.as_deref() != Some(name.as_str())
            && !entries.iter().any(|entry| entry.original.name == name)
        {
            if let Ok(original) = read_sink(&name) {
                active = Some(ActiveFade::new(
                    original,
                    f64::from(settings.target) / 100.0,
                    settings.down,
                ));
            }
        }
        if let Some(fade) = active.as_mut() {
            match fade_step(fade, &journal, &mut entries, settings.smooth_down) {
                Ok(true) => {}
                Ok(false) => {
                    skipped = Some(name);
                    active = None;
                }
                Err(_) => {
                    // The output may have disappeared before the default
                    // changes. Keep its journal entry and watch for a new one.
                    active = None;
                }
            }
        }
        let poll = if active.as_ref().is_some_and(ActiveFade::finished) {
            200
        } else {
            40
        };
        if stop_rx.recv_timeout(Duration::from_millis(poll)).is_ok() {
            break;
        }
    }
    if let Some(mut fade) = active {
        let _ = restore_active(
            &mut fade,
            &journal,
            &mut entries,
            settings.up,
            settings.smooth_up,
        );
    }
    recover_journal(&journal, &mut entries)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_the_device_channel_order() {
        let sinks = json!([{ "name": "speakers", "channel_map": "front-right,front-left", "mute": false,
            "volume": { "front-left": {"value": 40000}, "front-right": {"value": 30000} } }]);
        assert_eq!(
            sink_level(&sinks, "speakers"),
            Some(SinkLevel {
                name: "speakers".into(),
                channels: vec![30000, 40000],
                muted: false,
            })
        );
        assert_eq!(sink_level(&sinks, "missing"), None);
    }

    #[test]
    fn crash_recovery_accepts_both_sides_of_an_interrupted_write() {
        let entry = PendingRestore {
            original: SinkLevel {
                name: "speaker".into(),
                channels: vec![60000, 30000],
                muted: false,
            },
            before: vec![50000, 25000],
            after: vec![40000, 20000],
        };
        assert!(belongs_to_guardian(&entry, &[50000, 25000]));
        assert!(belongs_to_guardian(&entry, &[40000, 20000]));
        assert!(!belongs_to_guardian(&entry, &[45000, 22500]));
    }

    #[test]
    fn channel_fades_preserve_balance_and_exact_endpoints() {
        assert_eq!(
            interpolate(&[60000, 30000], &[15000, 7500], 0.0, true),
            vec![60000, 30000]
        );
        assert_eq!(
            interpolate(&[60000, 30000], &[15000, 7500], 1.0, true),
            vec![15000, 7500]
        );
        assert_eq!(
            interpolate(&[60000, 30000], &[15000, 7500], 0.5, true),
            vec![37500, 18750]
        );
    }

    #[test]
    fn target_is_an_absolute_ceiling_and_preserves_balance() {
        let sink = |channels| SinkLevel {
            name: "speaker".into(),
            channels,
            muted: false,
        };
        let fade = ActiveFade::new(sink(vec![26214, 13107]), 0.10, Duration::ZERO);
        assert_eq!(fade.target, vec![6554, 3277]);
        let quiet = ActiveFade::new(sink(vec![3277, 1638]), 0.10, Duration::ZERO);
        assert_eq!(quiet.target, vec![3277, 1638]);
        let silent = ActiveFade::new(sink(vec![0, 0]), 0.10, Duration::ZERO);
        assert_eq!(silent.target, vec![0, 0]);
    }

    #[test]
    #[ignore = "requires a disposable PipeWire output"]
    fn private_sink_round_trip_preserves_mute_and_channel_balance() {
        struct Restore(SinkLevel);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = write_sink(&self.0);
            }
        }
        let name = default_sink_name().unwrap();
        let original = read_sink(&name).unwrap();
        let _restore = Restore(original.clone());
        let reduced = SinkLevel {
            name: name.clone(),
            channels: original.channels.iter().map(|v| v / 2).collect(),
            muted: original.muted,
        };
        write_sink(&reduced).unwrap();
        assert_eq!(read_sink(&name).unwrap(), reduced);
        write_sink(&original).unwrap();
        assert_eq!(read_sink(&name).unwrap(), original);
    }

    #[test]
    #[ignore = "requires a disposable PipeWire output"]
    fn private_fade_restores_the_exact_original_level() {
        let name = default_sink_name().unwrap();
        let original = read_sink(&name).unwrap();
        let path =
            std::env::temp_dir().join(format!("verenu-fade-test-{}.json", std::process::id()));
        let mut fade = ActiveFade::new(original.clone(), 0.0, Duration::ZERO);
        let mut entries = Vec::new();
        fade_step(&mut fade, &path, &mut entries, true).unwrap();
        assert_ne!(read_sink(&name).unwrap().channels, original.channels);
        restore_active(
            &mut fade,
            &path,
            &mut entries,
            Duration::from_millis(120),
            true,
        )
        .unwrap();
        assert_eq!(read_sink(&name).unwrap().channels, original.channels);
        assert!(entries.is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[ignore = "requires a disposable PipeWire output"]
    fn private_new_recording_interrupts_a_long_restore() {
        let original = read_sink(&default_sink_name().unwrap()).unwrap();
        let directory =
            std::env::temp_dir().join(format!("verenu-interrupt-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let journal = directory.join("journal.json");
        let mut fade = ActiveFade::new(original.clone(), 0.0, Duration::ZERO);
        let mut entries = Vec::new();
        fade_step(&mut fade, &journal, &mut entries, true).unwrap();
        std::fs::write(directory.join("interrupt-restore"), b"").unwrap();
        let started = Instant::now();
        restore_active(
            &mut fade,
            &journal,
            &mut entries,
            Duration::from_secs(5),
            true,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            read_sink(&original.name).unwrap().channels,
            original.channels
        );
        assert!(entries.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires a disposable PipeWire output"]
    fn private_returned_nondefault_output_is_restored() {
        let original = read_sink(&default_sink_name().unwrap()).unwrap();
        let directory =
            std::env::temp_dir().join(format!("verenu-return-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let journal = directory.join("journal.json");
        let mut fade = ActiveFade::new(original.clone(), 0.0, Duration::ZERO);
        let mut entries = Vec::new();
        fade_step(&mut fade, &journal, &mut entries, true).unwrap();
        recover_inactive(&journal, &mut entries, Some("another-output")).unwrap();
        assert_eq!(
            read_sink(&original.name).unwrap().channels,
            original.channels
        );
        assert!(entries.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "requires a disposable PipeWire output and a controlled stdin"]
    fn private_guardian_fades_until_stdin_closes() {
        let target = std::env::var("VERENU_TEST_DUCK_TARGET")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(25);
        run_guardian(DuckSettings {
            target,
            down: Duration::from_millis(120),
            up: Duration::from_millis(120),
            smooth_down: true,
            smooth_up: true,
        })
        .unwrap();
    }
}
