//! XDG Desktop Portal global shortcut implementation for Wayland/Hyprland.
//!
//! The portal owns conflict detection and consent UI. We intentionally do not
//! read `/dev/input`, install X11 hooks, or use a privileged input helper.

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const CTRL: u32 = 1;
const ALT: u32 = 2;
const SHIFT: u32 = 3;
const SUPER: u32 = 4;
static KEY1: AtomicU32 = AtomicU32::new(CTRL);
// Ctrl+Space is a real two-key chord that is currently unused by Omarchy's
// global bindings (unlike Super+Space, which opens the Omarchy menu).
static KEY2: AtomicU32 = AtomicU32::new(200);
static HANDLESS: AtomicBool = AtomicBool::new(false);
static PROCESSING: AtomicU64 = AtomicU64::new(0);
/// Mirrors `PortalGesture::chord_active` across threads so Escape arming can
/// see a held chord. Updated by the portal thread on every edge; read by
/// `refresh_escape_listening` from any thread.
static CHORD_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Whether the dynamic bare-Escape Hyprland bind is currently installed.
/// Tracked so `hl.unbind("ESCAPE")` only ever removes a bind Verenu added
/// itself — never a pre-existing user binding on the same trigger.
static ESCAPE_ARMED: AtomicBool = AtomicBool::new(false);
/// App-scoped portal id of the trigger-less "cancel" shortcut (see
/// `PORTAL_CANCEL_ID`), resolved at startup. Needed to build the dynamic
/// Escape bind's `hl.dsp.global(...)` action.
static CANCEL_PORTAL_ID: OnceLock<String> = OnceLock::new();

static PRESS: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static RELEASE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static HANDLESS_CB: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static CANCEL: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static ESCAPE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static COPY: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static SUB_APP: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

const HANDSFREE_DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(350);
const TAP_MAX_HOLD: Duration = Duration::from_millis(250);
const STALE_CHORD_TIMEOUT: Duration = Duration::from_secs(2);
const PORTAL_SHORTCUT_ID: &str = "dictate";
const PORTAL_SHORTCUT_DESCRIPTION: &str = "Verenu dictation";
/// Second portal shortcut: the Escape-to-cancel action. Bound with NO
/// preferred trigger (verified: the portal registers a trigger-less action
/// fine) so there is nothing to collide with and no consent UI for a key the
/// user never presses directly. Hyprland dispatches this action through a
/// bare-Escape bind that only exists while a dictation is active (see
/// `refresh_escape_listening`) — a static Escape bind would swallow Escape in
/// every app, all the time.
const PORTAL_CANCEL_ID: &str = "cancel";
const PORTAL_CANCEL_DESCRIPTION: &str = "Verenu cancel";
/// Backoff between portal reconnect attempts. A dead portal stream used to
/// kill the hotkey thread silently (`else => break`), stranding any active
/// recording with no way to stop it short of killing the app.
const PORTAL_RECONNECT_DELAY: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PortalGestureAction {
    Press,
    Release,
    Handsfree,
    Cancel,
}

/// Turns the portal's chord-level Activated/Deactivated stream into the same
/// gesture contract used by the Windows and macOS hotkey backends.
///
/// Hyprland may deliver another Activated notification while a key is still
/// held. Tracking the physical chord edge here prevents that repeat from being
/// mistaken for the second tap of a hands-free gesture. Once a real quick tap
/// is released, its matching recording is cancelled and exactly one following
/// activation inside the double-tap window enters hands-free. The release of
/// that second tap is consumed here, so it cannot immediately stop the session
/// it just started.
#[derive(Default)]
struct PortalGesture {
    chord_active: bool,
    pressed_at: Option<Instant>,
    pending_tap_at: Option<Instant>,
    consume_deactivation: bool,
}

