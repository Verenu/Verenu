//! Linux AT-SPI access to the focused editable text control.
//!
//! Smart capitalization/spacing and AutoLearn read a bounded window of text
//! around the caret of the focused control, exactly like UI Automation on
//! Windows. This uses the session accessibility bus only: no keyboard
//! snooping, `/dev/input`, or compositor privileges. Text read here must never
//! be logged.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const TEXT: &str = "org.a11y.atspi.Text";
const COLLECTION: &str = "org.a11y.atspi.Collection";

const STATE_ACTIVE: u32 = 1;
const STATE_EDITABLE: u32 = 7;
const STATE_FOCUSED: u32 = 12;
const STATE_SHOWING: u32 = 25;
const STATE_MANAGES_DESCENDANTS: u32 = 31;
const ROLE_PASSWORD_TEXT: u32 = 40;
const ROLE_ENTRY: u32 = 79;
const ROLE_TOOL_BAR: u32 = 63;
const ROLE_PAGE_TAB_LIST: u32 = 38;
const ROLE_DOCUMENT_FRAME: u32 = 82;
const ROLE_DOCUMENT_WEB: u32 = 95;

/// Per-call ceiling. A hung application must not stall injection.
const METHOD_TIMEOUT: Duration = Duration::from_millis(150);
/// Upper bound on objects visited by the fallback tree walk.
const WALK_BUDGET: usize = 600;
/// Total time allowed for the browser address-bar walk.
const ADDRESS_BAR_DEADLINE: Duration = Duration::from_millis(250);
/// Includes connection setup and application/frame discovery. The worker
/// may finish a timed-out D-Bus call later, but recording must continue.
const ADDRESS_BAR_TIMEOUT: Duration = Duration::from_millis(350);
static ADDRESS_BAR_BUSY: AtomicBool = AtomicBool::new(false);
/// Characters read on each side of the caret.
pub const LOCAL_TEXT_CHARS: i32 = 2048;

#[derive(Clone, Debug)]
struct ObjRef {
    bus: String,
    path: OwnedObjectPath,
}

/// A bounded window of the focused control's text around the caret. Offsets
/// are character (code point) offsets inside `text`.
#[derive(Clone, Debug)]
pub struct FocusedText {
    pub text: String,
    pub caret: usize,
    pub selection: Option<(usize, usize)>,
    pub starts_at_field_start: bool,
    pub ends_at_field_end: bool,
    pub field_empty: bool,
    pub control_type: String,
    pub pid: u32,
    pub identity: String,
}

pub enum FocusProbe {
    Text(FocusedText),
    NonTextFocus,
    Unavailable,
}

static ENABLED_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Ask toolkits to expose their accessibility trees. Chromium, Electron and Qt
/// read this flag when they start, so already-running apps pick it up after a
/// restart. This is the same switch screen readers flip.
pub fn ensure_accessibility_enabled() {
    if ENABLED_REQUESTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let result = Connection::session().and_then(|session| {
            session.call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "Set",
                &("org.a11y.Status", "IsEnabled", zbus::zvariant::Value::from(true)),
            )
        });
        if let Err(err) = result {
            ENABLED_REQUESTED.store(false, Ordering::SeqCst);
            log::debug!("atspi: could not enable toolkit accessibility: {err}");
        }
    });
}

fn connection_slot() -> &'static Mutex<Option<Connection>> {
    static SLOT: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn connect() -> zbus::Result<Connection> {
    let session = zbus::blocking::connection::Builder::session()?
        .method_timeout(METHOD_TIMEOUT)
        .build()?;
    let reply = session.call_method(
        Some("org.a11y.Bus"),
        "/org/a11y/bus",
        Some("org.a11y.Bus"),
        "GetAddress",
        &(),
    )?;
    let address: String = reply.body().deserialize()?;
    zbus::blocking::connection::Builder::address(address.as_str())?
        .method_timeout(METHOD_TIMEOUT)
        .build()
}

/// zbus is built with its tokio backend, so its blocking API calls
/// `Runtime::block_on`, which panics on a tokio worker thread ("Cannot start
/// a runtime from within a runtime"). Callers include async pipeline stages
/// (recording start reads the browser address bar), so AT-SPI work always
/// runs on its own OS thread when a runtime is current.
fn with_connection<T: Send>(f: impl FnOnce(&Connection) -> Option<T> + Send) -> Option<T> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return std::thread::scope(|scope| {
            scope
                .spawn(|| with_connection_on_this_thread(f))
                .join()
                .ok()
                .flatten()
        });
    }
    with_connection_on_this_thread(f)
}

