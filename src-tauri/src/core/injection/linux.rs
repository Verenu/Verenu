//! Hyprland/Wayland clipboard paste. The compositor authenticates both focus
//! and shortcut dispatch; no input snooping, X11 hook, uinput, or root daemon.

use super::*;
use arboard::{Clipboard, GetExtLinux, LinuxClipboardKind, SetExtLinux};
use std::time::Duration;

const CLIPBOARD_SETTLE: Duration = Duration::from_millis(80);
const PASTE_SETTLE: Duration = Duration::from_millis(250);

pub(super) async fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    write_clipboard(text.to_owned()).await
}

async fn write_clipboard(text: String) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut clipboard = Clipboard::new().map_err(|e| anyhow::anyhow!("Wayland clipboard unavailable: {e}"))?;
        clipboard
            .set()
            .clipboard(LinuxClipboardKind::Clipboard)
            .text(text)
            .map_err(|e| anyhow::anyhow!("Could not write Wayland clipboard: {e}"))
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
    let mut probe = if contextual_caps || auto_spacing {
        crate::core::context_probe::read_injection_context_probe().await
    } else { unavailable_injection_probe() };
    // AT-SPI context probing is intentionally best effort; a failed probe
    // never prevents insertion.
    if probe.target_id != 0 && probe.target_id != linux_target.pid as usize { probe = unavailable_injection_probe(); }
    let (adjusted, context_kind, case_decision) = apply_probe_adjustments(
        text, contextual_caps, auto_spacing, profile, language, protected_initial_case, &probe,
    );
    let saved = tokio::task::spawn_blocking(|| {
        let mut clipboard = Clipboard::new().ok()?;
        clipboard.get().clipboard(LinuxClipboardKind::Clipboard).text().ok()
    }).await.ok().flatten();
    write_clipboard(adjusted.clone()).await?;
    tokio::time::sleep(CLIPBOARD_SETTLE).await;
    crate::core::hyprland::focus(&linux_target.address).map_err(anyhow::Error::msg)?;
    tokio::time::sleep(Duration::from_millis(60)).await;
    crate::core::hyprland::dispatch_paste_for_target(&linux_target.class_name, &linux_target.tags)
        .map_err(anyhow::Error::msg)?;
    tokio::time::sleep(PASTE_SETTLE).await;
    if let Some(saved) = saved {
        // Do not overwrite a user copy performed while the paste settled.
        // If a clipboard manager changed ownership we leave its newer value.
        let current = tokio::task::spawn_blocking(|| {
            let mut clipboard = Clipboard::new().ok()?;
            clipboard.get().clipboard(LinuxClipboardKind::Clipboard).text().ok()
        }).await.ok().flatten();
        if current.as_deref() == Some(adjusted.as_str()) { write_clipboard(saved).await?; }
    }
    Ok(InjectionOutcome { text: adjusted, context_state: context_kind.as_str(), case_decision: case_decision.as_str(), probe_source: probe.source.as_str(), selection_state: probe.selection_state.as_str() })
}
