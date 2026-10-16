use super::SharedState;
use crate::pipeline::pill_position::PillPlacement;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

const PILL_WIDTH_POINTS: f64 = super::DEFAULT_PILL_WIDTH_POINTS;
const PILL_HEIGHT_POINTS: f64 = super::DEFAULT_PILL_HEIGHT_POINTS;

#[cfg(target_os = "linux")]
const LINUX_EXIT_ANIMATION_MS: u64 = 260;

static REVEAL_GEN: AtomicU64 = AtomicU64::new(0);
static PILL_VISUALLY_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "linux")]
struct LinuxPillInput {
    interactive: bool,
    rect: Option<[i32; 4]>,
}
#[cfg(target_os = "linux")]
static LINUX_PILL_INPUT: Mutex<LinuxPillInput> = Mutex::new(LinuxPillInput {
    interactive: false,
    rect: None,
});
#[derive(Clone, Default)]
struct PillSnapshot {
    state: String,
    context: Option<String>,
}

impl PillSnapshot {
    fn set_state(&mut self, state: &str) {
        self.state = state.to_string();
        if !matches!(state, "processing" | "loading_local_model" | "handsfree") {
            self.context = None;
        }
    }
}

static CURRENT_PILL_SNAPSHOT: Mutex<PillSnapshot> = Mutex::new(PillSnapshot {
    state: String::new(),
    context: None,
});

#[cfg(target_os = "windows")]
static PILL_PLACED_ONCE: AtomicBool = AtomicBool::new(false);

static PENDING_PILL_CONTEXT: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn queue_pill_context(context: &str) {
    if let Ok(mut slot) = PENDING_PILL_CONTEXT.lock() {
        *slot = Some(context.to_string());
    }
}

pub(crate) fn current_pill_state() -> String {
    CURRENT_PILL_SNAPSHOT
        .lock()
        .ok()
        .filter(|snapshot| !snapshot.state.is_empty())
        .map(|snapshot| snapshot.state.clone())
        .unwrap_or_else(|| "idle".to_string())
}

pub(crate) fn replay_pill_state(app: &AppHandle) {
    let snapshot = CURRENT_PILL_SNAPSHOT
        .lock()
        .ok()
        .map(|snapshot| snapshot.clone());
    if let Some(snapshot) = snapshot {
        let state = if snapshot.state.is_empty() {
            "idle"
        } else {
            &snapshot.state
        };
        app.emit_to("pill", "pill-state", state).ok();
        if let Some(context) = snapshot.context {
            app.emit_to("pill", "pill-context", context).ok();
        }
    }
}