fn with_connection_on_this_thread<T>(f: impl FnOnce(&Connection) -> Option<T>) -> Option<T> {
    ensure_accessibility_enabled();
    let mut slot = connection_slot().lock().ok()?;
    if slot.as_ref().is_some_and(|conn| conn.is_closed()) {
        *slot = None;
    }
    if slot.is_none() {
        match connect() {
            Ok(conn) => *slot = Some(conn),
            Err(err) => {
                log::debug!("atspi: accessibility bus unavailable: {err}");
                return None;
            }
        }
    }
    let conn = slot.as_ref()?.clone();
    drop(slot);
    f(&conn)
}

fn call<B, T>(conn: &Connection, obj: &ObjRef, iface: &str, method: &str, body: &B) -> Option<T>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    T: for<'de> serde::Deserialize<'de> + zbus::zvariant::Type,
{
    let reply = conn
        .call_method(Some(obj.bus.as_str()), obj.path.as_str(), Some(iface), method, body)
        .ok()?;
    reply.body().deserialize().ok()
}

fn get_property<T>(conn: &Connection, obj: &ObjRef, iface: &str, name: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    let value: OwnedValue = call(
        conn,
        obj,
        "org.freedesktop.DBus.Properties",
        "Get",
        &(iface, name),
    )?;
    T::try_from(value).ok()
}

fn states(conn: &Connection, obj: &ObjRef) -> Option<u64> {
    let words: Vec<u32> = call(conn, obj, ACCESSIBLE, "GetState", &())?;
    let low = u64::from(*words.first()?);
    let high = u64::from(words.get(1).copied().unwrap_or(0));
    Some(low | (high << 32))
}

fn has(states: u64, state: u32) -> bool {
    states & (1u64 << state) != 0
}

fn children(conn: &Connection, obj: &ObjRef) -> Vec<ObjRef> {
    call::<_, Vec<(String, OwnedObjectPath)>>(conn, obj, ACCESSIBLE, "GetChildren", &())
        .unwrap_or_default()
        .into_iter()
        .map(|(bus, path)| ObjRef { bus, path })
        .collect()
}

fn root(bus: &str) -> Option<ObjRef> {
    Some(ObjRef {
        bus: bus.to_string(),
        path: OwnedObjectPath::try_from(ROOT_PATH).ok()?,
    })
}

/// Accessibility application roots owned by `pid`. Chromium and Electron
/// register from the browser process, which is the process Hyprland reports.
fn app_roots_for_pid(conn: &Connection, pid: u32) -> Vec<ObjRef> {
    let Some(registry) = root(REGISTRY) else {
        return Vec::new();
    };
    children(conn, &registry)
        .into_iter()
        .filter(|app| {
            conn.call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetConnectionUnixProcessID",
                &(app.bus.as_str(),),
            )
            .ok()
            .and_then(|reply| reply.body().deserialize::<u32>().ok())
                == Some(pid)
        })
        .collect()
}

/// One in-process query on toolkits that implement Collection (atk-bridge,
/// used by GTK3, Firefox, Chromium and Electron).
fn focused_via_collection(conn: &Connection, scope: &ObjRef) -> Option<ObjRef> {
    // Browsers also mark their document container focused. Matching EDITABLE
    // here prevents the one-result limit from hiding the actual text field.
    let focused_bits = [(1i32 << STATE_FOCUSED) | (1i32 << STATE_EDITABLE), 0i32];
    let rule = (
        focused_bits.to_vec(),
        1i32, // ALL
        HashMap::<String, String>::new(),
        1i32, // ALL with no attributes imposes no restriction; 0 is INVALID.
        Vec::<i32>::new(),
        1i32,
        Vec::<String>::new(),
        1i32,
        false,
    );
    let matches: Vec<(String, OwnedObjectPath)> =
        call(conn, scope, COLLECTION, "GetMatches", &(rule, 1u32, 1i32, true))?;
    matches
        .into_iter()
        .next()
        .map(|(bus, path)| ObjRef { bus, path })
}

/// Bounded depth-first walk limited to showing objects in the active window.
fn focused_via_walk(conn: &Connection, app: &ObjRef) -> Option<ObjRef> {
    let mut frames = children(conn, app);
    if let Some(active) = frames
        .iter()
        .find(|frame| states(conn, frame).is_some_and(|s| has(s, STATE_ACTIVE)))
        .cloned()
    {
        frames = vec![active];
    }
    let mut stack: Vec<ObjRef> = frames.into_iter().rev().collect();
    let mut visited = 0;
    // Chromium also marks the focused web document as FOCUSED; keep looking
    // below it for the editable control and only fall back to it.
    let mut focused_container = None;
    while let Some(obj) = stack.pop() {
        visited += 1;
        if visited > WALK_BUDGET {
            break;
        }
        let Some(s) = states(conn, &obj) else { continue };
        if has(s, STATE_FOCUSED) {
            if has(s, STATE_EDITABLE) {
                return Some(obj);
            }
            focused_container.get_or_insert_with(|| obj.clone());
        }
        if !has(s, STATE_SHOWING) || has(s, STATE_MANAGES_DESCENDANTS) {
            continue;
        }
        stack.extend(children(conn, &obj).into_iter().rev());
    }
    focused_container
}

