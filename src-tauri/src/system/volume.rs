#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use std::sync::Mutex;

#[cfg(windows)]
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
#[cfg(windows)]
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
#[cfg(windows)]
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

/// Result of an attempt to mute the default output. Dictation waits for this
/// before opening the microphone, so `Muted` and `AlreadyMuted` must only be
/// returned when the output really is silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteOutcome {
    /// Verenu muted the output and, where the platform allows, read it back.
    Muted,
    /// The output was already muted (by the user or by an earlier dictation).
    AlreadyMuted,
    /// The mute request was delivered, but the platform cannot report whether
    /// the output actually went silent (Android applies it in Kotlin and may
    /// leave the volume up after a user override).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    Requested,
    /// The output could not be muted; dictation continues unmuted.
    Failed,
}

#[cfg(windows)]
static MUTED_ENDPOINT: Mutex<WindowsMuteState> = Mutex::new(WindowsMuteState::new());

#[cfg(any(windows, test))]
const MAX_PENDING_WINDOWS_RESTORES: usize = 8;

/// Tracks the endpoint for the current mute owner separately from endpoints
/// whose restoration failed. A stale endpoint must not block muting a new one.
#[cfg(any(windows, test))]
#[derive(Default)]
struct WindowsMuteState {
    active_endpoint: Option<String>,
    pending_restores: Vec<String>,
}

#[cfg(any(windows, test))]
impl WindowsMuteState {
    const fn new() -> Self {
        Self {
            active_endpoint: None,
            pending_restores: Vec::new(),
        }
    }

    fn begin(
        &mut self,
        snapshot_and_mute: impl FnOnce() -> Result<Option<String>, String>,
        confirm_current_mute: impl FnOnce() -> Result<bool, String>,
        mut restore: impl FnMut(&str) -> Result<(), String>,
    ) -> (MuteOutcome, Vec<String>) {
        if self.active_endpoint.is_some() {
            return match confirm_current_mute() {
                Ok(true) => (MuteOutcome::AlreadyMuted, Vec::new()),
                Ok(false) => (
                    MuteOutcome::Failed,
                    vec!["Current Windows output is no longer muted".into()],
                ),
                Err(err) => (MuteOutcome::Failed, vec![err]),
            };
        }

        let mut failures = self.retry_pending(&mut restore);
        let outcome = match snapshot_and_mute() {
            Ok(Some(endpoint)) => {
                self.active_endpoint = Some(endpoint);
                MuteOutcome::Muted
            }
            Ok(None) => MuteOutcome::AlreadyMuted,
            Err(err) => {
                failures.push(err);
                MuteOutcome::Failed
            }
        };
        (outcome, failures)
    }

    fn end(&mut self, mut restore: impl FnMut(&str) -> Result<(), String>) -> Result<(), String> {
        let mut failures = self.retry_pending(&mut restore);
        if let Some(endpoint) = self.active_endpoint.take() {
            if let Err(err) = restore(&endpoint) {
                self.defer_restore(endpoint);
                failures.push(err);
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    fn retry_pending(
        &mut self,
        restore: &mut impl FnMut(&str) -> Result<(), String>,
    ) -> Vec<String> {
        let pending = std::mem::take(&mut self.pending_restores);
        let mut failures = Vec::new();
        for endpoint in pending {
            if let Err(err) = restore(&endpoint) {
                self.defer_restore(endpoint);
                failures.push(err);
            }
        }
        failures
    }

    fn defer_restore(&mut self, endpoint: String) {
        if self.pending_restores.contains(&endpoint) {
            return;
        }
        if self.pending_restores.len() == MAX_PENDING_WINDOWS_RESTORES {
            self.pending_restores.remove(0);
            log::warn!("Dropping oldest deferred Windows audio restore after reaching the bounded retry limit");
        }
        self.pending_restores.push(endpoint);
    }
}

#[cfg(target_os = "macos")]
static RESTORE_STATE: Mutex<MacRestoreState> = Mutex::new(MacRestoreState::Idle);
#[cfg(target_os = "macos")]
static WARNED_UNSUPPORTED_AUDIO_CONTROL: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
enum MacRestoreState {
    Idle,
    Muted {
        device_id: u32,
        was_muted: bool,
    },
    Volume {
        device_id: u32,
        previous_volume: f32,
    },
}

// ---- exclusive microphone access (macOS hog mode) ----

#[cfg(target_os = "macos")]
static HOG_STATE: Mutex<HogState> = Mutex::new(HogState::Idle);
#[cfg(target_os = "macos")]
static WARNED_HOG_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
#[cfg(target_os = "macos")]
static ACTIVE_SESSION_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug)]
enum HogState {
    Idle,
    Hogged { device_id: u32, session_id: u64 },
}

#[cfg(windows)]
fn snapshot_and_mute_windows() -> Result<Option<String>, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| e.to_string())?;
        let volume: IAudioEndpointVolume = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| e.to_string())?;
        if volume.GetMute().map_err(|e| e.to_string())?.as_bool() {
            return Ok(None);
        }
        let id = device.GetId().map_err(|e| e.to_string())?;
        let endpoint = id.to_string().map_err(|e| e.to_string());
        windows::Win32::System::Com::CoTaskMemFree(Some(id.0.cast()));
        let endpoint = endpoint?;
        volume
            .SetMute(true, std::ptr::null())
            .map_err(|e| e.to_string())?;
        Ok(Some(endpoint))
    }
}