impl PortalGesture {
    fn activated(&mut self, now: Instant) -> Option<PortalGestureAction> {
        if self.chord_active
            && self
                .pressed_at
                .is_some_and(|pressed_at| now.duration_since(pressed_at) > STALE_CHORD_TIMEOUT)
        {
            // A repeat activation arriving minutes into a hold (key repeat)
            // or after a lost deactivation: re-anchor the hold clock so the
            // eventual release is measured from a sane edge. Logged at info:
            // this distinction is the primary signal when diagnosing a hold
            // that "keeps going" after release.
            if let Some(pressed_at) = self.pressed_at {
                log::info!(
                    "linux hotkey: stale chord reset after {}ms",
                    now.duration_since(pressed_at).as_millis()
                );
            }
            *self = Self::default();
        }
        if self.chord_active || self.consume_deactivation {
            return None;
        }

        if self.pending_tap_at.take().is_some_and(|released_at| {
            now.duration_since(released_at) <= HANDSFREE_DOUBLE_TAP_WINDOW
        }) {
            self.consume_deactivation = true;
            return Some(PortalGestureAction::Handsfree);
        }

        self.chord_active = true;
        self.pressed_at = Some(now);
        Some(PortalGestureAction::Press)
    }

    fn deactivated(&mut self, now: Instant) -> Option<PortalGestureAction> {
        if self.consume_deactivation {
            self.consume_deactivation = false;
            return None;
        }
        if !self.chord_active {
            return None;
        }

        self.chord_active = false;
        let held = self
            .pressed_at
            .take()
            .map(|pressed_at| now.duration_since(pressed_at))
            .unwrap_or_default();
        // Info-level by design: the held duration is the primary signal for
        // diagnosing stuck holds (a release measured from a stale repeat
        // edge classifies as a tap and cancels instead of transcribing).
        log::info!(
            "linux hotkey: chord deactivated after {}ms",
            held.as_millis()
        );
        if held < TAP_MAX_HOLD {
            self.pending_tap_at = Some(now);
            Some(PortalGestureAction::Cancel)
        } else {
            self.pending_tap_at = None;
            Some(PortalGestureAction::Release)
        }
    }
}

pub fn map_code_to_vk(code: &str) -> u32 {
    match code {
        "ControlLeft" | "ControlRight" => CTRL,
        "AltLeft" | "AltRight" => ALT,
        "ShiftLeft" | "ShiftRight" => SHIFT,
        "MetaLeft" | "MetaRight" => SUPER,
        "F5" => 105,
        "F1" => 101,
        "F2" => 102,
        "F3" => 103,
        "F4" => 104,
        "F6" => 106,
        "F7" => 107,
        "F8" => 108,
        "F9" => 109,
        "F10" => 110,
        "F11" => 111,
        "F12" => 112,
        "Space" => 200,
        _ => 0,
    }
}

fn portal_trigger(modifier: u32, key: u32) -> Option<String> {
    let modifier = match modifier {
        CTRL => "CTRL",
        ALT => "ALT",
        SHIFT => "SHIFT",
        SUPER => "SUPER",
        _ => return None,
    };
    let key = match key {
        101..=112 => format!("F{}", key - 100),
        200 => "SPACE".to_string(),
        _ => return None,
    };
    // Never request bare Space/Escape. The portal can present an alternative
    // if this preferred chord collides with a compositor or application rule.
    Some(format!("{modifier}+{key}"))
}

