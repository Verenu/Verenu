//! Hyprland/Wayland clipboard paste. The compositor authenticates both focus
//! and shortcut dispatch; no input snooping, X11 hook, uinput, or root daemon.

use super::*;
use arboard::{ClearExtLinux, Clipboard, GetExtLinux, LinuxClipboardKind, SetExtLinux};
use std::{io::Read, time::Duration};
use wl_clipboard_rs::{
    copy::{self, ClipboardType as CopyClipboardType, MimeSource, MimeType, Options, Source},
    paste::{self, ClipboardType, MimeType as PasteMimeType, Seat},
};

const CLIPBOARD_SETTLE: Duration = Duration::from_millis(80);
const PASTE_SETTLE: Duration = Duration::from_millis(250);
const SNAPSHOT_TIMEOUT: Duration = Duration::from_millis(400);
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
const MAX_SNAPSHOT_MIME_TYPES: usize = 64;

enum ClipboardSnapshot {
    Empty,
    Data(Vec<MimeSource>),
}

fn read_clipboard_snapshot() -> anyhow::Result<ClipboardSnapshot> {
    let mime_types = match paste::get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified)
    {
        Ok(mime_types) => mime_types,
        Err(paste::Error::ClipboardEmpty | paste::Error::NoMimeType) => {
            return Ok(ClipboardSnapshot::Empty)
        }
        Err(err) => {
            return Err(anyhow::anyhow!(
                "Could not inspect Wayland clipboard: {err}"
            ))
        }
    };
    if mime_types.len() > MAX_SNAPSHOT_MIME_TYPES {
        anyhow::bail!("Clipboard has too many MIME types to preserve safely");
    }

    let mut total_bytes = 0usize;
    let mut sources = Vec::with_capacity(mime_types.len() + 1);
    for mime_type in mime_types {
        let (mut contents, actual_mime_type) = paste::get_contents(
            ClipboardType::Regular,
            Seat::Unspecified,
            PasteMimeType::Specific(&mime_type),
        )
        .map_err(|err| anyhow::anyhow!("Could not read Wayland clipboard data: {err}"))?;
        let remaining = MAX_SNAPSHOT_BYTES.saturating_sub(total_bytes);
        let mut bytes = Vec::new();
        contents
            .by_ref()
            .take(remaining.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|err| anyhow::anyhow!("Could not read Wayland clipboard data: {err}"))?;
        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > MAX_SNAPSHOT_BYTES {
            anyhow::bail!("Clipboard data is too large to preserve safely");
        }
        sources.push(MimeSource {
            source: Source::Bytes(bytes.into_boxed_slice()),
            mime_type: MimeType::Specific(actual_mime_type),
        });
    }

    if sources.is_empty() {
        Ok(ClipboardSnapshot::Empty)
    } else {
        // Match arboard's Linux hint so clipboard history skips this restore.
        sources.push(MimeSource {
            source: Source::Bytes(b"secret".to_vec().into_boxed_slice()),
            mime_type: MimeType::Specific("x-kde-passwordManagerHint".to_string()),
        });
        Ok(ClipboardSnapshot::Data(sources))
    }
}

async fn snapshot_clipboard() -> anyhow::Result<ClipboardSnapshot> {
    Ok(tokio::task::spawn_blocking(read_clipboard_snapshot).await??)
}

async fn restore_clipboard(sources: Vec<MimeSource>) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut options = Options::new();
        options.clipboard(CopyClipboardType::Regular);
        copy::copy_multi(options, sources)
            .map_err(|err| anyhow::anyhow!("Could not restore Wayland clipboard: {err}"))
    })
    .await??;
    Ok(())
}

pub(super) async fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    // An explicit copy is the user's own clipboard content; let history keep it.
    write_clipboard(text.to_owned(), false).await
}

