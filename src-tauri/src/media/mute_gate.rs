use crate::system::volume::MuteOutcome;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const MUTE_CONFIRM_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteGate {
    Skipped,
    Confirmed(MuteOutcome),
    Unconfirmed,
    Failed,
    TimedOut,
}

impl MuteGate {
    pub fn is_confirmed(self) -> bool {
        matches!(self, MuteGate::Confirmed(_))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MuteReport {
    pub gate: MuteGate,
    pub waited: Duration,
}

pub fn wait_for_mute<F>(timeout: Duration, mute: F) -> MuteReport
where
    F: FnOnce() -> MuteOutcome + Send + 'static,
{
    let started = Instant::now();
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("verenu-mute".into())
        .spawn(move || {
            let _ = tx.send(mute());
        });
    let gate = match spawned {
        Err(err) => {
            log::warn!("auto-mute: could not start the mute thread: {err}");
            MuteGate::Failed
        }
        Ok(_) => match rx.recv_timeout(timeout) {
            Ok(MuteOutcome::Failed) => MuteGate::Failed,
            Ok(MuteOutcome::Requested) => MuteGate::Unconfirmed,
            Ok(outcome) => MuteGate::Confirmed(outcome),
            Err(mpsc::RecvTimeoutError::Timeout) => MuteGate::TimedOut,
            Err(mpsc::RecvTimeoutError::Disconnected) => MuteGate::Failed,
        },
    };
    MuteReport {
        gate,
        waited: started.elapsed(),
    }
}

pub fn start_after_mute<T, M, S>(mute: Option<M>, timeout: Duration, start: S) -> (T, MuteReport)
where
    M: FnOnce() -> MuteOutcome + Send + 'static,
    S: FnOnce() -> T,
{
    let Some(mute) = mute else {
        let value = start();
        return (
            value,
            MuteReport {
                gate: MuteGate::Skipped,
                waited: Duration::ZERO,
            },
        );
    };

    let report = wait_for_mute(timeout, mute);
    let mute_settled = Instant::now();
    let value = start();
    let mute_to_record = mute_settled.elapsed();
    let level = match report.gate {
        MuteGate::Failed | MuteGate::TimedOut => {
            log::Level::Warn
        }
        _ => log::Level::Info,
    };
    log::log!(
        level,
        "auto-mute: {:?} after {} ms; recording opened {} ms later",
        report.gate,
        report.waited.as_millis(),
        mute_to_record.as_millis()
    );
    (value, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Events = Arc<Mutex<Vec<&'static str>>>;

    fn events() -> Events {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn push(events: &Events, event: &'static str) {
        events.lock().unwrap().push(event);
    }

    #[test]
    fn recording_opens_only_after_the_mute_is_confirmed() {
        let log = events();
        let (mute_log, start_log) = (log.clone(), log.clone());
        let (_, report) = start_after_mute(
            Some(move || {
                std::thread::sleep(Duration::from_millis(60));
                push(&mute_log, "mute-confirmed");
                MuteOutcome::Muted
            }),
            Duration::from_secs(2),
            move || push(&start_log, "recording-open"),
        );
        assert_eq!(*log.lock().unwrap(), ["mute-confirmed", "recording-open"]);
        assert_eq!(report.gate, MuteGate::Confirmed(MuteOutcome::Muted));
        assert!(report.waited >= Duration::from_millis(60));
    }

    #[test]
    fn an_already_muted_output_counts_as_confirmed() {
        let (_, report) = start_after_mute(
            Some(|| MuteOutcome::AlreadyMuted),
            Duration::from_secs(2),
            || (),
        );
        assert!(report.gate.is_confirmed());
    }

    #[test]
    fn a_stuck_mute_times_out_and_recording_still_starts() {
        let log = events();
        let (mute_log, start_log) = (log.clone(), log.clone());
        let (release, wedged) = mpsc::channel::<()>();
        let started = Instant::now();
        let (_, report) = start_after_mute(
            Some(move || {
                let _ = wedged.recv();
                push(&mute_log, "late-mute");
                MuteOutcome::Muted
            }),
            Duration::from_millis(80),
            move || push(&start_log, "recording-open"),
        );
        let elapsed = started.elapsed();
        assert_eq!(report.gate, MuteGate::TimedOut);
        assert_eq!(*log.lock().unwrap(), ["recording-open"]);
        assert!(
            elapsed >= Duration::from_millis(80) && elapsed < Duration::from_secs(2),
            "wait must be bounded by the timeout, took {elapsed:?}"
        );
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && log.lock().unwrap().len() < 2 {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*log.lock().unwrap(), ["recording-open", "late-mute"]);
    }

    #[test]
    fn a_requested_but_unconfirmable_mute_is_not_confirmed() {
        let log = events();
        let start_log = log.clone();
        let (_, report) = start_after_mute(
            Some(|| MuteOutcome::Requested),
            Duration::from_secs(2),
            move || push(&start_log, "recording-open"),
        );
        assert_eq!(report.gate, MuteGate::Unconfirmed);
        assert!(!report.gate.is_confirmed());
        assert_eq!(*log.lock().unwrap(), ["recording-open"]);
    }

    #[test]
    fn a_failed_mute_does_not_block_recording() {
        let log = events();
        let start_log = log.clone();
        let (_, report) = start_after_mute(
            Some(|| MuteOutcome::Failed),
            Duration::from_secs(2),
            move || push(&start_log, "recording-open"),
        );
        assert_eq!(report.gate, MuteGate::Failed);
        assert!(!report.gate.is_confirmed());
        assert_eq!(*log.lock().unwrap(), ["recording-open"]);
    }

    #[test]
    fn a_panicking_mute_does_not_block_recording() {
        let (value, report) = start_after_mute(
            Some(|| -> MuteOutcome { panic!("audio backend exploded") }),
            Duration::from_secs(2),
            || 7,
        );
        assert_eq!(value, 7);
        assert!(!report.gate.is_confirmed());
    }

    #[test]
    fn disabled_mute_never_runs_and_does_not_delay_recording() {
        let log = events();
        let start_log = log.clone();
        let (value, report) = start_after_mute(
            None::<fn() -> MuteOutcome>,
            Duration::from_secs(2),
            move || {
                push(&start_log, "recording-open");
                42
            },
        );
        assert_eq!(value, 42);
        assert_eq!(report.gate, MuteGate::Skipped);
        assert_eq!(*log.lock().unwrap(), ["recording-open"]);
    }
}