#[cfg(windows)]
fn restore_windows_endpoint(endpoint: &str) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let id = windows::core::HSTRING::from(endpoint);
        let device = enumerator.GetDevice(&id).map_err(|e| e.to_string())?;
        let volume: IAudioEndpointVolume = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| e.to_string())?;
        volume
            .SetMute(false, std::ptr::null())
            .map_err(|e| e.to_string())
    }
}

/// Resolve the default again rather than trusting the endpoint saved for restore.
#[cfg(windows)]
fn confirm_current_windows_mute() -> Result<bool, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| e.to_string())?;
        let volume: IAudioEndpointVolume = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| e.to_string())?;
        volume
            .GetMute()
            .map(|muted| muted.as_bool())
            .map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use coreaudio::sys::{
        kAudioDevicePropertyHogMode, kAudioDevicePropertyMute, kAudioDevicePropertyVolumeScalar,
        kAudioHardwareNoError, kAudioHardwarePropertyDefaultInputDevice,
        kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyElementMaster,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
        AudioDeviceID, AudioObjectGetPropertyData, AudioObjectHasProperty,
        AudioObjectIsPropertySettable, AudioObjectPropertyAddress, AudioObjectSetPropertyData,
        Boolean, OSStatus,
    };
    use std::mem;
    use std::ptr::null;

    use super::MacRestoreState;

    fn property_address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: scope,
            mElement: kAudioObjectPropertyElementMaster,
        }
    }

    fn default_output_device() -> Result<AudioDeviceID, String> {
        let property_address = property_address(
            kAudioHardwarePropertyDefaultOutputDevice,
            kAudioObjectPropertyScopeGlobal,
        );
        let mut device_id: AudioDeviceID = 0;
        let mut data_size = mem::size_of::<AudioDeviceID>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &property_address as *const _,
                0,
                null(),
                &mut data_size,
                &mut device_id as *mut _ as *mut _,
            )
        };
        if status == kAudioHardwareNoError as OSStatus {
            Ok(device_id)
        } else {
            Err(format!(
                "Failed to get default output device for system audio control: OSStatus {status}"
            ))
        }
    }

    fn property_is_settable(
        device_id: AudioDeviceID,
        selector: u32,
        scope: u32,
    ) -> Result<bool, String> {
        let address = property_address(selector, scope);
        let has_property =
            unsafe { AudioObjectHasProperty(device_id, &address as *const _) } != 0 as Boolean;
        if !has_property {
            return Ok(false);
        }

        let mut settable: Boolean = 0;
        let status = unsafe {
            AudioObjectIsPropertySettable(device_id, &address as *const _, &mut settable)
        };
        if status == kAudioHardwareNoError as OSStatus {
            Ok(settable != 0)
        } else {
            Err(format!(
                "Failed to inspect audio property settable state: selector={selector} status={status}"
            ))
        }
    }

    fn get_u32_property(
        device_id: AudioDeviceID,
        selector: u32,
        scope: u32,
    ) -> Result<u32, String> {
        let address = property_address(selector, scope);
        let mut value: u32 = 0;
        let mut data_size = mem::size_of::<u32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address as *const _,
                0,
                null(),
                &mut data_size,
                &mut value as *mut _ as *mut _,
            )
        };
        if status == kAudioHardwareNoError as OSStatus {
            Ok(value)
        } else {
            Err(format!(
                "Failed to read audio property: selector={selector} status={status}"
            ))
        }
    }

    fn get_f32_property(
        device_id: AudioDeviceID,
        selector: u32,
        scope: u32,
    ) -> Result<f32, String> {
        let address = property_address(selector, scope);
        let mut value: f32 = 0.0;
        let mut data_size = mem::size_of::<f32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address as *const _,
                0,
                null(),
                &mut data_size,
                &mut value as *mut _ as *mut _,
            )
        };
        if status == kAudioHardwareNoError as OSStatus {
            Ok(value)
        } else {
            Err(format!(
                "Failed to read audio property: selector={selector} status={status}"
            ))
        }
    }

    fn set_u32_property(
        device_id: AudioDeviceID,
        selector: u32,
        scope: u32,
        value: u32,
    ) -> Result<(), String> {
        let address = property_address(selector, scope);
        let data_size = mem::size_of::<u32>() as u32;
        let status = unsafe {
            AudioObjectSetPropertyData(
                device_id,
                &address as *const _,
                0,
                null(),
                data_size,
                &value as *const _ as *const _,
            )
        };
        if status == kAudioHardwareNoError as OSStatus {
            Ok(())
        } else {
            Err(format!(
                "Failed to set audio property: selector={selector} status={status}"
            ))
        }
    }

    fn set_f32_property(
        device_id: AudioDeviceID,
        selector: u32,
        scope: u32,
        value: f32,
    ) -> Result<(), String> {
        let address = property_address(selector, scope);
        let data_size = mem::size_of::<f32>() as u32;
        let status = unsafe {
            AudioObjectSetPropertyData(
                device_id,
                &address as *const _,
                0,
                null(),
                data_size,
                &value as *const _ as *const _,
            )
        };
        if status == kAudioHardwareNoError as OSStatus {
            Ok(())
        } else {
            Err(format!(
                "Failed to set audio property: selector={selector} status={status}"
            ))
        }
    }

    pub fn snapshot_and_mute() -> Result<Option<MacRestoreState>, String> {
        let device_id = default_output_device()?;

        if property_is_settable(
            device_id,
            kAudioDevicePropertyMute,
            kAudioObjectPropertyScopeOutput,
        )? {
            let was_muted = get_u32_property(
                device_id,
                kAudioDevicePropertyMute,
                kAudioObjectPropertyScopeOutput,
            )? != 0;
            set_u32_property(
                device_id,
                kAudioDevicePropertyMute,
                kAudioObjectPropertyScopeOutput,
                1,
            )?;
            return Ok(Some(MacRestoreState::Muted {
                device_id,
                was_muted,
            }));
        }

        if property_is_settable(
            device_id,
            kAudioDevicePropertyVolumeScalar,
            kAudioObjectPropertyScopeOutput,
        )? {
            let previous_volume = get_f32_property(
                device_id,
                kAudioDevicePropertyVolumeScalar,
                kAudioObjectPropertyScopeOutput,
            )?;
            set_f32_property(
                device_id,
                kAudioDevicePropertyVolumeScalar,
                kAudioObjectPropertyScopeOutput,
                0.0,
            )?;
            return Ok(Some(MacRestoreState::Volume {
                device_id,
                previous_volume,
            }));
        }

        Ok(None)
    }

    pub fn restore(previous: MacRestoreState) -> Result<(), String> {
        match previous {
            MacRestoreState::Idle => Ok(()),
            MacRestoreState::Muted {
                device_id,
                was_muted,
            } => set_u32_property(
                device_id,
                kAudioDevicePropertyMute,
                kAudioObjectPropertyScopeOutput,
                if was_muted { 1 } else { 0 },
            ),
            MacRestoreState::Volume {
                device_id,
                previous_volume,
            } => set_f32_property(
                device_id,
                kAudioDevicePropertyVolumeScalar,
                kAudioObjectPropertyScopeOutput,
                previous_volume,
            ),
        }
    }

    pub fn confirm_current_mute() -> Result<bool, String> {
        let device_id = default_output_device()?;
        if property_is_settable(
            device_id,
            kAudioDevicePropertyMute,
            kAudioObjectPropertyScopeOutput,
        )? && get_u32_property(
            device_id,
            kAudioDevicePropertyMute,
            kAudioObjectPropertyScopeOutput,
        )? != 0
        {
            return Ok(true);
        }
        if property_is_settable(
            device_id,
            kAudioDevicePropertyVolumeScalar,
            kAudioObjectPropertyScopeOutput,
        )? {
            return Ok(get_f32_property(
                device_id,
                kAudioDevicePropertyVolumeScalar,
                kAudioObjectPropertyScopeOutput,
            )? == 0.0);
        }
        Ok(false)
    }

    pub(super) fn default_input_device() -> Result<AudioDeviceID, String> {
        let property_address = property_address(
            kAudioHardwarePropertyDefaultInputDevice,
            kAudioObjectPropertyScopeGlobal,
        );
        let mut device_id: AudioDeviceID = 0;
        let mut data_size = mem::size_of::<AudioDeviceID>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                kAudioObjectSystemObject,
                &property_address as *const _,
                0,
                null(),
                &mut data_size,
                &mut device_id as *mut _ as *mut _,
            )
        };
        if status == kAudioHardwareNoError as OSStatus {
            Ok(device_id)
        } else {
            Err(format!(
                "Failed to get default input device for exclusive mic access: OSStatus {status}"
            ))
        }
    }

    fn get_i32_property(
        device_id: AudioDeviceID,
        selector: u32,
        scope: u32,
    ) -> Result<i32, String> {
        let address = property_address(selector, scope);
        let mut value: i32 = 0;
        let mut data_size = mem::size_of::<i32>() as u32;
        let status = unsafe {
            AudioObjectGetPropertyData(
                device_id,
                &address as *const _,
                0,
                null(),
                &mut data_size,
                &mut value as *mut _ as *mut _,
            )
        };
        if status == kAudioHardwareNoError as OSStatus {
            Ok(value)
        } else {
            Err(format!(
                "Failed to read audio property: selector={selector} status={status}"
            ))
        }
    }

    fn set_i32_property(
        device_id: AudioDeviceID,
        selector: u32,
        scope: u32,
        value: i32,
    ) -> Result<(), String> {
        let address = property_address(selector, scope);
        let data_size = mem::size_of::<i32>() as u32;
        let status = unsafe {
            AudioObjectSetPropertyData(
                device_id,
                &address as *const _,
                0,
                null(),
                data_size,
                &value as *const _ as *const _,
            )
        };
        if status == kAudioHardwareNoError as OSStatus {
            Ok(())
        } else {
            Err(format!(
                "Failed to set audio property: selector={selector} status={status}"
            ))
        }
    }

    /// Acquires exclusive (hog-mode) ownership of the default input device for
    /// this process. Returns the device id on success, `None` if hog mode isn't
    /// settable or another process already owns the device (nothing to restore).
    pub fn snapshot_and_hog() -> Result<Option<AudioDeviceID>, String> {
        let device_id = default_input_device()?;

        if !property_is_settable(
            device_id,
            kAudioDevicePropertyHogMode,
            kAudioObjectPropertyScopeGlobal,
        )? {
            return Ok(None);
        }

        // Hog mode owner is a pid_t; pass our pid to acquire, -1 to release.
        let our_pid = std::process::id() as i32;
        set_i32_property(
            device_id,
            kAudioDevicePropertyHogMode,
            kAudioObjectPropertyScopeGlobal,
            our_pid,
        )?;

        let owner = get_i32_property(
            device_id,
            kAudioDevicePropertyHogMode,
            kAudioObjectPropertyScopeGlobal,
        )?;
        if owner == our_pid {
            Ok(Some(device_id))
        } else {
            // The HAL declined or another process holds the device — we own
            // nothing, so there is nothing to release later.
            Ok(None)
        }
    }

    pub fn release_hog(device_id: AudioDeviceID) -> Result<(), String> {
        set_i32_property(
            device_id,
            kAudioDevicePropertyHogMode,
            kAudioObjectPropertyScopeGlobal,
            -1,
        )
    }
}