fn focused_object(conn: &Connection, pid: u32) -> Option<ObjRef> {
    for app in app_roots_for_pid(conn, pid) {
        // Chromium/Electron can expose a native placeholder until an assistive
        // client requests extended properties. GetAttributes activates the
        // renderer accessibility tree; GetState/GetChildren alone do not.
        // Discard attributes: they are not cursor text and may contain metadata.
        let _: Option<HashMap<String, String>> = call(conn, &app, ACCESSIBLE, "GetAttributes", &());
        let frames = children(conn, &app);
        for frame in &frames {
            let _: Option<HashMap<String, String>> = call(conn, frame, ACCESSIBLE, "GetAttributes", &());
        }
        let scopes = std::iter::once(app.clone()).chain(frames);
        for scope in scopes {
            if let Some(obj) = focused_via_collection(conn, &scope) {
                if states(conn, &obj)
                    .is_some_and(|s| has(s, STATE_FOCUSED) && has(s, STATE_EDITABLE))
                {
                    return Some(obj);
                }
            }
        }
        if let Some(obj) = focused_via_walk(conn, &app) {
            return Some(obj);
        }
    }
    None
}

fn char_slice(text: &str, start: usize, end: usize) -> String {
    text.chars().skip(start).take(end.saturating_sub(start)).collect()
}

fn read_text(conn: &Connection, obj: &ObjRef, pid: u32, radius: i32) -> FocusProbe {
    read_text_at_depth(conn, obj, pid, radius, 0)
}

fn read_text_at_depth(conn: &Connection, obj: &ObjRef, pid: u32, radius: i32, depth: usize) -> FocusProbe {
    if depth >= 8 {
        return FocusProbe::Unavailable;
    }
    let Some(s) = states(conn, obj) else {
        return FocusProbe::Unavailable;
    };
    let role: u32 = call(conn, obj, ACCESSIBLE, "GetRole", &()).unwrap_or(0);
    // Never read secrets out of password fields.
    if role == ROLE_PASSWORD_TEXT || !has(s, STATE_EDITABLE) {
        return FocusProbe::NonTextFocus;
    }
    let Some(count) = get_property::<i32>(conn, obj, TEXT, "CharacterCount") else {
        return FocusProbe::NonTextFocus;
    };
    // Chromium reports -1 when it does not know the caret. Treat that as
    // unknown instead of "start of field", which would force capitalization.
    let Some(caret) = get_property::<i32>(conn, obj, TEXT, "CaretOffset").filter(|c| *c >= 0) else {
        return FocusProbe::NonTextFocus;
    };
    let control_type: String = call(conn, obj, ACCESSIBLE, "GetRoleName", &())
        .unwrap_or_else(|| "text".to_string());
    let identity = crate::core::context_probe::stable_metadata_hash(&[
        &obj.bus,
        obj.path.as_str(),
        &control_type,
    ]);
    if count <= 0 {
        return FocusProbe::Text(FocusedText {
            text: String::new(),
            caret: 0,
            selection: None,
            starts_at_field_start: true,
            ends_at_field_end: true,
            field_empty: true,
            control_type,
            pid,
            identity,
        });
    }
    let caret = caret.clamp(0, count);
    let selection = if call::<_, i32>(conn, obj, TEXT, "GetNSelections", &()).unwrap_or(0) > 0 {
        call::<_, (i32, i32)>(conn, obj, TEXT, "GetSelection", &(0i32,))
            .map(|(a, b)| (a.min(b).clamp(0, count), a.max(b).clamp(0, count)))
            .filter(|(a, b)| a != b)
    } else {
        None
    };
    let (left_anchor, right_anchor) = match selection {
        Some((selection_start, selection_end)) => {
            (selection_start.min(caret), selection_end.max(caret))
        }
        None => (caret, caret),
    };
    let start = left_anchor.saturating_sub(radius).max(0);
    let end = (right_anchor.saturating_add(radius)).min(count);
    let Some(text) = call::<_, String>(conn, obj, TEXT, "GetText", &(start, end)) else {
        return FocusProbe::NonTextFocus;
    };
    // Rich contenteditable fields expose paragraphs as embedded objects. The
    // outer caret points at that object, not at the character inside it.
    // Follow only the object at the caret and require its own valid caret.
    let anchor = caret.min(count - 1);
    if selection.is_none() && text.chars().nth((anchor - start) as usize) == Some('\u{fffc}') {
        let embedded = (|| {
            let index: i32 = call(conn, obj, "org.a11y.atspi.Hypertext", "GetLinkIndex", &(anchor,))?;
            if index < 0 { return None; }
            let (bus, path): (String, OwnedObjectPath) =
                call(conn, obj, "org.a11y.atspi.Hypertext", "GetLink", &(index,))?;
            let link = ObjRef { bus, path };
            let (bus, path): (String, OwnedObjectPath) =
                call(conn, &link, "org.a11y.atspi.Hyperlink", "GetObject", &(0i32,))?;
            Some(ObjRef { bus, path })
        })();
        if let Some(embedded) = embedded {
            if let FocusProbe::Text(mut focused) = read_text_at_depth(conn, &embedded, pid, radius, depth + 1) {
                focused.starts_at_field_start &= anchor == 0;
                focused.ends_at_field_end &= anchor == count - 1;
                focused.field_empty &= count == 1;
                focused.identity = identity;
                return FocusProbe::Text(focused);
            }
        }
        // A replacement character is not real cursor text. Do not infer a
        // sentence start or spacing from an unresolved rich-text container.
        return FocusProbe::Unavailable;
    }
    // Some toolkits return fewer characters than requested (embedded objects).
    let len = text.chars().count();
    let local = |offset: i32| ((offset - start).max(0) as usize).min(len);
    FocusProbe::Text(FocusedText {
        caret: local(caret),
        selection: selection.map(|(a, b)| (local(a), local(b))),
        starts_at_field_start: start == 0,
        ends_at_field_end: end == count,
        field_empty: false,
        text,
        control_type,
        pid,
        identity,
    })
}

