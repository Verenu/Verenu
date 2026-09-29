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
const ROLE_DOCUMENT_FRAME: u32 = 82;
const ROLE_DOCUMENT_WEB: u32 = 95;

/// Per-call ceiling. A hung application must not stall injection.
const METHOD_TIMEOUT: Duration = Duration::from_millis(150);
/// Upper bound on objects visited by the fallback tree walk.
const WALK_BUDGET: usize = 600;
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
            log::debug!("atspi: could not enable toolkit accessibility: {err}");
        }
    });
}

fn connection_slot() -> &'static Mutex<Option<Connection>> {
    static SLOT: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn connect() -> zbus::Result<Connection> {
    let session = Connection::session()?;
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

fn with_connection<T>(f: impl FnOnce(&Connection) -> Option<T>) -> Option<T> {
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
    let focused_bits = [(1i32 << STATE_FOCUSED), 0i32];
    let rule = (
        focused_bits.to_vec(),
        1i32, // ALL
        HashMap::<String, String>::new(),
        0i32,
        Vec::<i32>::new(),
        0i32,
        Vec::<String>::new(),
        0i32,
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
        let scopes = std::iter::once(app.clone()).chain(children(conn, &app));
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
        let obj = focused_object(conn, pid)?;
        Some(read_text(conn, &obj, pid, radius))
    })
    .unwrap_or(FocusProbe::Unavailable)
}

/// Text of the first browser-chrome entry (outside web content) that looks
/// like an address. Chromium labels and structures its omnibox differently
/// from Firefox, so this matches on shape rather than on names.
pub fn read_address_bar(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    with_connection(|conn| {
        for app in app_roots_for_pid(conn, pid) {
            let mut frames = children(conn, &app);
            if let Some(active) = frames
                .iter()
                .find(|frame| states(conn, frame).is_some_and(|s| has(s, STATE_ACTIVE)))
                .cloned()
            {
                frames = vec![active];
            }
            let mut stack: Vec<ObjRef> = frames.into_iter().rev().collect();
            let mut visited = 0;
            while let Some(obj) = stack.pop() {
                visited += 1;
                if visited > WALK_BUDGET {
                    break;
                }
                let role: u32 = call(conn, &obj, ACCESSIBLE, "GetRole", &()).unwrap_or(0);
                if role == ROLE_DOCUMENT_WEB || role == ROLE_DOCUMENT_FRAME {
                    continue;
                }
                if role == ROLE_ENTRY {
                    let text = get_property::<i32>(conn, &obj, TEXT, "CharacterCount")
                        .filter(|count| (1..=2048).contains(count))
                        .and_then(|count| call::<_, String>(conn, &obj, TEXT, "GetText", &(0i32, count)));
                    if let Some(text) = text {
                        if crate::core::browser_probe::extract_domain(&text).is_some() {
                            return Some(text);
                        }
                    }
                    continue;
                }
                let showing = states(conn, &obj).is_some_and(|s| has(s, STATE_SHOWING));
                if showing {
                    stack.extend(children(conn, &obj).into_iter().rev());
                }
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
    /// `VERENU_ATSPI_PID=<browser pid> cargo test atspi_live_address -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn atspi_live_address_bar() {
        let pid = std::env::var("VERENU_ATSPI_PID").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
        let domain = super::read_address_bar(pid)
            .and_then(|text| crate::core::browser_probe::extract_domain(&text));
        println!("domain={domain:?}");
    }
}
