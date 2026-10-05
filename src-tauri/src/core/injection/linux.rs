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

struct ChunkBackend<'a> {
    target: &'a crate::core::window_geometry::LinuxWindowTarget,
    expected: Option<String>,
}

async fn current_clipboard_text() -> Option<String> {
    tokio::time::timeout(SNAPSHOT_TIMEOUT, tokio::task::spawn_blocking(|| {
        let mut clipboard = Clipboard::new().ok()?;
        clipboard.get().clipboard(LinuxClipboardKind::Clipboard).text().ok()
    })).await.ok()?.ok().flatten()
}

async fn restore_owned_clipboard(saved: Option<ClipboardSnapshot>, expected: Option<&str>) {
    if let Some(expected) = expected {
        if current_clipboard_text().await.as_deref() == Some(expected) {
            if let Some(saved) = saved {
                let result = match saved {
                    ClipboardSnapshot::Data(sources) => restore_clipboard(sources).await,
                    ClipboardSnapshot::Empty => clear_clipboard().await,
                };
                if let Err(error) = result { log::warn!("Could not restore clipboard: {error}"); }
            }
        }
    }
}

impl chunks::PasteBackend for ChunkBackend<'_> {
    async fn check(&mut self) -> anyhow::Result<()> {
        let focused = crate::core::hyprland::active_window()
            .is_some_and(|window| window.address == self.target.address);
        anyhow::ensure!(focused, "Paste target lost focus");
        if let Some(expected) = &self.expected {
            anyhow::ensure!(current_clipboard_text().await.as_ref() == Some(expected), "Clipboard changed during paste");
        }
        Ok(())
    }
    async fn write(&mut self, text: &str) -> anyhow::Result<()> {
        write_clipboard(text.to_owned(), true).await?;
        self.expected = Some(text.to_owned());
        Ok(())
    }
    async fn paste(&mut self) -> anyhow::Result<()> {
        crate::core::hyprland::dispatch_paste_for_target(&self.target.class_name, &self.target.tags)
            .map_err(anyhow::Error::msg)
    }
}

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

#[cfg(test)]
mod chunk_native_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Requires an owned terminal fixture on Hyprland/Wayland"]
    async fn paste_chunks_native_terminal_preserves_clipboard() {
        let address = std::env::var("VERENU_CHUNK_FIXTURE_ADDRESS").expect("owned terminal address");
        let _lock = super::super::injection_lock().lock().await;
        let window = crate::core::hyprland::window_by_address(&address).expect("fixture window");
        assert_eq!(window.class_name, "foot.verenu-paste-chunks-fixture");
        crate::core::hyprland::focus(&address).unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        let target = crate::core::window_geometry::WindowTarget::capture_foreground();
        assert_eq!(target.linux.as_ref().unwrap().address, address);
        // Release the lock before calling the production injection function.
        drop(_lock);
        let original = snapshot_clipboard().await.unwrap();
        let sentinel = "Public synthetic clipboard sentinel";
        write_clipboard(sentinel.into(), true).await.unwrap();
        let text = "First public synthetic line about dictation.\nSecond public synthetic line about clipboard insertion.\nThird public synthetic line about preserving formatting.\nFourth public synthetic line about reviewing the prompt.\nFifth public synthetic line completing this fixture. Unicode: 👩🏽‍💻 café 尾.";
        let result = inject_text(text, &target, false, false, "casual", "en", false, true).await;
        let preserved = current_clipboard_text().await.as_deref() == Some(sentinel);
        restore_owned_clipboard(Some(original), Some(sentinel)).await;
        assert_eq!(result.unwrap().text, text);
        assert!(preserved, "Synthetic clipboard sentinel should be restored");
    }
}

// Keep the existing injection arguments explicit across native backends.
#[allow(clippy::too_many_arguments)]
pub(super) async fn inject_text(
    text: &str,
    target: &crate::core::window_geometry::WindowTarget,
    contextual_caps: bool,
    auto_spacing: bool,
    profile: &str,
    language: &str,
    protected_initial_case: bool,
    paste_in_chunks: bool,
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
    log::info!(
        "injection: Linux cursor formatting enabled={} source={} control={} left_reliable={} right_reliable={}",
        contextual_caps || auto_spacing,
        probe.source.as_str(),
        probe.control_type,
        probe.left_reliable,
        probe.right_reliable,
    );
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
    if paste_in_chunks {
        let mut backend = ChunkBackend { target: linux_target, expected: None };
        let result = chunks::paste(&mut backend, &adjusted).await;
        restore_owned_clipboard(saved, backend.expected.as_deref()).await;
        result?;
        return Ok(InjectionOutcome {
            text: adjusted, context_state: context_kind.as_str(), case_decision: case_decision.as_str(),
            probe_source: probe.source.as_str(), selection_state: probe.selection_state.as_str(),
        });
    }
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