/// Reads the focused text control of the application owning `pid`.
pub fn read_focused(pid: u32, radius: i32) -> FocusProbe {
    if pid == 0 {
        return FocusProbe::Unavailable;
    }
    with_connection(|conn| {
        let read = || focused_object(conn, pid)
            .map(|obj| read_text(conn, &obj, pid, radius))
            .unwrap_or(FocusProbe::Unavailable);
        let probe = read();
        if matches!(probe, FocusProbe::Text(_)) {
            return Some(probe);
        }
        // Renderer activation is asynchronous. Give it one short retry on
        // the first read; the outer injection timeout still bounds this work.
        std::thread::sleep(Duration::from_millis(35));
        Some(read())
    })
    .unwrap_or(FocusProbe::Unavailable)
}

/// Compatibility entry point for native fixtures. Production uses the
/// captured title as well as the PID to distinguish browser windows.
#[cfg(test)]
fn read_address_bar(pid: u32) -> Option<String> {
    read_address_bar_for_window(pid, "")
}

fn address_bar_identifier(attributes: &HashMap<String, String>) -> bool {
    attributes.iter().any(|(key, value)| {
        matches!(key.as_str(), "class" | "id" | "html-id")
            && matches!(value.as_str(), "OmniboxView" | "OmniboxViewViews" | "urlbar" | "urlbar-input" | "urlbar-entry")
    })
}

fn frame_matches(active: bool, name: Option<&str>, title: &str) -> bool {
    // Chromium on Wayland can omit ACTIVE even for the compositor's focused
    // window. The caller validates its stable Hyprland address around this
    // read; an exact, unique frame title identifies the captured window here.
    if title.is_empty() { active } else { name == Some(title) }
}

fn is_browser_chrome(conn: &Connection, obj: &ObjRef, frame: &ObjRef) -> bool {
    let mut current = obj.clone();
    let mut toolbar = false;
    for _ in 0..16 {
        if current.bus == frame.bus && current.path == frame.path { return toolbar; }
        let role = call::<_, u32>(conn, &current, ACCESSIBLE, "GetRole", &());
        if matches!(role, Some(ROLE_DOCUMENT_WEB | ROLE_DOCUMENT_FRAME)) { return false; }
        toolbar |= role == Some(ROLE_TOOL_BAR);
        let Some((bus, path)) = get_property::<(String, OwnedObjectPath)>(conn, &current, ACCESSIBLE, "Parent") else { return false; };
        current = ObjRef { bus, path };
    }
    false
}