#[cfg(not(any(
    windows,
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
fn set_system_muted(_muted: bool) -> Result<(), String> {
    Ok(())
}

/// Default-sink mute through PipeWire (`wpctl`), with a PulseAudio `pactl`
/// fallback. The sink is pinned by id when muting so a default-device change
/// mid-dictation cannot leave a different sink muted.
#[cfg(target_os = "linux")]
mod linux {
    use super::MuteOutcome;
    use std::process::{Command, Stdio};

    fn run(program: &str, args: &[&str]) -> Result<String, String> {
        let output = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| format!("{program} unavailable: {e}"))?;
        if !output.status.success() {
            return Err(format!("{program} {} failed", args.first().unwrap_or(&"")));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Sink {
        PipeWire(String),
        Pulse(String),
    }

    pub(super) fn parse_wpctl_id(inspect: &str) -> Option<String> {
        inspect.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("id ")?;
            let id = rest.split(',').next()?.trim();
            (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then(|| id.to_string())
        })
    }

    /// Returns the default sink and whether it is currently muted. The node id
    /// is resolved first and the mute flag is read from that same id, so a
    /// default-device change in between cannot pair one sink with another
    /// sink's flag.
    pub fn default_sink() -> Result<(Sink, bool), String> {
        if let Ok(inspect) = run("wpctl", &["inspect", "@DEFAULT_AUDIO_SINK@"]) {
            if let Some(id) = parse_wpctl_id(&inspect) {
                if let Ok(volume) = run("wpctl", &["get-volume", &id]) {
                    return Ok((Sink::PipeWire(id), volume.contains("[MUTED]")));
                }
            }
        }
        let name = run("pactl", &["get-default-sink"])?.trim().to_string();
        if name.is_empty() {
            return Err("no default audio sink".into());
        }
        let mute = run("pactl", &["get-sink-mute", &name])?;
        Ok((Sink::Pulse(name), mute.contains("yes")))
    }