pub fn is_hotkey_available(key1: &str, key2: &str) -> bool {
    portal_trigger(map_code_to_vk(key1), map_code_to_vk(key2)).is_some()
}
pub fn update_keys(k1: u32, k2: u32) {
    KEY1.store(k1, Ordering::SeqCst);
    KEY2.store(k2, Ordering::SeqCst);
}
pub fn update_capture_keys(_k1: u32, _k2: u32) {}
pub fn reset_chord_state() {
    HANDLESS.store(false, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn set_handless_active(value: bool) {
    HANDLESS.store(value, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn begin_synthetic_paste_suppression(_duration_ms: u64) {}
pub fn set_processing_generation(generation: u64) {
    PROCESSING.store(generation, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn clear_processing_generation(expected: u64) {
    let _ = PROCESSING.compare_exchange(expected, 0, Ordering::SeqCst, Ordering::SeqCst);
    refresh_escape_listening();
}
pub fn is_win_key_down() -> bool {
    false
}
pub fn force_release_win_key() {}
pub fn caps_lock_is_on() -> bool {
    false
}

/// Lua snippet installing the bare-Escape bind that dispatches the portal
/// cancel action. Pure so the trigger text is unit-testable.
fn escape_bind_snippet(cancel_portal_id: &str) -> String {
    format!(
        "hl.bind(\"ESCAPE\", hl.dsp.global(\"{cancel_portal_id}\"), \
         {{ description = \"Verenu cancel dictation\" }})"
    )
}

fn escape_unbind_snippet() -> &'static str {
    "hl.unbind(\"ESCAPE\")"
}

fn eval_hyprland(snippet: &str) -> Result<(), String> {
    let status = std::process::Command::new("hyprctl")
        .args(["eval", snippet])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("hyprctl eval unavailable: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "hyprctl eval rejected the snippet".to_string())
}

/// Arms or disarms the bare-Escape cancel bind to match live dictation state.
///
/// Mirrors the macOS backend (`refresh_escape_listening` there registers a
/// Carbon Escape hotkey only while recording/hands-free/processing): Escape
/// must cancel an in-flight dictation, but a permanently installed bare-Escape
/// bind would swallow Escape in every application, all the time. Idempotent —
/// only shells out to `hyprctl` on transitions.
fn refresh_escape_listening() {
    let wanted = CHORD_ACTIVE.load(Ordering::SeqCst)
        || HANDLESS.load(Ordering::SeqCst)
        || PROCESSING.load(Ordering::SeqCst) != 0;
    if wanted == ESCAPE_ARMED.load(Ordering::SeqCst) {
        return;
    }
    if wanted {
        let Some(cancel_id) = CANCEL_PORTAL_ID.get() else {
            log::debug!("linux hotkey: Escape arming deferred — cancel action not resolved yet");
            return;
        };
        match eval_hyprland(&escape_bind_snippet(cancel_id)) {
            Ok(()) => {
                ESCAPE_ARMED.store(true, Ordering::SeqCst);
                log::info!("linux hotkey: Escape-to-cancel armed for this dictation");
            }
            Err(e) => log::warn!("linux hotkey: could not arm Escape-to-cancel: {e}"),
        }
    } else if eval_hyprland(escape_unbind_snippet()).is_ok() {
        ESCAPE_ARMED.store(false, Ordering::SeqCst);
        log::info!("linux hotkey: Escape-to-cancel disarmed");
    } else {
        // Stay armed so the next refresh retries the unbind instead of
        // leaking a live bare-Escape bind that swallows Escape everywhere.
        log::warn!("linux hotkey: could not disarm Escape-to-cancel, will retry");
    }
}

#[allow(clippy::too_many_arguments)]
pub fn start<P, R, H, C, E, L, S>(
    on_press: P,
    on_release: R,
    on_handless: H,
    on_cancel: C,
    on_escape: E,
    on_copy_last: L,
    on_capture_sub_app: S,
) -> Result<std::thread::JoinHandle<()>, String>
where
    P: Fn() + Send + Sync + 'static,
    R: Fn() + Send + Sync + 'static,
    H: Fn() + Send + Sync + 'static,
    C: Fn() + Send + Sync + 'static,
    E: Fn() + Send + Sync + 'static,
    L: Fn() + Send + Sync + 'static,
    S: Fn() + Send + Sync + 'static,
{
    let _ = PRESS.set(Box::new(on_press));
    let _ = RELEASE.set(Box::new(on_release));
    let _ = HANDLESS_CB.set(Box::new(on_handless));
    let _ = CANCEL.set(Box::new(on_cancel));
    let _ = ESCAPE.set(Box::new(on_escape));
    let _ = COPY.set(Box::new(on_copy_last));
    let _ = SUB_APP.set(Box::new(on_capture_sub_app));
    let trigger = portal_trigger(KEY1.load(Ordering::SeqCst), KEY2.load(Ordering::SeqCst))
        .ok_or_else(|| "Linux shortcut must be one modifier plus F1–F12 or Space".to_string())?;
    log::info!("linux hotkey: requesting XDG portal trigger {trigger}");
    Ok(std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(v) => v,
            Err(e) => {
                log::error!("portal runtime failed: {e}");
                return;
            }
        };
        runtime.block_on(async move {
            // A dropped portal stream must never kill dictation input
            // silently: an earlier version `break`ed out of the select loop,
            // ending the thread with no error surface, which stranded any
            // active recording (release could never arrive) until restart.
            loop {
                if run_portal_session(&trigger).await {
                    log::info!("linux hotkey: portal session ended, reconnecting");
                }
                tokio::time::sleep(PORTAL_RECONNECT_DELAY).await;
            }
        });
    }))
}

/// One portal session: bind, resolve compositor actions, and dispatch edges
/// until a stream ends. Returns always (the caller reconnects); logs loudly
/// on every failure so a dead hotkey is diagnosable from exported logs.
async fn run_portal_session(trigger: &str) -> bool {
    let portal = match GlobalShortcuts::new().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("XDG GlobalShortcuts portal unavailable: {e}");
            return true;
        }
    };
    let session = match portal.create_session(Default::default()).await {
        Ok(v) => v,
        Err(e) => {
            log::error!("XDG GlobalShortcuts session failed: {e}");
            return true;
        }
    };
    let before = list_portal_shortcuts();
    let shortcuts = [
        NewShortcut::new(PORTAL_SHORTCUT_ID, PORTAL_SHORTCUT_DESCRIPTION)
            .preferred_trigger(trigger),
        // No preferred trigger: this action exists only so the dynamic
        // bare-Escape bind has something to dispatch (see
        // `refresh_escape_listening`). Requesting no trigger means nothing
        // to collide with and no consent prompt for a key the user never
        // presses directly.
        NewShortcut::new(PORTAL_CANCEL_ID, PORTAL_CANCEL_DESCRIPTION),
    ];
    match portal
        .bind_shortcuts(&session, &shortcuts, None, Default::default())
        .await
        .and_then(|r| r.response())
    {
        Ok(_) => log::info!("linux hotkey: XDG portal shortcuts bound"),
        Err(e) => {
            log::error!("XDG portal rejected Verenu shortcuts (choose another shortcut): {e}");
            return true;
        }
    }
    // Hyprland requires a compositor-side bind to dispatch the portal
    // action. Discover its exact app-scoped ID, then update only the
    // marked block in the user's Lua bindings.
    let dictate_id = match discover_portal_id(&before, PORTAL_SHORTCUT_DESCRIPTION) {
        Some(id) => id,
        None => {
            log::error!("linux hotkey: Verenu portal ID was not visible in Hyprland");
            return true;
        }
    };
    log::info!("linux hotkey: discovered portal ID");
    // The static Lua block references the dictate action only; the cancel
    // action is dispatched through runtime-evaluated binds that resolve the
    // id below on every arm, so a reconnected session needs no config edit
    // for cancel — but the dictate bind may still reference the dead
    // session's handle, so always re-ensure it (the helper reloads even
    // when the text is unchanged, which re-resolves the action).
    if let Err(error) = crate::core::hyprland::ensure_global_shortcut_binding(
        &trigger.replace('+', " + "),
        &dictate_id,
    ) {
        log::error!("linux hotkey: Hyprland binding setup failed: {error}");
        return true;
    }
    log::info!("linux hotkey: Hyprland binding installed");
    match discover_portal_id(&before, PORTAL_CANCEL_DESCRIPTION) {
        Some(id) => {
            let _ = CANCEL_PORTAL_ID.set(id);
        }
        None => log::warn!(
            "linux hotkey: cancel portal action not visible — Escape-to-cancel unavailable this session"
        ),
    }
    let mut activated = match portal.receive_activated().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("portal activation stream failed: {e}");
            return true;
        }
    };
    let mut deactivated = match portal.receive_deactivated().await {
        Ok(v) => v,
        Err(e) => {
            log::error!("portal deactivation stream failed: {e}");
            return true;
        }
    };
    // Hyprland reports an activation each time Space goes down while
    // the configured modifier remains held, so both a full double-tap
    // of the chord and "hold Ctrl, tap Space twice" use this path.
    // Classify complete edges instead of treating any activation near
    // the previous release as hands-free; that distinction filters key
    // repeat and consumes the second tap's release.
    let mut gesture = PortalGesture::default();
    loop {
        tokio::select! {
            Some(event) = activated.next() => {
                let id = event.shortcut_id();
                if id == PORTAL_SHORTCUT_ID {
                    match gesture.activated(Instant::now()) {
                        Some(PortalGestureAction::Press) => {
                            // Info-level: one line per dictation start is the
                            // cheapest reliable trace for stuck-hold reports.
                            log::info!("linux hotkey: portal press");
                            CHORD_ACTIVE.store(true, Ordering::SeqCst);
                            refresh_escape_listening();
                            if let Some(cb) = PRESS.get() { cb(); }
                        }
                        Some(PortalGestureAction::Handsfree) => {
                            log::info!("linux hotkey: portal double-tap, toggling handsfree");
                            CHORD_ACTIVE.store(true, Ordering::SeqCst);
                            refresh_escape_listening();
                            if let Some(cb) = HANDLESS_CB.get() { cb(); }
                        }
                        _ => log::debug!("linux hotkey: ignored duplicate portal activation"),
                    }
                } else if id == PORTAL_CANCEL_ID {
                    log::info!("linux hotkey: portal Escape-to-cancel fired");
                    if let Some(cb) = ESCAPE.get() { cb(); }
                }
            },
            Some(event) = deactivated.next() => {
                if event.shortcut_id() == PORTAL_SHORTCUT_ID {
                    match gesture.deactivated(Instant::now()) {
                        Some(PortalGestureAction::Release) => {
                            log::info!("linux hotkey: portal hold released");
                            CHORD_ACTIVE.store(false, Ordering::SeqCst);
                            refresh_escape_listening();
                            if let Some(cb) = RELEASE.get() { cb(); }
                        }
                        Some(PortalGestureAction::Cancel) => {
                            log::info!(
                                "linux hotkey: portal quick tap cancelled; handsfree armed"
                            );
                            CHORD_ACTIVE.store(false, Ordering::SeqCst);
                            refresh_escape_listening();
                            if let Some(cb) = CANCEL.get() { cb(); }
                        }
                        _ => log::info!("linux hotkey: consumed portal deactivation"),
                    }
                }
            },
            else => {
                log::error!("linux hotkey: portal event streams ended — reconnecting");
                CHORD_ACTIVE.store(false, Ordering::SeqCst);
                refresh_escape_listening();
                return true;
            },
        }
    }
}