fn create_pill_if_needed(app: &AppHandle) -> bool {
    if app.get_webview_window("pill").is_some() {
        return false;
    }
    match tauri::WebviewWindowBuilder::new(app, "pill", tauri::WebviewUrl::App("/pill.html".into()))
        .title("Verenu Dictation Pill")
        .inner_size(PILL_WIDTH_POINTS, PILL_HEIGHT_POINTS)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        // GTK advertises a non-resizable window's size as both its min and
        // max, and Hyprland enforces those hints, pinning the pill at 200x200
        // so wide error states clipped. Without decorations or focus it still
        // cannot be resized by the user.
        .resizable(cfg!(target_os = "linux"))
        .shadow(false)
        .focused(false)
        .build()
    {
        Ok(pill) => {
            crate::apply_runtime_icons(app, None);
            #[cfg(target_os = "linux")]
            crate::system::linux_webview::configure_window(&pill);
            pill.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)))
                .ok();
            harden_pill_window(&pill);
            true
        }
        Err(err) => {
            log::warn!("Failed to create dictation pill window: {err}");
            false
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn initialize_pill(app: &AppHandle) {
    create_pill_if_needed(app);
}

#[cfg(target_os = "windows")]
fn harden_pill_window<R: Runtime>(pill: &WebviewWindow<R>) {
    use windows::Win32::{
        Foundation::{GetLastError, SetLastError, WIN32_ERROR},
        Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
            DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        },
        UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
            HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_CAPTION,
            WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
            WS_SYSMENU, WS_THICKFRAME,
        },
    };

    let Ok(hwnd) = pill.hwnd() else {
        return;
    };

    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &DWMWA_COLOR_NONE as *const _ as *const _,
            std::mem::size_of_val(&DWMWA_COLOR_NONE) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &DWMWCP_DONOTROUND as *const _ as *const _,
            std::mem::size_of_val(&DWMWCP_DONOTROUND) as u32,
        );
        SetLastError(WIN32_ERROR(0));
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if current == 0 {
            let err = GetLastError();
            if err != WIN32_ERROR(0) {
                log::warn!("Failed to read pill extended window styles: {err:?}");
                return;
            }
        }
        let desired = (current | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize)
            & !(WS_EX_APPWINDOW.0 as isize);

        if desired != current {
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired);
        }

        // Tauri creates this window borderless, but a later WebView2 hit-test
        // transition can make tao reapply caption styles. Remove every native
        // frame bit defensively so clicking the transparent area can never
        // expose a title bar, close button, or resize border around the pill.
        SetLastError(WIN32_ERROR(0));
        let current_style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        if current_style == 0 {
            let err = GetLastError();
            if err != WIN32_ERROR(0) {
                log::warn!("Failed to read pill window styles: {err:?}");
            }
        } else {
            let frame_bits = WS_CAPTION.0 as isize
                | WS_THICKFRAME.0 as isize
                | WS_SYSMENU.0 as isize
                | WS_MINIMIZEBOX.0 as isize
                | WS_MAXIMIZEBOX.0 as isize;
            let desired_style = current_style & !frame_bits;
            if desired_style != current_style {
                let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, desired_style);
            }
        }

        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );

        // Re-present the transparent overlay after native hit-testing or
        // frame changes. Windows can leave a WebView-backed overlay hidden
        // when it switches between click-through and interactive states.
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
}

#[cfg(not(target_os = "windows"))]
fn harden_pill_window<R: Runtime>(_pill: &WebviewWindow<R>) {}

fn apply_pill_hit_testing(pill: &WebviewWindow, interactive: bool) {
    // Linux owns an explicit GTK input region instead of tao's all-or-nothing
    // cursor-ignore flag: the transparent margin must pass input through,
    // and tao unwraps the GTK surface (aborting on
    // Hyprland) when it is toggled before the window is first realized.
    #[cfg(target_os = "linux")]
    {
        if let Ok(mut input) = LINUX_PILL_INPUT.lock() {
            input.interactive = interactive;
        }
        apply_linux_pill_input(pill);
    }
    #[cfg(not(target_os = "linux"))]
    {
        pill.set_ignore_cursor_events(!interactive).ok();
        harden_pill_window(pill);
        pill.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)))
            .ok();
    }
}

#[cfg(target_os = "linux")]
fn apply_linux_pill_input(pill: &WebviewWindow) {
    apply_linux_pill_input_for_window(pill, None);
}

#[cfg(target_os = "linux")]
fn apply_linux_pill_input_for_window(pill: &WebviewWindow, address: Option<String>) {
    let target = pill.clone();
    pill.run_on_main_thread(move || {
        let rect = LINUX_PILL_INPUT
            .lock()
            .ok()
            .and_then(|input| input.rect.filter(|_| input.interactive));
        crate::system::linux_webview::set_input_region(&target, rect);
        schedule_linux_pointer_sync(address);
    })
    .ok();
}

#[cfg(target_os = "linux")]
static LINUX_POINTER_SYNC_GENERATION: AtomicU64 = AtomicU64::new(0);
#[cfg(target_os = "linux")]
static LINUX_POINTER_SYNC_RUNNING: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "linux")]
fn schedule_linux_pointer_sync(mut address: Option<String>) {
    LINUX_POINTER_SYNC_GENERATION.fetch_add(1, Ordering::SeqCst);
    if LINUX_POINTER_SYNC_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn_blocking(move || loop {
        let generation = LINUX_POINTER_SYNC_GENERATION.load(Ordering::SeqCst);
        if let Some(address) = address.take().or_else(|| {
            crate::core::hyprland::pill_window().map(|window| window.address)
        }) {
            let interactive = LINUX_PILL_INPUT
                .lock()
                .map(|input| input.interactive && input.rect.is_some())
                .unwrap_or(false);
            if let Err(error) =
                crate::core::hyprland::set_pointer_input(&address, interactive)
            {
                log::warn!("{error}");
            }
        }
        LINUX_POINTER_SYNC_RUNNING.store(false, Ordering::SeqCst);
        if LINUX_POINTER_SYNC_GENERATION.load(Ordering::SeqCst) == generation
            || LINUX_POINTER_SYNC_RUNNING.swap(true, Ordering::SeqCst)
        {
            break;
        }
    });
}

