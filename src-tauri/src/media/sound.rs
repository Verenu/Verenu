use rodio::buffer::SamplesBuffer;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::OnceLock;

use crate::system::volume::MuteOutcome;

#[derive(Clone, Copy)]
pub enum SoundCue {
    Start,
    Stop,
    Error,
    Cancel,
}

const SAMPLE_RATE: u32 = 44_100;
const ATTACK_MS: f32 = 6.0;
const RELEASE_MS: f32 = 60.0;
const AMPLITUDE: f32 = 0.30;

pub const START_CUE_NORMAL_DELAY_MS: u64 = 260;
pub const START_CUE_HANDSFREE_DELAY_MS: u64 = 220;

static START_CUE_GEN: AtomicU64 = AtomicU64::new(0);
static SOUND_TX: OnceLock<mpsc::Sender<SoundCommand>> = OnceLock::new();
static LOCAL_MUTE_CLAIM: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct MuteOwners {
    shutting_down: bool,
    local: bool,
    remote: std::collections::HashSet<u64>,
}

impl MuteOwners {
    fn muted(&self) -> bool {
        self.local || !self.remote.is_empty()
    }

    fn acquire_local(&mut self, active: bool, claim_is_current: bool) -> bool {
        let acquired = !self.shutting_down && active && claim_is_current;
        if acquired {
            self.local = true;
        }
        acquired
    }
}

static MUTE_OWNERS: std::sync::LazyLock<std::sync::Mutex<MuteOwners>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(MuteOwners::default()));

fn apply_local_mute(
    owners: &mut MuteOwners,
    claim_is_current: bool,
    mute: impl FnOnce() -> MuteOutcome,
) -> MuteOutcome {
    if owners.acquire_local(true, claim_is_current) {
        mute()
    } else {
        MuteOutcome::Failed
    }
}

fn update_mute_owners(update: impl FnOnce(&mut MuteOwners)) -> Option<MuteOutcome> {
    let mut owners = MUTE_OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    let before = owners.muted();
    update(&mut owners);
    match (before, owners.muted()) {
        (false, true) => Some(crate::system::volume::mute()),
        (true, false) => {
            crate::system::volume::unmute();
            None
        }
        (true, true) => Some(crate::system::volume::mute()),
        (false, false) => None,
    }
}

fn release_mute_owners_on_exit(owners: &mut MuteOwners, restore_audio: impl FnOnce()) {
    owners.shutting_down = true;
    owners.local = false;
    owners.remote.clear();
    restore_audio();
}

#[cfg(test)]
mod mute_tests {
    use super::{MuteOutcome, MuteOwners};
    use std::cell::Cell;

    #[test]
    fn ending_one_dictation_preserves_other_mute_owners() {
        let mut owners = MuteOwners {
            local: true,
            ..Default::default()
        };
        owners.remote.extend([1, 2]);
        owners.local = false;
        assert!(owners.muted());
        owners.remote.remove(&1);
        assert!(owners.muted());
        owners.remote.remove(&2);
        assert!(!owners.muted());
        owners.local = true;
        owners.remote.insert(3);
        owners.remote.remove(&3);
        assert!(
            owners.muted(),
            "remote stop cannot release local dictation mute"
        );
    }

    #[test]
    fn immediate_local_mute_acquires_the_same_owner_as_the_delayed_path() {
        let mut owners = MuteOwners::default();
        assert!(owners.acquire_local(true, true));
        assert!(owners.local);
        assert!(owners.muted());
    }

    #[test]
    fn stale_claim_is_not_acquired() {
        let mut owners = MuteOwners::default();
        assert!(!owners.acquire_local(true, false));
        assert!(!owners.local);
        assert!(owners.acquire_local(true, true));
    }

