<script lang="ts">
  import { formatIpcError } from '../../errors';
  import { onMount } from 'svelte';
  import { invoke } from '../../tauri';
  import { listenForSyncCompletion } from '../../syncStore.svelte';
  import { isAndroid } from '../../platform';
  import Toggle from '../Toggle.svelte';
  import Disclosure from '../Disclosure.svelte';
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
  import { MOTION_PX, modalBackdrop, modalCard, motionPx } from '../../motion';

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
  </div>
  <Toggle checked={voiceCommands} onchange={handleVoiceCommands} disabled={voiceCommandsSaving} label="Voice commands" bind:error={voiceCommandsError} />
  <div class="voice-commands-disclosure">
    <Disclosure label="What you can say" id="voice-commands-details" bind:open={voiceCommandsDetailsOpen}>
      <dl class="voice-commands-details">
        <dt>Punctuation</dt><dd>“comma”, “semicolon”, “full stop”, “question mark”, “exclamation mark”. For a period, colon, dash or hyphen, start with add, insert or put: “put a colon”.</dd>
        <dt>Line breaks</dt><dd>“new line”, “next line”, “new paragraph”, “next paragraph”.</dd>
        <dt>Mentions</dt><dd>“at sign maria” becomes @maria. “At the rate” works too.</dd>
        <dt>Take back</dt><dd>“scratch that” or “strike that” removes the latest fragment. “Remove that”, “delete that”, “undo that” and “cancel that” do the same only when set off by punctuation.</dd>
      </dl>
      <ul class="voice-commands-limits">
        <li>English only, with Spoken Language set to English, not Auto.</li>
        <li>Cleanup must be on and not set to Off in Style.</li>
        <li>Commands act only on what you are dictating now and never edit text already in another app.</li>
        <li>Snippet and clipboard text, quoted text, and code stay literal.</li>
        <li>Talking about a command without quoting it may still run it.</li>
      </ul>
    </Disclosure>
  </div>
</div>
{#if voiceCommandsMessage}
  <p class="settings-error" role="alert">{voiceCommandsMessage}</p>
{/if}

<div class="setting-row" data-setting-target="text-learning">
  <div>
    <div class="label">Auto-learn corrections</div>
    <div class="desc">Adds corrections you confirm to your dictionary automatically. Entirely on-device; no text is sent to any API.</div>
    <div class="desc learn-status" data-setting-target="text-learning-activity">
      {autoLearnSummary.promotions} promoted · {autoLearnSummary.low_confidence_rejections} blocked · {autoLearnSummary.anchor_misses} missed{#if latestAutoLearn} · Latest: {latestAutoLearn.event_type} ({latestAutoLearn.reason_code}){/if}
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
  .setting-row.voice-commands-row { row-gap: 0; }
  .voice-commands-disclosure { grid-column: 1 / -1; margin-top: 6px; }
  .voice-commands-details {
    margin: 6px 0 0; padding: 12px 14px; border-radius: var(--r-md);
    background: color-mix(in srgb, var(--paper) 55%, var(--bg-elev));
    font-size: 12px; line-height: 1.5; color: var(--ink-soft);
    display: grid; grid-template-columns: max-content 1fr; gap: 6px 14px;
  }
  .voice-commands-details dt { color: var(--ink); font-weight: 500; }
  .voice-commands-details dd { margin: 0; }
  .voice-commands-limits {
    margin: 8px 0 0; padding: 0 0 0 28px; font-size: 12px; line-height: 1.5; color: var(--ink-mute);
  }
  @container settings-panel (max-width: 440px) {
    .setting-row.voice-commands-row { grid-template-columns: minmax(0, 1fr) auto; align-items: center; column-gap: 16px; }
    .setting-row.voice-commands-row > :global(.toggle) { justify-self: end; }
  }
  @container settings-panel (max-width: 520px) {
    .voice-commands-details { grid-template-columns: 1fr; gap: 0; }
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