/// Transient paste payloads and clipboard restores carry the KDE password
/// manager hint, which Omarchy and other clipboard histories skip, matching the
/// Windows exclusion from clipboard history.
async fn write_clipboard(text: String, exclude_from_history: bool) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut clipboard = Clipboard::new().map_err(|e| anyhow::anyhow!("Wayland clipboard unavailable: {e}"))?;
        let mut set = clipboard.set().clipboard(LinuxClipboardKind::Clipboard);
        if exclude_from_history {
            set = set.exclude_from_history();
        }
        set.text(text)
            .map_err(|e| anyhow::anyhow!("Could not write Wayland clipboard: {e}"))
    })
    .await??;
    Ok(())
}

async fn clear_clipboard() -> anyhow::Result<()> {
    tokio::task::spawn_blocking(|| {
        let mut clipboard = Clipboard::new().map_err(|e| anyhow::anyhow!("Wayland clipboard unavailable: {e}"))?;
        clipboard
            .clear_with()
            .clipboard(LinuxClipboardKind::Clipboard)
            .map_err(|e| anyhow::anyhow!("Could not clear Wayland clipboard: {e}"))
    })
    .await??;
    Ok(())
}

pub(super) async fn inject_text(
    text: &str,
    target: &crate::core::window_geometry::WindowTarget,
    contextual_caps: bool,
    auto_spacing: bool,
    profile: &str,
    language: &str,
    protected_initial_case: bool,
) -> anyhow::Result<InjectionOutcome> {
    let _guard = super::injection_lock().lock().await;
    let linux_target = target.linux.as_ref().ok_or_else(|| anyhow::anyhow!(
        "No Hyprland target was captured. Focus the app and start dictation again."
    ))?;
    // Refocus first: AT-SPI only reports a FOCUSED control inside the active
    // window, so the caret probe must run after the target is active again.
    crate::core::hyprland::focus(&linux_target.address).map_err(anyhow::Error::msg)?;
    tokio::time::sleep(Duration::from_millis(60)).await;
    let mut probe = if contextual_caps || auto_spacing {
        crate::core::context_probe::read_linux_injection_context_probe_async(linux_target.pid).await
    } else { unavailable_injection_probe() };
    // AT-SPI context probing is intentionally best effort; a failed probe
    // never prevents insertion.
    if probe.target_id != 0 && probe.target_id != linux_target.pid as usize { probe = unavailable_injection_probe(); }
    let (adjusted, context_kind, case_decision) = apply_probe_adjustments(
        text, contextual_caps, auto_spacing, profile, language, protected_initial_case, &probe,
    );
    // Preserving the existing clipboard is best effort. A broken or unusually
    // large clipboard must not prevent the dictated text from being inserted.
    let saved = match tokio::time::timeout(SNAPSHOT_TIMEOUT, snapshot_clipboard()).await {
        Ok(Ok(snapshot)) => Some(snapshot),
        Ok(Err(err)) => {
            log::warn!("injection: could not snapshot Wayland clipboard: {err}");
            None
        }
        Err(_) => {
            log::warn!("injection: Wayland clipboard snapshot timed out");
            None
        }
    };
    write_clipboard(adjusted.clone(), true).await?;
    tokio::time::sleep(CLIPBOARD_SETTLE).await;
    crate::core::hyprland::dispatch_paste_for_target(&linux_target.class_name, &linux_target.tags)
        .map_err(anyhow::Error::msg)?;
    tokio::time::sleep(PASTE_SETTLE).await;
    // Do not overwrite a user copy performed while the paste settled.
    let current = tokio::task::spawn_blocking(|| {
        let mut clipboard = Clipboard::new().ok()?;
        clipboard.get().clipboard(LinuxClipboardKind::Clipboard).text().ok()
    }).await.ok().flatten();
    if current.as_deref() == Some(adjusted.as_str()) {
        if let Some(saved) = saved {
            let restored = match saved {
                ClipboardSnapshot::Data(sources) => restore_clipboard(sources).await,
                ClipboardSnapshot::Empty => clear_clipboard().await,
            };
            if let Err(err) = restored {
                log::warn!("injection: could not restore Wayland clipboard: {err}");
            }
        }
    }
    Ok(InjectionOutcome { text: adjusted, context_state: context_kind.as_str(), case_decision: case_decision.as_str(), probe_source: probe.source.as_str(), selection_state: probe.selection_state.as_str() })
}
