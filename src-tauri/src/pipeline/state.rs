use super::*;
use crate::core::context::ResolvedContextIdentity;
use crate::core::window_geometry::WindowTarget;
use crate::pipeline::pill_position::PillPlacement;
use std::sync::atomic::AtomicU64;

pub(super) const RETRY_WINDOW: std::time::Duration = std::time::Duration::from_secs(600);

pub(super) const CANCEL_RESUME_WINDOW: std::time::Duration = std::time::Duration::from_secs(600);

pub(super) const PASTE_FAILURE_WINDOW: std::time::Duration = std::time::Duration::from_secs(300);

fn paste_failure_is_fresh(captured_at: std::time::Instant, now: std::time::Instant) -> bool {
    now.checked_duration_since(captured_at)
        .is_some_and(|age| age < PASTE_FAILURE_WINDOW)
}

pub const DEFAULT_PILL_WIDTH_POINTS: f64 = 96.0;

pub const DEFAULT_PILL_HEIGHT_POINTS: f64 = 54.0;

pub struct AppState {
    pub lifecycle: DictationLifecycle,
    pub analytics_run_id: Option<String>,
    pub target: WindowTarget,
    pub pill_placement: Option<PillPlacement>,
    pub pill_placement_stale: bool,
    pub pill_width_points: f64,
    pub pill_height_points: f64,
    pub recording_context: Option<ResolvedContextIdentity>,
    pub retry_capture: Option<RetryCapture>,
    pub cancelled_capture: Option<CancelledCapture>,
    pub paste_failure: Option<PasteFailure>,
    pub adaptive_sensitivity_level: u8,
    pub sensitivity_retry_available: bool,
    pub sensitivity_rejected_at: Option<std::time::Instant>,
    pub failover_session_id: Option<String>,
    pub failover_reuse_id: bool,
    pub failover_started_at_unix: i64,
}

pub enum DictationLifecycle {
    Idle,
    Starting {
        prepend_audio: Option<CapturedAudio>,
    },
    Recording {
        session: audio::RecordingSession,
        mute_claim: Option<crate::media::sound::LocalMuteClaim>,
        exclusive_mic_session_id: Option<u64>,
        handless: bool,
        handless_from_hold: bool,
        prepend_audio: Option<CapturedAudio>,
    },
    Stopping {
        generation: u64,
        #[allow(dead_code)]
        prepend_audio: Option<CapturedAudio>,
    },
    Processing(ActivePipeline),
    Finalizing {
        generation: u64,
    },
}

impl DictationLifecycle {
    pub fn is_idle(&self) -> bool {
        matches!(self, DictationLifecycle::Idle)
    }
    pub fn is_recording(&self) -> bool {
        matches!(self, DictationLifecycle::Recording { .. })
    }
    pub fn is_handless_recording(&self) -> bool {
        matches!(self, DictationLifecycle::Recording { handless: true, .. })
    }
}

pub(super) fn describe_lifecycle(lifecycle: &DictationLifecycle) -> &'static str {
    match lifecycle {
        DictationLifecycle::Idle => "idle",
        DictationLifecycle::Starting { .. } => "starting",
        DictationLifecycle::Recording { handless, .. } if *handless => "recording_handsfree",
        DictationLifecycle::Recording { .. } => "recording",
        DictationLifecycle::Stopping { .. } => "stopping",
        DictationLifecycle::Processing(_) => "processing",
        DictationLifecycle::Finalizing { .. } => "finalizing",
    }
}

pub struct ActivePipeline {
    pub generation: u64,
    pub cancel_tx: tokio::sync::watch::Sender<bool>,
    pub captured_audio: CapturedAudio,
    pub context: ResolvedContextIdentity,
    #[allow(dead_code)]
    pub target: WindowTarget,
}

pub type SharedState = Arc<Mutex<AppState>>;

#[derive(Clone)]
pub struct RetryCapture {
    pub speech_vocabulary: crate::local_stt::vocabulary::Vocabulary,
    pub audio: CapturedAudio,
    pub captured_at: std::time::Instant,
    pub target: WindowTarget,
    pub process_name: String,
    pub context: ResolvedContextIdentity,
    pub profile: String,
    pub caps_lock_on: bool,
    pub paste_in_chunks: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureOrigin {
    UserCancelled,
    Interrupted,
}

impl CaptureOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureOrigin::UserCancelled => "cancelled",
            CaptureOrigin::Interrupted => "interrupted",
        }
    }
}

#[derive(Clone)]
pub struct CancelledCapture {
    pub audio: CapturedAudio,
    pub captured_at: std::time::Instant,
    pub id: String,
    pub origin: CaptureOrigin,
    pub created_at_rfc3339: String,
    pub started_at_unix: i64,
    pub target: WindowTarget,
    pub context: ResolvedContextIdentity,
}

#[derive(Clone)]
pub struct PasteFailure {
    pub text: String,
    pub captured_at: std::time::Instant,
}

pub(super) fn lock_state(state: &SharedState) -> anyhow::Result<MutexGuard<'_, AppState>> {
    state
        .lock()
        .map_err(|_| anyhow::anyhow!("Recording state lock was poisoned"))
}