    #[test]
    fn superseded_mute_task_fails_even_while_another_owner_holds_the_mute() {
        let remote_owner = u64::MAX - 7;
        let mut owners = MuteOwners::default();
        owners.remote.insert(remote_owner);
        let outcome = super::apply_local_mute(&mut owners, false, || {
            panic!("a rejected claim must not call the OS mute")
        });
        assert_eq!(outcome, MuteOutcome::Failed);
        assert!(!owners.local);
        assert_eq!(owners.remote, [remote_owner].into_iter().collect());
    }

    #[test]
    fn current_local_claim_returns_the_actual_mute_outcome() {
        let mut owners = MuteOwners::default();
        let calls = Cell::new(0);
        let outcome = super::apply_local_mute(&mut owners, true, || {
            calls.set(calls.get() + 1);
            MuteOutcome::Failed
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(outcome, MuteOutcome::Failed);
        assert!(
            owners.local,
            "failed mute still needs the normal release path"
        );
    }

    #[test]
    fn late_opener_and_queued_release_cannot_release_a_replacement_mute() {
        use std::sync::atomic::{AtomicU64, Ordering};
        let claims = AtomicU64::new(1);
        let queued_release = super::invalidate_local_claim(&claims, 1).unwrap();
        let b = claims.fetch_add(1, Ordering::SeqCst) + 1;
        let mut owners = MuteOwners {
            local: true,
            ..Default::default()
        };
        assert_eq!(super::invalidate_local_claim(&claims, 1), None);
        assert_eq!(claims.load(Ordering::SeqCst), b);
        super::apply_local_release(
            &mut owners,
            claims.load(Ordering::SeqCst) == queued_release,
            || panic!("A must not restore B's output"),
        );
        assert!(owners.local);
        let released_b = super::invalidate_local_claim(&claims, b).unwrap();
        let restored = Cell::new(false);
        super::apply_local_release(
            &mut owners,
            claims.load(Ordering::SeqCst) == released_b,
            || restored.set(true),
        );
        assert!(restored.get());
        assert!(!owners.muted());
    }

    #[test]
    fn releasing_local_claim_preserves_remote_mute() {
        let mut owners = MuteOwners {
            local: true,
            ..Default::default()
        };
        owners.remote.insert(7);
        super::apply_local_release(&mut owners, true, || {
            panic!("remote owner still needs mute")
        });
        assert!(!owners.local);
        assert!(owners.muted());
    }

    #[test]
    fn superseded_mute_task_mutes_nothing() {
        let stale = super::claim_local_mute().task();
        let _newer = super::claim_local_mute();
        assert_eq!(stale(), MuteOutcome::Failed);
    }

    #[test]
    fn exit_retries_os_restore_even_when_no_mute_owner_remains() {
        let mut owners = MuteOwners::default();
        let restore_calls = Cell::new(0);

        super::release_mute_owners_on_exit(&mut owners, || {
            restore_calls.set(restore_calls.get() + 1);
        });

        assert_eq!(restore_calls.get(), 1);
        assert!(owners.shutting_down);
        assert!(!owners.muted());
        assert!(!owners.acquire_local(true, true));
        assert!(!owners.local);
    }
}

pub fn set_remote_mute(owner: u64, muted: bool) {
    update_mute_owners(|owners| {
        if muted {
            if !owners.shutting_down {
                owners.remote.insert(owner);
            }
        } else {
            owners.remote.remove(&owner);
        }
    });
}

pub fn release_all_mutes_on_exit() {
    let mut owners = MUTE_OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    release_mute_owners_on_exit(&mut owners, crate::system::volume::unmute);
}
static SOUND_EFFECTS_VOLUME: AtomicU32 = AtomicU32::new(1.0f32.to_bits());

type AfterPlay = Box<dyn FnOnce() + Send + 'static>;

enum SoundCommand {
    Play {
        cue: SoundCue,
        generation: Option<u64>,
        after: Option<AfterPlay>,
    },
}

#[derive(Clone, Copy)]
enum Timbre {
    Soft,
    Buzz,
}

struct Harmonic {
    ratio: f32,
    gain: f32,
    decay_mult: f32,
}

struct Note {
    freq: f32,
    start_ms: f32,
    dur_ms: f32,
    gain: f32,
    timbre: Timbre,
}

pub fn play(cue: SoundCue) {
    if sound_tx()
        .send(SoundCommand::Play {
            cue,
            generation: None,
            after: None,
        })
        .is_err()
    {
        log::debug!("sound cue failed: playback worker is unavailable");
    }
}

pub fn set_volume(volume: f32) {
    SOUND_EFFECTS_VOLUME.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

pub fn play_start_delayed(delay_ms: u64) {
    play_start_delayed_then(delay_ms, || {});
}

pub fn play_start_delayed_then<F>(delay_ms: u64, after: F)
where
    F: FnOnce() + Send + 'static,
{
    let generation = START_CUE_GEN.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
    tauri::async_runtime::spawn(async move {
        if delay_ms != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        if START_CUE_GEN.load(Ordering::SeqCst) != generation {
            return;
        }

        let after_cb = Box::new(after) as AfterPlay;
        if let Err(mpsc::SendError(SoundCommand::Play {
            after: Some(cb), ..
        })) = sound_tx().send(SoundCommand::Play {
            cue: SoundCue::Start,
            generation: Some(generation),
            after: Some(after_cb),
        }) {
            log::debug!("sound cue failed: playback worker is unavailable");
            cb();
        }
    });
}

pub fn cancel_pending_start() {
    START_CUE_GEN.fetch_add(1, Ordering::SeqCst);
}

#[derive(Clone, Copy)]
pub struct LocalMuteClaim(u64);

pub fn claim_local_mute() -> LocalMuteClaim {
    let generation = LOCAL_MUTE_CLAIM
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    LocalMuteClaim(generation)
}

impl LocalMuteClaim {
    pub fn is_current(self) -> bool {
        LOCAL_MUTE_CLAIM.load(Ordering::SeqCst) == self.0
    }

    pub fn task(self) -> impl FnOnce() -> MuteOutcome + Send + 'static {
        move || {
            let mut owners = MUTE_OWNERS.lock().unwrap_or_else(|e| e.into_inner());
            apply_local_mute(&mut owners, self.is_current(), crate::system::volume::mute)
        }
    }

    pub fn release(self) {
        if let Some(generation) = invalidate_local_claim(&LOCAL_MUTE_CLAIM, self.0) {
            queue_local_release(generation);
        }
    }
}

fn invalidate_local_claim(counter: &AtomicU64, claim: u64) -> Option<u64> {
    let released = claim.wrapping_add(1);
    counter
        .compare_exchange(claim, released, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| released)
}

fn apply_local_release(owners: &mut MuteOwners, claim_is_current: bool, restore: impl FnOnce()) {
    if !claim_is_current {
        return;
    }
    let before = owners.muted();
    owners.local = false;
    if before && !owners.muted() {
        restore();
    }
}

fn queue_local_release(generation: u64) {
    tauri::async_runtime::spawn_blocking(move || {
        let mut owners = MUTE_OWNERS.lock().unwrap_or_else(|e| e.into_inner());
        apply_local_release(
            &mut owners,
            LOCAL_MUTE_CLAIM.load(Ordering::SeqCst) == generation,
            crate::system::volume::unmute,
        );
    });
}

pub fn coordinated_unmute() {
    let generation = LOCAL_MUTE_CLAIM
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    queue_local_release(generation);
}

fn sound_tx() -> &'static mpsc::Sender<SoundCommand> {
    SOUND_TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<SoundCommand>();
        std::thread::spawn(move || sound_worker(rx));
        tx
    })
}

