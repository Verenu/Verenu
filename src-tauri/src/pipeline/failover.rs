//! Disk-backed dictation failover: crash/reboot survival for in-progress takes.
//!
//! Audio is stored as append-only i16le mono 16 kHz PCM plus an atomic JSON
//! sidecar. The sidecar `sample_count` is published only after the matching
//! PCM bytes have been flushed, and load clamps to the shorter of the two.

use super::gates::{MIN_RECORDING_MS, MIN_RECORDING_RMS};
use super::pill::{show_cancelled_pill, show_interrupted_pill};
use super::state::{
    lock_state, CancelledCapture, CaptureOrigin, SharedState, CANCEL_RESUME_WINDOW,
};
use super::{state, CapturedAudio};
use crate::core::context::ResolvedContextIdentity;
use crate::core::window_geometry::WindowTarget;
use crate::data::store;
use crate::media::audio::{self, DurableSink, DurableSinkError, DurableSinkResult};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

const SESSION_VERSION: u32 = 1;
const TARGET_RATE: u32 = 16_000;
const LIVE_DIR: &str = "live";
const COMMITTED_DIR: &str = "committed";
const SESSION_FILE: &str = "session.json";
const AUDIO_FILE: &str = "audio.pcm";
const TTL_SECS: i64 = 24 * 60 * 60;
const MAX_RECOVERY_SAMPLES: u64 = audio::MAX_RECORDING_SAMPLES as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailoverKind {
    Recording,
    Cancelled,
    Processing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMeta {
    pub version: u32,
    pub id: String,
    pub kind: FailoverKind,
    pub started_at_unix: i64,
    pub sample_rate: u32,
    pub sample_count: u64,
    pub duration_ms: u64,
    #[serde(default)]
    pub rms: f32,
    /// Stable local Context identity captured when this durable take began.
    /// Old sidecars omit it and are intentionally treated as Everywhere on
    /// recovery because their originating Context cannot be inferred safely.
    #[serde(default)]
    pub context_id: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct LoadedTake {
    pub meta: SessionMeta,
    pub samples_16k: Vec<f32>,
}

impl LoadedTake {
    fn usable_samples(&self) -> u64 {
        self.samples_16k.len() as u64
    }

    fn duration_ms(&self) -> u64 {
        if self.meta.sample_rate == 0 {
            0
        } else {
            self.usable_samples() * 1000 / u64::from(self.meta.sample_rate)
        }
    }

    fn passes_gates(&self) -> bool {
        if self.duration_ms() < MIN_RECORDING_MS {
            return false;
        }
        audio::rms_f32(&self.samples_16k) >= MIN_RECORDING_RMS
    }

    fn origin(&self) -> CaptureOrigin {
        match self.meta.kind {
            FailoverKind::Cancelled => CaptureOrigin::UserCancelled,
            FailoverKind::Recording | FailoverKind::Processing => CaptureOrigin::Interrupted,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct CancelledCapturePayload {
    pub created_at: String,
    pub kind: String,
}

pub fn failover_dir() -> PathBuf {
    crate::app_data_dir().join("dictation-failover")
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn f32_to_i16(s: f32) -> i16 {
    let v = if s.is_finite() {
        s.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    (v * i16::MAX as f32) as i16
}

fn i16_to_f32(s: i16) -> f32 {
    s as f32 / i16::MAX as f32
}

fn slot_dir(root: &Path, live: bool) -> PathBuf {
    root.join(if live { LIVE_DIR } else { COMMITTED_DIR })
}

/// Keep a durable live prefix from being overwritten by the next fresh take.
/// The live slot is intentionally left in place while the failure is surfaced;
/// move it to the committed slot only when a subsequent recording needs the
/// live slot. This avoids rewriting PCM and preserves crash recovery even when
/// the original failure was caused by a slow/full disk.
fn preserve_previous_live(root: &Path, current_id: &str) {
    let live = slot_dir(root, true);
    let Some(meta) = load_session(&live.join(SESSION_FILE)) else {
        return;
    };
    if meta.id == current_id {
        return;
    }
    delete_committed(root);
    if let Err(e) = fs::rename(&live, slot_dir(root, false)) {
        log::warn!(
            "failover: could not preserve previous live take id_prefix={}: {e}",
            id_prefix(&meta.id)
        );
    }
}

fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        // Windows: access denied (5), sharing violation (32), already exists (183).
        Err(e) if matches!(e.raw_os_error(), Some(5 | 32 | 183)) => {
            let _ = fs::remove_file(to);
            if fs::rename(from, to).is_ok() {
                return Ok(());
            }
            fs::copy(from, to)?;
            let _ = fs::remove_file(from);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn write_session_atomic(path: &Path, meta: &SessionMeta) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(meta)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    if let Err(e) = replace_file(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

fn load_session(path: &Path) -> Option<SessionMeta> {
    let raw = fs::read_to_string(path).ok()?;
    let meta: SessionMeta = serde_json::from_str(&raw).ok()?;
    if meta.version != SESSION_VERSION || meta.sample_rate != TARGET_RATE {
        return None;
    }
    if meta.id.is_empty() {
        return None;
    }
    Some(meta)
}

/// Load a slot, clamping published `sample_count` to the PCM file length.
pub fn load_slot(root: &Path, live: bool) -> Option<LoadedTake> {
    let dir = slot_dir(root, live);
    let mut meta = load_session(&dir.join(SESSION_FILE))?;
    let pcm_path = dir.join(AUDIO_FILE);
    let mut file = File::open(&pcm_path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let file_samples = file_len / 2;
    let usable = meta
        .sample_count
        .min(file_samples)
        .min(MAX_RECOVERY_SAMPLES);
    if usable == 0 {
        return None;
    }
    let mut samples = Vec::with_capacity(usable as usize);
    let mut chunk = [0u8; 64 * 1024];
    let mut pending = None;
    let mut remaining = usable * 2;
    while remaining > 0 {
        let read_len = remaining.min(chunk.len() as u64) as usize;
        let read_len = file.read(&mut chunk[..read_len]).ok()?;
        if read_len == 0 {
            break;
        }
        remaining -= read_len as u64;
        let mut bytes = &chunk[..read_len];
        if let Some(first) = pending.take() {
            let value = i16::from_le_bytes([first, bytes[0]]);
            samples.push(i16_to_f32(value));
            bytes = &bytes[1..];
        }
        let even_len = bytes.len() & !1;
        for pair in bytes[..even_len].as_chunks::<2>().0 {
            samples.push(i16_to_f32(i16::from_le_bytes([pair[0], pair[1]])));
        }
        if even_len != bytes.len() {
            pending = bytes.last().copied();
        }
    }
    if let Some(first) = pending {
        // The published sample count is authoritative; an incomplete final
        // PCM sample is intentionally discarded.
        let _ = first;
    }
    if samples.is_empty() {
        return None;
    }
    let actual_count = samples.len() as u64;
    meta.sample_count = actual_count;
    meta.duration_ms = actual_count * 1000 / u64::from(TARGET_RATE);
    Some(LoadedTake {
        meta,
        samples_16k: samples,
    })
}

struct SlotInspection {
    meta: SessionMeta,
    usable_samples: u64,
}

fn inspect_slot(root: &Path, live: bool) -> Option<SlotInspection> {
    let dir = slot_dir(root, live);
    let meta = load_session(&dir.join(SESSION_FILE))?;
    let file_samples = File::open(dir.join(AUDIO_FILE))
        .ok()?
        .metadata()
        .ok()?
        .len()
        / 2;
    let usable_samples = meta
        .sample_count
        .min(file_samples)
        .min(MAX_RECOVERY_SAMPLES);
    (usable_samples > 0).then_some(SlotInspection {
        meta,
        usable_samples,
    })
}

fn inspection_passes_gates(slot: &SlotInspection) -> bool {
    if slot.usable_samples * 1000 / u64::from(TARGET_RATE) < MIN_RECORDING_MS {
        return false;
    }
    // New files publish RMS in metadata, so startup can select a winner
    // without decoding every candidate. A zero value is retained as a
    // conservative compatibility fallback for pre-metadata files.
    slot.meta.rms == 0.0 || slot.meta.rms >= MIN_RECORDING_RMS
}

pub fn delete_slot(root: &Path, live: bool) {
    let dir = slot_dir(root, live);
    let _ = fs::remove_file(dir.join(AUDIO_FILE));
    let _ = fs::remove_file(dir.join(SESSION_FILE));
    let _ = fs::remove_file(dir.join("session.json.tmp"));
    let _ = fs::remove_file(dir.join("audio.pcm.tmp"));
    let _ = fs::remove_dir(&dir);
}

pub fn delete_live(root: &Path) {
    delete_slot(root, true);
}

pub fn delete_committed(root: &Path) {
    delete_slot(root, false);
}

pub fn delete_all(root: &Path) {
    delete_live(root);
    delete_committed(root);
    let _ = fs::remove_dir(root);
}

pub fn abandon_live() {
    delete_live(&failover_dir());
}

pub fn discard_durable() {
    delete_all(&failover_dir());
}

fn samples_to_pcm(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        out.extend_from_slice(&f32_to_i16(s).to_le_bytes());
    }
    out
}

#[cfg(test)]
fn write_slot(
    root: &Path,
    live: bool,
    id: &str,
    kind: FailoverKind,
    started_at_unix: i64,
    samples_16k: &[f32],
) -> anyhow::Result<()> {
    write_slot_with_context(root, live, id, kind, started_at_unix, None, samples_16k)
}

fn write_slot_with_context(
    root: &Path,
    live: bool,
    id: &str,
    kind: FailoverKind,
    started_at_unix: i64,
    context_id: Option<i64>,
    samples_16k: &[f32],
) -> anyhow::Result<()> {
    if samples_16k.is_empty() {
        anyhow::bail!("no samples to commit");
    }
    if samples_16k.len() as u64 > MAX_RECOVERY_SAMPLES {
        anyhow::bail!("recovery audio exceeds the recording duration cap");
    }
    let dir = slot_dir(root, live);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join("audio.pcm.tmp");
    let dest = dir.join(AUDIO_FILE);
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        for chunk in samples_16k.chunks(TARGET_RATE as usize) {
            let pcm = samples_to_pcm(chunk);
            f.write_all(&pcm)?;
        }
        f.sync_all()?;
    }
    replace_file(&tmp, &dest)?;
    let sample_count = samples_16k.len() as u64;
    let meta = SessionMeta {
        version: SESSION_VERSION,
        id: id.to_string(),
        kind,
        started_at_unix,
        sample_rate: TARGET_RATE,
        sample_count,
        duration_ms: sample_count * 1000 / u64::from(TARGET_RATE),
        rms: audio::rms_f32(samples_16k),
        context_id,
    };
    write_session_atomic(&dir.join(SESSION_FILE), &meta)?;
    Ok(())
}

#[cfg(test)]
pub fn write_committed(
    root: &Path,
    id: &str,
    kind: FailoverKind,
    started_at_unix: i64,
    samples_16k: &[f32],
) -> anyhow::Result<()> {
    write_committed_with_context(root, id, kind, started_at_unix, None, samples_16k)
}

pub fn write_committed_with_context(
    root: &Path,
    id: &str,
    kind: FailoverKind,
    started_at_unix: i64,
    context_id: Option<i64>,
    samples_16k: &[f32],
) -> anyhow::Result<()> {
    write_slot_with_context(
        root,
        false,
        id,
        kind,
        started_at_unix,
        context_id,
        samples_16k,
    )?;
    delete_live(root);
    log::info!(
        "failover: committed id_prefix={} kind={:?} samples={} duration_ms={}",
        id_prefix(id),
        kind,
        samples_16k.len(),
        samples_16k.len() as u64 * 1000 / u64::from(TARGET_RATE)
    );
    Ok(())
}

fn id_prefix(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

/// Startup restore: pick the best candidate, validate it fully, then delete
/// the alternative and return it.
pub fn restore_choice(root: &Path, now_unix: i64) -> Option<LoadedTake> {
    // Inspect sidecars and file lengths first. This avoids decoding both a
    // stale committed take and a newer live take just to discard one of them.
    let live = inspect_slot(root, true);
    let committed = inspect_slot(root, false);
    let live = live.filter(|slot| {
        slot.meta.started_at_unix <= now_unix
            && slot.meta.started_at_unix.saturating_add(TTL_SECS) > now_unix
            && inspection_passes_gates(slot)
    });
    let committed = committed.filter(|slot| {
        slot.meta.started_at_unix <= now_unix
            && slot.meta.started_at_unix.saturating_add(TTL_SECS) > now_unix
            && inspection_passes_gates(slot)
    });

    let winner_live = match (&live, &committed) {
        (Some(l), Some(c)) if l.meta.id == c.meta.id => {
            if c.meta.kind == FailoverKind::Processing {
                false
            } else {
                l.usable_samples >= c.usable_samples
            }
        }
        (Some(l), Some(c)) => l.meta.started_at_unix >= c.meta.started_at_unix,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => {
            delete_all(root);
            return None;
        }
    };
    // Decode and re-check the selected winner before touching the alternative.
    // Metadata inspection is only a cheap filter; the decoded PCM is the
    // authoritative validation.
    if let Some(take) = load_slot(root, winner_live).filter(LoadedTake::passes_gates) {
        delete_slot(root, !winner_live);
        return Some(take);
    }

    // A candidate can pass the sidecar gates but fail after decoding (for
    // example, if its PCM was damaged after the last sidecar checkpoint).
    // Preserve the alternative long enough to recover it in that case.
    delete_slot(root, winner_live);
    let alternative_live = !winner_live;
    let alternative_is_valid_candidate = if alternative_live {
        live.is_some()
    } else {
        committed.is_some()
    };
    if alternative_is_valid_candidate {
        if let Some(take) = load_slot(root, alternative_live).filter(LoadedTake::passes_gates) {
            return Some(take);
        }
    }

    delete_all(root);
    None
}

fn loaded_to_capture(take: LoadedTake) -> Option<CancelledCapture> {
    let duration_ms = take.duration_ms();
    let origin = take.origin();
    let started_at_unix = take.meta.started_at_unix;
    let id = take.meta.id.clone();
    let created_at_rfc3339 = Utc
        .timestamp_opt(started_at_unix, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    Some(CancelledCapture {
        audio: CapturedAudio::from_samples(take.samples_16k, TARGET_RATE, duration_ms),
        captured_at: Instant::now(),
        id,
        origin,
        created_at_rfc3339,
        started_at_unix,
        // Recovered after a crash/restart, so there is no live foreground
        // window to reuse — the watchdog's resume goes through the same
        // resume_cancelled_capture path, which re-captures the (now current)
        // foreground window whenever the stored target is the zero default.
        target: WindowTarget::default(),
        // Older crash-recovery sidecars did not persist the originating
        // Context. Use the safe fallback rather than resolving a new Context
        // from whatever window happens to be focused during recovery. New
        // sidecars carry only the stable local ID; the label is diagnostic
        // because restore runs before the database is opened.
        context: take
            .meta
            .context_id
            .map(|id| ResolvedContextIdentity {
                id,
                label: "Recovered context".to_string(),
            })
            .unwrap_or_else(ResolvedContextIdentity::everywhere),
    })
}

/// Load a surviving take into RAM. Does not show the pill (the watchdog does).
pub fn restore_into_state(state: &SharedState) {
    let Some(take) = restore_choice(&failover_dir(), now_unix()) else {
        return;
    };
    let origin = take.origin();
    let samples = take.usable_samples();
    let Some(capture) = loaded_to_capture(take) else {
        log::warn!("failover: restore encode failed samples={samples}");
        return;
    };
    match lock_state(state) {
        Ok(mut st) => {
            if !st.lifecycle.is_idle() {
                return;
            }
            log::info!(
                "failover: restored id_prefix={} origin={:?} samples={}",
                id_prefix(&capture.id),
                origin,
                samples
            );
            st.cancelled_capture = Some(capture);
        }
        Err(_) => log::warn!("failover: restore skipped (state lock poisoned)"),
    }
}

/// After the pill window can be shown, surface a restored take.
pub fn offer_restored_capture_pill(app: &AppHandle) -> bool {
    let Some(state) = app.try_state::<SharedState>() else {
        return false;
    };
    let Some(capture) = state::peek_cancelled_capture_if_fresh(state.inner()) else {
        return false;
    };
    if capture.captured_at.elapsed() >= CANCEL_RESUME_WINDOW {
        return false;
    }
    emit_cancelled_payload(app, &capture.created_at_rfc3339, capture.origin.as_str());
    match capture.origin {
        CaptureOrigin::UserCancelled => show_cancelled_pill(app),
        CaptureOrigin::Interrupted => show_interrupted_pill(app),
    }
    true
}

pub fn emit_cancelled_payload(app: &AppHandle, created_at: &str, kind: &str) {
    app.emit(
        "verenu:cancelled-capture",
        CancelledCapturePayload {
            created_at: created_at.to_string(),
            kind: kind.to_string(),
        },
    )
    .ok();
}

pub fn commit_capture(audio: &CapturedAudio, id: &str, kind: FailoverKind, started_at_unix: i64) {
    commit_capture_with_context(audio, id, kind, started_at_unix, None);
}

pub fn commit_capture_with_context(
    audio: &CapturedAudio,
    id: &str,
    kind: FailoverKind,
    started_at_unix: i64,
    context_id: Option<i64>,
) {
    let root = failover_dir();
    if promote_live(&root, id, kind, started_at_unix, context_id).is_ok() {
        return;
    }
    if let Err(e) = write_committed_with_context(
        &root,
        id,
        kind,
        started_at_unix,
        context_id,
        &audio.samples_16k,
    ) {
        log::warn!(
            "failover: commit failed id_prefix={} samples={}: {e}",
            id_prefix(id),
            audio.samples_16k.len()
        );
    }
}

/// Promote the already durable live spool instead of rewriting the full take
/// after capture stops. The sidecar is updated atomically before the directory
/// is moved, so a crash can leave either a valid live or committed candidate.
fn promote_live(
    root: &Path,
    id: &str,
    kind: FailoverKind,
    started_at_unix: i64,
    context_id: Option<i64>,
) -> anyhow::Result<()> {
    let dir = slot_dir(root, true);
    let mut meta = load_session(&dir.join(SESSION_FILE))
        .ok_or_else(|| anyhow::anyhow!("no live recovery sidecar"))?;
    if meta.id != id {
        anyhow::bail!("live recovery id mismatch");
    }
    meta.kind = kind;
    meta.started_at_unix = started_at_unix;
    if context_id.is_some() {
        meta.context_id = context_id;
    }
    write_session_atomic(&dir.join(SESSION_FILE), &meta)?;
    let committed = slot_dir(root, false);
    delete_committed(root);
    fs::rename(&dir, &committed)?;
    log::info!(
        "failover: promoted live id_prefix={} kind={:?} samples={}",
        id_prefix(id),
        kind,
        meta.sample_count
    );
    Ok(())
}

pub fn retire_committed() {
    delete_committed(&failover_dir());
}

/// Incremental live writer. PCM is appended and synced before `sample_count`
/// is published in the sidecar.
pub struct LiveWriter {
    root: PathBuf,
    file: File,
    meta: SessionMeta,
    pending: Vec<f32>,
    rms_sum_sq: f64,
    failed: bool,
    storage_full: bool,
    last_checkpoint: Instant,
    superseded: bool,
    app: Option<AppHandle>,
    state: Option<SharedState>,
}

impl LiveWriter {
    #[cfg(test)]
    pub fn open(
        root: PathBuf,
        id: String,
        prepend_16k: Option<&[f32]>,
        app: Option<AppHandle>,
        state: Option<SharedState>,
    ) -> anyhow::Result<Self> {
        Self::open_with_context(root, id, prepend_16k, None, app, state)
    }

    pub fn open_with_context(
        root: PathBuf,
        id: String,
        prepend_16k: Option<&[f32]>,
        context_id: Option<i64>,
        app: Option<AppHandle>,
        state: Option<SharedState>,
    ) -> anyhow::Result<Self> {
        if prepend_16k.is_some_and(|samples| samples.len() as u64 > MAX_RECOVERY_SAMPLES) {
            anyhow::bail!("recovery audio exceeds the recording duration cap");
        }
        let dir = slot_dir(&root, true);
        if let Some(prepend) = prepend_16k {
            let committed_dir = slot_dir(&root, false);
            let committed_meta = load_session(&committed_dir.join(SESSION_FILE));
            let committed_len = File::open(committed_dir.join(AUDIO_FILE))
                .ok()
                .and_then(|file| file.metadata().ok())
                .map(|meta| meta.len() / 2);
            if let (Some(mut meta), Some(file_samples)) = (committed_meta, committed_len) {
                if meta.id == id
                    && meta.sample_count == prepend.len() as u64
                    && meta.sample_count <= MAX_RECOVERY_SAMPLES
                    && file_samples >= meta.sample_count
                {
                    fs::rename(&committed_dir, &dir)?;
                    meta.kind = FailoverKind::Recording;
                    if context_id.is_some() {
                        meta.context_id = context_id;
                    }
                    write_session_atomic(&dir.join(SESSION_FILE), &meta)?;
                    let file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(dir.join(AUDIO_FILE))?;
                    return Ok(Self {
                        root,
                        file,
                        pending: Vec::with_capacity(TARGET_RATE as usize),
                        rms_sum_sq: f64::from(meta.rms)
                            * f64::from(meta.rms)
                            * meta.sample_count as f64,
                        last_checkpoint: Instant::now(),
                        superseded: false,
                        app,
                        state,
                        meta,
                        failed: false,
                        storage_full: false,
                    });
                }
            }
        }
        preserve_previous_live(&root, &id);
        fs::create_dir_all(&dir)?;
        let pcm_path = dir.join(AUDIO_FILE);
        let _ = fs::remove_file(&pcm_path);
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&pcm_path)?;
        let started_at_unix = now_unix();
        let mut meta = SessionMeta {
            version: SESSION_VERSION,
            id,
            kind: FailoverKind::Recording,
            started_at_unix,
            sample_rate: TARGET_RATE,
            sample_count: 0,
            duration_ms: 0,
            rms: 0.0,
            context_id,
        };
        if let Some(prepend) = prepend_16k {
            if !prepend.is_empty() {
                let pcm = samples_to_pcm(prepend);
                file.write_all(&pcm)?;
                file.flush()?;
                file.sync_data()?;
                meta.sample_count = prepend.len() as u64;
                meta.duration_ms = meta.sample_count * 1000 / u64::from(TARGET_RATE);
                meta.rms = audio::rms_f32(prepend);
            }
        }
        write_session_atomic(&dir.join(SESSION_FILE), &meta)?;
        Ok(Self {
            root,
            file,
            meta,
            pending: Vec::with_capacity(TARGET_RATE as usize),
            rms_sum_sq: prepend_16k
                .map(|samples| samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum())
                .unwrap_or(0.0),
            failed: false,
            last_checkpoint: Instant::now(),
            superseded: false,
            app,
            state,
            storage_full: false,
        })
    }

    fn mark_storage_full(&mut self, error: &impl std::fmt::Display) {
        self.storage_full |= store::is_storage_full_error(&error.to_string());
    }

    fn checkpoint(&mut self, finish: bool) {
        if self.pending.is_empty() && !finish {
            return;
        }
        if !self.pending.is_empty() {
            let pcm = samples_to_pcm(&self.pending);
            if let Err(e) = self.file.write_all(&pcm).and_then(|_| self.file.flush()) {
                log::warn!("failover: live pcm write failed: {e}");
                self.mark_storage_full(&e);
                self.failed = true;
                return;
            }
            if let Err(e) = self.file.sync_data() {
                log::warn!("failover: live pcm sync failed: {e}");
                self.mark_storage_full(&e);
                self.failed = true;
                return;
            }
            self.meta.sample_count += self.pending.len() as u64;
            self.rms_sum_sq += self
                .pending
                .iter()
                .map(|s| f64::from(*s) * f64::from(*s))
                .sum::<f64>();
            self.meta.duration_ms = self.meta.sample_count * 1000 / u64::from(TARGET_RATE);
            self.meta.rms = (self.rms_sum_sq / self.meta.sample_count as f64).sqrt() as f32;
            let session_path = slot_dir(&self.root, true).join(SESSION_FILE);
            if let Err(e) = write_session_atomic(&session_path, &self.meta) {
                log::warn!("failover: live sidecar write failed: {e}");
                self.mark_storage_full(&e);
                self.failed = true;
            }
            self.pending.clear();
        }
        self.last_checkpoint = Instant::now();
        self.maybe_supersede();
    }

    fn maybe_supersede(&mut self) {
        if self.superseded || self.meta.duration_ms < MIN_RECORDING_MS {
            return;
        }
        self.superseded = true;
        delete_committed(&self.root);
        if let Some(state) = &self.state {
            if let Ok(mut st) = lock_state(state) {
                let current_id = self.meta.id.as_str();
                let stale = st
                    .cancelled_capture
                    .as_ref()
                    .is_some_and(|c| c.id != current_id);
                if stale {
                    st.cancelled_capture = None;
                    if let Some(app) = &self.app {
                        state::emit_cancelled_capture_cleared(app);
                    }
                }
            }
        }
        log::debug!(
            "failover: live superseded committed id_prefix={} duration_ms={}",
            id_prefix(&self.meta.id),
            self.meta.duration_ms
        );
    }

    fn invalidate_recovery(&self) {
        delete_live(&self.root);
        if let Some(state) = &self.state {
            if let Ok(mut st) = lock_state(state) {
                st.failover_session_id = None;
                st.failover_reuse_id = false;
                st.failover_started_at_unix = 0;
            }
        }
    }
}

impl DurableSink for LiveWriter {
    fn extend(&mut self, samples_16k: &[f32]) -> DurableSinkResult {
        if samples_16k.is_empty() {
            return if self.failed {
                Err(DurableSinkError::Disk)
            } else {
                Ok(())
            };
        }
        if self.failed {
            return Err(DurableSinkError::Disk);
        }
        let buffered = self
            .meta
            .sample_count
            .saturating_add(self.pending.len() as u64);
        let remaining = MAX_RECOVERY_SAMPLES.saturating_sub(buffered) as usize;
        if samples_16k.len() > remaining {
            if remaining > 0 {
                self.pending.extend_from_slice(&samples_16k[..remaining]);
                self.checkpoint(true);
            }
            self.failed = true;
            return Err(DurableSinkError::Disk);
        }
        self.pending.extend_from_slice(samples_16k);
        if self.pending.len() >= TARGET_RATE as usize
            || self.last_checkpoint.elapsed() >= Duration::from_secs(1)
        {
            self.checkpoint(false);
        }
        if self.failed {
            Err(DurableSinkError::Disk)
        } else {
            Ok(())
        }
    }

    fn finish(&mut self) -> DurableSinkResult {
        self.checkpoint(true);
        if let Err(e) = self.file.flush().and_then(|_| self.file.sync_all()) {
            log::warn!("failover: final live pcm sync failed: {e}");
            self.mark_storage_full(&e);
            self.failed = true;
        }
        if self.failed {
            Err(DurableSinkError::Disk)
        } else {
            Ok(())
        }
    }
}

enum WriterMessage {
    Samples(Vec<f32>),
    Finish,
}

struct AsyncLiveWriter {
    tx: Option<std::sync::mpsc::SyncSender<WriterMessage>>,
    handle: Option<std::thread::JoinHandle<()>>,
    failed: Arc<AtomicBool>,
}

impl DurableSink for AsyncLiveWriter {
    fn extend(&mut self, samples_16k: &[f32]) -> DurableSinkResult {
        if samples_16k.is_empty() {
            return if self.failed.load(Ordering::Acquire) {
                Err(DurableSinkError::Disk)
            } else {
                Ok(())
            };
        }
        if self.failed.load(Ordering::Acquire) {
            return Err(DurableSinkError::Disk);
        }
        if let Some(tx) = &self.tx {
            match tx.try_send(WriterMessage::Samples(samples_16k.to_vec())) {
                Ok(()) => {}
                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                    // A slow disk must stop the take, not make the audio
                    // processing worker wait behind a bounded queue.
                    log::warn!("failover: recovery writer queue is full");
                    self.failed.store(true, Ordering::Release);
                    return Err(DurableSinkError::Backpressure);
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                    self.failed.store(true, Ordering::Release);
                    return Err(DurableSinkError::Disk);
                }
            }
        } else {
            self.failed.store(true, Ordering::Release);
            return Err(DurableSinkError::Disk);
        }
        Ok(())
    }

    fn finish(&mut self) -> DurableSinkResult {
        if let Some(tx) = self.tx.take() {
            // Finish is best-effort and deliberately nonblocking as well. If
            // the queue is full, dropping the sender lets the writer drain its
            // already accepted prefix before it performs its final sync.
            let _ = tx.try_send(WriterMessage::Finish);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        if self.failed.load(Ordering::Acquire) {
            Err(DurableSinkError::Disk)
        } else {
            Ok(())
        }
    }
}

impl Drop for AsyncLiveWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub fn open_live_writer(
    id: String,
    prepend_16k: Option<&[f32]>,
    app: &AppHandle,
    state: &SharedState,
) -> Option<Box<dyn DurableSink>> {
    let context_id = lock_state(state)
        .ok()
        .and_then(|st| st.recording_context.as_ref().map(|context| context.id));
    match LiveWriter::open_with_context(
        failover_dir(),
        id,
        prepend_16k,
        context_id,
        Some(app.clone()),
        Some(state.clone()),
    ) {
        Ok(writer) => {
            let (tx, rx) = std::sync::mpsc::sync_channel(8);
            let failed = Arc::new(AtomicBool::new(false));
            let failed_thread = Arc::clone(&failed);
            let handle = std::thread::spawn(move || {
                let mut writer = writer;
                while let Ok(message) = rx.recv() {
                    match message {
                        WriterMessage::Samples(samples) => {
                            let _ = writer.extend(&samples);
                            if writer.failed {
                                failed_thread.store(true, Ordering::Release);
                                break;
                            }
                            if failed_thread.load(Ordering::Acquire) {
                                writer.invalidate_recovery();
                                break;
                            }
                        }
                        WriterMessage::Finish => break,
                    }
                }
                let _ = writer.finish();
                if writer.storage_full {
                    if let Some(app) = &writer.app {
                        app.emit("verenu:storage-full", ()).ok();
                    }
                }
                if writer.failed || failed_thread.load(Ordering::Acquire) {
                    writer.invalidate_recovery();
                }
                if writer.failed {
                    failed_thread.store(true, Ordering::Release);
                }
            });
            Some(Box::new(AsyncLiveWriter {
                tx: Some(tx),
                handle: Some(handle),
                failed,
            }))
        }
        Err(e) => {
            log::warn!("failover: live writer open failed: {e}");
            abandon_live();
            if let Ok(mut st) = lock_state(state) {
                st.failover_session_id = None;
                st.failover_reuse_id = false;
                st.failover_started_at_unix = 0;
            }
            if store::is_storage_full_error(&e.to_string()) {
                app.emit("verenu:storage-full", ()).ok();
            }
            None
        }
    }
}

pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn flush_on_exit(app: &AppHandle) {
    let Some(state) = app.try_state::<SharedState>() else {
        return;
    };
    let Some((session, mic_id)) = state::take_recording_plain(state.inner()) else {
        return;
    };
    match session.stop() {
        Ok(result) => {
            let (id, started) = match lock_state(state.inner()) {
                Ok(st) => (
                    st.failover_session_id.clone(),
                    if st.failover_started_at_unix != 0 {
                        st.failover_started_at_unix
                    } else {
                        now_unix()
                    },
                ),
                Err(_) => (None, now_unix()),
            };
            if let Some(id) = id {
                if result.duration_ms >= MIN_RECORDING_MS {
                    let audio = CapturedAudio::from_samples(
                        result.samples_16k,
                        result.sample_rate,
                        result.duration_ms,
                    );
                    commit_capture(&audio, &id, FailoverKind::Recording, started);
                }
            }
        }
        Err(e) => log::warn!("failover: exit flush stop failed: {e}"),
    }
    if let Some(session_id) = mic_id {
        crate::system::volume::release_mic(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root() -> PathBuf {
        let p = std::env::temp_dir().join(format!("verenu-failover-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn tone(samples: usize, amp: f32) -> Vec<f32> {
        (0..samples)
            .map(|i| amp * ((i as f32) * 0.1).sin())
            .collect()
    }

    fn loud_ms(ms: u64) -> Vec<f32> {
        tone((TARGET_RATE as u64 * ms / 1000) as usize, 0.4)
    }

    #[test]
    fn async_recovery_queue_full_returns_without_blocking() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        tx.send(WriterMessage::Samples(vec![0.1])).unwrap();
        let failed = Arc::new(AtomicBool::new(false));
        let mut writer = AsyncLiveWriter {
            tx: Some(tx),
            handle: None,
            failed,
        };
        let started = Instant::now();
        let result = writer.extend(&[0.2]);
        assert_eq!(result, Err(DurableSinkError::Backpressure));
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(writer.finish(), Err(DurableSinkError::Disk));
    }

    #[test]
    fn live_writer_keeps_durable_prefix_after_disk_failure() {
        let root = test_root();
        let prefix = loud_ms(1000);
        let mut writer = LiveWriter::open(
            root.clone(),
            "disk-failure".into(),
            Some(&prefix),
            None,
            None,
        )
        .unwrap();
        // Make the next sidecar write fail while leaving the already-published
        // PCM prefix and its metadata intact.
        fs::create_dir(slot_dir(&root, true).join("session.json.tmp")).unwrap();
        assert_eq!(writer.extend(&loud_ms(1000)), Err(DurableSinkError::Disk));
        let _ = writer.finish();
        let loaded = load_slot(&root, true).expect("published prefix survives");
        assert_eq!(loaded.samples_16k.len(), prefix.len());
        delete_all(&root);
    }

    #[test]
    fn fresh_take_does_not_overwrite_previous_live_prefix() {
        let root = test_root();
        let prefix = loud_ms(1000);
        let mut previous =
            LiveWriter::open(root.clone(), "previous".into(), Some(&prefix), None, None).unwrap();
        let _ = previous.finish();
        assert!(
            load_slot(&root, true).is_some(),
            "previous live take exists"
        );
        drop(previous);

        let mut current =
            LiveWriter::open(root.clone(), "current".into(), None, None, None).unwrap();
        let _ = current.finish();
        let preserved = load_slot(&root, false).expect("previous live prefix is preserved");
        assert_eq!(preserved.meta.id, "previous");
        assert_eq!(preserved.samples_16k.len(), prefix.len());
        delete_all(&root);
    }

    #[test]
    fn round_trip_write_read() {
        let root = test_root();
        let samples = loud_ms(800);
        write_committed(
            &root,
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            FailoverKind::Cancelled,
            1_700_000_000,
            &samples,
        )
        .unwrap();
        let loaded = load_slot(&root, false).unwrap();
        assert_eq!(loaded.samples_16k.len(), samples.len());
        assert!(loaded.passes_gates());
        delete_all(&root);
    }

    #[test]
    fn recovered_take_retains_originating_context_id() {
        let root = test_root();
        let samples = loud_ms(800);
        write_committed_with_context(
            &root,
            "contextual-recovery",
            FailoverKind::Cancelled,
            now_unix(),
            Some(42),
            &samples,
        )
        .unwrap();
        let loaded = restore_choice(&root, now_unix()).unwrap();
        assert_eq!(loaded.meta.context_id, Some(42));
        let capture = loaded_to_capture(loaded).unwrap();
        assert_eq!(capture.context.id, 42);
        delete_all(&root);
    }

    #[test]
    fn pcm_ahead_of_sidecar_uses_published_count() {
        let root = test_root();
        let samples = loud_ms(800);
        write_committed(
            &root,
            "id-published",
            FailoverKind::Recording,
            now_unix(),
            &samples,
        )
        .unwrap();
        let pcm_path = slot_dir(&root, false).join(AUDIO_FILE);
        let extra = samples_to_pcm(&loud_ms(200));
        let mut f = OpenOptions::new().append(true).open(&pcm_path).unwrap();
        f.write_all(&extra).unwrap();
        f.sync_all().unwrap();
        drop(f);
        let loaded = load_slot(&root, false).unwrap();
        assert_eq!(loaded.samples_16k.len(), samples.len());
        delete_all(&root);
    }

    #[test]
    fn sidecar_ahead_of_pcm_clamps_to_file() {
        let root = test_root();
        let samples = loud_ms(800);
        write_committed(
            &root,
            "id-clamp",
            FailoverKind::Recording,
            now_unix(),
            &samples,
        )
        .unwrap();
        let mut meta = load_session(&slot_dir(&root, false).join(SESSION_FILE)).unwrap();
        meta.sample_count = samples.len() as u64 + 4000;
        write_session_atomic(&slot_dir(&root, false).join(SESSION_FILE), &meta).unwrap();
        let loaded = load_slot(&root, false).unwrap();
        assert_eq!(loaded.samples_16k.len(), samples.len());
        delete_all(&root);
    }

    #[test]
    fn recovery_load_caps_sidecar_before_allocating_audio() {
        let root = test_root();
        let dir = slot_dir(&root, false);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(AUDIO_FILE), [0u8, 0u8]).unwrap();
        write_session_atomic(
            &dir.join(SESSION_FILE),
            &SessionMeta {
                version: SESSION_VERSION,
                id: "oversized-sidecar".into(),
                kind: FailoverKind::Cancelled,
                started_at_unix: now_unix(),
                sample_rate: TARGET_RATE,
                sample_count: MAX_RECOVERY_SAMPLES + 1,
                duration_ms: u64::MAX,
                rms: 1.0,
                context_id: None,
            },
        )
        .unwrap();

        let loaded = load_slot(&root, false).expect("valid PCM prefix remains recoverable");
        assert_eq!(loaded.samples_16k.len(), 1);
        assert_eq!(loaded.meta.sample_count, 1);
        delete_all(&root);
    }

    #[test]
    fn odd_trailing_byte_is_dropped() {
        let root = test_root();
        let samples = loud_ms(800);
        write_committed(
            &root,
            "id-odd",
            FailoverKind::Recording,
            now_unix(),
            &samples,
        )
        .unwrap();
        let pcm_path = slot_dir(&root, false).join(AUDIO_FILE);
        let mut f = OpenOptions::new().append(true).open(&pcm_path).unwrap();
        f.write_all(&[0x7f]).unwrap();
        f.sync_all().unwrap();
        drop(f);
        let loaded = load_slot(&root, false).unwrap();
        assert_eq!(loaded.samples_16k.len(), samples.len());
        delete_all(&root);
    }

    #[test]
    fn expired_sidecar_is_not_restored() {
        let root = test_root();
        let samples = loud_ms(800);
        write_committed(&root, "id-old", FailoverKind::Cancelled, 1_000, &samples).unwrap();
        assert!(restore_choice(&root, 1_000 + TTL_SECS + 10).is_none());
        assert!(load_slot(&root, false).is_none());
        delete_all(&root);
    }

    #[test]
    fn processing_committed_beats_same_id_live() {
        let root = test_root();
        let committed = loud_ms(1200);
        let live = loud_ms(2000);
        let t = now_unix();
        write_committed(&root, "same-id", FailoverKind::Processing, t, &committed).unwrap();
        write_slot(&root, true, "same-id", FailoverKind::Recording, t, &live).unwrap();
        let restored = restore_choice(&root, t).unwrap();
        assert_eq!(restored.meta.kind, FailoverKind::Processing);
        assert_eq!(restored.samples_16k.len(), committed.len());
        delete_all(&root);
    }

    #[test]
    fn short_new_live_keeps_old_committed() {
        let root = test_root();
        let old = loud_ms(1200);
        write_committed(&root, "old-id", FailoverKind::Cancelled, now_unix(), &old).unwrap();
        let short = loud_ms(200);
        let mut w =
            LiveWriter::open(root.clone(), "new-id".into(), Some(&short), None, None).unwrap();
        let _ = w.finish();
        drop(w);
        // 200ms fails gates, so restore_choice drops live and keeps committed.
        let restored = restore_choice(&root, now_unix()).unwrap();
        assert_eq!(restored.meta.id, "old-id");
        delete_all(&root);
    }

    #[test]
    fn long_new_live_wins_over_committed() {
        let root = test_root();
        let old = loud_ms(1200);
        write_committed(&root, "old-id", FailoverKind::Cancelled, now_unix(), &old).unwrap();
        let neu = loud_ms(900);
        let mut w =
            LiveWriter::open(root.clone(), "new-id".into(), Some(&neu), None, None).unwrap();
        let _ = w.finish();
        drop(w);
        let restored = restore_choice(&root, now_unix()).unwrap();
        assert_eq!(restored.meta.id, "new-id");
        assert!(load_slot(&root, false).is_none());
        delete_all(&root);
    }

    #[test]
    fn failed_winner_validation_keeps_alternative_for_recovery() {
        let root = test_root();
        let t = now_unix();
        let committed = loud_ms(1200);
        write_committed(
            &root,
            "committed-winner",
            FailoverKind::Cancelled,
            t,
            &committed,
        )
        .unwrap();
        let live = loud_ms(1600);
        write_slot(
            &root,
            true,
            "damaged-winner",
            FailoverKind::Recording,
            t + 1,
            &live,
        )
        .unwrap();

        // Keep the sidecar's optimistic RMS, but make the selected live PCM
        // fail the full decoded-audio gate.
        fs::write(
            slot_dir(&root, true).join(AUDIO_FILE),
            vec![0_u8; live.len() * 2],
        )
        .unwrap();

        let restored = restore_choice(&root, t + 1).expect("alternative should survive");
        assert_eq!(restored.meta.id, "committed-winner");
        assert!(!slot_dir(&root, true).exists());
        delete_all(&root);
    }

    #[test]
    fn live_writer_publish_after_sync() {
        let root = test_root();
        let mut w = LiveWriter::open(root.clone(), "live-1".into(), None, None, None).unwrap();
        let _ = DurableSink::extend(&mut w, &loud_ms(1000));
        let _ = w.finish();
        drop(w);
        let loaded = load_slot(&root, true).unwrap();
        assert!(loaded.samples_16k.len() >= 15_000);
        delete_all(&root);
    }

    #[test]
    fn live_spool_is_promoted_without_rewriting_pcm() {
        let root = test_root();
        let samples = loud_ms(1000);
        write_slot(
            &root,
            true,
            "promote-id",
            FailoverKind::Recording,
            now_unix(),
            &samples,
        )
        .unwrap();
        let before = fs::read(slot_dir(&root, true).join(AUDIO_FILE)).unwrap();
        promote_live(
            &root,
            "promote-id",
            FailoverKind::Processing,
            now_unix(),
            None,
        )
        .unwrap();
        assert_eq!(
            fs::read(slot_dir(&root, false).join(AUDIO_FILE)).unwrap(),
            before
        );
        assert!(!slot_dir(&root, true).exists());
        assert_eq!(
            load_session(&slot_dir(&root, false).join(SESSION_FILE))
                .unwrap()
                .kind,
            FailoverKind::Processing
        );
        delete_all(&root);
    }

    #[test]
    fn continuation_reuses_matching_committed_spool() {
        let root = test_root();
        let previous = loud_ms(1000);
        write_committed(
            &root,
            "continue-id",
            FailoverKind::Cancelled,
            now_unix(),
            &previous,
        )
        .unwrap();
        let mut writer = LiveWriter::open(
            root.clone(),
            "continue-id".into(),
            Some(&previous),
            None,
            None,
        )
        .unwrap();
        let _ = DurableSink::extend(&mut writer, &loud_ms(250));
        let _ = writer.finish();
        let live = load_slot(&root, true).unwrap();
        assert_eq!(live.samples_16k.len(), previous.len() + 4_000);
        assert!(!slot_dir(&root, false).exists());
        delete_all(&root);
    }

    #[test]
    fn resume_seed_without_supersede_keeps_committed() {
        let root = test_root();
        let original = loud_ms(1000);
        let t = now_unix();
        write_committed(&root, "resume-id", FailoverKind::Cancelled, t, &original).unwrap();
        write_slot(
            &root,
            true,
            "resume-id",
            FailoverKind::Recording,
            t,
            &original,
        )
        .unwrap();
        assert!(load_slot(&root, false).is_some());
        assert!(load_slot(&root, true).is_some());
        let restored = restore_choice(&root, t).unwrap();
        assert_eq!(restored.meta.id, "resume-id");
        delete_all(&root);
    }

    #[test]
    fn crash_before_seed_keeps_committed() {
        let root = test_root();
        let original = loud_ms(1000);
        write_committed(
            &root,
            "resume-id",
            FailoverKind::Cancelled,
            now_unix(),
            &original,
        )
        .unwrap();
        let restored = restore_choice(&root, now_unix()).unwrap();
        assert_eq!(restored.meta.id, "resume-id");
        assert_eq!(restored.meta.kind, FailoverKind::Cancelled);
        delete_all(&root);
    }

    #[test]
    fn too_quiet_is_not_restored() {
        let root = test_root();
        let quiet = vec![0.0f32; (TARGET_RATE as u64 * 800 / 1000) as usize];
        write_committed(&root, "quiet", FailoverKind::Cancelled, now_unix(), &quiet).unwrap();
        assert!(restore_choice(&root, now_unix()).is_none());
        delete_all(&root);
    }
}