    /// Re-resolve identity after reading mute state, including on the
    /// already-muted path. Do not pair a new default's flag with an old id.
    pub fn current_default(sink: &Sink) -> Result<Sink, String> {
        if matches!(sink, Sink::PipeWire(_)) {
            let inspect = run("wpctl", &["inspect", "@DEFAULT_AUDIO_SINK@"])?;
            return parse_wpctl_id(&inspect)
                .map(Sink::PipeWire)
                .ok_or_else(|| "could not resolve the current PipeWire default".into());
        }
        let name = run("pactl", &["get-default-sink"])?.trim().to_string();
        if name.is_empty() {
            return Err("no default audio sink".into());
        }
        Ok(Sink::Pulse(name))
    }

    pub fn mute_current_output(
        muted_sinks: &mut Vec<Sink>,
        mut snapshot: impl FnMut() -> Result<(Sink, bool), String>,
        mut set_mute: impl FnMut(&Sink) -> Result<(), String>,
        mut read_mute: impl FnMut(&Sink) -> Result<bool, String>,
        mut current: impl FnMut(&Sink) -> Result<Sink, String>,
    ) -> Result<MuteOutcome, String> {
        // The outer gate bounds recording's wait even if an OS read stalls.
        // Also bound churn so a changing default cannot create an endless task.
        for _ in 0..3 {
            let (sink, already_muted) = snapshot()?;
            if !already_muted {
                set_mute(&sink)?;
                // Keep restoration ownership even if confirmation fails.
                if !muted_sinks.contains(&sink) {
                    muted_sinks.push(sink.clone());
                }
                if !read_mute(&sink)? {
                    return Err("System audio mute was not reflected by the audio server".into());
                }
            }
            if current(&sink)? == sink {
                return Ok(if already_muted {
                    MuteOutcome::AlreadyMuted
                } else {
                    MuteOutcome::Muted
                });
            }
        }
        Err("Default audio output kept changing during mute confirmation".into())
    }