#[cfg(target_os = "linux")]
pub(crate) fn set_pill_hit_rect(app: &AppHandle, x: f64, y: f64, width: f64, height: f64) {
    if ![x, y, width, height].iter().all(|v| v.is_finite()) {
        return;
    }
    let left = (x.floor() as i32).max(0);
    let top = (y.floor() as i32).max(0);
    let right = ((x + width).ceil() as i32).max(left);
    let bottom = ((y + height).ceil() as i32).max(top);
    let rect = [left, top, right - left, bottom - top];

    let interactive = {
        let Ok(mut input) = LINUX_PILL_INPUT.lock() else {
            return;
        };
        if input.rect == Some(rect) {
            return;
        }
        input.rect = Some(rect);
        input.interactive
    };
    if interactive {
        if let Some(pill) = app.get_webview_window("pill") {
            apply_linux_pill_input(&pill);
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn set_pill_hit_rect(_app: &AppHandle, _x: f64, _y: f64, _width: f64, _height: f64) {}

pub(crate) fn set_pill_interactive(
    app: &AppHandle,
    interactive: bool,
    expected_state: Option<&str>,
) {
    let Some(pill) = app.get_webview_window("pill") else {
        return;
    };
    let Ok(snapshot) = CURRENT_PILL_SNAPSHOT.lock() else {
        return;
    };
    let Some(interactive) = pill_interactivity_for_state(&snapshot.state, interactive, expected_state)
    else {
        return;
    };
    apply_pill_hit_testing(&pill, interactive);
    drop(snapshot);

    #[cfg(target_os = "linux")]
    if current_pill_state() != "idle" {
        raise_linux_pill_now();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            if current_pill_state() != "idle" && app.get_webview_window("pill").is_some() {
                raise_linux_pill_now();
            }
        });
    }
}

#[cfg(target_os = "linux")]
fn raise_linux_pill_now() {
    if let Some(window) = crate::core::hyprland::pill_window() {
        if let Err(error) = crate::core::hyprland::raise_window(&window.address) {
            log::warn!("Failed to keep Linux dictation pill above other windows: {error}");
        }
    }
}

pub(crate) fn show_pill(app: &AppHandle, state: &str) {
    show_pill_msg(app, state, None);
}

pub(crate) fn update_pill_state(app: &AppHandle, state: &str) {
    let Some(pill) = app.get_webview_window("pill") else {
        show_pill(app, state);
        return;
    };

    REVEAL_GEN.fetch_add(1, Ordering::SeqCst);
    super::pill_animation::cancel_pending_pill_tween();

    // Reuse the native reveal sequence without recalculating placement. The
    // Windows window can remain logically visible while its compositor surface
    // is behind another window after click-through is changed, so a conditional
    // `show()` is not enough here. `reveal_pill` uses SW_SHOWNOACTIVATE and
    // HWND_TOPMOST, which re-presents the existing window without activating it
    // or running the placement animation.
    reveal_pill(app, &pill, state, None);
}

fn show_pill_msg(app: &AppHandle, state: &str, message: Option<&str>) {
    if crate::is_dev_session() {
        app.emit(
            "verenu:pill-state",
            serde_json::json!({"state": state, "message": message}),
        )
        .ok();
        return;
    }
    show_native_pill_msg(app, state, message);
}

fn show_native_pill_msg(app: &AppHandle, state: &str, message: Option<&str>) {
    let created = create_pill_if_needed(app);
    let Some(pill) = app.get_webview_window("pill") else {
        return;
    };

    if created {
        wait_for_pill_frontend(app);
    }

    let generation = REVEAL_GEN.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
    #[cfg(not(target_os = "windows"))]
    let _ = generation;

    let Some(placement) = next_pill_placement(app, &pill) else {
        reveal_pill(app, &pill, state, message);
        return;
    };

    #[cfg(target_os = "windows")]
    let already_placed = PILL_PLACED_ONCE.swap(true, Ordering::SeqCst);

    #[cfg(target_os = "windows")]
    if let Some(current) = super::pill_position::current_placement(&pill) {
        let needs_animated_move = super::pill_position::should_animate_cross_monitor_move(
            already_placed,
            current,
            placement,
        );

        if crate::system::logger::is_verbose() {
            log::debug!(
                "pill show_pill_msg: state={state} already_placed={already_placed} current={current:?} target={placement:?} animate={needs_animated_move}"
            );
        }

        if needs_animated_move {
            let app = app.clone();
            let state = state.to_string();
            let message = message.map(str::to_string);
            super::pill_animation::animate_pill_placement(&pill, current, placement, move || {
                if REVEAL_GEN.load(Ordering::SeqCst) != generation {
                    return;
                }
                if let Some(pill) = app.get_webview_window("pill") {
                    reveal_pill(&app, &pill, &state, message.as_deref());
                }
            });
            return;
        }
    }

    super::pill_position::apply_pill_placement(&pill, placement);
    reveal_pill(app, &pill, state, message);
}

fn wait_for_pill_frontend(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    let Some(readiness) = app.try_state::<crate::FrontendReadiness>() else {
        return;
    };
    let deadline = Instant::now() + Duration::from_millis(750);
    while !readiness.pill.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !readiness.pill.load(Ordering::Acquire) {
        log::warn!("pill frontend did not complete its listener startup handshake");
    }
}

fn reveal_pill(app: &AppHandle, pill: &WebviewWindow, state: &str, message: Option<&str>) {
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    PILL_VISUALLY_ACTIVE.store(true, Ordering::SeqCst);

    if let Ok(mut current) = CURRENT_PILL_SNAPSHOT.lock() {
        current.set_state(state);
    }
    apply_pill_hit_testing(pill, false);

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
            SW_SHOWNOACTIVATE,
        };
        if let Ok(hwnd) = pill.hwnd() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    pill.show().ok();
    // GTK only has a realized input surface after show(). Applying the input
    // region before that point can abort tao on Wayland, so do it afterward.
    #[cfg(target_os = "linux")]
    apply_pill_hit_testing(pill, false);
    #[cfg(target_os = "linux")]
    place_linux_pill(app, true);

    // macOS: `show()` (orderFront:) is ignored for a background app, so the
    // pill only appeared when Verenu was frontmost. Force it above the
    // active app's windows without stealing focus. AppKit window calls must
    // run on the main thread - show_pill is invoked from pipeline worker
    // threads, so dispatch there (a raw msg_send off-thread raises an ObjC
    // exception and aborts the process).
    #[cfg(target_os = "macos")]
    {
        let pill_for_main = pill.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(ns_window) = pill_for_main.ns_window() {
                crate::system::mac_app::float_pill_window(ns_window);
            }
        });
    }

    if let Some(msg) = message {
        pill.emit("pill-error", msg).ok();
    }
    pill.emit("pill-state", state).ok();

    if let Some(context) = PENDING_PILL_CONTEXT
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
    {
        emit_pill_context(app, &context);
    }

    #[cfg(target_os = "linux")]
    schedule_linux_raise(app);
}

