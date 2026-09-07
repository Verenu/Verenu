//! Digital-silence detector for microphones that zero PCM when hardware-muted.
//!
//! Treats a sample as silent when `|s| <= 1/32768` (one LSB at 16-bit). A window
//! must be almost entirely silent before "muted"; quiet ambient noise must not
//! trip that threshold.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// One LSB of a full-scale 16-bit sample, expressed as f32 in [-1, 1].
pub const DIGITAL_SILENCE_EPS: f32 = 1.0 / 32768.0;

const DEFAULT_WINDOW: Duration = Duration::from_millis(250);
const MUTE_SILENT_FRACTION: f64 = 0.98;
const UNMUTE_SILENT_FRACTION: f64 = 0.90;
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SilenceTransition {
    BecameMuted,
    BecameUnmuted,
}

#[derive(Debug, Clone, Copy)]
struct ChunkStat {
    at: Instant,
    silent: u32,
    total: u32,
    abs_max: f32,
}

#[derive(Debug, Clone)]
pub struct DigitalSilenceDetector {
    window: Duration,
    debounce: Duration,
    muted: bool,
    chunks: VecDeque<ChunkStat>,
    silent_count: u64,
    total_count: u64,
    abs_max: f32,
    pending: Option<(bool, Instant)>,
    seeded: bool,
}

impl Default for DigitalSilenceDetector {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW, DEFAULT_DEBOUNCE)
    }
}

impl DigitalSilenceDetector {
    pub fn new(window: Duration, debounce: Duration) -> Self {
        Self {
            window,
            debounce,
            muted: false,
            chunks: VecDeque::new(),
            silent_count: 0,
            total_count: 0,
            abs_max: 0.0,
            pending: None,
            seeded: false,
        }
    }

    #[allow(dead_code)]
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn is_digital_silent_sample(sample: f32) -> bool {
        sample.abs() <= DIGITAL_SILENCE_EPS
    }

    /// Push a block of PCM samples observed at `now`. Returns a debounced
    /// transition when the mute state has been stable long enough.
    pub fn push_samples(
        &mut self,
        samples: &[f32],
        now: Instant,
    ) -> Option<SilenceTransition> {
        if !samples.is_empty() {
            let mut silent = 0u32;
            let mut abs_max = 0.0f32;
            for &sample in samples {
                let a = sample.abs();
                if a > abs_max {
                    abs_max = a;
                }
                if Self::is_digital_silent_sample(sample) {
                    silent += 1;
                }
            }
            let total = samples.len() as u32;
            self.chunks.push_back(ChunkStat {
                at: now,
                silent,
                total,
                abs_max,
            });
            self.silent_count += u64::from(silent);
            self.total_count += u64::from(total);
            if abs_max > self.abs_max {
                self.abs_max = abs_max;
            }
        }
        self.prune(now);
        self.evaluate(now)
    }

    fn prune(&mut self, now: Instant) {
        while let Some(front) = self.chunks.front().copied() {
            if now.saturating_duration_since(front.at) <= self.window {
                break;
            }
            self.chunks.pop_front();
            self.silent_count = self.silent_count.saturating_sub(u64::from(front.silent));
            self.total_count = self.total_count.saturating_sub(u64::from(front.total));
            self.recompute_abs_max();
        }
    }

    fn recompute_abs_max(&mut self) {
        self.abs_max = self
            .chunks
            .iter()
            .map(|c| c.abs_max)
            .fold(0.0f32, f32::max);
    }

    fn evaluate(&mut self, now: Instant) -> Option<SilenceTransition> {
        if self.total_count == 0 {
            return None;
        }
        let silent_fraction = self.silent_count as f64 / self.total_count as f64;
        let looks_muted = silent_fraction >= MUTE_SILENT_FRACTION;
        let looks_unmuted =
            self.abs_max > DIGITAL_SILENCE_EPS && silent_fraction < UNMUTE_SILENT_FRACTION;
        let want_muted = if self.muted {
            !looks_unmuted
        } else {
            looks_muted
        };

        if !self.seeded {
            self.muted = want_muted;
            self.seeded = true;
            self.pending = None;
            return None;
        }

        if want_muted == self.muted {
            self.pending = None;
            return None;
        }

        match self.pending {
            Some((target, since)) if target == want_muted => {
                if now.saturating_duration_since(since) < self.debounce {
                    return None;
                }
                self.pending = None;
                self.muted = want_muted;
                Some(if want_muted {
                    SilenceTransition::BecameMuted
                } else {
                    SilenceTransition::BecameUnmuted
                })
            }
            _ => {
                self.pending = Some((want_muted, now));
                None
            }
        }
    }
}

/// Debounces boolean mute transitions so brief glitches do not fire.
#[derive(Debug, Clone)]
pub struct MuteDebouncer {
    debounce: Duration,
    last_stable: Option<bool>,
    pending: Option<(bool, Instant)>,
}

impl MuteDebouncer {
    pub fn new(debounce: Duration) -> Self {
        Self {
            debounce,
            last_stable: None,
            pending: None,
        }
    }

    /// Seeds the current mute state without emitting a transition.
    pub fn seed(&mut self, muted: bool) {
        self.last_stable = Some(muted);
        self.pending = None;
    }