/// Let the toolkit find known browser-owned controls in one query, avoiding
/// a D-Bus round trip for every button/tab in a large Chromium window.
/// Verify ancestry before reading text, since page CSS can reuse class names.
fn address_bar_via_collection(conn: &Connection, frame: &ObjRef) -> Option<Option<String>> {
    let rule = (
        vec![1i32 << STATE_SHOWING, 0i32], 1i32,
        HashMap::from([("class".to_string(), "OmniboxViewViews".to_string())]), 1i32,
        // Collection's D-Bus roles are 32-bit bitset words, unlike the
        // libatspi constructor's enum array. Entry 79 is word 2, bit 15.
        // https://github.com/GNOME/at-spi2-core/blob/main/xml/Collection.xml
        vec![0i32, 0i32, 1i32 << (ROLE_ENTRY - 64), 0i32], 1i32,
        Vec::<String>::new(), 1i32, false,
    );
    let matches: Vec<(String, OwnedObjectPath)> =
        call(conn, frame, COLLECTION, "GetMatches", &(rule, 1u32, 8i32, true))?;
    for (bus, path) in matches {
        let obj = ObjRef { bus, path };
        if !is_browser_chrome(conn, &obj, frame) { continue; }
        let text = get_property::<i32>(conn, &obj, TEXT, "CharacterCount")
            .filter(|count| (1..=2048).contains(count))
            .and_then(|count| call::<_, String>(conn, &obj, TEXT, "GetText", &(0i32, count)));
        return Some(text);
    }
    None
}

/// Read browser chrome only. Activating Chromium's native placeholder is
/// necessary even when renderer accessibility has not been requested yet.
/// No page text, tab titles, or URL values are logged or cached.
pub fn read_address_bar_for_window(pid: u32, title: &str) -> Option<String> {
    if pid == 0 {
        return None;
    }
    bounded_address_read(&ADDRESS_BAR_BUSY, ADDRESS_BAR_TIMEOUT, {
        let title = title.to_owned();
        move || read_address_bar_inner(pid, &title)
    })
}

fn bounded_address_read(
    busy: &'static AtomicBool,
    timeout: Duration,
    read: impl FnOnce() -> Option<String> + Send + 'static,
) -> Option<String> {
    if busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return None;
    }
    struct BusyGuard(&'static AtomicBool);
    impl Drop for BusyGuard {
        fn drop(&mut self) { self.0.store(false, Ordering::Release); }
    }
    let guard = BusyGuard(busy);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new().name("verenu-browser-context".into()).spawn(move || {
        let _guard = guard;
        let _ = sender.send(read());
    }).ok()?;
    receiver.recv_timeout(timeout).ok().flatten()
}

fn read_address_bar_inner(pid: u32, title: &str) -> Option<String> {
    let started = std::time::Instant::now();
    with_connection(|conn| {
        for app in app_roots_for_pid(conn, pid) {
            let _: Option<HashMap<String, String>> = call(conn, &app, ACCESSIBLE, "GetAttributes", &());
            let frames: Vec<_> = children(conn, &app).into_iter().filter(|frame| {
                if started.elapsed() > ADDRESS_BAR_DEADLINE { return false; }
                let _: Option<HashMap<String, String>> = call(conn, frame, ACCESSIBLE, "GetAttributes", &());
                let active = states(conn, frame).is_some_and(|s| has(s, STATE_ACTIVE));
                let name = if title.is_empty() { None } else {
                    get_property::<String>(conn, frame, ACCESSIBLE, "Name")
                };
                frame_matches(active, name.as_deref(), title)
            }).collect();
            if frames.len() != 1 { continue; }
            if let Some(text) = address_bar_via_collection(conn, &frames[0]) { return text; }
            // Toolbar nodes are visited before siblings such as tab strips.
            // Never walk inactive frames when none matches the capture.
            let mut stack: Vec<(ObjRef, bool, Option<u32>)> = frames.into_iter().rev().map(|obj| (obj, false, None)).collect();
            let mut visited = 0;
            let mut fallback = None;
            let mut ambiguous = false;
            while let Some((obj, in_toolbar, cached_role)) = stack.pop() {
                visited += 1;
                // Runs on the recording-start path: bound total time as well
                // as node count so a slow browser cannot delay dictation.
                if visited > WALK_BUDGET || started.elapsed() > ADDRESS_BAR_DEADLINE {
                    break;
                }
                let role: u32 = cached_role.or_else(|| call(conn, &obj, ACCESSIBLE, "GetRole", &())).unwrap_or(0);
                if role == ROLE_DOCUMENT_WEB || role == ROLE_DOCUMENT_FRAME || role == ROLE_PAGE_TAB_LIST {
                    continue;
                }
                if role == ROLE_ENTRY {
                    if !states(conn, &obj).is_some_and(|state| has(state, STATE_SHOWING)) { continue; }
                    let attrs = call::<_, HashMap<String, String>>(conn, &obj, ACCESSIBLE, "GetAttributes", &()).unwrap_or_default();
                    let exact = address_bar_identifier(&attrs);
                    if !exact && !in_toolbar { continue; }
                    let text = get_property::<i32>(conn, &obj, TEXT, "CharacterCount")
                        .filter(|count| (1..=2048).contains(count))
                        .and_then(|count| call::<_, String>(conn, &obj, TEXT, "GetText", &(0i32, count)));
                    // A known omnibox is authoritative, including an empty
                    // value, a search query, or an internal browser page.
                    // Another toolbar input cannot replace that answer.
                    if exact { return text; }
                    if let Some(text) = text {
                        if crate::core::browser_probe::extract_domain(&text).is_some() {
                            ambiguous |= fallback.is_some();
                            fallback = Some(text);
                        }
                    }
                    continue;
                }
                let showing = states(conn, &obj).is_some_and(|s| has(s, STATE_SHOWING));
                if showing {
                    let mut next = Vec::new();
                    for child in children(conn, &obj).into_iter().take(WALK_BUDGET - visited).rev() {
                        if started.elapsed() > ADDRESS_BAR_DEADLINE { break; }
                        let role = call::<_, u32>(conn, &child, ACCESSIBLE, "GetRole", &());
                        next.push((child, in_toolbar || role == Some(ROLE_TOOL_BAR), role));
                    }
                    // The tree is normally small once page documents and
                    // tab strips are excluded. Prefer toolbars at each level.
                    next.sort_by_key(|(_, _, role)| *role == Some(ROLE_TOOL_BAR));
                    stack.extend(next);
                }
            }
            if !ambiguous && started.elapsed() <= ADDRESS_BAR_DEADLINE && fallback.is_some() {
                return fallback;
            }
        }
        None
    })
}

