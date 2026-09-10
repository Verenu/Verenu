//! Optional Windows dictation trigger driven by the selected microphone's mute
//! button. Watches, in parallel:
//! - endpoint (mixer) mute bit
//! - hardware mute subunits on the capture topology path
//! - digital-silence PCM when hardware mute never flips a Windows mute bit
//!
//! Binds the selected capture device with relaxed name matching so WASAPI and
//! CPAL labels for the same mic still resolve. Never calls SetMute.

#[cfg(windows)]
mod win {
    use crate::data::store;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum DetectionMethod {
        EndpointNotify,
        EndpointPoll,
        HardwareMute,
        DigitalSilence,
    }

    impl DetectionMethod {
        fn as_str(self) -> &'static str {
            match self {
                Self::EndpointNotify => "endpoint_notify",
                Self::EndpointPoll => "endpoint_poll",
                Self::HardwareMute => "hardware_mute",
                Self::DigitalSilence => "digital_silence",
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum MuteTriggerEvent {
        BecameMuted { method: DetectionMethod },
        BecameUnmuted { method: DetectionMethod },
    }
    use crate::core::window_geometry::WindowTarget;
    use crate::media::digital_silence::{DigitalSilenceDetector, MuteDebouncer, SilenceTransition};
    use crate::pipeline::{self, start_recording_session, SharedState};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use tauri::AppHandle;
    use windows::core::{implement, GUID};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::Endpoints::{
        IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
    };
    use windows::Win32::Media::Audio::{
        eCapture, eConsole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
        AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE_ACTIVE, ENDPOINT_HARDWARE_SUPPORT_MUTE,
    };
    use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

    const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };
    const PKEY_DEVICE_DEVICE_DESC: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 2,
    };

    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const REBIND_INTERVAL: Duration = Duration::from_secs(2);
    const DISABLED_IDLE: Duration = Duration::from_secs(2);
    const DEBOUNCE: Duration = Duration::from_millis(25);
    /// Ignore PCM unmute/mute if endpoint already reported a matching transition
    /// within this window — endpoint path wins.
    const ENDPOINT_PRIORITY_WINDOW: Duration = Duration::from_millis(500);
    /// A dictation gesture is unmuted → muted → unmuted. The mute→unmute half
    /// must land inside this window or it is treated as an ordinary unmute.
    const PULSE_MIN: Duration = Duration::from_millis(10);
    const PULSE_MAX: Duration = Duration::from_millis(3000);
    /// PCM mute detection window. 250ms made a physical mute click feel like
    /// it needed a half-second hold before unmute would count.
    const PCM_WINDOW: Duration = Duration::from_millis(40);
    const PCM_DEBOUNCE: Duration = Duration::from_millis(15);

    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static STARTED: AtomicBool = AtomicBool::new(false);
    static WATCHER_THREAD: Mutex<Option<std::thread::Thread>> = Mutex::new(None);
    static EVENT_TX: OnceLock<
        std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<MuteTriggerEvent>>>,
    > = OnceLock::new();
    /// Instant the selected mic last entered the muted bit, used to recognize
    /// a mute→unmute pulse as a hands-free toggle.
    static PULSE_MUTED_AT: Mutex<Option<Instant>> = Mutex::new(None);
    /// Guards against endpoint+PCM both delivering the same unmute edge and
    /// starting then immediately stopping dictation.
    static LAST_PULSE_AT: Mutex<Option<Instant>> = Mutex::new(None);
    /// Idle digital-silence capture client. Taken/dropped before dictation
    /// opens the same mic so the streams never overlap.
    static ACTIVE_PCM: Mutex<Option<PcmMonitorGuard>> = Mutex::new(None);
    const PULSE_COOLDOWN: Duration = Duration::from_millis(180);

    fn release_active_pcm() {
        let guard = ACTIVE_PCM.lock().ok().and_then(|mut slot| slot.take());
        if guard.is_some() {
            log::info!("mic_mute_trigger: PCM monitor released");
        }
        // Join happens in Drop outside the mutex.
        drop(guard);
    }

    fn pcm_is_running() -> bool {
        let Ok(slot) = ACTIVE_PCM.lock() else {
            return false;
        };
        slot.as_ref().is_some_and(|guard| {
            guard
                .join
                .as_ref()
                .is_some_and(|handle| !handle.is_finished())
        })
    }