fn sound_worker(rx: mpsc::Receiver<SoundCommand>) {
    let mut output: Option<(rodio::OutputStream, rodio::OutputStreamHandle)> = None;
    let playback_id = Arc::new(AtomicU64::new(0));
    let mut current_sink: Option<Arc<rodio::Sink>> = None;

    loop {
        let command = match rx.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if current_sink.as_ref().is_some_and(|sink| sink.empty()) {
                    current_sink = None;
                    output = None;
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let SoundCommand::Play {
            cue,
            generation,
            after,
        } = command;

        if generation.is_some_and(|g| START_CUE_GEN.load(Ordering::SeqCst) != g) {
            continue;
        }

        let this_playback_id = playback_id.fetch_add(1, Ordering::SeqCst).wrapping_add(1);

        if current_sink.as_ref().is_some_and(|sink| sink.empty()) {
            current_sink = None;
            output = None;
        }

        if let Some(sink) = current_sink.take() {
            sink.stop();
        }

        let sink = match play_with_cached_output(&mut output, cue) {
            Ok(sink) => Arc::new(sink),
            Err(e) => {
                log::warn!("sound cue playback failed: {e:?}");
                output = None;

                if generation.is_some_and(|g| START_CUE_GEN.load(Ordering::SeqCst) != g) {
                    continue;
                }

                if let Some(after) = after {
                    after();
                }
                continue;
            }
        };

        current_sink = Some(sink.clone());

        if generation.is_some_and(|g| START_CUE_GEN.load(Ordering::SeqCst) != g) {
            sink.stop();
            continue;
        }

        if let Some(after) = after {
            let playback_id = playback_id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                sink.sleep_until_end();

                if playback_id.load(Ordering::SeqCst) != this_playback_id {
                    return;
                }
                if generation.is_some_and(|g| START_CUE_GEN.load(Ordering::SeqCst) != g) {
                    return;
                }

                after();
            });
        }
    }
}