/// Focused text in the currently active Hyprland window.
pub fn read_active(radius: i32) -> FocusProbe {
    match crate::core::hyprland::active_window() {
        Some(window) => read_focused(window.pid, radius),
        None => FocusProbe::Unavailable,
    }
}

impl FocusedText {
    pub fn left_of_caret(&self) -> String {
        let edge = self.selection.map(|(a, _)| a).unwrap_or(self.caret);
        char_slice(&self.text, 0, edge)
    }

    pub fn right_of_caret(&self) -> String {
        let edge = self.selection.map(|(_, b)| b).unwrap_or(self.caret);
        char_slice(&self.text, edge, usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(text: &str, caret: usize, selection: Option<(usize, usize)>) -> FocusedText {
        FocusedText {
            text: text.to_string(),
            caret,
            selection,
            starts_at_field_start: true,
            ends_at_field_end: true,
            field_empty: text.is_empty(),
            control_type: "entry".into(),
            pid: 1,
            identity: String::new(),
        }
    }

    /// Regression: calling from a tokio worker used to panic inside zbus's
    /// blocking API and strand dictation in `Starting`.
    #[test]
    fn calls_from_a_tokio_runtime_do_not_panic() {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            tokio::spawn(async {
                let _ = read_address_bar(std::process::id());
                let _ = read_focused(std::process::id(), 16);
            })
            .await
            .expect("AT-SPI call must not panic on a runtime worker");
        });
    }

    #[test]
    fn caret_splits_on_character_offsets() {
        let focused = sample("héllo wörld", 6, None);
        assert_eq!(focused.left_of_caret(), "héllo ");
        assert_eq!(focused.right_of_caret(), "wörld");
    }

    #[test]
    fn selection_edges_exclude_selected_text() {
        let focused = sample("one two three", 3, Some((4, 7)));
        assert_eq!(focused.left_of_caret(), "one ");
        assert_eq!(focused.right_of_caret(), " three");
    }