    fn install_active_pcm(guard: PcmMonitorGuard) {
        let previous = if let Ok(mut slot) = ACTIVE_PCM.lock() {
            let old = slot.take();
            *slot = Some(guard);
            old
        } else {
            None
        };
        // Join outside the mutex — Drop may block on JoinHandle::join.
        drop(previous);
    }

    fn event_tx_slot(
    ) -> &'static std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<MuteTriggerEvent>>>
    {
        EVENT_TX.get_or_init(|| std::sync::Mutex::new(None))
    }

    pub fn setup(app: &mut tauri::App, shared: SharedState) {
        if STARTED.swap(true, Ordering::SeqCst) {
            reload(app.handle());
            return;
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<MuteTriggerEvent>();
        if let Ok(mut slot) = event_tx_slot().lock() {
            *slot = Some(tx);
        }

        let app_handle = app.handle().clone();
        let state_watch = shared.clone();
        std::thread::Builder::new()
            .name("mic-mute-trigger".into())
            .spawn(move || watcher_loop(app_handle, state_watch))
            .expect("spawn mic-mute-trigger thread");

        let app_hk = app.handle().clone();
        let state_hk = shared;
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                // Joining the idle PCM WASAPI client can take tens of ms.
                // Do that on a blocking pool thread before unmute dispatch may
                // open the dictation capture stream on this Tokio worker.
                if matches!(event, MuteTriggerEvent::BecameUnmuted { .. }) {
                    let _ = tauri::async_runtime::spawn_blocking(release_active_pcm).await;
                }
                dispatch_event(&app_hk, &state_hk, event);
            }
        });

        // Kick an initial bind attempt for the current setting.
        GENERATION.fetch_add(1, Ordering::SeqCst);
        log::info!("mic_mute_trigger: watcher started");
    }

    pub fn reload(app: &AppHandle) {
        let _ = app;
        let gen = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        log::info!("mic_mute_trigger: reload requested (generation={gen})");
        if let Ok(slot) = WATCHER_THREAD.lock() {
            if let Some(thread) = slot.as_ref() {
                thread.unpark();
            }
        }
    }

    fn emit(event: MuteTriggerEvent) {
        let Ok(slot) = event_tx_slot().lock() else {
            return;
        };
        if let Some(tx) = slot.as_ref() {
            let _ = tx.send(event);
        }
    }

    fn feature_enabled(app: &AppHandle) -> bool {
        store::settings_handle(app)
            .ok()
            .and_then(|s| s.get(store::MIC_MUTE_BUTTON_DICTATION))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn selected_device_name(app: &AppHandle) -> Option<String> {
        store::settings_handle(app)
            .ok()
            .and_then(|s| s.get(store::MICROPHONE_DEVICE))
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .filter(|s| !s.trim().is_empty())
    }

    fn take_pulse(now: Instant) -> Option<Duration> {
        let Ok(mut slot) = PULSE_MUTED_AT.lock() else {
            return None;
        };
        let muted_at = slot.take()?;
        let elapsed = now.saturating_duration_since(muted_at);
        if elapsed < PULSE_MIN || elapsed > PULSE_MAX {
            log::info!(
                "mic_mute_trigger: ignored unmute — not a mute→unmute pulse (held {elapsed:?}, want {PULSE_MIN:?}..{PULSE_MAX:?})"
            );
            return None;
        }
        Some(elapsed)
    }

    fn mark_pulse_muted(now: Instant) {
        if let Ok(mut slot) = PULSE_MUTED_AT.lock() {
            *slot = Some(now);
        }
    }

    fn clear_pulse() {
        if let Ok(mut slot) = PULSE_MUTED_AT.lock() {
            *slot = None;
        }
    }

    fn dispatch_event(app: &AppHandle, state: &SharedState, event: MuteTriggerEvent) {
        if !feature_enabled(app) {
            log::debug!("mic_mute_trigger: ignored event — feature disabled");
            clear_pulse();
            return;
        }

        match event {
            MuteTriggerEvent::BecameMuted { method } => {
                // Arm the pulse only. Stopping on mute alone leaves the mic
                // muted mid-gesture and fires before the unmute half arrives.
                mark_pulse_muted(Instant::now());
                log::info!(
                    "mic_mute_trigger: muted via {} — pulse armed (waiting for unmute)",
                    method.as_str()
                );
            }
            MuteTriggerEvent::BecameUnmuted { method } => {
                let now = Instant::now();
                let Some(held) = take_pulse(now) else {
                    return;
                };
                if let Ok(mut last) = LAST_PULSE_AT.lock() {
                    if let Some(prev) = *last {
                        if now.saturating_duration_since(prev) < PULSE_COOLDOWN {
                            log::info!(
                                "mic_mute_trigger: ignored pulse via {} — cooldown after prior pulse",
                                method.as_str()
                            );
                            return;
                        }
                    }
                    *last = Some(now);
                }
                log::info!(
                    "mic_mute_trigger: mute→unmute pulse via {} (held {held:?}) — toggling hands-free",
                    method.as_str()
                );

                let recording = {
                    let Ok(st) = state.lock() else {
                        log::error!("mic_mute_trigger: state lock poisoned");
                        return;
                    };
                    st.lifecycle.is_recording()
                };

                if recording {
                    crate::core::hotkey::set_handless_active(false);
                    tauri::async_runtime::spawn(pipeline::run_pipeline(app.clone(), state.clone()));
                    log::info!("mic_mute_trigger: dictation stop requested");
                    return;
                }

                let busy = {
                    let Ok(st) = state.lock() else {
                        log::error!("mic_mute_trigger: state lock poisoned");
                        return;
                    };
                    !matches!(st.lifecycle, pipeline::DictationLifecycle::Idle)
                };
                if busy {
                    log::info!("mic_mute_trigger: ignored pulse — lifecycle busy");
                    return;
                }
                if pipeline::reserve_starting(state).is_err() {
                    log::info!("mic_mute_trigger: ignored pulse — could not reserve starting");
                    return;
                }
                // Idle PCM was already released on the blocking pool before
                // this unmute dispatch ran.
                let target = WindowTarget::capture_foreground();
                if let Ok(mut st) = state.lock() {
                    st.target = target;
                    st.pill_placement_stale = true;
                }
                // Use the normal recording capsule (waveform), not the
                // hands-free Cancel/Confirm chrome — mute-pulse dictation
                // doesn't need those buttons and that UI read as "broken".
                start_recording_session(app, state, "handsfree", true);
                crate::core::hotkey::set_handless_active(true);
                log::info!("mic_mute_trigger: dictation started (handsfree)");
            }
        }
    }

    fn dictation_holds_mic(state: &SharedState) -> bool {
        let Ok(st) = state.lock() else {
            return false;
        };
        !st.lifecycle.is_idle()
    }

    fn recording_raw_level(state: &SharedState) -> Option<f32> {
        let Ok(st) = state.lock() else {
            return None;
        };
        match &st.lifecycle {
            pipeline::DictationLifecycle::Recording { session, .. } => {
                Some(f32::from_bits(session.raw_level.load(Ordering::Relaxed)))
            }
            _ => None,
        }
    }

    fn watcher_loop(app: AppHandle, state: SharedState) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        if let Ok(mut slot) = WATCHER_THREAD.lock() {
            *slot = Some(std::thread::current());
        }

        loop {
            let gen = GENERATION.load(Ordering::SeqCst);
            if !feature_enabled(&app) {
                // No GetMute / PCM work while the setting is off. Park until
                // save_setting calls reload(), with a long timeout as a backstop.
                std::thread::park_timeout(DISABLED_IDLE);
                continue;
            }

            let desired = selected_device_name(&app);
            match bind_and_watch(&app, &state, desired.as_deref(), gen) {
                Ok(()) => {}
                Err(err) => {
                    log::warn!("mic_mute_trigger: bind failed: {err}");
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }

    fn bind_and_watch(
        app: &AppHandle,
        state: &SharedState,
        desired_name: Option<&str>,
        gen: u64,
    ) -> Result<(), String> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|e| format!("MMDeviceEnumerator: {e}"))?;

        let (device, device_id, friendly) = resolve_capture_device(&enumerator, desired_name)?;
        let volume: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }
            .map_err(|e| format!("Activate IAudioEndpointVolume: {e}"))?;

        let hw_support = unsafe { volume.QueryHardwareSupport() }.unwrap_or(0);
        let hw_mute = (hw_support & ENDPOINT_HARDWARE_SUPPORT_MUTE) != 0;
        let mut topology = crate::media::hardware_mute::TopologyMuteWatch::from_device(&device);
        let mut topo_debouncer = MuteDebouncer::new(DEBOUNCE);
        topo_debouncer.seed(topology.any_muted().unwrap_or(false));
        let mut topo_held = topology.any_muted();
        log::info!(
            "mic_mute_trigger: watching device '{friendly}' id={device_id} hw_support=0x{hw_support:x} endpoint_hw_mute={hw_mute} topology_mutes={}",
            topology.control_count()
        );

        let initial_muted = unsafe { volume.GetMute() }
            .map(|v| v.as_bool())
            .unwrap_or(false);

        // Shared raw mute bit: notify callback writes, watch loop debounces once.
        let raw_muted = Arc::new(Mutex::new(EndpointRawMute {
            muted: initial_muted,
            source: DetectionMethod::EndpointPoll,
            changed: false,
        }));
        let last_endpoint_event = Arc::new(Mutex::new(None::<(bool, Instant)>));

        let callback = MuteCallback {
            raw: Arc::clone(&raw_muted),
        };
        let callback_iface: IAudioEndpointVolumeCallback = callback.into();
        unsafe {
            volume
                .RegisterControlChangeNotify(&callback_iface)
                .map_err(|e| format!("RegisterControlChangeNotify: {e}"))?;
        }
        log::info!(
            "mic_mute_trigger: endpoint mute watcher bound (initial_muted={initial_muted}, seeded without firing)"
        );

        // PCM digital-silence covers mute buttons that never flip GetMute
        // (common on USB mics even when QueryHardwareSupport advertises mute).
        // Skip it only when the capture path already exposes IAudioMute.
        // Always join/drop the stream before dictation so a second WASAPI
        // client cannot starve the pill visualizer.
        let need_pcm = topology.control_count() == 0;
        if need_pcm {
            log::info!(
                "mic_mute_trigger: enabling digital-silence PCM fallback (no topology mute; endpoint_hw_mute={hw_mute})"
            );
        } else {
            log::info!(
                "mic_mute_trigger: skipping idle PCM monitor (topology_mutes={})",
                topology.control_count()
            );
        }
        let mut level_debouncer = MuteDebouncer::new(DEBOUNCE);
        let mut level_seeded = false;
        let mut last_pcm_start_at: Option<Instant> = None;
        const PCM_RETRY: Duration = Duration::from_secs(5);

        let mut debouncer = MuteDebouncer::new(DEBOUNCE);
        debouncer.seed(initial_muted);
        let mut last_rebind_check = Instant::now();
        let started_gen = gen;

        loop {
            if GENERATION.load(Ordering::SeqCst) != started_gen {
                log::info!("mic_mute_trigger: generation changed — rebinding");
                break;
            }
            if !feature_enabled(app) {
                log::info!("mic_mute_trigger: feature disabled — releasing watch");
                clear_pulse();
                break;
            }

            if last_rebind_check.elapsed() >= REBIND_INTERVAL {
                last_rebind_check = Instant::now();
                let current_desired = selected_device_name(app);
                match resolve_capture_device(&enumerator, current_desired.as_deref()) {
                    Ok((_, id, name)) if id != device_id || name != friendly => {
                        log::info!(
                            "mic_mute_trigger: device changed ('{friendly}' -> '{name}') — rebinding"
                        );
                        break;
                    }
                    Err(err) => {
                        log::warn!("mic_mute_trigger: device resolve failed during watch: {err}");
                        break;
                    }
                    Ok(_) => {}
                }
            }

            let holding_mic = dictation_holds_mic(state);
            if holding_mic {
                if pcm_is_running() {
                    release_active_pcm();
                    log::info!("mic_mute_trigger: PCM monitor paused (dictation holds the mic)");
                }
                // Allow idle PCM to restart immediately after this dictation.
                last_pcm_start_at = None;
                if let Some(level) = recording_raw_level(state) {
                    let silent = level <= crate::media::digital_silence::DIGITAL_SILENCE_EPS * 4.0;
                    if !level_seeded {
                        level_debouncer.seed(silent);
                        level_seeded = true;
                    } else if let Some(stable) = level_debouncer.observe(silent, Instant::now()) {
                        let recent_endpoint = last_endpoint_event
                            .lock()
                            .ok()
                            .and_then(|last| *last)
                            .is_some_and(|(muted, at)| {
                                muted == stable
                                    && Instant::now().saturating_duration_since(at)
                                        <= ENDPOINT_PRIORITY_WINDOW
                            });
                        if !recent_endpoint {
                            log::info!(
                                "mic_mute_trigger: {} via recording-level (rms={level:.6})",
                                if stable { "muted" } else { "unmuted" }
                            );
                            emit(if stable {
                                MuteTriggerEvent::BecameMuted {
                                    method: DetectionMethod::DigitalSilence,
                                }
                            } else {
                                MuteTriggerEvent::BecameUnmuted {
                                    method: DetectionMethod::DigitalSilence,
                                }
                            });
                        }
                    }
                }
            } else {
                level_seeded = false;
                if need_pcm && !pcm_is_running() {
                    let can_retry = last_pcm_start_at
                        .map(|at| at.elapsed() >= PCM_RETRY)
                        .unwrap_or(true);
                    if can_retry {
                        log::info!(
                            "mic_mute_trigger: starting digital-silence PCM monitor (hw_mute={hw_mute})"
                        );
                        last_pcm_start_at = Some(Instant::now());
                        install_active_pcm(start_pcm_monitor(
                            Some(friendly.clone()),
                            Arc::clone(&last_endpoint_event),
                        ));
                    }
                }
            }

            let mut method = DetectionMethod::EndpointPoll;
            let muted = match unsafe { volume.GetMute() } {
                Ok(flag) => {
                    let polled = flag.as_bool();
                    if let Ok(mut raw) = raw_muted.lock() {
                        if raw.changed {
                            method = raw.source;
                            raw.changed = false;
                            log::info!(
                                "mic_mute_trigger: endpoint notify edge muted={} (debouncing)",
                                raw.muted
                            );
                            raw.muted
                        } else {
                            if polled != raw.muted {
                                log::info!(
                                    "mic_mute_trigger: endpoint poll edge muted={polled} (debouncing)"
                                );
                            }
                            raw.muted = polled;
                            polled
                        }
                    } else {
                        polled
                    }
                }
                Err(err) => {
                    log::warn!("mic_mute_trigger: GetMute failed: {err}");
                    break;
                }
            };

            let now = Instant::now();
            if let Some(edge) = topology.poll_edge() {
                topo_held = Some(edge);
            }
            if let Some(raw) = topo_held {
                if let Some(stable) = topo_debouncer.observe(raw, now) {
                    if let Ok(mut last) = last_endpoint_event.lock() {
                        *last = Some((stable, now));
                    }
                    log::info!(
                        "mic_mute_trigger: mute bit {} via {} (device='{friendly}')",
                        if stable { "muted" } else { "unmuted" },
                        DetectionMethod::HardwareMute.as_str()
                    );
                    emit(if stable {
                        MuteTriggerEvent::BecameMuted {
                            method: DetectionMethod::HardwareMute,
                        }
                    } else {
                        MuteTriggerEvent::BecameUnmuted {
                            method: DetectionMethod::HardwareMute,
                        }
                    });
                }
            }
            if let Some(stable) = debouncer.observe(muted, now) {
                if let Ok(mut last) = last_endpoint_event.lock() {
                    *last = Some((stable, now));
                }
                log::info!(
                    "mic_mute_trigger: mute bit {} via {} (device='{friendly}')",
                    if stable { "muted" } else { "unmuted" },
                    method.as_str()
                );
                emit(if stable {
                    MuteTriggerEvent::BecameMuted { method }
                } else {
                    MuteTriggerEvent::BecameUnmuted { method }
                });
            }

            std::thread::sleep(POLL_INTERVAL);
        }

        release_active_pcm();
        unsafe {
            let _ = volume.UnregisterControlChangeNotify(&callback_iface);
        }
        Ok(())
    }

    struct EndpointRawMute {
        muted: bool,
        source: DetectionMethod,
        changed: bool,
    }

    #[implement(IAudioEndpointVolumeCallback)]
    struct MuteCallback {
        raw: Arc<Mutex<EndpointRawMute>>,
    }

    impl IAudioEndpointVolumeCallback_Impl for MuteCallback_Impl {
        fn OnNotify(
            &self,
            pnotify: *mut AUDIO_VOLUME_NOTIFICATION_DATA,
        ) -> windows_core::Result<()> {
            if pnotify.is_null() {
                return Ok(());
            }
            let data = unsafe { &*pnotify };
            // Volume-only notifications share this callback; only mute-bit edges matter.
            let muted = data.bMuted.as_bool();
            if let Ok(mut raw) = self.raw.lock() {
                if muted != raw.muted {
                    raw.muted = muted;
                    raw.source = DetectionMethod::EndpointNotify;
                    raw.changed = true;
                    log::debug!(
                        "mic_mute_trigger: notify mute bit {} (queued for debounce)",
                        muted
                    );
                }
            }
            Ok(())
        }
    }

    fn resolve_capture_device(
        enumerator: &IMMDeviceEnumerator,
        desired_name: Option<&str>,
    ) -> Result<(IMMDevice, String, String), String> {
        if let Some(name) = desired_name {
            let collection = unsafe {
                enumerator
                    .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
                    .map_err(|e| format!("EnumAudioEndpoints: {e}"))?
            };
            let count = unsafe { collection.GetCount() }.unwrap_or(0);
            let mut best: Option<(u8, IMMDevice, String, String)> = None;
            for i in 0..count {
                let device = unsafe { collection.Item(i) }
                    .map_err(|e| format!("IMMDeviceCollection::Item: {e}"))?;
                let friendly = device_friendly_name(&device).unwrap_or_default();
                let desc = device_property(&device, &PKEY_DEVICE_DEVICE_DESC).unwrap_or_default();
                let score = crate::media::device_match::best_score_against(
                    name,
                    &[friendly.as_str(), desc.as_str()],
                );
                if score == crate::media::device_match::SCORE_NONE {
                    continue;
                }
                let better = match &best {
                    None => true,
                    Some((best_score, _, _, _)) => score > *best_score,
                };
                if better {
                    let id = device_id(&device)?;
                    if score == crate::media::device_match::SCORE_EXACT {
                        return Ok((device, id, friendly));
                    }
                    best = Some((score, device, id, friendly));
                }
            }
            if let Some((score, device, id, friendly)) = best {
                log::info!(
                    "mic_mute_trigger: matched selected mic '{name}' to '{friendly}' (score={score})"
                );
                return Ok((device, id, friendly));
            }
            log::warn!(
                "mic_mute_trigger: selected device '{name}' not found — falling back to default input"
            );
        }

        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eCapture, eConsole) }
            .map_err(|e| format!("GetDefaultAudioEndpoint(eCapture): {e}"))?;
        let friendly = device_friendly_name(&device).unwrap_or_else(|_| "default".into());
        let id = device_id(&device)?;
        Ok((device, id, friendly))
    }

    fn device_id(device: &IMMDevice) -> Result<String, String> {
        let id = unsafe { device.GetId() }.map_err(|e| format!("GetId: {e}"))?;
        unsafe { id.to_string() }.map_err(|e| format!("device id utf16: {e}"))
    }

    fn device_friendly_name(device: &IMMDevice) -> Result<String, String> {
        device_property(device, &PKEY_DEVICE_FRIENDLY_NAME)
    }

    fn device_property(device: &IMMDevice, key: &PROPERTYKEY) -> Result<String, String> {
        let store: IPropertyStore = unsafe { device.OpenPropertyStore(STGM_READ) }
            .map_err(|e| format!("OpenPropertyStore: {e}"))?;
        let mut pv: PROPVARIANT =
            unsafe { store.GetValue(key) }.map_err(|e| format!("GetValue: {e}"))?;
        let name = propvariant_to_string(&pv);
        unsafe {
            let _ = PropVariantClear(&mut pv);
        }
        name
    }

    fn propvariant_to_string(pv: &PROPVARIANT) -> Result<String, String> {
        let vt = unsafe { pv.Anonymous.Anonymous.vt };
        if vt != VT_LPWSTR {
            return Err(format!("unsupported PROPVARIANT vt={vt:?}"));
        }
        let ptr = unsafe { pv.Anonymous.Anonymous.Anonymous.pwszVal };
        if ptr.0.is_null() {
            return Err("null LPWSTR".into());
        }
        unsafe { ptr.to_string() }.map_err(|e| e.to_string())
    }

    struct PcmMonitorGuard {
        stop: Arc<AtomicBool>,
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for PcmMonitorGuard {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.join.take() {
                // Join so WASAPI releases the capture client before dictation
                // opens the same mic. Fire-and-forget stop left the stream up
                // long enough to starve the pill visualizer.
                let _ = handle.join();
            }
        }
    }

    fn start_pcm_monitor(
        device_name: Option<String>,
        last_endpoint_event: Arc<Mutex<Option<(bool, Instant)>>>,
    ) -> PcmMonitorGuard {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let join = std::thread::Builder::new()
            .name("mic-mute-pcm".into())
            .spawn(move || {
                if let Err(err) = run_pcm_monitor(device_name, stop_thread, last_endpoint_event) {
                    log::warn!("mic_mute_trigger: PCM monitor stopped: {err}");
                }
            })
            .ok();
        PcmMonitorGuard { stop, join }
    }

    fn run_pcm_monitor(
        device_name: Option<String>,
        stop: Arc<AtomicBool>,
        last_endpoint_event: Arc<Mutex<Option<(bool, Instant)>>>,
    ) -> Result<(), String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = if let Some(name) = device_name.as_deref() {
            let mut best: Option<(u8, cpal::Device)> = None;
            if let Ok(iter) = host.input_devices() {
                for device in iter {
                    let Ok(candidate) = device.name() else {
                        continue;
                    };
                    let score =
                        crate::media::device_match::device_name_match_score(&candidate, name);
                    if score == crate::media::device_match::SCORE_NONE {
                        continue;
                    }
                    let better = match &best {
                        None => true,
                        Some((best_score, _)) => score > *best_score,
                    };
                    if better {
                        if score == crate::media::device_match::SCORE_EXACT {
                            best = Some((score, device));
                            break;
                        }
                        best = Some((score, device));
                    }
                }
            }
            best.map(|(_, device)| device)
                .or_else(|| host.default_input_device())
                .ok_or_else(|| "no input device for PCM monitor".to_string())?
        } else {
            host.default_input_device()
                .ok_or_else(|| "no default input device for PCM monitor".to_string())?
        };
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        let name = device.name().unwrap_or_else(|_| "unknown".into());
        let config = device
            .default_input_config()
            .map_err(|e| format!("default_input_config: {e}"))?;
        log::info!(
            "mic_mute_trigger: PCM digital-silence monitor on '{name}' ({:?})",
            config.sample_format()
        );

        let detector = Arc::new(Mutex::new(DigitalSilenceDetector::new(
            PCM_WINDOW,
            PCM_DEBOUNCE,
        )));
        let err_fn = |err| log::warn!("mic_mute_trigger: PCM stream error: {err}");
        let detector_cb = Arc::clone(&detector);
        let last_ep = Arc::clone(&last_endpoint_event);

        let cfg: cpal::StreamConfig = config.clone().into();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &cfg,
                    move |data: &[f32], _| {
                        handle_pcm_block(data, &detector_cb, &last_ep);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?,
            cpal::SampleFormat::F64 => device
                .build_input_stream(
                    &cfg,
                    move |data: &[f64], _| {
                        let (silent, abs_max) = pcm_stats(data.iter().map(|s| (*s as f32).abs()));
                        handle_pcm_stats(data.len(), silent, abs_max, &detector_cb, &last_ep);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?,
            cpal::SampleFormat::I16 => device
                .build_input_stream(
                    &cfg,
                    move |data: &[i16], _| {
                        let (silent, abs_max) =
                            pcm_stats(data.iter().map(|s| (*s as f32 / 32768.0).abs()));
                        handle_pcm_stats(data.len(), silent, abs_max, &detector_cb, &last_ep);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?,
            cpal::SampleFormat::I32 => device
                .build_input_stream(
                    &cfg,
                    move |data: &[i32], _| {
                        let (silent, abs_max) =
                            pcm_stats(data.iter().map(|s| (*s as f32 / 2147483648.0).abs()));
                        handle_pcm_stats(data.len(), silent, abs_max, &detector_cb, &last_ep);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?,
            cpal::SampleFormat::I8 => device
                .build_input_stream(
                    &cfg,
                    move |data: &[i8], _| {
                        let (silent, abs_max) =
                            pcm_stats(data.iter().map(|s| (*s as f32 / 128.0).abs()));
                        handle_pcm_stats(data.len(), silent, abs_max, &detector_cb, &last_ep);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?,
            cpal::SampleFormat::U16 => device
                .build_input_stream(
                    &cfg,
                    move |data: &[u16], _| {
                        let (silent, abs_max) =
                            pcm_stats(data.iter().map(|s| ((*s as f32 - 32768.0) / 32768.0).abs()));
                        handle_pcm_stats(data.len(), silent, abs_max, &detector_cb, &last_ep);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?,
            cpal::SampleFormat::U8 => device
                .build_input_stream(
                    &cfg,
                    move |data: &[u8], _| {
                        let (silent, abs_max) =
                            pcm_stats(data.iter().map(|s| ((*s as f32 - 128.0) / 128.0).abs()));
                        handle_pcm_stats(data.len(), silent, abs_max, &detector_cb, &last_ep);
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| e.to_string())?,
            other => {
                return Err(format!("unsupported PCM sample format: {other:?}"));
            }
        };
        stream.play().map_err(|e| e.to_string())?;

        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(stream);
        Ok(())
    }

    fn pcm_stats(abs_samples: impl Iterator<Item = f32>) -> (u32, f32) {
        let mut silent = 0u32;
        let mut abs_max = 0.0f32;
        for a in abs_samples {
            if a > abs_max {
                abs_max = a;
            }
            if a <= crate::media::digital_silence::DIGITAL_SILENCE_EPS {
                silent += 1;
            }
        }
        (silent, abs_max)
    }

    fn handle_pcm_stats(
        total: usize,
        silent: u32,
        abs_max: f32,
        detector: &Mutex<DigitalSilenceDetector>,
        last_endpoint_event: &Mutex<Option<(bool, Instant)>>,
    ) {
        let now = Instant::now();
        let Ok(mut det) = detector.lock() else {
            return;
        };
        let Some(transition) = det.push_chunk_stats(now, silent, total as u32, abs_max) else {
            return;
        };
        finish_pcm_transition(transition, now, last_endpoint_event);
    }

    fn handle_pcm_block(
        samples: &[f32],
        detector: &Mutex<DigitalSilenceDetector>,
        last_endpoint_event: &Mutex<Option<(bool, Instant)>>,
    ) {
        let now = Instant::now();
        let Ok(mut det) = detector.lock() else {
            return;
        };
        let Some(transition) = det.push_samples(samples, now) else {
            return;
        };
        finish_pcm_transition(transition, now, last_endpoint_event);
    }

    fn finish_pcm_transition(
        transition: SilenceTransition,
        now: Instant,
        last_endpoint_event: &Mutex<Option<(bool, Instant)>>,
    ) {
        let muted = matches!(transition, SilenceTransition::BecameMuted);
        if let Ok(last) = last_endpoint_event.lock() {
            if let Some((ep_muted, at)) = *last {
                if ep_muted == muted
                    && now.saturating_duration_since(at) <= ENDPOINT_PRIORITY_WINDOW
                {
                    log::debug!(
                        "mic_mute_trigger: PCM {} ignored — endpoint already reported it",
                        if muted { "mute" } else { "unmute" }
                    );
                    return;
                }
            }
        }
        log::info!(
            "mic_mute_trigger: {} via {}",
            if muted { "muted" } else { "unmuted" },
            DetectionMethod::DigitalSilence.as_str()
        );
        emit(if muted {
            MuteTriggerEvent::BecameMuted {
                method: DetectionMethod::DigitalSilence,
            }
        } else {
            MuteTriggerEvent::BecameUnmuted {
                method: DetectionMethod::DigitalSilence,
            }
        });
    }
}

#[cfg(windows)]
pub use win::{reload, setup};

#[cfg(not(windows))]
pub fn setup(_app: &mut tauri::App, _shared: crate::pipeline::SharedState) {}

#[cfg(not(windows))]
pub fn reload(_app: &tauri::AppHandle) {}