#[cfg(target_os = "linux")]
fn place_linux_pill(app: &AppHandle, wait_for_map: bool) {
    use crate::core::hyprland;

    let placement = app
        .try_state::<SharedState>()
        .and_then(|state| state.lock().ok().and_then(|guard| guard.pill_placement));
    let window = if wait_for_map {
        hyprland::pill_window_after_show()
    } else {
        hyprland::pill_window()
    };
    let Some(window) = window else {
        return;
    };

    if let Some(placement) = placement {
        let mut size = window.size;
        if size != [placement.width, placement.height]
            && hyprland::resize_window(&window.address, placement.width, placement.height).is_ok()
        {
            size = [placement.width, placement.height];
        }
        let target = super::pill_position::snap_linux_placement_to_mapped_size(placement, size);
        if super::pill_position::position_changed(window.at[0], target.x)
            || super::pill_position::position_changed(window.at[1], target.y)
        {
            if let Err(error) = hyprland::move_window(&window.address, target.x, target.y) {
                log::warn!("Failed to position Linux dictation pill: {error}");
            }
        }
    }
    if let Err(error) = hyprland::raise_window(&window.address) {
        log::warn!("Failed to raise Linux dictation pill: {error}");
    }
    if let Some(pill) = app.get_webview_window("pill") {
        apply_linux_pill_input_for_window(&pill, Some(window.address));
    }
}