#[cfg(target_os = "linux")]
fn list_portal_shortcuts() -> Vec<(String, String)> {
    let output = std::process::Command::new("hyprctl")
        .arg("globalshortcuts")
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    parse_portal_shortcuts(&String::from_utf8_lossy(&output.stdout))
}

fn parse_portal_shortcuts(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (id, description) = line.split_once(" -> ")?;
            let id = id.trim();
            let description = description.trim();
            (!id.is_empty() && !description.is_empty())
                .then(|| (id.to_string(), description.to_string()))
        })
        .collect()
}

/// Prefer a shortcut that appeared after we bound, with Verenu's description.
/// Never grab another app's leftover "Hold to dictate" ID just because it is
/// listed first — that is how Ctrl+Space started dispatching a dead t3code
/// action while the live portal sat unused.
fn discover_portal_id(before: &[(String, String)], description: &str) -> Option<String> {
    let after = list_portal_shortcuts();
    pick_portal_id(before, &after, description)
}

fn pick_portal_id(
    before: &[(String, String)],
    after: &[(String, String)],
    description: &str,
) -> Option<String> {
    let before_ids: Vec<&str> = before.iter().map(|(id, _)| id.as_str()).collect();
    let matching: Vec<&(String, String)> = after
        .iter()
        .filter(|(_, candidate)| candidate == description)
        .collect();
    matching
        .iter()
        .find(|(id, _)| !before_ids.contains(&id.as_str()))
        .or_else(|| matching.last())
        .map(|(id, _)| id.clone())
}