fn play_with_cached_output(
    output: &mut Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,
    cue: SoundCue,
) -> anyhow::Result<rodio::Sink> {
    if output.is_none() {
        *output = Some(rodio::OutputStream::try_default()?);
    }

    let sink_result = {
        let (_, handle) = output
            .as_ref()
            .expect("cached output stream must exist before creating a sink");
        rodio::Sink::try_new(handle)
    };

    let sink = match sink_result {
        Ok(sink) => sink,
        Err(_) => {
            *output = Some(rodio::OutputStream::try_default()?);
            let (_, handle) = output
                .as_ref()
                .expect("cached output stream must exist after reinitialization");
            rodio::Sink::try_new(handle)?
        }
    };

    let volume = f32::from_bits(SOUND_EFFECTS_VOLUME.load(Ordering::Relaxed));
    sink.append(render(notes(cue), volume));
    Ok(sink)
}

fn harmonics(timbre: Timbre) -> &'static [Harmonic] {
    match timbre {
        Timbre::Soft => &[
            Harmonic {
                ratio: 1.0,
                gain: 1.0,
                decay_mult: 1.0,
            },
            Harmonic {
                ratio: 2.0,
                gain: 0.08,
                decay_mult: 1.6,
            },
        ],
        Timbre::Buzz => &[
            Harmonic {
                ratio: 1.0,
                gain: 1.0,
                decay_mult: 1.0,
            },
            Harmonic {
                ratio: 2.0,
                gain: 0.20,
                decay_mult: 1.0,
            },
            Harmonic {
                ratio: 3.0,
                gain: 0.34,
                decay_mult: 1.0,
            },
            Harmonic {
                ratio: 5.0,
                gain: 0.12,
                decay_mult: 1.0,
            },
        ],
    }
}

fn is_sustained(timbre: Timbre) -> bool {
    matches!(timbre, Timbre::Buzz)
}