pub(super) fn analytics_run_id(state: &SharedState) -> Option<String> {
    lock_state(state)
        .ok()
        .and_then(|st| st.analytics_run_id.clone())
}

fn bump_sensitivity_level(st: &mut AppState) {
    st.adaptive_sensitivity_level = st
        .adaptive_sensitivity_level
        .saturating_add(1)
        .min(MAX_ADAPTIVE_SENSITIVITY);
}

pub fn note_sensitivity_for_new_recording(state: &SharedState) -> u8 {
    let Ok(mut st) = lock_state(state) else {
        return 0;
    };

    if st.sensitivity_retry_available {
        let within_window = st
            .sensitivity_rejected_at
            .is_some_and(|rejected_at| rejected_at.elapsed() <= ADAPTIVE_SENSITIVITY_WINDOW);
        if within_window {
            bump_sensitivity_level(&mut st);
            log::debug!(
                "audio sensitivity: fresh recording counted as retry level={}",
                st.adaptive_sensitivity_level
            );
        } else {
            st.adaptive_sensitivity_level = 0;
        }
        st.sensitivity_retry_available = false;
        st.sensitivity_rejected_at = None;
    }

    st.adaptive_sensitivity_level
}

pub fn note_sensitivity_rejection(state: &SharedState) {
    if let Ok(mut st) = lock_state(state) {
        st.sensitivity_retry_available = true;
        st.sensitivity_rejected_at = Some(std::time::Instant::now());
    }
}

pub fn note_sensitivity_retry(state: &SharedState) -> bool {
    let Ok(mut st) = lock_state(state) else {
        return false;
    };
    bump_sensitivity_level(&mut st);
    st.sensitivity_retry_available = false;
    st.sensitivity_rejected_at = None;
    log::debug!(
        "audio sensitivity: explicit retry counted level={}",
        st.adaptive_sensitivity_level
    );
    true
}

pub fn current_sensitivity_level(state: &SharedState) -> u8 {
    lock_state(state)
        .map(|st| st.adaptive_sensitivity_level)
        .unwrap_or(0)
}

pub fn note_sensitivity_success(state: &SharedState) {
    if let Ok(mut st) = lock_state(state) {
        st.adaptive_sensitivity_level = 0;
        st.sensitivity_retry_available = false;
        st.sensitivity_rejected_at = None;
    }
}

static PIPELINE_GENERATION: AtomicU64 = AtomicU64::new(0);

fn next_pipeline_generation() -> u64 {
    PIPELINE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

#[cfg(all(
    debug_assertions,
    any(
        all(feature = "dev-session", desktop),
        all(feature = "android-local-testing", target_os = "android")
    )
))]
pub(super) fn reserve_provided_capture(state: &SharedState) -> Result<u64, String> {
    let mut st = lock_state(state).map_err(|e| e.to_string())?;
    if !st.lifecycle.is_idle() {
        return Err("A dictation is already running in this session".into());
    }
    let generation = next_pipeline_generation();
    st.lifecycle = DictationLifecycle::Stopping {
        generation,
        prepend_audio: None,
    };
    Ok(generation)
}

pub fn reserve_starting(state: &SharedState) -> Result<(), String> {
    let mut st = lock_state(state).map_err(|e| e.to_string())?;
    if !st.lifecycle.is_idle() {
        return Err("Already recording".to_string());
    }
    log::debug!(
        "lifecycle: {} -> starting",
        describe_lifecycle(&st.lifecycle)
    );
    st.lifecycle = DictationLifecycle::Starting {
        prepend_audio: None,
    };
    st.recording_context = None;
    Ok(())
}

pub fn peek_cancelled_capture_if_fresh(state: &SharedState) -> Option<CancelledCapture> {
    let st = lock_state(state).ok()?;
    st.cancelled_capture.as_ref().and_then(|c| {
        if c.captured_at.elapsed() < CANCEL_RESUME_WINDOW {
            Some(c.clone())
        } else {
            None
        }
    })
}

pub fn clear_cancelled_capture(state: &SharedState) {
    if let Ok(mut st) = lock_state(state) {
        st.cancelled_capture = None;
    }
}

pub fn release_expired_capture_audio(state: &SharedState) {
    fn release(audio: &mut CapturedAudio) {
        if audio.wav_len() != 0 || !audio.samples_16k.is_empty() {
            audio.clear_wav_cache();
            audio.samples_16k = Arc::new(Vec::new());
        }
    }
    if let Ok(mut st) = lock_state(state) {
        if let Some(retry) = st.retry_capture.as_mut() {
            if retry.captured_at.elapsed() > RETRY_WINDOW {
                release(&mut retry.audio);
            }
        }
        if let Some(cancelled) = st.cancelled_capture.as_mut() {
            if cancelled.captured_at.elapsed() >= CANCEL_RESUME_WINDOW {
                release(&mut cancelled.audio);
            }
        }
    }
}

pub fn set_starting_prepend_audio(state: &SharedState, audio: CapturedAudio) {
    if let Ok(mut st) = lock_state(state) {
        if let DictationLifecycle::Starting { prepend_audio } = &mut st.lifecycle {
            *prepend_audio = Some(audio);
        }
    }
}