    pub fn set_muted(sink: &Sink, muted: bool) -> Result<(), String> {
        let flag = if muted { "1" } else { "0" };
        match sink {
            Sink::PipeWire(id) => run("wpctl", &["set-mute", id, flag]).map(|_| ()),
            Sink::Pulse(name) => run("pactl", &["set-sink-mute", name, flag]).map(|_| ()),
        }
    }

    /// Reads the sink's mute flag back from the audio server.
    pub fn is_muted(sink: &Sink) -> Result<bool, String> {
        match sink {
            Sink::PipeWire(id) => Ok(run("wpctl", &["get-volume", id])?.contains("[MUTED]")),
            Sink::Pulse(name) => Ok(run("pactl", &["get-sink-mute", name])?.contains("yes")),
        }
    }
}

/// The sinks Verenu muted. A sink that was already muted by the user is never
/// recorded, so it is never unmuted on their behalf. More than one sink is
/// possible when the default output changes while a mute is held.
#[cfg(target_os = "linux")]
static LINUX_MUTED_SINKS: Mutex<Vec<linux::Sink>> = Mutex::new(Vec::new());

#[cfg(windows)]
pub fn mute() -> MuteOutcome {
    let mut muted = match MUTED_ENDPOINT.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::warn!("System mute state lock was poisoned; recovering");
            poisoned.into_inner()
        }
    };

    let (outcome, failures) = muted.begin(
        snapshot_and_mute_windows,
        confirm_current_windows_mute,
        restore_windows_endpoint,
    );
    for err in &failures {
        log::warn!("Failed to apply Windows audio mute state: {err}");
    }
    outcome
}

#[cfg(windows)]
pub fn unmute() {
    let mut muted = match MUTED_ENDPOINT.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::warn!("System mute state lock was poisoned; recovering");
            poisoned.into_inner()
        }
    };

    if let Err(err) = muted.end(restore_windows_endpoint) {
        log::warn!("Failed to unmute system audio; restoration will be retried: {err}");
    }
}

#[cfg(target_os = "macos")]
pub fn mute() -> MuteOutcome {
    let mut restore_state = match RESTORE_STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::warn!("System audio restore state lock was poisoned; recovering");
            poisoned.into_inner()
        }
    };

    if !matches!(*restore_state, MacRestoreState::Idle) {
        return match macos::confirm_current_mute() {
            Ok(true) => MuteOutcome::AlreadyMuted,
            Ok(false) => MuteOutcome::Failed,
            Err(err) => {
                log::warn!("Could not confirm current macOS output mute: {err}");
                MuteOutcome::Failed
            }
        };
    }

    match macos::snapshot_and_mute() {
        Ok(Some(snapshot)) => {
            *restore_state = snapshot;
            MuteOutcome::Muted
        }
        Ok(None) => {
            if !WARNED_UNSUPPORTED_AUDIO_CONTROL.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "Default macOS output device does not expose a writable mute or master volume property; auto-mute is unavailable for this device"
                );
            }
            MuteOutcome::Failed
        }
        Err(err) => {
            log::warn!("Failed to mute system audio: {err}");
            MuteOutcome::Failed
        }
    }
}

#[cfg(target_os = "macos")]
pub fn unmute() {
    let mut restore_state = match RESTORE_STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::warn!("System audio restore state lock was poisoned; recovering");
            poisoned.into_inner()
        }
    };

    let snapshot = *restore_state;
    if matches!(snapshot, MacRestoreState::Idle) {
        return;
    }

    *restore_state = MacRestoreState::Idle;

    if let Err(err) = macos::restore(snapshot) {
        log::warn!("Failed to restore system audio: {err}");
    }
}

/// Reserves the microphone exclusively for this process (macOS hog mode), so no
/// other app can capture it while dictating. Safe to call repeatedly — a no-op
/// if exclusive access is already held.
#[cfg(target_os = "macos")]
pub fn register_session() -> u64 {
    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::SeqCst);
    ACTIVE_SESSION_ID.store(session_id, Ordering::SeqCst);
    session_id
}