fn notes(cue: SoundCue) -> &'static [Note] {
    match cue {
        SoundCue::Start => &[
            Note {
                freq: 523.25,
                start_ms: 0.0,
                dur_ms: 220.0,
                gain: 1.0,
                timbre: Timbre::Soft,
            },
            Note {
                freq: 659.25,
                start_ms: 60.0,
                dur_ms: 240.0,
                gain: 1.05,
                timbre: Timbre::Soft,
            },
            Note {
                freq: 783.99,
                start_ms: 120.0,
                dur_ms: 340.0,
                gain: 1.1,
                timbre: Timbre::Soft,
            },
        ],
        SoundCue::Stop => &[
            Note {
                freq: 587.33,
                start_ms: 0.0,
                dur_ms: 200.0,
                gain: 1.0,
                timbre: Timbre::Soft,
            },
            Note {
                freq: 880.00,
                start_ms: 90.0,
                dur_ms: 360.0,
                gain: 1.05,
                timbre: Timbre::Soft,
            },
        ],
        SoundCue::Error => &[
            Note {
                freq: 220.00,
                start_ms: 0.0,
                dur_ms: 150.0,
                gain: 0.6,
                timbre: Timbre::Buzz,
            },
            Note {
                freq: 174.61,
                start_ms: 165.0,
                dur_ms: 240.0,
                gain: 0.6,
                timbre: Timbre::Buzz,
            },
        ],
        SoundCue::Cancel => &[
            Note {
                freq: 392.00,
                start_ms: 0.0,
                dur_ms: 150.0,
                gain: 0.5,
                timbre: Timbre::Soft,
            },
            Note {
                freq: 293.66,
                start_ms: 70.0,
                dur_ms: 220.0,
                gain: 0.5,
                timbre: Timbre::Soft,
            },
        ],
    }
}

fn render(notes: &[Note], volume: f32) -> SamplesBuffer<f32> {
    SamplesBuffer::new(1, SAMPLE_RATE, render_samples(notes, volume))
}