/// GTK/Wayland maps and configures a shown WebView asynchronously. Hyprland can
/// acknowledge the first z-order request while that configure is still in
/// flight, then place the committed surface back below the previously focused
/// window, or apply its own default floating placement after our first move.
/// Re-assert placement and z-order after mapping and after the frontend's
/// first layout pass so neither a teleported nor a buried pill survives.
/// Generation and visibility checks prevent a stale retry from raising an idle
/// pill or a superseded state.
#[cfg(target_os = "linux")]
fn schedule_linux_raise(app: &AppHandle) {
    let app = app.clone();
    let generation = REVEAL_GEN.load(Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        for delay in [120_u64, 280, 600] {
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            if !PILL_VISUALLY_ACTIVE.load(Ordering::SeqCst)
                || REVEAL_GEN.load(Ordering::SeqCst) != generation
            {
                return;
            }
            if app.get_webview_window("pill").is_none() {
                return;
            }
            place_linux_pill(&app, false);
        }
    });
}

fn pill_state_has_clickable_buttons(state: &str) -> bool {
    matches!(
        state,
        "handsfree"
            | "error"
            | "cancelled"
            | "interrupted"
            | "paste_failed"
            | "copied"
            | "clipboard_warning"
    )
}

fn pill_interactivity_for_state(
    current: &str,
    requested: bool,
    expected: Option<&str>,
) -> Option<bool> {
    if expected.is_some_and(|state| state != current) {
        None
    } else {
        Some(requested && pill_state_has_clickable_buttons(current))
    }
}

fn next_pill_placement<R: Runtime>(
    app: &AppHandle,
    pill: &WebviewWindow<R>,
) -> Option<PillPlacement> {
    let (target_point, width_points, height_points, cached, stale) = {
        let state = app.try_state::<SharedState>()?;
        let guard = state.lock().ok()?;
        (
            guard.target.display_point,
            guard.pill_width_points,
            guard.pill_height_points,
            guard.pill_placement,
            guard.pill_placement_stale,
        )
    };

    if !stale && cached.is_some() {
        return None;
    }

    let resolved = super::pill_position::resolve_pill_placement(
        pill,
        target_point,
        width_points,
        height_points,
    )
    .or(cached);

    if let Some(placement) = resolved {
        if stale || cached != Some(placement) {
            if let Some(state) = app.try_state::<SharedState>() {
                if let Ok(mut guard) = state.lock() {
                    guard.pill_placement = Some(placement);
                    guard.pill_placement_stale = false;
                }
            }
        }
    }

    resolved
}

pub(crate) fn hide_pill(app: &AppHandle) {
    if crate::is_dev_session() {
        app.emit("verenu:pill-state", serde_json::json!({"state": "idle"}))
            .ok();
        if app.get_webview_window("pill").is_none() {
            return;
        }
    }
    if let Some(pill) = app.get_webview_window("pill") {
        PILL_VISUALLY_ACTIVE.store(false, Ordering::SeqCst);
        REVEAL_GEN.fetch_add(1, Ordering::SeqCst);
        super::pill_animation::cancel_pending_pill_tween();

        #[cfg(target_os = "linux")]
        let was_idle = current_pill_state() == "idle";
        if let Ok(mut current) = CURRENT_PILL_SNAPSHOT.lock() {
            current.set_state("idle");
        }
        pill.emit("pill-state", "idle").ok();
        #[cfg(target_os = "linux")]
        {
            apply_pill_hit_testing(&pill, false);
            if was_idle {
                pill.hide().ok();
            } else {
                let generation = REVEAL_GEN.load(Ordering::SeqCst);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(LINUX_EXIT_ANIMATION_MS))
                        .await;
                    if REVEAL_GEN.load(Ordering::SeqCst) == generation {
                        if let Some(pill) = app.get_webview_window("pill") {
                            pill.hide().ok();
                        }
                    }
                });
            }
        }
        // Do not call pill.hide() on Windows - hiding the window suspends the
        // WebView2 renderer. The next show_pill("recording") emit would then
        // be lost before WebView2 wakes up.
        // Hit-testing + harden only — never set_decorations(false) here (see
        // apply_pill_hit_testing / reveal_pill).
        #[cfg(not(target_os = "linux"))]
        apply_pill_hit_testing(&pill, false);
    }
}