#[cfg(target_os = "macos")]
pub fn hog_mic(session_id: u64) {
    if ACTIVE_SESSION_ID.load(Ordering::SeqCst) != session_id {
        return;
    }

    let state = match HOG_STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::warn!("Exclusive mic state lock was poisoned; recovering");
            poisoned.into_inner()
        }
    };

    let mut state = state;
    if let HogState::Hogged { device_id, .. } = *state {
        match macos::default_input_device() {
            Ok(default_device) if default_device == device_id => {
                if ACTIVE_SESSION_ID.load(Ordering::SeqCst) == session_id {
                    *state = HogState::Hogged {
                        device_id,
                        session_id,
                    };
                }
                return;
            }
            _ => {
                let _ = macos::release_hog(device_id);
                *state = HogState::Idle;
            }
        }
    }

    let result = macos::snapshot_and_hog();

    if ACTIVE_SESSION_ID.load(Ordering::SeqCst) != session_id {
        if let Ok(Some(device_id)) = result {
            let _ = macos::release_hog(device_id);
        }
        return;
    }

    if !matches!(*state, HogState::Idle) {
        if let Ok(Some(device_id)) = result {
            let _ = macos::release_hog(device_id);
        }
        return;
    }

    match result {
        Ok(Some(device_id)) => {
            *state = HogState::Hogged {
                device_id,
                session_id,
            };
        }
        Ok(None) => {
            if !WARNED_HOG_UNAVAILABLE.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "Exclusive microphone access is unavailable for the default input device (not settable or already owned by another process)"
                );
            }
        }
        Err(err) => {
            log::warn!("Failed to acquire exclusive microphone access: {err}");
        }
    }
}

/// Releases exclusive microphone access taken by [`hog_mic`]. No-op if not held.
#[cfg(target_os = "macos")]
pub fn release_mic(session_id: u64) {
    let _ = ACTIVE_SESSION_ID.compare_exchange(session_id, 0, Ordering::SeqCst, Ordering::SeqCst);

    let mut state = match HOG_STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::warn!("Exclusive mic state lock was poisoned; recovering");
            poisoned.into_inner()
        }
    };

    let device_id = match *state {
        HogState::Idle => return,
        HogState::Hogged {
            device_id,
            session_id: owner_session_id,
        } if owner_session_id == session_id => device_id,
        HogState::Hogged { .. } => return,
    };

    *state = HogState::Idle;

    if let Err(err) = macos::release_hog(device_id) {
        log::warn!("Failed to release exclusive microphone access: {err}");
    }
}

#[cfg(target_os = "linux")]
pub fn mute() -> MuteOutcome {
    let mut muted_sinks = match LINUX_MUTED_SINKS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    match linux::mute_current_output(
        &mut muted_sinks,
        linux::default_sink,
        |sink| linux::set_muted(sink, true),
        linux::is_muted,
        linux::current_default,
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            log::warn!("Could not confirm system audio mute: {err}");
            MuteOutcome::Failed
        }
    }
}