    pub fn observe(&mut self, muted: bool, now: Instant) -> Option<bool> {
        let Some(last) = self.last_stable else {
            self.last_stable = Some(muted);
            return None;
        };
        if muted == last {
            self.pending = None;
            return None;
        }
        match self.pending {
            Some((target, since)) if target == muted => {
                if now.saturating_duration_since(since) < self.debounce {
                    return None;
                }
                self.pending = None;
                self.last_stable = Some(muted);
                Some(muted)
            }
            _ => {
                self.pending = Some((muted, now));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zeros(n: usize) -> Vec<f32> {
        vec![0.0; n]
    }

    fn noise(n: usize, amplitude: f32) -> Vec<f32> {
        (0..n)
            .map(|i| if i % 2 == 0 { amplitude } else { -amplitude })
            .collect()
    }

    #[test]
    fn exact_zeros_become_muted_after_debounce() {
        let start = Instant::now();
        let mut det = DigitalSilenceDetector::new(
            Duration::from_millis(200),
            Duration::from_millis(100),
        );
        // Seed with non-silent so the first mute is a real transition.
        assert!(det.push_samples(&noise(512, 0.01), start).is_none());
        assert!(!det.is_muted());

        // Fill the window with zeros so the earlier noise ages out.
        assert!(det
            .push_samples(&zeros(2048), start + Duration::from_millis(50))
            .is_none());
        assert!(det
            .push_samples(&zeros(2048), start + Duration::from_millis(220))
            .is_none());
        // Window is now all zeros; debounce still needs to elapse.
        assert_eq!(
            det.push_samples(&zeros(2048), start + Duration::from_millis(330)),
            Some(SilenceTransition::BecameMuted)
        );
        assert!(det.is_muted());
    }

    #[test]
    fn whisper_quiet_noise_is_not_muted() {
        let start = Instant::now();
        let mut det = DigitalSilenceDetector::new(
            Duration::from_millis(200),
            Duration::from_millis(100),
        );
        // ~0.001 is far above 1/32768 (~3e-5) but still "quiet".
        let quiet = noise(2048, 0.001);
        assert!(det.push_samples(&quiet, start).is_none());
        assert!(det
            .push_samples(&quiet, start + Duration::from_millis(50))
            .is_none());
        assert!(det
            .push_samples(&quiet, start + Duration::from_millis(150))
            .is_none());
        assert!(det
            .push_samples(&quiet, start + Duration::from_millis(250))
            .is_none());
        assert!(!det.is_muted());
    }

    #[test]
    fn unmute_requires_sustained_non_silent() {
        let start = Instant::now();
        let mut det = DigitalSilenceDetector::new(
            Duration::from_millis(200),
            Duration::from_millis(100),
        );
        assert!(det.push_samples(&noise(256, 0.01), start).is_none());
        assert!(det
            .push_samples(&zeros(4096), start + Duration::from_millis(50))
            .is_none());
        assert!(det
            .push_samples(&zeros(4096), start + Duration::from_millis(220))
            .is_none());
        assert_eq!(
            det.push_samples(&zeros(4096), start + Duration::from_millis(330)),
            Some(SilenceTransition::BecameMuted)
        );

        // A single non-zero sample inside an otherwise silent window must not unmute.
        let mut mostly_silent = zeros(1000);
        mostly_silent[0] = 0.05;
        assert!(det
            .push_samples(&mostly_silent, start + Duration::from_millis(340))
            .is_none());
        assert!(det.is_muted());

        let open = noise(4096, 0.02);
        assert!(det
            .push_samples(&open, start + Duration::from_millis(350))
            .is_none());
        // Age out silent chunks and complete unmute debounce.
        assert_eq!(
            det.push_samples(&open, start + Duration::from_millis(560)),
            Some(SilenceTransition::BecameUnmuted)
        );
        assert!(!det.is_muted());
    }

    #[test]
    fn mute_debouncer_ignores_seed_and_short_glitches() {
        let start = Instant::now();
        let mut deb = MuteDebouncer::new(Duration::from_millis(120));
        deb.seed(true);
        assert!(deb.observe(false, start).is_none());
        assert!(deb
            .observe(false, start + Duration::from_millis(50))
            .is_none());
        // Bounce back before debounce completes.
        assert!(deb
            .observe(true, start + Duration::from_millis(60))
            .is_none());
        assert!(deb
            .observe(false, start + Duration::from_millis(70))
            .is_none());
        assert_eq!(
            deb.observe(false, start + Duration::from_millis(200)),
            Some(false)
        );
    }

    #[test]
    fn digital_silence_eps_matches_16bit_lsb() {
        assert!(DigitalSilenceDetector::is_digital_silent_sample(0.0));
        assert!(DigitalSilenceDetector::is_digital_silent_sample(
            DIGITAL_SILENCE_EPS
        ));
        assert!(!DigitalSilenceDetector::is_digital_silent_sample(
            DIGITAL_SILENCE_EPS * 2.0
        ));
        assert!(!DigitalSilenceDetector::is_digital_silent_sample(0.001));
    }
}