#[cfg(all(feature = "native-testing", debug_assertions, desktop))]
pub(crate) fn native_test_pill(
    app: &AppHandle,
    state: Option<&str>,
    message: Option<&str>,
    context: Option<&str>,
) -> Result<serde_json::Value, String> {
    if !crate::is_dev_session() {
        return Err("Native pill tests require an isolated dev session".into());
    }
    if let Some(state) = state {
        if !matches!(
            state,
            "idle"
                | "recording"
                | "handsfree"
                | "processing"
                | "loading_local_model"
                | "error"
                | "cancelled"
                | "interrupted"
                | "paste_failed"
                | "copied"
                | "clipboard_warning"
        ) {
            return Err("Unknown pill test state".into());
        }
        if state == "idle" {
            hide_pill(app);
        } else {
            show_native_pill_msg(app, state, message);
            if let Some(context) = context {
                if let Ok(mut snapshot) = CURRENT_PILL_SNAPSHOT.lock() {
                    snapshot.context = Some(context.to_string());
                }
                app.emit_to("pill", "pill-context", context).ok();
            }
        }
    }
    let result = serde_json::json!({"state": current_pill_state()});
    #[cfg(target_os = "linux")]
    let result = {
        let mut result = result;
        if let Ok(input) = LINUX_PILL_INPUT.lock() {
            result["interactive"] = input.interactive.into();
            result["rect"] = serde_json::json!(input.rect);
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let owner = app.clone();
        app.run_on_main_thread(move || {
            use gtk::prelude::GtkWindowExt;
            let icon = |label| owner.get_webview_window(label)
                .and_then(|window| window.gtk_window().ok())
                .and_then(|window| window.icon());
            let main = icon("main");
            let pill = icon("pill");
            let matches = match (main, pill) {
                (Some(main), Some(pill)) => main.width() == pill.width()
                    && main.height() == pill.height()
                    && main.read_pixel_bytes() == pill.read_pixel_bytes(),
                _ => false,
            };
            let _ = sender.send(serde_json::json!({
                "windowClass": gtk::glib::prgname().map(|name| name.to_string()),
                "matchesMain": matches,
            }));
        }).map_err(|error| error.to_string())?;
        result["appIcon"] = receiver.recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        result
    };
    Ok(result)
}

pub(super) fn show_cancelled_pill(app: &AppHandle) {
    show_pill_msg(app, "cancelled", None);
}

pub(super) fn show_interrupted_pill(app: &AppHandle) {
    show_pill_msg(app, "interrupted", None);
}

pub(super) fn show_paste_failed_pill(app: &AppHandle) {
    if super::start_stop_sounds_enabled(app) {
        crate::media::sound::play(crate::media::sound::SoundCue::Error);
    }
    show_pill_msg(app, "paste_failed", Some("Not pasted"));
}

pub(super) fn show_partial_paste_pill(app: &AppHandle) {
    show_pill_msg(app, "paste_failed", Some("Partly pasted"));
}

pub(crate) fn show_copied_pill(app: &AppHandle, msg: &str) {
    if super::start_stop_sounds_enabled(app) {
        crate::media::sound::play(crate::media::sound::SoundCue::Stop);
    }
    show_pill_msg(app, "copied", Some(msg));
}

pub(crate) fn emit_pill_stage(app: &AppHandle, stage: &str) {
    if crate::is_dev_session() {
        app.emit("verenu:pill-stage", stage).ok();
        return;
    }
    if let Some(pill) = app.get_webview_window("pill") {
        pill.emit("pill-stage", stage).ok();
    }
}

pub(crate) fn emit_pill_context(app: &AppHandle, context: &str) {
    if let Ok(mut snapshot) = CURRENT_PILL_SNAPSHOT.lock() {
        snapshot.context = Some(context.to_string());
    }
    if crate::is_dev_session() {
        app.emit("verenu:pill-context", context).ok();
        return;
    }
    match app.get_webview_window("pill") {
        Some(pill) => {
            let sent = pill.emit("pill-context", context).is_ok();
            log::debug!("pill: context={context} sent={sent}");
        }
        None => log::debug!("pill: context={context} sent=false (no pill window)"),
    }
}

#[cfg(test)]
mod context_replay_tests {
    use super::{pill_interactivity_for_state, PillSnapshot};

    #[test]
    fn passive_pills_reject_even_an_explicit_input_enable() {
        for state in [
            "idle",
            "recording",
            "processing",
            "loading_local_model",
            "unknown",
        ] {
            assert_eq!(
                pill_interactivity_for_state(state, true, Some(state)),
                Some(false),
                "{state}"
            );
            assert_eq!(
                pill_interactivity_for_state(state, true, None),
                Some(false),
                "{state}"
            );
        }
    }

    #[test]
    fn actionable_pills_open_input_only_when_the_frontend_requests_it() {
        for state in [
            "handsfree",
            "error",
            "cancelled",
            "interrupted",
            "paste_failed",
            "copied",
            "clipboard_warning",
        ] {
            assert_eq!(
                pill_interactivity_for_state(state, true, Some(state)),
                Some(true),
                "{state}"
            );
            assert_eq!(
                pill_interactivity_for_state(state, false, Some(state)),
                Some(false),
                "{state}"
            );
        }
    }

    #[test]
    fn stale_enables_and_disables_do_not_override_a_new_pill_state() {
        assert_eq!(
            pill_interactivity_for_state("recording", true, Some("handsfree")),
            None
        );
        assert_eq!(
            pill_interactivity_for_state("handsfree", false, Some("recording")),
            None
        );
        assert_eq!(
            pill_interactivity_for_state("idle", true, Some("error")),
            None
        );
    }

    #[test]
    fn context_survives_local_model_loading_and_processing() {
        let mut snapshot = PillSnapshot::default();
        snapshot.set_state("recording");
        snapshot.context = Some("Synthetic context".to_string());
        for state in [
            "handsfree",
            "processing",
            "loading_local_model",
            "processing",
        ] {
            snapshot.set_state(state);
            assert_eq!(snapshot.context.as_deref(), Some("Synthetic context"));
        }
        let replay = snapshot.clone();
        assert_eq!(replay.state, "processing");
        assert_eq!(replay.context, snapshot.context);
    }

    #[test]
    fn new_and_terminal_states_do_not_replay_a_previous_context() {
        for state in [
            "recording",
            "idle",
            "error",
            "cancelled",
            "interrupted",
            "paste_failed",
            "copied",
            "clipboard_warning",
        ] {
            let mut snapshot = PillSnapshot {
                state: "processing".to_string(),
                context: Some("Previous context".to_string()),
            };
            snapshot.set_state(state);
            assert!(snapshot.context.is_none(), "stale context in {state}");
        }
    }
}

pub(super) async fn show_error_pill(app: &AppHandle, msg: &str) {
    log::error!("pipeline error: {msg}");
    app.emit("verenu:error", msg).ok();
    if super::start_stop_sounds_enabled(app) {
        crate::media::sound::play(crate::media::sound::SoundCue::Error);
    }
    show_pill_msg(app, "error", Some(msg));
}

pub(crate) fn show_clipboard_warning_pill(app: &AppHandle, msg: &str) {
    show_pill_msg(app, "clipboard_warning", Some(msg));
}

pub(super) fn reject_with_pill(app: &AppHandle, msg: &str) {
    app.emit("verenu:error", msg).ok();
    if super::start_stop_sounds_enabled(app) {
        crate::media::sound::play(crate::media::sound::SoundCue::Error);
    }
    show_pill_msg(app, "error", Some(msg));
}