#[cfg(target_os = "linux")]
pub fn unmute() {
    let mut muted_sinks = match LINUX_MUTED_SINKS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    for sink in muted_sinks.drain(..) {
        if let Err(err) = linux::set_muted(&sink, false) {
            log::warn!("Failed to unmute system audio: {err}");
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::linux::{mute_current_output, Sink};
    use super::MuteOutcome;
    use std::cell::{Cell, RefCell};

    #[test]
    fn default_switch_during_confirmation_mutes_the_new_output() {
        for sink in [Sink::PipeWire as fn(String) -> Sink, Sink::Pulse] {
            for initially_muted in [false, true] {
                let a = sink("A".into());
                let b = sink("B".into());
                let os_muted = RefCell::new(if initially_muted {
                    vec![a.clone()]
                } else {
                    vec![]
                });
                let snapshots = Cell::new(0);
                let mut owned = Vec::new();
                let outcome = mute_current_output(
                    &mut owned,
                    || {
                        let output = if snapshots.replace(snapshots.get() + 1) == 0 {
                            &a
                        } else {
                            &b
                        };
                        Ok((output.clone(), os_muted.borrow().contains(output)))
                    },
                    |output| {
                        os_muted.borrow_mut().push(output.clone());
                        Ok(())
                    },
                    |output| Ok(os_muted.borrow().contains(output)),
                    |_| Ok(b.clone()),
                )
                .unwrap();
                assert_eq!(outcome, MuteOutcome::Muted);
                assert_eq!(snapshots.get(), 2);
                assert!(os_muted.borrow().contains(&b));
                assert_eq!(owned, if initially_muted { vec![b] } else { vec![a, b] });
            }
        }
    }

    #[test]
    fn repeated_default_changes_are_bounded_and_keep_restore_ownership() {
        let attempts = Cell::new(0);
        let mut owned = Vec::new();
        let outcome = mute_current_output(
            &mut owned,
            || {
                let id = attempts.replace(attempts.get() + 1);
                Ok((Sink::PipeWire(id.to_string()), false))
            },
            |_| Ok(()),
            |_| Ok(true),
            |_| Ok(Sink::PipeWire("next".into())),
        );
        assert!(outcome.is_err());
        assert_eq!(attempts.get(), 3);
        assert_eq!(owned.len(), 3);
    }

    #[test]
    fn stable_already_muted_output_is_confirmed_without_taking_restore_ownership() {
        let sink = Sink::PipeWire("A".into());
        let mut owned = Vec::new();
        let outcome = mute_current_output(
            &mut owned,
            || Ok((sink.clone(), true)),
            |_| panic!("must not modify an already-muted output"),
            |_| panic!("snapshot already read mute state"),
            |_| Ok(sink.clone()),
        )
        .unwrap();
        assert_eq!(outcome, MuteOutcome::AlreadyMuted);
        assert!(owned.is_empty());
    }

    #[test]
    fn wpctl_inspect_header_yields_node_id() {
        assert_eq!(
            super::linux::parse_wpctl_id(
                "id 34, type PipeWire:Interface:Node\n  alsa.card = \"0\""
            ),
            Some("34".to_string())
        );
        assert_eq!(
            super::linux::parse_wpctl_id(
                "diagnostic banner\n  id 52, type PipeWire:Interface:Node"
            ),
            Some("52".to_string())
        );
        assert_eq!(super::linux::parse_wpctl_id("Object not found"), None);
    }

    fn wpctl_default_sink_muted() -> bool {
        let out = std::process::Command::new("wpctl")
            .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .output()
            .expect("wpctl");
        String::from_utf8_lossy(&out.stdout).contains("[MUTED]")
    }

    /// Real-hardware measurement: `cargo test live_mute_latency -- --ignored --nocapture`.
    /// Briefly mutes the default sink a few times and checks the state
    /// independently with `wpctl`. Skips when the sink is already muted.
    #[test]
    #[ignore = "mutes the real default sink"]
    fn live_mute_latency() {
        if wpctl_default_sink_muted() {
            eprintln!("default sink already muted; skipping");
            return;
        }
        let mut samples = Vec::new();
        for _ in 0..8 {
            let start = std::time::Instant::now();
            let _ = super::mute();
            let elapsed = start.elapsed();
            let muted = wpctl_default_sink_muted();
            super::unmute();
            assert!(muted, "mute() returned but the sink is not muted");
            assert!(!wpctl_default_sink_muted(), "unmute() did not restore");
            samples.push(elapsed.as_secs_f64() * 1000.0);
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!(
            "LIVE mute() latency ms: min={:.1} median={:.1} max={:.1} all={:?}",
            samples[0],
            samples[samples.len() / 2],
            samples[samples.len() - 1],
            samples
                .iter()
                .map(|s| format!("{s:.1}"))
                .collect::<Vec<_>>()
        );
    }
}

#[cfg(not(any(
    windows,
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn mute() -> MuteOutcome {
    match set_system_muted(true) {
        Ok(()) => MuteOutcome::Muted,
        Err(_) => MuteOutcome::Failed,
    }
}

#[cfg(not(any(
    windows,
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
pub fn unmute() {
    let _ = set_system_muted(false);
}

// The bridge mirrors this state for the pill; the Application applies it directly.
static ANDROID_MUTE_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(crate) fn android_mute_requested() -> bool {
    ANDROID_MUTE_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(target_os = "android")]
pub fn mute() -> MuteOutcome {
    ANDROID_MUTE_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
    if update_android_media_mute(true) {
        MuteOutcome::Requested
    } else {
        MuteOutcome::Failed
    }
}

#[cfg(target_os = "android")]
pub fn unmute() {
    ANDROID_MUTE_REQUESTED.store(false, std::sync::atomic::Ordering::Relaxed);
    update_android_media_mute(false);
}

#[cfg(target_os = "android")]
fn update_android_media_mute(muted: bool) -> bool {
    use jni::objects::{JObject, JValue};

    let result = (|| -> jni::errors::Result<()> {
        let context = ndk_context::android_context();
        // ndk-context owns a process-lifetime Application global reference.
        let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast())? };
        let mut env = vm.attach_current_thread()?;
        let application = unsafe { JObject::from_raw(context.context().cast()) };
        let result = env.call_method(
            &application,
            "updateMediaMute",
            "(Z)V",
            &[JValue::Bool(muted.into())],
        );
        // Never leave a pending Java exception on a Rust worker thread.
        if env.exception_check()? {
            env.exception_clear()?;
        }
        result.map(|_| ())
    })();
    if let Err(error) = &result {
        log::warn!("Could not update Android media mute: {error}");
    }
    result.is_ok()
}

/// Exclusive microphone access is macOS-only; no-op everywhere else.
#[cfg(not(target_os = "macos"))]
pub fn register_session() -> u64 {
    0
}

#[cfg(not(target_os = "macos"))]
pub fn hog_mic(_session_id: u64) {}

#[cfg(not(target_os = "macos"))]
pub fn release_mic(_session_id: u64) {}

#[cfg(test)]
mod windows_mute_state_tests {
    use super::{MuteOutcome, WindowsMuteState};
    use std::cell::Cell;

    #[test]
    fn failed_restore_does_not_block_a_later_default_endpoint_mute() {
        let mut state = WindowsMuteState::new();
        let mut restore_attempts = Vec::new();

        state.begin(
            || Ok(Some("removed-endpoint".to_string())),
            || panic!("no existing lease"),
            |endpoint| {
                restore_attempts.push(endpoint.to_string());
                Ok(())
            },
        );
        assert_eq!(state.active_endpoint.as_deref(), Some("removed-endpoint"));

        let restore_result = state.end(|endpoint| {
            restore_attempts.push(endpoint.to_string());
            Err("endpoint disappeared".to_string())
        });
        assert!(restore_result.is_err());
        assert_eq!(state.active_endpoint, None);
        assert_eq!(state.pending_restores, ["removed-endpoint"]);

        let new_endpoint_muted = Cell::new(false);
        state.begin(
            || {
                new_endpoint_muted.set(true);
                Ok(Some("current-endpoint".to_string()))
            },
            || panic!("no existing lease"),
            |endpoint| {
                restore_attempts.push(endpoint.to_string());
                if endpoint == "removed-endpoint" {
                    Err("endpoint is still unavailable".to_string())
                } else {
                    Ok(())
                }
            },
        );
        assert!(new_endpoint_muted.get());
        assert_eq!(state.active_endpoint.as_deref(), Some("current-endpoint"));
        assert_eq!(state.pending_restores, ["removed-endpoint"]);

        state
            .end(|endpoint| {
                restore_attempts.push(endpoint.to_string());
                Ok(())
            })
            .unwrap();
        assert_eq!(state.active_endpoint, None);

        state.begin(
            || Ok(None),
            || panic!("no existing lease"),
            |endpoint| {
                restore_attempts.push(endpoint.to_string());
                Ok(())
            },
        );
        assert!(state.pending_restores.is_empty());
        assert_eq!(state.active_endpoint, None);
        assert!(
            restore_attempts
                .iter()
                .filter(|id| *id == "removed-endpoint")
                .count()
                >= 3
        );
    }

    #[test]
    fn deferred_windows_restores_are_deduplicated_and_bounded() {
        let mut state = WindowsMuteState::new();
        for index in 0..12 {
            state.defer_restore(format!("endpoint-{index}"));
        }
        state.defer_restore("endpoint-11".to_string());

        assert_eq!(
            state.pending_restores.len(),
            super::MAX_PENDING_WINDOWS_RESTORES
        );
        assert_eq!(
            state.pending_restores.last().map(String::as_str),
            Some("endpoint-11")
        );
        let mut sorted = state.pending_restores.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), state.pending_restores.len());
    }

    #[test]
    fn unmute_retries_pending_restore_without_an_active_endpoint() {
        let mut state = WindowsMuteState::new();
        state.begin(
            || Ok(Some("temporarily-missing".to_string())),
            || panic!("no existing lease"),
            |_| Ok(()),
        );
        assert!(state
            .end(|_| Err("endpoint unavailable".to_string()))
            .is_err());
        assert_eq!(state.active_endpoint, None);
        assert_eq!(state.pending_restores, ["temporarily-missing"]);

        let mut restored = None;
        state
            .end(|endpoint| {
                restored = Some(endpoint.to_string());
                Ok(())
            })
            .unwrap();

        assert_eq!(restored.as_deref(), Some("temporarily-missing"));
        assert!(state.pending_restores.is_empty());
        assert_eq!(state.active_endpoint, None);
    }

    #[test]
    fn existing_windows_lease_requires_fresh_default_output_confirmation() {
        let mut state = WindowsMuteState::new();
        state.begin(
            || Ok(Some("original-output".into())),
            || unreachable!(),
            |_| Ok(()),
        );

        // Switching to an audible output, manually unmuting, or losing access
        // to the audio server must not confirm silence from a saved lease.
        for current in [Ok(false), Err("audio server unavailable".into()), Ok(true)] {
            let expected = if current == Ok(true) {
                MuteOutcome::AlreadyMuted
            } else {
                MuteOutcome::Failed
            };
            let (outcome, _) = state.begin(
                || panic!("must preserve restore snapshot"),
                || current,
                |_| panic!("must not restore during confirmation"),
            );
            assert_eq!(outcome, expected);
            assert_eq!(state.active_endpoint.as_deref(), Some("original-output"));
        }
        let mut restored = Vec::new();
        state
            .end(|endpoint| {
                restored.push(endpoint.to_owned());
                Ok(())
            })
            .unwrap();
        assert_eq!(restored, ["original-output"]);
    }
}