pub fn take_active_pipeline_for_interrupt(state: &SharedState) -> Option<ActivePipeline> {
    let mut st = lock_state(state).ok()?;
    match std::mem::replace(&mut st.lifecycle, DictationLifecycle::Idle) {
        DictationLifecycle::Processing(active) => {
            crate::core::hotkey::clear_processing_generation(active.generation);
            let prepend_audio = Some(active.captured_audio.clone());
            st.recording_context = Some(active.context.clone());
            log::debug!(
                "lifecycle: processing -> starting gen={} (interrupt, audio prepended)",
                active.generation
            );
            st.lifecycle = DictationLifecycle::Starting { prepend_audio };
            Some(active)
        }
        other => {
            st.lifecycle = other;
            None
        }
    }
}

pub fn take_cancelled_capture_if_fresh(state: &SharedState) -> Option<CapturedAudio> {
    let mut st = lock_state(state).ok()?;
    st.cancelled_capture.take().and_then(|c| {
        if c.captured_at.elapsed() < CANCEL_RESUME_WINDOW {
            Some(c.audio)
        } else {
            None
        }
    })
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn take_paste_failure_if_fresh(state: &SharedState) -> Option<String> {
    take_paste_failure_if_fresh_at(state, std::time::Instant::now())
}

fn take_paste_failure_if_fresh_at(state: &SharedState, now: std::time::Instant) -> Option<String> {
    let mut st = lock_state(state).ok()?;
    st.paste_failure.take().and_then(|f| {
        if paste_failure_is_fresh(f.captured_at, now) {
            Some(f.text)
        } else {
            None
        }
    })
}

pub fn peek_paste_failure_if_fresh(state: &SharedState) -> Option<String> {
    let st = lock_state(state).ok()?;
    st.paste_failure.as_ref().and_then(|f| {
        if paste_failure_is_fresh(f.captured_at, std::time::Instant::now()) {
            Some(f.text.clone())
        } else {
            None
        }
    })
}

pub fn clear_paste_failure(state: &SharedState) {
    if let Ok(mut st) = lock_state(state) {
        st.paste_failure = None;
    }
}

pub fn take_active_pipeline_for_escape(state: &SharedState) -> Option<ActivePipeline> {
    let mut st = lock_state(state).ok()?;
    match std::mem::replace(&mut st.lifecycle, DictationLifecycle::Idle) {
        DictationLifecycle::Processing(active) => {
            crate::core::hotkey::clear_processing_generation(active.generation);
            log::debug!(
                "lifecycle: processing -> idle gen={} (escape, audio discarded)",
                active.generation
            );
            Some(active)
        }
        other => {
            st.lifecycle = other;
            None
        }
    }
}

pub fn take_recording_plain(state: &SharedState) -> Option<(audio::RecordingSession, Option<u64>)> {
    take_recording_plain_with_prepend(state)
        .map(|(session, mic_id, _prepend, _context)| (session, mic_id))
}

pub fn take_recording_plain_with_prepend(
    state: &SharedState,
) -> Option<(
    audio::RecordingSession,
    Option<u64>,
    Option<CapturedAudio>,
    ResolvedContextIdentity,
)> {
    take_recording_with_prepend(state, || {})
}

pub fn take_recording_for_cancel(
    state: &SharedState,
) -> Option<(
    audio::RecordingSession,
    Option<u64>,
    Option<CapturedAudio>,
    ResolvedContextIdentity,
)> {
    take_recording_with_prepend(state, crate::system::media_control::end_dictation_media_pause)
}

fn take_recording_with_prepend(
    state: &SharedState,
    release_media: impl FnOnce(),
) -> Option<(
    audio::RecordingSession,
    Option<u64>,
    Option<CapturedAudio>,
    ResolvedContextIdentity,
)> {
    let mut st = lock_state(state).ok()?;
    match std::mem::replace(&mut st.lifecycle, DictationLifecycle::Idle) {
        DictationLifecycle::Recording {
            session,
            mute_claim,
            exclusive_mic_session_id,
            prepend_audio,
            ..
        } => {
            release_media();
            if let Some(claim) = mute_claim {
                claim.release();
            }
            let context = st
                .recording_context
                .take()
                .unwrap_or_else(ResolvedContextIdentity::everywhere);
            log::info!("lifecycle: recording -> idle (plain cancel/discard)");
            Some((session, exclusive_mic_session_id, prepend_audio, context))
        }
        DictationLifecycle::Starting { .. } => {
            crate::media::sound::coordinated_unmute();
            st.recording_context = None;
            None
        }
        other => {
            st.lifecycle = other;
            None
        }
    }
}

pub fn cancel_starting_reservation(state: &SharedState) -> bool {
    cancel_starting_reservation_with(state, crate::media::sound::coordinated_unmute)
}

fn cancel_starting_reservation_with(state: &SharedState, release_mute: impl FnOnce()) -> bool {
    let Ok(mut st) = lock_state(state) else {
        return false;
    };
    if matches!(st.lifecycle, DictationLifecycle::Starting { .. }) {
        log::info!("lifecycle: starting -> idle (start reservation cancelled)");
        release_mute();
        st.lifecycle = DictationLifecycle::Idle;
        st.recording_context = None;
        true
    } else {
        false
    }
}

pub fn promote_recording_to_handless(state: &SharedState) -> bool {
    let Ok(mut st) = lock_state(state) else {
        return false;
    };
    if let DictationLifecycle::Recording {
        handless,
        handless_from_hold,
        ..
    } = &mut st.lifecycle
    {
        if !*handless {
            log::info!("lifecycle: recording -> recording_handsfree (Space conversion)");
            *handless = true;
            *handless_from_hold = true;
            return true;
        }
    }
    false
}

pub fn clear_handless_hold_marker(state: &SharedState) {
    if let Ok(mut st) = lock_state(state) {
        if let DictationLifecycle::Recording {
            handless: true,
            handless_from_hold,
            ..
        } = &mut st.lifecycle
        {
            *handless_from_hold = false;
        }
    }
}

pub fn consume_handless_hold_stop(state: &SharedState) -> bool {
    let Ok(mut st) = lock_state(state) else {
        return false;
    };
    if let DictationLifecycle::Recording {
        handless: true,
        handless_from_hold,
        ..
    } = &mut st.lifecycle
    {
        if *handless_from_hold {
            *handless_from_hold = false;
            return true;
        }
    }
    false
}

pub(super) type StoppingHandoff = (
    audio::RecordingSession,
    WindowTarget,
    Option<u64>,
    u64,
    Option<CapturedAudio>,
    ResolvedContextIdentity,
);

pub(super) fn take_recording_for_stopping(state: &SharedState) -> Option<StoppingHandoff> {
    let mut st = match lock_state(state) {
        Ok(st) => st,
        Err(e) => {
            log::error!("recording state: {e}");
            return None;
        }
    };
    let target = st.target.clone();
    match std::mem::replace(&mut st.lifecycle, DictationLifecycle::Idle) {
        DictationLifecycle::Recording {
            session,
            mute_claim,
            exclusive_mic_session_id,
            handless: _,
            prepend_audio,
            ..
        } => {
            if let Some(claim) = mute_claim {
                claim.release();
            }
            let recording_context = st
                .recording_context
                .take()
                .unwrap_or_else(ResolvedContextIdentity::everywhere);
            let generation = next_pipeline_generation();
            log::info!("lifecycle: recording -> stopping gen={generation}");
            st.lifecycle = DictationLifecycle::Stopping {
                generation,
                prepend_audio: prepend_audio.clone(),
            };
            Some((
                session,
                target,
                exclusive_mic_session_id,
                generation,
                prepend_audio,
                recording_context,
            ))
        }
        DictationLifecycle::Starting { .. } => {
            crate::media::sound::coordinated_unmute();
            st.recording_context = None;
            None
        }
        other => {
            st.lifecycle = other;
            None
        }
    }
}

pub(super) fn leave_stopping_if_owned(state: &SharedState, generation: u64) -> bool {
    let Ok(mut st) = lock_state(state) else {
        return false;
    };
    let owns = matches!(
        &st.lifecycle,
        DictationLifecycle::Stopping { generation: g, .. } if *g == generation
    );
    if owns {
        log::info!("lifecycle: stopping -> idle gen={generation} (rejected/discarded)");
        st.lifecycle = DictationLifecycle::Idle;
    }
    owns
}

pub(super) fn install_processing(
    state: &SharedState,
    generation: u64,
    active: ActivePipeline,
) -> bool {
    let Ok(mut st) = lock_state(state) else {
        return false;
    };
    let owns = matches!(
        &st.lifecycle,
        DictationLifecycle::Stopping { generation: g, .. } if *g == generation
    );
    if owns {
        log::info!("lifecycle: stopping -> processing gen={generation}");
        st.lifecycle = DictationLifecycle::Processing(active);
        crate::core::hotkey::set_processing_generation(generation);
    }
    owns
}

pub(super) fn enter_finalizing(state: &SharedState, generation: u64) -> bool {
    let Ok(mut st) = lock_state(state) else {
        return false;
    };
    let owns =
        matches!(&st.lifecycle, DictationLifecycle::Processing(a) if a.generation == generation);
    if owns {
        log::info!("lifecycle: processing -> finalizing gen={generation}");
        st.lifecycle = DictationLifecycle::Finalizing { generation };
        crate::core::hotkey::clear_processing_generation(generation);
    }
    owns
}

pub(super) fn leave_processing_if_owned(state: &SharedState, generation: u64) -> bool {
    let Ok(mut st) = lock_state(state) else {
        return false;
    };
    let owns =
        matches!(&st.lifecycle, DictationLifecycle::Processing(a) if a.generation == generation);
    if owns {
        log::info!("lifecycle: processing -> idle gen={generation} (failed/cancelled)");
        st.lifecycle = DictationLifecycle::Idle;
        crate::core::hotkey::clear_processing_generation(generation);
    }
    owns
}

pub(super) fn leave_finalizing(state: &SharedState, generation: u64) {
    let Ok(mut st) = lock_state(state) else {
        return;
    };
    if matches!(&st.lifecycle, DictationLifecycle::Finalizing { generation: g } if *g == generation)
    {
        log::info!("lifecycle: finalizing -> idle gen={generation}");
        st.lifecycle = DictationLifecycle::Idle;
    }
}

pub(super) fn emit_pipeline_failed(app: &AppHandle) {
    app.emit(
        "verenu:pipeline-failed",
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
    )
    .ok();
}

pub(super) fn emit_cancelled_capture(app: &AppHandle, capture: &CancelledCapture) {
    super::failover::emit_cancelled_payload(
        app,
        &capture.created_at_rfc3339,
        capture.origin.as_str(),
    );
}

pub fn emit_cancelled_capture_cleared(app: &AppHandle) {
    app.emit("verenu:cancelled-capture-cleared", ()).ok();
}

pub(super) fn emit_provider_recheck(app: &AppHandle) {
    app.emit("verenu:recheck-provider-status", ()).ok();
}

pub(super) fn emit_connectivity_recheck(app: &AppHandle) {
    app.emit("verenu:recheck-connectivity", ()).ok();
}

#[cfg_attr(not(windows), allow(unused_variables))]
pub(super) fn self_injection_requires_clipboard(app: &AppHandle, target_hwnd: usize) -> bool {
    #[cfg(windows)]
    let main_hwnd = app
        .get_webview_window("main")
        .and_then(|window| window.hwnd().ok())
        .map(|hwnd| hwnd.0 as usize);
    #[cfg(not(windows))]
    let main_hwnd = None;

    should_copy_self_target(
        target_hwnd,
        main_hwnd,
        foreground_is_own_process(),
        hwnd_is_own_process(target_hwnd),
    )
}

fn should_copy_self_target(
    target_hwnd: usize,
    main_hwnd: Option<usize>,
    foreground_owned: bool,
    target_owned: bool,
) -> bool {
    let captured_main = target_hwnd != 0 && main_hwnd == Some(target_hwnd);
    !captured_main && (foreground_owned || target_owned)
}

pub(super) fn foreground_is_own_process() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == std::process::id()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg_attr(not(windows), allow(unused_variables))]
pub(super) fn hwnd_is_own_process(hwnd: usize) -> bool {
    if hwnd == 0 {
        return false;
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
        let mut pid = 0u32;
        GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
        pid == std::process::id()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_window_dictation_uses_normal_injection() {
        for foreground_owned in [false, true] {
            assert!(!should_copy_self_target(
                42,
                Some(42),
                foreground_owned,
                true
            ));
        }
    }

    #[test]
    fn incidental_self_targets_keep_clipboard_fallback() {
        assert!(should_copy_self_target(43, Some(42), true, true));
        assert!(should_copy_self_target(43, Some(42), false, true));
        assert!(should_copy_self_target(99, Some(42), true, false));
        assert!(should_copy_self_target(42, None, true, true));
        assert!(should_copy_self_target(0, Some(0), true, false));
        assert!(!should_copy_self_target(99, Some(42), false, false));
    }

    fn fresh_state() -> SharedState {
        Arc::new(Mutex::new(AppState {
            lifecycle: DictationLifecycle::Idle,
            analytics_run_id: None,
            target: WindowTarget::default(),
            pill_placement: None,
            pill_placement_stale: false,
            pill_width_points: DEFAULT_PILL_WIDTH_POINTS,
            pill_height_points: DEFAULT_PILL_HEIGHT_POINTS,
            recording_context: None,
            retry_capture: None,
            cancelled_capture: None,
            paste_failure: None,
            adaptive_sensitivity_level: 0,
            sensitivity_retry_available: false,
            sensitivity_rejected_at: None,
            failover_session_id: None,
            failover_reuse_id: false,
            failover_started_at_unix: 0,
        }))
    }

    fn fake_audio(duration_ms: u64) -> CapturedAudio {
        CapturedAudio {
            wav_cache: Arc::new(Mutex::new(None)),
            samples_16k: Arc::new(vec![0.0; 16]),
            sample_rate: 16_000,
            duration_ms,
        }
    }

    fn fake_active(generation: u64) -> ActivePipeline {
        let (cancel_tx, _rx) = tokio::sync::watch::channel(false);
        ActivePipeline {
            generation,
            cancel_tx,
            captured_audio: fake_audio(1000),
            context: ResolvedContextIdentity::everywhere(),
            target: WindowTarget::default(),
        }
    }

    #[test]
    fn reserve_starting_fails_unless_idle() {
        let state = fresh_state();
        assert!(reserve_starting(&state).is_ok());
        assert!(reserve_starting(&state).is_err());
    }

    #[test]
    fn starting_reservation_clears_stale_recording_context() {
        let state = fresh_state();
        {
            let mut st = lock_state(&state).expect("state lock");
            st.recording_context = Some(ResolvedContextIdentity {
                t3_skills: None,
                website_domain: None,
                id: 42,
                label: "Old context".into(),
            });
        }

        reserve_starting(&state).expect("reserve starting");
        assert!(lock_state(&state)
            .expect("state lock")
            .recording_context
            .is_none());
    }

    #[test]
    fn quick_follow_up_recording_counts_as_a_sensitivity_retry() {
        let state = fresh_state();
        note_sensitivity_rejection(&state);
        assert_eq!(note_sensitivity_for_new_recording(&state), 1);

        note_sensitivity_rejection(&state);
        assert_eq!(note_sensitivity_for_new_recording(&state), 2);
        assert_eq!(current_sensitivity_level(&state), 2);

        note_sensitivity_success(&state);
        assert_eq!(current_sensitivity_level(&state), 0);
    }

    #[test]
    fn explicit_retry_increases_sensitivity_and_is_bounded() {
        let state = fresh_state();
        for expected in 1..=MAX_ADAPTIVE_SENSITIVITY {
            assert!(note_sensitivity_retry(&state));
            assert_eq!(current_sensitivity_level(&state), expected);
        }
        assert_eq!(current_sensitivity_level(&state), MAX_ADAPTIVE_SENSITIVITY);
    }

    #[test]
    fn late_follow_up_recording_drops_the_pending_boost() {
        let state = fresh_state();
        {
            let mut st = lock_state(&state).unwrap();
            st.adaptive_sensitivity_level = 2;
            st.sensitivity_retry_available = true;
            st.sensitivity_rejected_at = Some(
                std::time::Instant::now()
                    - ADAPTIVE_SENSITIVITY_WINDOW
                    - std::time::Duration::from_millis(1),
            );
        }
        assert_eq!(note_sensitivity_for_new_recording(&state), 0);
    }

    #[test]
    fn peek_cancelled_capture_does_not_consume() {
        let state = fresh_state();
        let context = ResolvedContextIdentity {
            t3_skills: None,
            website_domain: None,
            id: 7,
            label: "Development".into(),
        };
        {
            let mut st = lock_state(&state).unwrap();
            st.cancelled_capture = Some(CancelledCapture {
                audio: fake_audio(500),
                captured_at: std::time::Instant::now(),
                id: "test-id".into(),
                origin: CaptureOrigin::UserCancelled,
                created_at_rfc3339: "2026-01-01T00:00:00Z".into(),
                started_at_unix: 0,
                target: WindowTarget::default(),
                context: context.clone(),
            });
        }
        let peeked = peek_cancelled_capture_if_fresh(&state).unwrap();
        assert_eq!(peeked.audio.duration_ms, 500);
        assert_eq!(peeked.context, context);
        assert!(lock_state(&state).unwrap().cancelled_capture.is_some());
    }

    #[test]
    fn clear_cancelled_capture_removes_the_slot() {
        let state = fresh_state();
        {
            let mut st = lock_state(&state).unwrap();
            st.cancelled_capture = Some(CancelledCapture {
                audio: fake_audio(500),
                captured_at: std::time::Instant::now(),
                id: "test-id".into(),
                origin: CaptureOrigin::UserCancelled,
                created_at_rfc3339: "2026-01-01T00:00:00Z".into(),
                started_at_unix: 0,
                target: WindowTarget::default(),
                context: ResolvedContextIdentity::everywhere(),
            });
        }
        clear_cancelled_capture(&state);
        assert!(lock_state(&state).unwrap().cancelled_capture.is_none());
    }

    #[test]
    fn expired_audio_is_released_without_consuming_fresh_or_live_owners() {
        let state = fresh_state();
        let audio = fake_audio(500);
        let live_owner = audio.samples_16k.clone();
        {
            let mut st = lock_state(&state).unwrap();
            st.cancelled_capture = Some(CancelledCapture {
                audio: audio.clone(),
                captured_at: std::time::Instant::now(),
                id: "expiry-test".into(),
                origin: CaptureOrigin::UserCancelled,
                created_at_rfc3339: "2026-01-01T00:00:00Z".into(),
                started_at_unix: 0,
                target: WindowTarget::default(),
                context: ResolvedContextIdentity::everywhere(),
            });
            st.retry_capture = Some(RetryCapture {
                speech_vocabulary: Default::default(),
                audio,
                captured_at: std::time::Instant::now()
                    - RETRY_WINDOW
                    - std::time::Duration::from_secs(1),
                target: WindowTarget::default(),
                process_name: String::new(),
                context: ResolvedContextIdentity {
                    t3_skills: None,
                    website_domain: None,
                    id: 1,
                    label: "Everywhere".into(),
                },
                profile: String::new(),
                caps_lock_on: false,
                paste_in_chunks: false,
            });
        }
        release_expired_capture_audio(&state);
        {
            let mut st = lock_state(&state).unwrap();
            assert!(st
                .retry_capture
                .as_ref()
                .unwrap()
                .audio
                .samples_16k
                .is_empty());
            let capture = st.cancelled_capture.as_mut().unwrap();
            assert_eq!(capture.audio.samples_16k.len(), 16);
            capture.captured_at = std::time::Instant::now() - CANCEL_RESUME_WINDOW;
        }
        release_expired_capture_audio(&state);
        assert_eq!(Arc::strong_count(&live_owner), 1);
        assert_eq!(live_owner.len(), 16);
        assert!(peek_cancelled_capture_if_fresh(&state).is_none());
        assert!(lock_state(&state).unwrap().retry_capture.is_some());
    }

    #[test]
    fn set_starting_prepend_audio_attaches_to_reserved_start() {
        let state = fresh_state();
        reserve_starting(&state).unwrap();
        set_starting_prepend_audio(&state, fake_audio(500));
        match &lock_state(&state).unwrap().lifecycle {
            DictationLifecycle::Starting { prepend_audio } => {
                assert_eq!(prepend_audio.as_ref().unwrap().duration_ms, 500)
            }
            _ => panic!("expected Starting with prepend_audio"),
        };
    }

    #[test]
    fn take_paste_failure_if_fresh_returns_text_once() {
        let state = fresh_state();
        {
            let mut st = lock_state(&state).unwrap();
            st.paste_failure = Some(PasteFailure {
                text: "hello world".to_string(),
                captured_at: std::time::Instant::now(),
            });
        }
        assert_eq!(
            take_paste_failure_if_fresh(&state),
            Some("hello world".to_string())
        );
        assert_eq!(take_paste_failure_if_fresh(&state), None);
    }

    #[test]
    fn take_paste_failure_if_fresh_expires_after_window() {
        let state = fresh_state();
        let captured_at = std::time::Instant::now();
        let now = captured_at + PASTE_FAILURE_WINDOW + std::time::Duration::from_secs(1);
        {
            let mut st = lock_state(&state).unwrap();
            st.paste_failure = Some(PasteFailure {
                text: "stale".to_string(),
                captured_at,
            });
        }
        assert_eq!(take_paste_failure_if_fresh_at(&state, now), None);
    }

    #[test]
    fn take_recording_plain_cancels_a_pending_start() {
        let state = fresh_state();
        reserve_starting(&state).unwrap();

        assert!(take_recording_plain(&state).is_none());
        assert!(lock_state(&state).unwrap().lifecycle.is_idle());
    }

    #[test]
    fn take_recording_for_stopping_cancels_a_pending_start() {
        let state = fresh_state();
        reserve_starting(&state).unwrap();

        assert!(take_recording_for_stopping(&state).is_none());
        assert!(lock_state(&state).unwrap().lifecycle.is_idle());
    }

    #[test]
    fn cancel_starting_reservation_does_not_touch_other_lifecycle() {
        let state = fresh_state();
        assert!(!cancel_starting_reservation(&state));
        assert!(lock_state(&state).unwrap().lifecycle.is_idle());

        reserve_starting(&state).unwrap();
        assert!(cancel_starting_reservation(&state));
        assert!(lock_state(&state).unwrap().lifecycle.is_idle());
    }

    #[test]
    fn cancelling_start_releases_mute_before_unlocking_the_reservation() {
        let state = fresh_state();
        reserve_starting(&state).unwrap();
        let released = std::cell::Cell::new(false);
        assert!(cancel_starting_reservation_with(&state, || {
            assert!(state.try_lock().is_err());
            released.set(true);
        }));
        assert!(released.get());
        assert!(lock_state(&state).unwrap().lifecycle.is_idle());
        assert!(!cancel_starting_reservation_with(&state, || panic!(
            "Idle has no pending start"
        )));
    }

    #[test]
    fn old_opener_error_preserves_replacement_start_reservation() {
        let state = fresh_state();
        reserve_starting(&state).unwrap();
        let old = crate::media::sound::claim_local_mute();
        let _replacement = crate::media::sound::claim_local_mute();
        crate::pipeline::session::release_starting_reservation_for_claim(&state, Some(old));
        assert!(matches!(lock_state(&state).unwrap().lifecycle, DictationLifecycle::Starting { .. }));
    }

    #[test]
    fn cancellation_releases_media_before_exposing_idle() {
        let state = fresh_state();
        lock_state(&state).unwrap().lifecycle = DictationLifecycle::Recording {
            session: audio::RecordingSession::lifecycle_fixture(),
            mute_claim: None,
            exclusive_mic_session_id: None,
            handless: false,
            handless_from_hold: false,
            prepend_audio: None,
        };
        let released = std::cell::Cell::new(false);
        assert!(take_recording_with_prepend(&state, || {
            assert!(state.try_lock().is_err());
            released.set(true);
        }).is_some());
        assert!(released.get());
        assert!(lock_state(&state).unwrap().lifecycle.is_idle());
    }

    #[test]
    fn cancellation_without_recording_does_not_release_media() {
        let state = fresh_state();
        assert!(take_recording_with_prepend(&state, || panic!("idle owns no recording pause"))
            .is_none());
        reserve_starting(&state).unwrap();
        assert!(take_recording_with_prepend(&state, || panic!("starting owns no recording pause"))
            .is_none());
        reserve_starting(&state).unwrap();
        assert!(matches!(
            lock_state(&state).unwrap().lifecycle,
            DictationLifecycle::Starting { .. }
        ));
        lock_state(&state).unwrap().lifecycle = DictationLifecycle::Processing(fake_active(1));
        assert!(take_recording_with_prepend(&state, || panic!("processing retains its own pause"))
            .is_none());
        assert!(matches!(
            lock_state(&state).unwrap().lifecycle,
            DictationLifecycle::Processing(_)
        ));
    }

    #[test]
    fn taking_recording_releases_its_mute_before_exposing_a_new_start() {
        for transcribing in [false, true] {
            let state = fresh_state();
            let old = crate::media::sound::claim_local_mute();
            lock_state(&state).unwrap().lifecycle = DictationLifecycle::Recording {
                session: audio::RecordingSession::lifecycle_fixture(),
                mute_claim: Some(old),
                exclusive_mic_session_id: None,
                handless: false,
                handless_from_hold: false,
                prepend_audio: None,
            };
            if transcribing {
                assert!(take_recording_for_stopping(&state).is_some());
            } else {
                assert!(take_recording_plain(&state).is_some());
            }
            assert!(
                !old.is_current(),
                "take must release before returning the session"
            );
            if transcribing {
                assert!(matches!(
                    lock_state(&state).unwrap().lifecycle,
                    DictationLifecycle::Stopping { .. }
                ));
            } else {
                reserve_starting(&state).unwrap();
                let _replacement = crate::media::sound::claim_local_mute();
                old.release();
                assert!(matches!(
                    lock_state(&state).unwrap().lifecycle,
                    DictationLifecycle::Starting { .. }
                ));
            }
        }
    }

    #[test]
    fn interrupt_takes_processing_and_installs_starting_with_prepend_audio() {
        let state = fresh_state();
        let context = ResolvedContextIdentity {
            t3_skills: None,
            website_domain: None,
            id: 8,
            label: "Development".into(),
        };
        {
            let mut st = lock_state(&state).unwrap();
            let mut active = fake_active(1);
            active.context = context.clone();
            st.lifecycle = DictationLifecycle::Processing(active);
        }
        let taken = take_active_pipeline_for_interrupt(&state);
        assert!(taken.is_some());
        assert_eq!(lock_state(&state).unwrap().recording_context, Some(context));
        let st = lock_state(&state).unwrap();
        match &st.lifecycle {
            DictationLifecycle::Starting { prepend_audio } => {
                assert!(prepend_audio.is_some());
            }
            _ => panic!("expected Starting with prepend_audio"),
        }
    }

    #[test]
    fn interrupt_is_a_noop_outside_processing() {
        let state = fresh_state();
        assert!(take_active_pipeline_for_interrupt(&state).is_none());
        let st = lock_state(&state).unwrap();
        assert!(st.lifecycle.is_idle());
    }

    #[test]
    fn escape_takes_processing_and_discards_audio_into_idle() {
        let state = fresh_state();
        let context = ResolvedContextIdentity {
            t3_skills: None,
            website_domain: None,
            id: 9,
            label: "Writing".into(),
        };
        {
            let mut st = lock_state(&state).unwrap();
            let mut active = fake_active(1);
            active.context = context.clone();
            st.lifecycle = DictationLifecycle::Processing(active);
        }
        let taken = take_active_pipeline_for_escape(&state);
        assert_eq!(taken.as_ref().map(|active| &active.context), Some(&context));
        let st = lock_state(&state).unwrap();
        assert!(st.lifecycle.is_idle());
    }

    #[test]
    fn install_processing_rejects_stale_generation() {
        let state = fresh_state();
        {
            let mut st = lock_state(&state).unwrap();
            st.lifecycle = DictationLifecycle::Stopping {
                generation: 5,
                prepend_audio: None,
            };
        }
        assert!(!install_processing(&state, 4, fake_active(4)));
        let st = lock_state(&state).unwrap();
        assert!(matches!(
            &st.lifecycle,
            DictationLifecycle::Stopping { generation: 5, .. }
        ));
    }

    #[test]
    fn install_processing_succeeds_for_matching_generation() {
        let state = fresh_state();
        {
            let mut st = lock_state(&state).unwrap();
            st.lifecycle = DictationLifecycle::Stopping {
                generation: 7,
                prepend_audio: None,
            };
        }
        assert!(install_processing(&state, 7, fake_active(7)));
        let st = lock_state(&state).unwrap();
        assert!(matches!(&st.lifecycle, DictationLifecycle::Processing(a) if a.generation == 7));
    }

    #[test]
    fn enter_finalizing_and_leave_processing_are_generation_checked() {
        let state = fresh_state();
        {
            let mut st = lock_state(&state).unwrap();
            st.lifecycle = DictationLifecycle::Processing(fake_active(9));
        }
        assert!(!leave_processing_if_owned(&state, 8));
        assert!(!enter_finalizing(&state, 8));
        {
            let st = lock_state(&state).unwrap();
            assert!(
                matches!(&st.lifecycle, DictationLifecycle::Processing(a) if a.generation == 9)
            );
        }
        assert!(enter_finalizing(&state, 9));
        let st = lock_state(&state).unwrap();
        assert!(matches!(
            &st.lifecycle,
            DictationLifecycle::Finalizing { generation: 9 }
        ));
    }

    #[test]
    fn leave_finalizing_only_clears_matching_generation() {
        let state = fresh_state();
        {
            let mut st = lock_state(&state).unwrap();
            st.lifecycle = DictationLifecycle::Finalizing { generation: 3 };
        }
        leave_finalizing(&state, 2);
        {
            let st = lock_state(&state).unwrap();
            assert!(matches!(
                &st.lifecycle,
                DictationLifecycle::Finalizing { generation: 3 }
            ));
        }
        leave_finalizing(&state, 3);
        let st = lock_state(&state).unwrap();
        assert!(st.lifecycle.is_idle());
    }

    #[test]
    fn lifecycle_idle_reflects_state() {
        let state = fresh_state();
        assert!(lock_state(&state).unwrap().lifecycle.is_idle());
        reserve_starting(&state).unwrap();
        assert!(!lock_state(&state).unwrap().lifecycle.is_idle());
    }
}