fn render_samples(notes: &[Note], volume: f32) -> Vec<f32> {
    let sr = SAMPLE_RATE as f32;
    let total_ms = notes
        .iter()
        .map(|n| n.start_ms + n.dur_ms)
        .fold(0.0_f32, f32::max);
    let total = ((total_ms / 1000.0) * sr).ceil() as usize + 1;
    let mut buf = vec![0.0_f32; total];

    let attack = (ATTACK_MS / 1000.0) * sr;
    for note in notes {
        let start = ((note.start_ms / 1000.0) * sr) as usize;
        let len = ((note.dur_ms / 1000.0) * sr) as usize;
        if len == 0 {
            continue;
        }
        let lenf = len as f32;
        let sustained = is_sustained(note.timbre);
        let base_decay = 6.9 / lenf;
        let release = ((RELEASE_MS / 1000.0) * sr).min(lenf * 0.5);

        for h in harmonics(note.timbre) {
            let freq = note.freq * h.ratio;
            if freq > 18_000.0 {
                continue;
            }
            let amp = AMPLITUDE * note.gain * h.gain * volume;
            let h_decay = base_decay * h.decay_mult;
            let w = TAU * freq / sr;
            let decay_step = (-h_decay).exp();
            let mut decay_env = (-(attack.ceil() - attack) * h_decay).exp();
            for i in 0..len {
                let idx = start + i;
                if idx >= total {
                    break;
                }
                let n = i as f32;
                let env = if n < attack {
                    0.5 - 0.5 * (PI * n / attack).cos()
                } else if sustained {
                    let tail = lenf - n;
                    if tail < release {
                        0.5 - 0.5 * (PI * tail / release).cos()
                    } else {
                        1.0
                    }
                } else {
                    let current = decay_env;
                    decay_env *= decay_step;
                    current
                };
                buf[idx] += (w * n).sin() * env * amp;
            }
        }
    }

    for s in &mut buf {
        *s = s.clamp(-1.0, 1.0);
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn render_cue_previews() {
        let out_dir = std::env::var("VERENU_CUE_OUT").unwrap_or_else(|_| ".".to_string());
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        for (name, cue) in [
            ("start", SoundCue::Start),
            ("stop", SoundCue::Stop),
            ("error", SoundCue::Error),
            ("cancel", SoundCue::Cancel),
        ] {
            let path = format!("{out_dir}/cue_{name}.wav");
            let mut writer = hound::WavWriter::create(&path, spec).expect("create wav");
            for s in render_samples(notes(cue), 1.0) {
                let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                writer.write_sample(v).expect("write sample");
            }
            writer.finalize().expect("finalize wav");
            eprintln!("wrote {path}");
        }
    }

    #[test]
    fn render_volume_scales_generated_samples() {
        let full = render_samples(notes(SoundCue::Start), 1.0);
        let half = render_samples(notes(SoundCue::Start), 0.5);
        let full_peak = full.iter().map(|sample| sample.abs()).fold(0.0, f32::max);
        let half_peak = half.iter().map(|sample| sample.abs()).fold(0.0, f32::max);

        assert!(full_peak > 0.0);
        assert!((half_peak / full_peak - 0.5).abs() < 0.001);
        assert!(render_samples(notes(SoundCue::Start), 0.0)
            .iter()
            .all(|sample| *sample == 0.0));
    }
    #[cfg(target_os = "linux")]
    fn wpctl_default_sink_muted() -> bool {
        let out = std::process::Command::new("wpctl")
            .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .output()
            .expect("wpctl");
        String::from_utf8_lossy(&out.stdout).contains("[MUTED]")
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "mutes the real default sink"]
    fn live_mute_precedes_recording() {
        if wpctl_default_sink_muted() {
            eprintln!("default sink already muted; skipping");
            return;
        }
        let mut totals = Vec::new();
        for _ in 0..8 {
            let begin = std::time::Instant::now();
            let (muted_at_open, report) = crate::media::mute_gate::start_after_mute(
                Some(claim_local_mute().task()),
                crate::media::mute_gate::MUTE_CONFIRM_TIMEOUT,
                wpctl_default_sink_muted,
            );
            let confirmed_total = begin.elapsed();
            coordinated_unmute();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while wpctl_default_sink_muted() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!wpctl_default_sink_muted(), "sink was not restored");
            assert!(report.gate.is_confirmed(), "gate: {:?}", report.gate);
            assert!(muted_at_open, "recording opened before the sink was muted");
            totals.push(report.waited.as_secs_f64() * 1000.0);
            eprintln!(
                "LIVE gate={:?} mute confirmed in {:.1} ms (hotkey-to-mic-open path {:.1} ms)",
                report.gate,
                report.waited.as_secs_f64() * 1000.0,
                confirmed_total.as_secs_f64() * 1000.0
            );
        }
        totals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!(
            "LIVE confirmed-mute ms: min={:.1} median={:.1} max={:.1}",
            totals[0],
            totals[totals.len() / 2],
            totals[totals.len() - 1]
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "mutes the real default sink and opens the real microphone"]
    fn live_mute_then_real_mic_open() {
        if wpctl_default_sink_muted() {
            eprintln!("default sink already muted; skipping");
            return;
        }
        for _ in 0..3 {
            let (opened, report) = crate::media::mute_gate::start_after_mute(
                Some(claim_local_mute().task()),
                crate::media::mute_gate::MUTE_CONFIRM_TIMEOUT,
                || {
                    let muted_before_open = wpctl_default_sink_muted();
                    let opening = std::time::Instant::now();
                    let session =
                        crate::media::audio::RecordingSession::start(None, false, 1.0, None, None, false, None);
                    (muted_before_open, session, opening.elapsed())
                },
            );
            coordinated_unmute();
            let (muted_before_open, session, open_time) = opened;
            let session = session.expect("default microphone failed to open");
            eprintln!(
                "LIVE mute confirmed in {:.1} ms; mic opened {:.1} ms after that",
                report.waited.as_secs_f64() * 1000.0,
                open_time.as_secs_f64() * 1000.0
            );
            let _ = session.stop();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while wpctl_default_sink_muted() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!wpctl_default_sink_muted(), "sink was not restored");
            assert!(report.gate.is_confirmed(), "gate: {:?}", report.gate);
            assert!(muted_before_open, "mic opened before the sink was muted");
        }
    }
}