    fn wait_for_fixture<T>(timeout: Duration, mut discover: impl FnMut(std::time::Instant) -> Option<T>) -> Option<T> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            let found = discover(deadline);
            if std::time::Instant::now() >= deadline {
                return None;
            }
            if found.is_some() {
                return found;
            }
            std::thread::sleep(Duration::from_millis(50).min(deadline.saturating_duration_since(std::time::Instant::now())));
        }
        None
    }

    struct DiscoveryChild(std::process::Child);

    impl Drop for DiscoveryChild {
        fn drop(&mut self) {
            // Also reap on parsing errors or unwinding. No discovery thread is detached.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn wait_for_discovery_child(child: &mut DiscoveryChild, deadline: std::time::Instant) -> std::io::Result<bool> {
        loop {
            if std::time::Instant::now() >= deadline {
                child.0.kill()?;
                child.0.wait()?;
                return Ok(false);
            }
            if let Some(status) = child.0.try_wait()? {
                return Ok(status.success() && std::time::Instant::now() < deadline);
            }
            std::thread::sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(std::time::Instant::now())));
        }
    }

    fn discover_fixture_in_process(deadline: std::time::Instant) -> Option<ObjRef> {
        use std::io::Read;
        // Synchronous zbus setup/calls can block beyond their method timeout.
        // Keep them in an owned process, cancellable at the same absolute deadline.
        // Inherit the tester's process group so runner cancellation covers both.
        let mut child = DiscoveryChild(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "core::atspi::tests::formatting_fixture_discovery_worker", "--ignored", "--nocapture"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn().expect("start owned discovery process"));
        if !wait_for_discovery_child(&mut child, deadline).expect("cancel/reap owned discovery process") {
            return None;
        }
        let mut output = String::new();
        child.0.stdout.take()?.read_to_string(&mut output).expect("read discovery result");
        let encoded = output.lines().find_map(|line| line.strip_prefix("VERENU_FIXTURE_DISCOVERED:"))?;
        let (bus, path): (String, String) = serde_json::from_str(encoded).expect("decode owned fixture identity");
        Some(ObjRef { bus, path: OwnedObjectPath::try_from(path).expect("fixture object path") })
    }

    #[test]
    fn formatting_fixture_readiness_waits_and_times_out() {
        let mut attempts = 0;
        assert_eq!(wait_for_fixture(Duration::from_secs(1), |_| {
            attempts += 1;
            (attempts == 3).then_some(42)
        }), Some(42));
        let started = std::time::Instant::now();
        assert!(wait_for_fixture::<()>(Duration::from_millis(100), |_| None).is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn formatting_fixture_readiness_rejects_late_success() {
        assert!(wait_for_fixture::<u32>(Duration::ZERO, |_| {
            panic!("expired deadline must not start discovery");
        }).is_none());
        assert_eq!(wait_for_fixture(Duration::from_millis(10), |_| {
            std::thread::sleep(Duration::from_millis(100));
            Some(42)
        }), None);
    }

    #[test]
    fn formatting_fixture_readiness_cancels_and_reaps_blocked_discovery() {
        let mut child = DiscoveryChild(std::process::Command::new("sleep")
            .arg("60").spawn().unwrap());
        let started = std::time::Instant::now();
        assert!(!wait_for_discovery_child(&mut child, started + Duration::from_millis(20)).unwrap());
        assert!(child.0.try_wait().unwrap().is_some(), "blocked discovery must be reaped");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    #[ignore]
    fn formatting_fixture_discovery_worker() {
        let pid = std::env::var("VERENU_FORMAT_FIXTURE_PID").unwrap().parse::<u32>().unwrap();
        let discovered = with_connection(|conn| {
            for app in app_roots_for_pid(conn, pid) {
                for frame in children(conn, &app) {
                    if get_property::<String>(conn, &frame, ACCESSIBLE, "Name").as_deref()
                        != Some("Verenu formatting verification") {
                        continue;
                    }
                    if let Some(obj) = focused_via_collection(conn, &frame) {
                        return Some(obj);
                    }
                }
            }
            None
        });
        if let Some(obj) = discovered {
            println!("VERENU_FIXTURE_DISCOVERED:{}", serde_json::to_string(&(obj.bus, obj.path.as_str())).unwrap());
        }
    }

    /// Opt-in verification against a disposable GTK entry, never a user's document.
    #[test]
    #[ignore]
    fn atspi_live_formats_disposable_entry() {
        let pid = std::env::var("VERENU_FORMAT_FIXTURE_PID")
            .expect("start the disposable formatting fixture first")
            .parse::<u32>().unwrap();
        let obj = wait_for_fixture(Duration::from_secs(10), discover_fixture_in_process).unwrap_or_else(|| {
            panic!("VERENU_FIXTURE_PREREQUISITE_UNAVAILABLE: expected PID/window/focused editable entry not discovered within 10 seconds");
        });
        for (index, (before, payload, expected)) in [
            ("", "hello", "Hello"),
            ("Hello", "World", " world"),
            ("Hello.", "next sentence", " Next sentence"),
            ("Hello ", "World", "world"),
        ].into_iter().enumerate() {
            with_connection(|conn| {
                let accepted: bool = call(conn, &obj, "org.a11y.atspi.EditableText",
                    "SetTextContents", &(before,)).unwrap();
                assert!(accepted);
                let accepted: bool = call(conn, &obj, TEXT, "SetCaretOffset",
                    &(before.chars().count() as i32,)).unwrap();
                assert!(accepted);
                Some(())
            }).unwrap();
            let probe = crate::core::context_probe::read_linux_injection_context_probe(pid);
            assert!(probe.left_reliable && probe.right_reliable);
            let adjusted = crate::core::text_context::decide_insertion(payload,
                crate::core::text_context::CaretTextContext {
                    left: &probe.context_tail,
                    right: &probe.context_head,
                    left_reliable: probe.left_reliable,
                    right_reliable: probe.right_reliable,
                    language: "en",
                    casing_enabled: true,
                    preserve_sentence_case: false,
                    protected_initial_case: false,
                });
            assert_eq!(adjusted.text, expected);
            println!("VERENU_FORMAT_CASE_PASSED:{}", index + 1);
        }
    }

    /// Live desktop check: `cargo test atspi_live -- --ignored --nocapture`
    /// with a text field focused in the active window. Prints lengths only.
    #[test]
    #[ignore]
    fn atspi_live_reads_active_focus() {
        let probe = match std::env::var("VERENU_ATSPI_PID").ok().and_then(|p| p.parse().ok()) {
            Some(pid) => read_focused(pid, LOCAL_TEXT_CHARS),
            None => read_active(LOCAL_TEXT_CHARS),
        };
        match probe {
            FocusProbe::Text(t) => println!(
                "text control={} len={} caret={} selection={:?}",
                t.control_type,
                t.text.chars().count(),
                t.caret,
                t.selection
            ),
            FocusProbe::NonTextFocus => println!("non-text focus"),
            FocusProbe::Unavailable => println!("unavailable"),
        }
    }
}

#[cfg(test)]
mod live_address_bar {
    #[test]
    fn slow_address_read_times_out_without_stacking_workers_and_recovers() {
        static BUSY: super::AtomicBool = super::AtomicBool::new(false);
        let (release, blocked) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        assert!(super::bounded_address_read(&BUSY, std::time::Duration::from_millis(20), move || {
            blocked.recv().unwrap();
            Some("https://example.com".into())
        }).is_none());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(super::bounded_address_read(&BUSY, std::time::Duration::from_millis(20), || {
            panic!("a second browser worker must not start while one is blocked")
        }).is_none());
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while BUSY.load(super::Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(super::bounded_address_read(&BUSY, std::time::Duration::from_secs(1), || {
            Some("https://example.org".into())
        }).as_deref(), Some("https://example.org"));
    }

    /// `VERENU_ATSPI_PID=<browser pid> cargo test atspi_live_address -- --ignored --nocapture`
    #[test]
    #[ignore = "requires an owned focused browser and VERENU_ATSPI_EXPECTED_DOMAIN"]
    fn atspi_live_address_bar() {
        let pid: u32 = std::env::var("VERENU_ATSPI_PID").expect("owned browser PID").parse().unwrap();
        let expected = std::env::var("VERENU_ATSPI_EXPECTED_DOMAIN").expect("public fixture domain");
        let window = std::env::var("VERENU_ATSPI_WINDOW_ADDRESS").ok()
            .and_then(|address| crate::core::hyprland::window_by_address(&address))
            .or_else(|| crate::core::hyprland::window_by_pid(pid)).expect("owned browser window");
        assert_eq!(window.pid, pid);
        crate::core::hyprland::focus(&window.address).expect("focus owned fixture");
        std::thread::sleep(std::time::Duration::from_millis(150));
        let target = crate::core::window_geometry::WindowTarget::capture_foreground();
        assert_eq!(target.id, pid as usize, "fixture must be focused");
        let started = std::time::Instant::now();
        let domain = crate::core::browser_probe::read_browser_domain_for_target(&target);
        if expected == "unavailable" {
            assert!(domain.is_none(), "ambiguous window must use app Context");
            return;
        }
        assert_eq!(domain.as_deref(), Some(expected.as_str()));
        assert!(started.elapsed() < std::time::Duration::from_millis(800), "domain read exceeded recording-start budget");
        let db = crate::data::db::open(":memory:").unwrap();
        let context = crate::data::db::insert_context_returning(&db, "Website fixture", None, None, None, None, false).unwrap();
        crate::data::db::assign_context_website(&db, context.id, &expected).unwrap();
        let resolved = crate::core::context::resolve_context_for_captured_window(
            &db, &target.process_name().unwrap(), domain.as_deref(), target.window_title().as_deref(), target.id,
        ).unwrap();
        assert_eq!(resolved.0.id, context.id);
        let mut wrong_window = target.clone();
        wrong_window.linux.as_mut().unwrap().address = "0x0".into();
        assert!(crate::core::browser_probe::read_browser_domain_for_target(&wrong_window).is_none());
        let mut changed_tab = target;
        changed_tab.linux.as_mut().unwrap().title = "Other public fixture tab".into();
        assert!(crate::core::browser_probe::read_browser_domain_for_target(&changed_tab).is_none());
    }

    #[test]
    fn address_search_requires_captured_active_frame_and_browser_identifiers() {
        assert!(super::frame_matches(true, Some("Fixture - Browser"), "Fixture - Browser"));
        assert!(super::frame_matches(false, Some("Fixture - Browser"), "Fixture - Browser"));
        assert!(!super::frame_matches(false, None, ""));
        assert!(!super::frame_matches(true, Some("Other - Browser"), "Fixture - Browser"));
        assert!(!super::frame_matches(true, None, "Fixture - Browser"));
        for value in ["OmniboxView", "OmniboxViewViews", "urlbar-input", "urlbar-entry"] {
            assert!(super::address_bar_identifier(&[("class".into(), value.into())].into()));
        }
        assert!(!super::address_bar_identifier(&[("class".into(), "SearchField".into())].into()));
    }
}
