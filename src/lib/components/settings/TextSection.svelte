<script lang="ts">
  import { formatIpcError } from '../../errors';
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { invoke } from '../../tauri';
  import { listenForSyncCompletion } from '../../syncStore.svelte';
  import { isAndroid } from '../../platform';
  import Toggle from '../Toggle.svelte';
  import { appStore } from '../../stores';
  import {
    loadSettingsSnapshot,
    saveSetting,
    saveSettingIfUnchanged,
    type SettingsSnapshot,
    type SettingsValueMap,
    type WritableSettingKey,
  } from '../../settings';
  import { modalFocusTrap } from '../../modalFocus';
  import { MOTION_MS, MOTION_PX, modalBackdrop, modalCard, motionMs, motionPx } from '../../motion';

  let settingsBaseline: SettingsSnapshot | null = null;

  async function saveTextSetting<K extends WritableSettingKey>(key: K, value: SettingsValueMap[K]) {
    const baseline = settingsBaseline;
    if (!baseline) throw new Error('Settings are still loading.');
    const keys: WritableSettingKey[] = key === 'contextual_formatting_enabled'
      ? ['contextual_formatting_enabled', 'contextual_caps_enabled', 'auto_spacing_enabled'] : [key];
    const expected = Object.fromEntries(keys.map(name => [name, baseline[name as keyof SettingsSnapshot] ?? null]));
    try {
      await saveSettingIfUnchanged(key, value, expected);
      for (const name of keys) Object.assign(baseline, { [name]: value });
    } catch (error) {
      if (String(error).includes('Settings changed while you were editing')) {
        window.dispatchEvent(new CustomEvent('verenu:agent-settings-changed', { detail: keys }));
      }
      throw error;
    }
  }

  let contextualFormatting = $state(true);
  let capsLockUppercase = $state(false);
  let voiceCommands = $state(false);
  let autoLearn = $state(false);
  let autoLearnSummary = $state({
    promotions: 0,
    low_confidence_rejections: 0,
    anchor_misses: 0,
  });
  let latestAutoLearn = $state<{ event_type: string; reason_code: string } | null>(null);

  let cleanupError = $state(false);
  let contextualFormattingError = $state(false);
  let capsLockUppercaseError = $state(false);
  let voiceCommandsError = $state(false);
  let autoLearnError = $state(false);
  let voiceCommandsSaving = $state(false);
  let voiceCommandsMessage = $state('');
  let voiceCommandsDetailsOpen = $state(false);

  async function loadSettings() {
    const cleanupVersion = appStore.cleanupChangeVersion;
    try {
      const settings = await loadSettingsSnapshot();
      settingsBaseline = settings;
      if (cleanupVersion === appStore.cleanupChangeVersion) appStore.cleanupEnabled = settings.cleanup_enabled ?? true;
      contextualFormatting = settings.contextual_formatting_enabled ?? true;
      capsLockUppercase = settings.caps_lock_uppercase_enabled ?? false;
      voiceCommands = settings.voice_commands_enabled ?? false;
      autoLearn = settings.auto_learn_enabled ?? false;
    } catch (err) {
      console.error('TextSection load failed:', err);
    }
    try {
      const [summary, recent] = await Promise.all([
        invoke<typeof autoLearnSummary>('get_auto_learn_status_summary'),
        invoke<Array<{ event_type: string; reason_code: string }>>('get_recent_auto_learn_activity', { limit: 1 }),
      ]);
      autoLearnSummary = summary ?? autoLearnSummary;
      latestAutoLearn = recent?.[0] ?? null;
    } catch (err) {
      console.error('TextSection auto-learn status failed:', err);
    }
  }

  async function applyCleanup(value: boolean) {
    const version = ++appStore.cleanupChangeVersion;
    const previous = appStore.cleanupEnabled;
    appStore.cleanupEnabled = value;
    try {
      await saveTextSetting('cleanup_enabled', value);
      const refreshVersion = ++appStore.cleanupChangeVersion;
      const persisted = await invoke<boolean | null>('get_setting', { key: 'cleanup_enabled' });
      if (refreshVersion === appStore.cleanupChangeVersion) appStore.cleanupEnabled = persisted ?? true;
    } catch (err) {
      if (version === appStore.cleanupChangeVersion) appStore.cleanupEnabled = previous;
      cleanupError = true;
      console.error('save cleanup_enabled failed:', err);
    }
  }

  let confirmCleanupOff = $state(false);
  let cleanupCancelButton: HTMLButtonElement | null = $state(null);

  function handleCleanup(value: boolean) {
    if (!value) {
      confirmCleanupOff = true;
      return;
    }
    applyCleanup(true);
  }

  async function confirmCleanupOffAction() {
    confirmCleanupOff = false;
    await applyCleanup(false);
  }

  function handleModalKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && confirmCleanupOff) confirmCleanupOff = false;
  }

  async function handleContextualFormatting(value: boolean) {
    contextualFormatting = value;
    try {
      await saveTextSetting('contextual_formatting_enabled', value);
    } catch (err) {
      contextualFormatting = !value;
      contextualFormattingError = true;
      console.error('save contextual_formatting_enabled failed:', err);
    }
  }

  async function handleCapsLockUppercase(value: boolean) {
    capsLockUppercase = value;
    try {
      await saveTextSetting('caps_lock_uppercase_enabled', value);
    } catch (err) {
      capsLockUppercase = !value;
      capsLockUppercaseError = true;
      console.error('save caps_lock_uppercase_enabled failed:', err);
    }
  }

  async function handleVoiceCommands(value: boolean) {
    voiceCommandsSaving = true;
    voiceCommandsMessage = '';
    try {
      await saveTextSetting('voice_commands_enabled', value);
      voiceCommands = value;
    } catch (err) {
      voiceCommandsError = true;
      voiceCommandsMessage = formatIpcError(err, 'Could not save voice commands');
    } finally {
      voiceCommandsSaving = false;
    }
  }

  async function handleAutoLearn(value: boolean) {
    autoLearn = value;
    try {
      await saveSetting('auto_learn_enabled', value);
    } catch (err) {
      autoLearn = !value;
      autoLearnError = true;
      console.error('save auto_learn_enabled failed:', err);
    }
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;
    let active = true;
    listenForSyncCompletion(() => {
      const cleanupVersion = appStore.cleanupChangeVersion;
      void loadSettingsSnapshot().then(({ contextual_formatting_enabled: formatting,
        cleanup_enabled: cleanup, voice_commands_enabled: commands }) => {
        if (!active) return;
        contextualFormatting = formatting ?? true;
        if (cleanupVersion === appStore.cleanupChangeVersion) appStore.cleanupEnabled = cleanup ?? true;
        voiceCommands = commands ?? false;
      }).catch(() => {});
    }).then(cleanup => { if (active) unlisten = cleanup; else cleanup(); }).catch(() => {});
    return () => {
      active = false;
      unlisten?.();
    };
  });

  loadSettings();