#[cfg(test)]
mod tests {
    use super::{
        escape_bind_snippet, escape_unbind_snippet, parse_portal_shortcuts, pick_portal_id,
        PortalGesture, PortalGestureAction, PORTAL_CANCEL_DESCRIPTION, PORTAL_SHORTCUT_DESCRIPTION,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn hold_starts_on_press_and_processes_on_release() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(
            gesture.deactivated(start + Duration::from_millis(400)),
            Some(PortalGestureAction::Release)
        );
    }

    #[test]
    fn quick_double_tap_enters_handsfree_and_consumes_second_release() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(
            gesture.deactivated(start + Duration::from_millis(80)),
            Some(PortalGestureAction::Cancel)
        );
        assert_eq!(
            gesture.activated(start + Duration::from_millis(180)),
            Some(PortalGestureAction::Handsfree)
        );
        assert_eq!(
            gesture.deactivated(start + Duration::from_millis(240)),
            None
        );
    }

    #[test]
    fn repeated_activation_during_one_hold_cannot_trigger_handsfree() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(gesture.activated(start + Duration::from_millis(100)), None);
        assert_eq!(
            gesture.deactivated(start + Duration::from_millis(500)),
            Some(PortalGestureAction::Release)
        );
    }

    #[test]
    fn second_tap_after_window_is_a_fresh_hold() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(
            gesture.deactivated(start + Duration::from_millis(80)),
            Some(PortalGestureAction::Cancel)
        );
        assert_eq!(
            gesture.activated(start + Duration::from_millis(500)),
            Some(PortalGestureAction::Press)
        );
    }

    #[test]
    fn unmatched_deactivation_is_ignored() {
        let mut gesture = PortalGesture::default();
        assert_eq!(gesture.deactivated(Instant::now()), None);
    }

    #[test]
    fn stale_held_chord_recovers_on_the_next_activation() {
        let start = Instant::now();
        let mut gesture = PortalGesture::default();

        assert_eq!(gesture.activated(start), Some(PortalGestureAction::Press));
        assert_eq!(
            gesture.activated(start + Duration::from_secs(3)),
            Some(PortalGestureAction::Press)
        );
    }

    #[test]
    fn portal_id_prefers_the_new_verenu_description_over_a_stale_hold_to_dictate() {
        let before = parse_portal_shortcuts(
            "t3code:dictate -> Hold to dictate\ncom.t3tools.T3Code:capture-window -> Capture a window\n",
        );
        let after = parse_portal_shortcuts(
            "t3code:dictate -> Hold to dictate\ncom.t3tools.T3Code:capture-window -> Capture a window\ncom.t3tools.T3Code:dictate -> Verenu dictation\n",
        );

        assert_eq!(
            pick_portal_id(&before, &after, PORTAL_SHORTCUT_DESCRIPTION).as_deref(),
            Some("com.t3tools.T3Code:dictate")
        );
        assert_eq!(PORTAL_SHORTCUT_DESCRIPTION, "Verenu dictation");
    }

    #[test]
    fn portal_id_does_not_grab_the_first_hold_to_dictate_leftover() {
        let listing = parse_portal_shortcuts(
            "t3code:dictate -> Hold to dictate\ncom.t3tools.T3Code:dictate -> Hold to dictate\n",
        );
        assert_eq!(
            pick_portal_id(&listing, &listing, PORTAL_SHORTCUT_DESCRIPTION),
            None
        );
    }

    #[test]
    fn cancel_action_is_discovered_independently_of_the_dictate_action() {
        let before = parse_portal_shortcuts("com.t3tools.T3Code:dictate -> Verenu dictation\n");
        let after = parse_portal_shortcuts(
            "com.t3tools.T3Code:dictate -> Verenu dictation\ncom.t3tools.T3Code:cancel -> Verenu cancel\n",
        );

        assert_eq!(
            pick_portal_id(&before, &after, PORTAL_CANCEL_DESCRIPTION).as_deref(),
            Some("com.t3tools.T3Code:cancel")
        );
        // The dictate lookup must not grab the cancel action and vice versa.
        assert_eq!(
            pick_portal_id(&before, &after, PORTAL_SHORTCUT_DESCRIPTION).as_deref(),
            Some("com.t3tools.T3Code:dictate")
        );
        assert_eq!(PORTAL_CANCEL_DESCRIPTION, "Verenu cancel");
    }

    #[test]
    fn escape_bind_targets_the_cancel_action_and_unbind_clears_bare_escape() {
        let snippet = escape_bind_snippet("com.t3tools.T3Code:cancel");
        assert!(snippet.contains("hl.bind(\"ESCAPE\""));
        assert!(snippet.contains("hl.dsp.global(\"com.t3tools.T3Code:cancel\")"));
        assert_eq!(escape_unbind_snippet(), "hl.unbind(\"ESCAPE\")");
    }
}
