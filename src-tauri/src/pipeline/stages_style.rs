use super::*;
use crate::core::window_geometry::WindowTarget;
pub(super) fn resolve_app_mapping(
    store: Option<&store::SettingsSnapshot>,
    process_name: &str,
) -> Option<AppMapping> {
    store.and_then(|s| {
        s.get(store::APP_MAPPINGS)
            .and_then(|v| serde_json::from_value::<Vec<AppMapping>>(v.clone()).ok())
            .and_then(|list| {
                list.into_iter()
                    .find(|m| m.exe.trim().eq_ignore_ascii_case(process_name))
            })
    })
}

pub(super) fn apply_app_style_overrides(
    cfg: &mut store::PipelineConfig,
    mapping: Option<&AppMapping>,
    context: Option<&db::Context>,
    everywhere_context: Option<&db::Context>,
) -> String {
    cfg.paste_in_chunks = cfg!(desktop) && context.is_some_and(|c| c.paste_in_chunks);
    let specific_context = context.filter(|value| !value.is_everywhere);
    let global_context = context
        .filter(|value| value.is_everywhere)
        .or(everywhere_context);
    let context_intensity = context
        .filter(|value| !value.is_everywhere)
        .and_then(|c| c.cleanup_intensity.as_deref())
        .map(str::trim)
        .filter(|i| !i.is_empty());
    let mapping_intensity = mapping
        .and_then(|m| m.cleanup_intensity.as_deref())
        .map(str::trim)
        .filter(|i| !i.is_empty());
    let everywhere_intensity = global_context
        .and_then(|c| c.cleanup_intensity.as_deref())
        .map(str::trim)
        .filter(|i| !i.is_empty());
    if let Some(intensity) = context_intensity
        .or(mapping_intensity)
        .or(everywhere_intensity)
    {
        cfg.cleanup_intensity = intensity.to_owned();
    }
    let formatting_disabled_override = specific_context
        .and_then(|value| value.contextual_formatting_override)
        .or_else(|| {
            specific_context
                .filter(|value| value.contextual_formatting_disabled)
                .map(|_| true)
        })
        .or_else(|| {
            global_context.and_then(|value| {
                value
                    .contextual_formatting_override
                    .or_else(|| value.contextual_formatting_disabled.then_some(true))
            })
        });
    if let Some(disabled) = formatting_disabled_override {
        cfg.contextual_formatting_enabled = !disabled;
    }

    let context_tone = specific_context
        .and_then(|c| c.tone.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let mapping_profile = mapping.map(|m| m.profile.trim()).filter(|p| !p.is_empty());
    let everywhere_tone = global_context
        .and_then(|c| c.tone.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty());
    context_tone
        .or(mapping_profile)
        .or(everywhere_tone)
        .map(str::to_owned)
        .unwrap_or_else(|| cfg.default_tone.clone())
}

pub(super) fn resolve_context_for_window(
    app: &AppHandle,
    target: &WindowTarget,
) -> Option<crate::core::context::ResolvedContextIdentity> {
    let process_name = target.process_name().unwrap_or_default();
    let browser_domain = if window_context::is_browser_target(&process_name, target.id) {
        crate::core::browser_probe::read_browser_domain_for_target(target)
    } else {
        None
    };
    let window_title = target.window_title();
    let db_handle = app.state::<crate::DbHandle>().inner().clone();
    let (context, sub_app) = crate::core::context::resolve_context_for_captured_window(
        &db_handle,
        &process_name,
        browser_domain.as_deref(),
        window_title.as_deref(),
        target.id,
    )
    .ok()?;
    let mut identity = crate::core::context::ResolvedContextIdentity::from_context_and_sub_app(
        &context,
        sub_app.as_ref(),
    );
    identity.t3_skills = store::settings_snapshot(app)
        .ok()
        .as_ref()
        .and_then(|settings| {
            crate::commands::catalog_for_destination(
                settings,
                &process_name,
                context.t3_skill_mentions_disabled,
            )
        });
    Some(identity)
}

pub(super) fn emit_context_for_window(
    app: &AppHandle,
    target: &WindowTarget,
) -> Option<crate::core::context::ResolvedContextIdentity> {
    let context = resolve_context_for_window(app, target)?;
    crate::pipeline::pill::queue_pill_context(&context.label);
    Some(context)
}

pub(super) fn ensure_terminal_punctuation(
    text: &str,
    profile: &str,
    cleanup_intensity: &str,
) -> String {
    if profile == "very_casual" || cleanup_intensity == "none" {
        return text.to_owned();
    }
    let trimmed = text.trim_end();
    match trimmed.chars().next_back() {
        Some(last) if last.is_alphanumeric() => {
            let punct = if is_cjk(last) { "。" } else { "." };
            format!("{trimmed}{punct}{}", &text[trimmed.len()..])
        }
        _ => text.to_owned(),
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{3040}'..='\u{309f}'
        | '\u{30a0}'..='\u{30ff}'
    )
}