</script>
<svelte:window onkeydown={handleModalKeydown} />

<h2 class="settings-h">Text</h2>
<p class="settings-lede">How Verenu turns what you say into the text it types.</p>

<h3 class="settings-subhead first">Cleanup and formatting</h3>
<div class="setting-row" data-setting-target="text-cleanup">
  <div><div class="label">Cleanup</div><div class="desc">Cleans up dictation after transcription. Choose the level in Style and a cleanup model in Models.</div></div>
  <Toggle checked={appStore.cleanupEnabled} onchange={handleCleanup} label="Cleanup" bind:error={cleanupError} />
</div>

<div class="setting-row" data-setting-target="text-spacing">
  <div><div class="label">Smart spacing &amp; capitalization</div><div class="desc">Adjusts capitalization and spacing around inserted text when the cursor context is clear.</div></div>
  <Toggle checked={contextualFormatting} onchange={handleContextualFormatting} label="Smart spacing and capitalization" bind:error={contextualFormattingError} />
</div>
{#if !isAndroid}
  <div class="setting-row" data-setting-target="text-caps-lock">
    <div><div class="label">Automatic caps lock detection</div><div class="desc">When Caps Lock is on, output your dictation in ALL CAPS</div></div>
    <Toggle checked={capsLockUppercase} onchange={handleCapsLockUppercase} label="Automatic caps lock detection" bind:error={capsLockUppercaseError} />
  </div>
{/if}

<h3 class="settings-subhead">Voice and learning</h3>
<div class="setting-row voice-commands-row" data-setting-target="text-voice-commands">
  <div>
    <div class="label">Voice commands</div>
    <div class="desc">Say punctuation, line breaks, or “scratch that” while dictating and Verenu applies them instead of typing the words. Off by default.</div>
    <button
      type="button"
      class="voice-commands-more ui-focus-ring"
      aria-expanded={voiceCommandsDetailsOpen}
      aria-controls={voiceCommandsDetailsOpen ? 'voice-commands-details' : undefined}
      onclick={() => (voiceCommandsDetailsOpen = !voiceCommandsDetailsOpen)}
    >
      <span>What you can say</span>
      <svg class="ui-chevron" class:open={voiceCommandsDetailsOpen} width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>
    </button>
  </div>
  <Toggle checked={voiceCommands} onchange={handleVoiceCommands} disabled={voiceCommandsSaving} label="Voice commands" bind:error={voiceCommandsError} />
</div>
{#if voiceCommandsDetailsOpen}
  <div id="voice-commands-details" class="voice-commands-details" transition:slide={{ duration: motionMs(MOTION_MS.base), easing: cubicOut }}>
    <dl>
      <dt>Punctuation</dt><dd>“comma”, “semicolon”, “full stop”, “question mark”, “exclamation mark” or “exclamation point”. For period, colon, dash or hyphen, say add, insert or put first: “put a colon”. A period at the very end also works.</dd>
      <dt>Line breaks</dt><dd>“new line”, “next line”, “new paragraph” or “next paragraph”.</dd>
      <dt>Mentions</dt><dd>“at sign” or “at the rate” followed by one word, such as “at sign maria”, becomes @maria.</dd>
      <dt>Take back</dt><dd>“scratch that” or “strike that” removes the latest fragment of this dictation. “Remove that”, “delete that”, “undo that” or “cancel that” does the same only when set off by punctuation.</dd>
    </dl>
    <p class="voice-commands-limits">Limits: only English works, and Spoken Language must be set to English, not Auto. Cleanup must be on and not set to Off in Style. Commands only act on what you are dictating now and never edit text already in another app. Snippet and clipboard text, quoted text, and code stay literal. If you talk about a command without quoting it, it may still run.</p>
  </div>
{/if}
{#if voiceCommandsMessage}
  <p class="settings-error" role="alert">{voiceCommandsMessage}</p>
{/if}

<div class="setting-row" data-setting-target="text-learning">
  <div>
    <div class="label">Auto-learn corrections</div>
    <div class="desc">Adds corrections you confirm to your dictionary automatically. Entirely on-device; no text is sent to any API.</div>
    <div class="desc learn-status" data-setting-target="text-learning-activity">
      Promoted {autoLearnSummary.promotions} · Low-confidence blocked {autoLearnSummary.low_confidence_rejections} · Anchor misses {autoLearnSummary.anchor_misses}{#if latestAutoLearn} · Latest: {latestAutoLearn.event_type} ({latestAutoLearn.reason_code}){/if}
    </div>
  </div>
  <Toggle checked={autoLearn} onchange={handleAutoLearn} label="Auto-learn corrections" bind:error={autoLearnError} />
</div>

{#if confirmCleanupOff}
  <button class="modal-backdrop" aria-label="Close dialog" onclick={() => (confirmCleanupOff = false)} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
  <div
    class="modal-card"
    use:modalFocusTrap={{
      active: confirmCleanupOff,
      initialFocus: () => cleanupCancelButton,
    }}
    role="dialog"
    aria-modal="true"
    aria-labelledby="cleanup-off-confirm-title"
    tabindex="-1"
    in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
    out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
  >
    <div class="modal-header">
      <h2 id="cleanup-off-confirm-title" class="modal-title">Turn Cleanup off?</h2>
    </div>
    <div class="modal-body">
      <p class="confirm-copy">
        Dictation will keep the raw transcript as-is — faster, but tone, formatting, and the
        Style and App Mappings pages stop having any effect. You can turn this back on anytime.
      </p>
    </div>
    <div class="modal-footer">
      <div class="footer-actions">
        <button bind:this={cleanupCancelButton} class="btn-ghost" onclick={() => (confirmCleanupOff = false)}>Cancel</button>
        <button class="btn-primary" onclick={confirmCleanupOffAction}>Turn off</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .learn-status { font-variant-numeric: tabular-nums; }
  .voice-commands-more {
    display: inline-flex; align-items: center; gap: 5px; margin-top: 6px; padding: 2px 4px; margin-left: -4px;
    background: transparent; border: 0; border-radius: var(--r-sm); color: var(--accent-ink);
    font-size: 12px; font-weight: 500; cursor: pointer;
    transition: background var(--ui-duration-fast) var(--ui-ease-out);
  }
  .voice-commands-more:hover { background: var(--control-hover); }
  .voice-commands-details {
    margin: -4px 0 12px; padding: 12px 14px; border: 1px solid var(--line); border-radius: var(--r-md);
    background: color-mix(in srgb, var(--paper) 55%, var(--bg-elev)); font-size: 12px; line-height: 1.55; color: var(--ink-soft);
  }
  .voice-commands-details dl { margin: 0; display: grid; grid-template-columns: max-content 1fr; gap: 6px 14px; }
  .voice-commands-details dt { color: var(--ink); font-weight: 500; }
  .voice-commands-details dd { margin: 0; }
  .voice-commands-limits { margin: 10px 0 0; color: var(--ink-mute); }
  @media (max-width: 520px) {
    .voice-commands-details dl { grid-template-columns: 1fr; gap: 0; }
    .voice-commands-details dd { margin-bottom: 8px; }
  }
  .settings-error {
    margin: -2px 0 12px;
    color: var(--danger);
    font-size: 12px;
    line-height: 1.45;
  }
  .modal-backdrop {
    position: fixed;
    inset: 0;
    border: 0;
    padding: 0;
    appearance: none;
    background: var(--overlay);
    z-index: 50;
    outline: none;
  }
  .modal-card {
    position: fixed;
    top: 50%;
    left: 50%;
    translate: -50% -50%;
    z-index: 51;
    isolation: isolate;
    background: var(--bg-elev);
    border: 1px solid var(--line);
    border-radius: var(--r-lg);
    width: min(420px, calc(100vw - 40px));
    box-shadow: var(--shadow-elev);
    overflow: hidden;
  }
  .modal-header { padding: 20px 20px 0; }
  .modal-title {
    font-family: var(--sans);
    font-size: 17px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--ink);
    margin: 0;
  }
  .modal-body { padding: 10px 20px 18px; }
  .confirm-copy {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--ink-soft);
  }
  .modal-footer { padding: 0 20px 20px; }
  .footer-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
