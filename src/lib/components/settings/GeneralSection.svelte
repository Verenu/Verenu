<script lang="ts">
  import { formatIpcError } from '../../errors';
  import { onDestroy, onMount } from 'svelte';
  import { listenForSyncCompletion } from '../../syncStore.svelte';
  import { emit, invoke, listen } from '../../tauri';
  import { fly, fade } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import { isAndroid, isLinux, isMac, formatKeyLabel, defaultHotkey } from '../../platform';
  import Toggle from '../Toggle.svelte';
  import { appStore } from '../../stores';
  import {
    saveSetting,
    ANDROID_PILL_POSITION_OPTIONS,
    ANDROID_PILL_SCREEN_POSITION_OPTIONS,
    DEFAULT_ANDROID_PILL_POSITION,
    DEFAULT_ANDROID_PILL_DOCK_POSITION,
    type AndroidPillPosition,
    type AndroidPillScreenPosition,
    type AppearanceMode,
  } from '../../settings';
  import { modalFocusTrap } from '../../modalFocus';
  import { MOTION_MS, MOTION_PX, modalBackdrop, modalCard, motionMs, motionPx, animateWidth } from '../../motion';
  import {
    getTranscriptionLanguageLabel,
    transcriptionLanguages,
    type TranscriptionLanguageCode,
  } from '../../transcriptionLanguages';
  import { getLanguageSupport } from '../../transcriptionLanguageSupport';
  import { transcriptionModelStore } from '../../transcriptionModelStore.svelte';
  import { modelDisplayLabel, splitModelId } from './models';
  import AccentColorPicker from './AccentColorPicker.svelte';
  import AppearanceSettings from './AppearanceSettings.svelte';
  import { persistAccentColor, withAppearanceLock } from '../../appearanceActions';
  import { guardThemeEditor, themeEditor } from '../../themeEditor.svelte';
  import { desktopShortcut } from '../../shortcutStatus.svelte';
  import DesktopShortcutStatus from './DesktopShortcutStatus.svelte';
  import { HotkeyCapture } from '../../hotkeyCapture';
  import { loadHotkey } from '../../hotkey.svelte';

  let selectedLanguage = $state<TranscriptionLanguageCode>('en');
  let languageDropdownOpen = $state(false);
  let languageTouched = false;
  // Guards the language auto-correct effect below until the real persisted
  // selection has loaded — without this, the effect could see the placeholder
  // initial state (selectedLanguage='en', transcriptionModelStore at its
  // default) as "unsupported," call saveLanguage('en'), and that call's
  // languageTouched=true side effect would then make loadSettings() skip
  // applying the user's actual saved language once it resolves.
  //
  // Must be $state, not a plain let: the auto-correct effect's first line is
  // `if (!initialLanguageLoaded) return`, which on the mount run short-circuits
  // before reading any reactive value — so unless THIS flag is reactive, the
  // effect registers no dependencies and never re-runs when loadSettings later
  // flips it true.
  let initialLanguageLoaded = $state(false);
  let microphones = $state<string[]>([]);
  let selectedMic = $state('');
  let micDropdownOpen = $state(false);
  let pillPosition = $state<AndroidPillPosition>(DEFAULT_ANDROID_PILL_POSITION);
  let pillDockPosition = $state<AndroidPillScreenPosition>(DEFAULT_ANDROID_PILL_DOCK_POSITION);
  let coverKeyboardMic = $state(true);
  let coverKeyboardMicError = $state(false);
  let hidePillOffline = $state(true);
  let hidePillOfflineError = $state(false);

  async function handleHidePillOffline(value: boolean) {
    hidePillOffline = value;
    try {
      await saveSetting('android_pill_hide_offline', value);
    } catch (err) {
      hidePillOffline = !value;
      hidePillOfflineError = true;
      console.error('save android_pill_hide_offline failed:', err);
    }
  }

  async function handleCoverKeyboardMic(value: boolean) {
    coverKeyboardMic = value;
    try {
      await saveSetting('android_pill_cover_keyboard_mic', value);
    } catch (err) {
      coverKeyboardMic = !value;
      coverKeyboardMicError = true;
      console.error('save android_pill_cover_keyboard_mic failed:', err);
    }
  }
  let pillDropdownOpen = $state(false);
  let dockDropdownOpen = $state(false);
  const defaultPillPositionLabel =
    ANDROID_PILL_POSITION_OPTIONS.find((o) => o.id === DEFAULT_ANDROID_PILL_POSITION)?.label ??
    ANDROID_PILL_POSITION_OPTIONS[0]?.label ??
    '';
  const pillPositionLabel = $derived(
    ANDROID_PILL_POSITION_OPTIONS.find((o) => o.id === pillPosition)?.label ?? defaultPillPositionLabel,
  );
  const pillDockPositionLabel = $derived(
    ANDROID_PILL_SCREEN_POSITION_OPTIONS.find((o) => o.id === pillDockPosition)?.label ??
      ANDROID_PILL_SCREEN_POSITION_OPTIONS[0]?.label ??
      '',
  );
  const microphoneCopy = {
    inputDeviceLabel: 'Input device',
    inputDeviceDescription: 'Choose which microphone Verenu should record from',
    defaultDevice: 'Default Device',
    noDevicesFound: 'No devices found',
  };
  let autostart = $state(false);
  let contextualFormatting = $state(true);
  let capsLockUppercase = $state(false);
  let legacyFeaturesError = $state('');
  let hotkey = $state(defaultHotkey);
  let recordingHotkey = $state(false);
  let capturedKeys = $state<string[]>([]);
  let hotkeyError = $state('');
  const capture = new HotkeyCapture();
  let feedbackTimer: ReturnType<typeof setTimeout> | undefined;
  let destroyed = false;
  let hotkeyState = $state<'idle' | 'armed' | 'first' | 'saving' | 'success' | 'error'>('idle');
  const HOTKEY_SUCCESS_MS = 700;
  const HOTKEY_ERROR_MS   = 900;
  const LANGUAGE_MENU_ID = 'spoken-language-menu';
  const MIC_MENU_ID = 'microphone-menu';
  const PILL_MENU_ID = 'pill-position-menu';
  const DOCK_MENU_ID = 'pill-dock-position-menu';
  let keybindEl: HTMLElement | null = $state(null);
  let capturedWidth = 0;

  const readableMac: Record<string, string> = {
    MetaLeft: 'Cmd',
    MetaRight: 'Cmd',
    ControlLeft: 'Ctrl',
    ControlRight: 'Ctrl',
    AltLeft: 'Option',
    AltRight: 'Option',
    ShiftLeft: 'Shift',
    ShiftRight: 'Shift',
    Fn: 'Fn',
  };

  function formatHotkeyBadgeLabel(code: string): string {
    if (isMac && readableMac[code]) {
      return readableMac[code];
    }
    return formatKeyLabel(code);
  }

  // A macOS hotkey can be a single key (e.g. F5) — later slots are empty.
  function formatHotkeyDisplay(hk: string[]): string {
    return hk.filter(Boolean).map(formatHotkeyBadgeLabel).join(' + ');
  }

  let buttonText = $derived(
    recordingHotkey
      ? capturedKeys.length ? formatHotkeyDisplay(capturedKeys) : 'Hold your shortcut...'
      : hotkeyState === 'saving' ? 'Saving...'
      : isLinux && !isAndroid && desktopShortcut('dictation') ? desktopShortcut('dictation')?.active?.split('+').join(' + ') ?? 'Unavailable' : formatHotkeyDisplay(hotkey)
  );

  $effect.pre(() => {
    void buttonText;
    if (keybindEl) capturedWidth = keybindEl.getBoundingClientRect().width;
  });

  $effect(() => {
    void buttonText;
    if (!keybindEl || capturedWidth === 0) return;
    const el = keybindEl;
    const prevW = capturedWidth;
    el.style.transition = 'none';
    el.style.width = 'max-content';
    const newW = Math.ceil(el.getBoundingClientRect().width);
    el.style.width = `${prevW}px`;
    void el.offsetWidth;
    el.style.transition = '';
    el.style.width = `${newW}px`;
  });

  /** Take saved placements as given; an unknown or missing value falls back to the default. */
  function applyPillPlacement(position: AndroidPillPosition | null, dock: AndroidPillScreenPosition | null) {
    pillPosition = ANDROID_PILL_POSITION_OPTIONS.some((o) => o.id === position)
      ? (position as AndroidPillPosition)
      : DEFAULT_ANDROID_PILL_POSITION;
    pillDockPosition = ANDROID_PILL_SCREEN_POSITION_OPTIONS.some((o) => o.id === dock)
      ? (dock as AndroidPillScreenPosition)
      : DEFAULT_ANDROID_PILL_DOCK_POSITION;
  }

  /**
   * The pill can change its own placement (dragging it snaps and saves), and
   * this page only read the settings once on mount, so re-read them whenever
   * the pill reports a move or the app comes back to the foreground.
   */
  async function refreshPillPlacement() {
    if (!isAndroid) return;
    try {
      const [position, dock] = await Promise.all([
        invoke<AndroidPillPosition | null>('get_setting', { key: 'android_pill_position' }),
        invoke<AndroidPillScreenPosition | null>('get_setting', { key: 'android_pill_dock_position' }),
      ]);
      if (!destroyed) applyPillPlacement(position, dock);
    } catch (err) {
      console.error('refreshPillPlacement failed:', err);
    }
  }

  async function loadSettings() {
    const results = await Promise.allSettled([
      invoke<boolean | null>('get_setting', { key: 'autostart_enabled' }),
      invoke<string[] | null>('get_setting', { key: 'hotkey' }),
      invoke<AppearanceMode | null>('get_setting', { key: 'appearance_mode' }),
      invoke<TranscriptionLanguageCode | null>('get_setting', { key: 'transcription_language' }),
      invoke<boolean | null>('get_setting', { key: 'cleanup_enabled' }),
      invoke<boolean | null>('get_setting', { key: 'contextual_formatting_enabled' }),
      invoke<boolean | null>('get_setting', { key: 'caps_lock_uppercase_enabled' }),
      invoke<string[]>('get_microphones'),
      invoke<string | null>('get_setting', { key: 'microphone_device' }),
      invoke<boolean | null>('get_setting', { key: 'legacy_features_enabled' }),
      invoke<AndroidPillPosition | null>('get_setting', { key: 'android_pill_position' }),
      invoke<boolean | null>('get_setting', { key: 'android_pill_cover_keyboard_mic' }),
      invoke<boolean | null>('get_setting', { key: 'android_pill_hide_offline' }),
      invoke<AndroidPillScreenPosition | null>('get_setting', { key: 'android_pill_dock_position' }),
    ]);

    const val = <T>(i: number, fallback: T): T =>
      results[i].status === 'fulfilled' ? (results[i] as PromiseFulfilledResult<T>).value ?? fallback : fallback;

    autostart = val<boolean | null>(0, null) ?? false;
    appStore.cleanupEnabled = val<boolean | null>(4, null) ?? true;
    contextualFormatting = val<boolean | null>(5, null) ?? true;
    capsLockUppercase = val<boolean | null>(6, null) ?? false;

    const hk = val<string[] | null>(1, null);
    if (hk && hk.length > 0 && hk.some(Boolean)) hotkey = hk.filter(Boolean);

    const appearance = val<AppearanceMode | null>(2, null);
    // A theme being edited is previewing through these store fields; the saved
    // values must not overwrite the preview.
    if (!themeEditor.open && (appearance === 'system' || appearance === 'light' || appearance === 'dark' || appearance === 'omarchy' || appearance === 'custom')) {
      appStore.appearanceMode = appearance;
    }

    const language = val<TranscriptionLanguageCode | null>(3, null);
    if (!languageTouched && language && transcriptionLanguages.some((option) => option.code === language)) {
      selectedLanguage = language;
    }
    initialLanguageLoaded = true;

    microphones = val<string[]>(7, []);
    selectedMic = val<string | null>(8, null) ?? '';
    appStore.legacyFeaturesEnabled = val<boolean | null>(9, null) ?? false;
    applyPillPlacement(val<AndroidPillPosition | null>(10, null), val<AndroidPillScreenPosition | null>(13, null));

    coverKeyboardMic = val<boolean | null>(11, null) ?? true;
    hidePillOffline = val<boolean | null>(12, null) ?? true;

    results.forEach((r, i) => {
      if (r.status === 'rejected') console.error(`GeneralSection: invoke[${i}] failed:`, r.reason);
    });
  }

  function handleWindowClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (micDropdownOpen && !target.closest('.mic-dropdown')) micDropdownOpen = false;
    if (pillDropdownOpen && !target.closest('.pill-dropdown')) pillDropdownOpen = false;
    if (dockDropdownOpen && !target.closest('.dock-dropdown')) dockDropdownOpen = false;
    if (languageDropdownOpen && !target.closest('.language-dropdown')) languageDropdownOpen = false;
  }

  async function savePillPosition(position: AndroidPillPosition) {
    pillPosition = position;
    pillDropdownOpen = false;
    try {
      await saveSetting('android_pill_position', position);
    } catch (err) {
      console.error('savePillPosition failed:', err);
      void refreshPillPlacement();
    }
  }

  async function savePillDockPosition(position: AndroidPillScreenPosition) {
    pillDockPosition = position;
    dockDropdownOpen = false;
    try {
      await saveSetting('android_pill_dock_position', position);
    } catch (err) {
      console.error('savePillDockPosition failed:', err);
      void refreshPillPlacement();
    }
  }

  async function saveMic(name: string) {
    selectedMic = name;
    micDropdownOpen = false;
    try {
      await saveSetting('microphone_device', name || null);
    } catch (err) {
      console.error('saveMic failed:', err);
    }
  }

  async function saveLanguage(code: TranscriptionLanguageCode) {
    languageTouched = true;
    selectedLanguage = code;
    languageDropdownOpen = false;
    try {
      await saveSetting('transcription_language', code);
    } catch (err) {
      console.error('save transcription_language failed:', err);
    }
  }

  // Which of the 57 Spoken Language options the active transcription model
  // actually supports — 'all' for every current cloud model (Verenu's full
  // list already matches Whisper's official language support exactly, and
  // Gemini publishes no narrower restriction), a real subset for most local
  // models (e.g. Moonshine is English-only).
  const languageScope = $derived.by(() => {
    const parsed = splitModelId(transcriptionModelStore.defaultModel);
    return parsed ? getLanguageSupport(parsed.provider, parsed.model) : 'all';
  });
  const visibleLanguages = $derived(
    languageScope === 'all'
      ? transcriptionLanguages
      : transcriptionLanguages.filter((language) => languageScope.includes(language.code)),
  );
  const languageScopeNote = $derived.by(() => {
    if (languageScope === 'all') return '';
    const parsed = splitModelId(transcriptionModelStore.defaultModel);
    const modelName = parsed ? modelDisplayLabel(parsed.provider, parsed.model) : 'this model';
    const count = visibleLanguages.length;
    return ` · ${count} ${count === 1 ? 'language' : 'languages'} for ${modelName}`;
  });

  // If switching models drops the currently selected language out of the
  // now-narrower list, snap back to a supported one rather than leaving a
  // silently unsupported selection in place. Prefer English (most models
  // include it), but fall back to the model's first supported language for
  // the English-excluding ones (e.g. GigaAM is Russian-only). Gated on
  // initialLanguageLoaded so this never fires during the initial hydration
  // race (see the flag's declaration comment).
  $effect(() => {
    if (!initialLanguageLoaded) return;
    if (languageScope === 'all') return;
    if (visibleLanguages.some((language) => language.code === selectedLanguage)) return;
    const fallback = visibleLanguages.some((language) => language.code === 'en')
      ? 'en'
      : visibleLanguages[0]?.code;
    if (fallback) {
      saveLanguage(fallback).catch((err) => console.error('auto-correct transcription_language failed:', err));
    }
  });

  let autostartError = $state(false);

  async function handleAutostart(value: boolean) {
    autostart = value;
    try {
      await invoke('set_autostart', { enabled: value });
    } catch (err) {
      autostart = !value;
      autostartError = true;
      console.error('set_autostart failed:', err);
      void emit('verenu:error', formatIpcError(err, 'Could not change whether Verenu starts on boot'));
    }
  }

  let legacyFeaturesToggleError = $state(false);

  async function applyLegacyFeatures(value: boolean) {
    legacyFeaturesError = '';
    appStore.legacyFeaturesEnabled = value;
    try {
      await saveSetting('legacy_features_enabled', value);
    } catch (err) {
      appStore.legacyFeaturesEnabled = !value;
      legacyFeaturesError = 'Could not save Legacy pages. Your change was reverted.';
      legacyFeaturesToggleError = true;
      console.error('save legacy_features_enabled failed:', err);
    }
  }

  // Turning Legacy on surfaces unmaintained pages (App Mappings, Dictionary,
  // Snippets), so it gets a heads-up first. Turning it back off needs no
  // confirmation — that's just restoring the default.
  let confirmLegacyOn = $state(false);
  let legacyCancelButton: HTMLButtonElement | null = $state(null);

  function handleLegacyFeatures(value: boolean) {
    if (value) {
      confirmLegacyOn = true;
      return;
    }
    applyLegacyFeatures(false);
  }

  async function confirmLegacyOnAction() {
    confirmLegacyOn = false;
    await applyLegacyFeatures(true);
  }

  function handleLegacyModalKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && confirmLegacyOn) confirmLegacyOn = false;
  }

  let cleanupError = $state(false);

  async function applyCleanup(value: boolean) {
    appStore.cleanupEnabled = value;
    try {
      await saveSetting('cleanup_enabled', value);
    } catch (err) {
      appStore.cleanupEnabled = !value;
      cleanupError = true;
      console.error('save cleanup_enabled failed:', err);
    }
  }

  // Turning Cleanup off is a bigger behavioral change than most toggles here
  // (it silently makes the Style and App Mappings pages inert), so it gets a
  // confirmation instead of taking effect immediately. Turning it back on
  // needs no confirmation — that's just restoring the default.
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

  function handleCleanupModalKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && confirmCleanupOff) confirmCleanupOff = false;
  }

  let contextualFormattingError = $state(false);

  async function handleContextualFormatting(value: boolean) {
    contextualFormatting = value;
    try {
      await saveSetting('contextual_formatting_enabled', value);
    } catch (err) {
      contextualFormatting = !value;
      contextualFormattingError = true;
      console.error('save contextual_formatting_enabled failed:', err);
    }
  }

  let capsLockUppercaseError = $state(false);

  async function handleCapsLockUppercase(value: boolean) {
    capsLockUppercase = value;
    try {
      await saveSetting('caps_lock_uppercase_enabled', value);
    } catch (err) {
      capsLockUppercase = !value;
      capsLockUppercaseError = true;
      console.error('save caps_lock_uppercase_enabled failed:', err);
    }
  }

  async function handleAccentColor(color: string | null) {
    guardThemeEditor(() => void withAppearanceLock(() => persistAccentColor(color)));
  }

  async function startRecordingHotkey(e: MouseEvent | KeyboardEvent) {
    e.stopPropagation();
    if (recordingHotkey || hotkeyState === 'saving') return;
    clearTimeout(feedbackTimer);
    hotkeyError = '';
    capture.reset();
    hotkeyState = 'saving';
    try {
      await invoke('set_hotkey_capture', { active: true });
      if (destroyed) {
        await invoke('set_hotkey_capture', { active: false });
        return;
      }
    } catch (error) {
      hotkeyState = 'error';
      hotkeyError = formatIpcError(error, 'Could not start shortcut capture');
      return;
    }
    recordingHotkey = true;
    hotkeyState = 'armed';
    capturedKeys = [];
    window.addEventListener('keydown', handleHotkeyKeydown, { capture: true });
    window.addEventListener('keyup', handleHotkeyKeyup, { capture: true });
    window.addEventListener('mousedown', cancelRecordingHotkey, { capture: true });
    window.addEventListener('blur', handleCaptureBlur);
  }

  function removeHotkeyCaptureListeners() {
    window.removeEventListener('keydown', handleHotkeyKeydown, { capture: true });
    window.removeEventListener('keyup', handleHotkeyKeyup, { capture: true });
    window.removeEventListener('mousedown', cancelRecordingHotkey, { capture: true });
    window.removeEventListener('blur', handleCaptureBlur);
  }

  async function cancelRecordingHotkey(e?: MouseEvent | KeyboardEvent) {
    if (e && (e.target as HTMLElement).closest('.keybind-btn')) return;
    if (recordingHotkey) {
      removeHotkeyCaptureListeners();
      recordingHotkey = false;
      hotkeyState = 'saving';
      capturedKeys = [];
      capture.reset();
      try {
        await invoke('set_hotkey_capture', { active: false });
        hotkeyState = 'idle';
      } catch (error) {
        hotkeyState = 'error';
        hotkeyError = formatIpcError(error, 'Could not restore shortcuts');
      }
    }
  }

  function handleCaptureBlur() { cancelRecordingHotkey(); }

  function handleHotkeyKeydown(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.code === 'Escape' && capturedKeys.length === 0) {
      cancelRecordingHotkey();
      return;
    }
    capturedKeys = capture.press(e.code, e.repeat, { Control: e.ctrlKey, Alt: e.altKey, Shift: e.shiftKey, Meta: e.metaKey });
    hotkeyState = capturedKeys.length ? 'first' : 'armed';
  }

  function handleHotkeyKeyup(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    const keys = capture.release(e.code);
    if (keys) void finishRecordingHotkey(keys);
  }

  async function finishRecordingHotkey(keys: string[]) {
    removeHotkeyCaptureListeners();
    recordingHotkey = false;
    hotkeyState = 'saving';
    let outcome: 'success' | 'error' = 'success';
    try {
      const available = await invoke<boolean>('check_hotkey', { keys });
      if (!available) throw new Error('That shortcut is already assigned. Choose another combination.');
      await invoke('save_hotkey', { keys });
      hotkey = keys;
      await loadHotkey();
    } catch (error) {
      outcome = 'error';
      hotkeyError = formatIpcError(error, 'Could not save this shortcut');
    } finally {
      try {
        await invoke('set_hotkey_capture', { active: false });
      } catch (error) {
        outcome = 'error';
        hotkeyError = formatIpcError(error, 'Could not restore shortcuts');
      }
      hotkeyState = outcome;
      feedbackTimer = setTimeout(() => { hotkeyState = 'idle'; }, outcome === 'success' ? HOTKEY_SUCCESS_MS : HOTKEY_ERROR_MS);
    }
  }

  onDestroy(() => {
    destroyed = true;
    if (recordingHotkey) void invoke('set_hotkey_capture', { active: false }).catch(() => {});
    clearTimeout(feedbackTimer);
    removeHotkeyCaptureListeners();
    capture.reset();
    recordingHotkey = false;
  });

  onMount(() => {
    let unlisten: (() => void) | undefined;
    let unlistenPill: (() => void) | undefined;
    let active = true;
    const onVisible = () => { if (document.visibilityState === 'visible') void refreshPillPlacement(); };
    if (isAndroid) {
      document.addEventListener('visibilitychange', onVisible);
      listen('verenu:android-pill-position-changed', () => void refreshPillPlacement())
        .then(cleanup => { if (active) unlistenPill = cleanup; else cleanup(); })
        .catch(() => {});
    }
    listenForSyncCompletion(() => {
      // Refresh shared values only. Do not reset a theme preview, microphone,
      // shortcut capture, or other device-local controls during a peer sync.
      void Promise.all([
        invoke<boolean | null>('get_setting', { key: 'contextual_formatting_enabled' }),
        invoke<boolean | null>('get_setting', { key: 'cleanup_enabled' }),
        invoke<TranscriptionLanguageCode | null>('get_setting', { key: 'transcription_language' }),
      ]).then(([formatting, cleanup, language]) => {
        if (!active) return;
        contextualFormatting = formatting ?? true;
        appStore.cleanupEnabled = cleanup ?? true;
        if (!languageDropdownOpen && language && transcriptionLanguages.some(option => option.code === language)) {
          selectedLanguage = language;
        }
      }).catch(() => {});
    }).then(cleanup => { if (active) unlisten = cleanup; else cleanup(); }).catch(() => {});
    return () => {
      active = false;
      unlisten?.();
      unlistenPill?.();
      document.removeEventListener('visibilitychange', onVisible);
    };
  });

  loadSettings();
</script>
<svelte:window onclick={handleWindowClick} onkeydown={(e) => { handleCleanupModalKeydown(e); handleLegacyModalKeydown(e); }} />

<h2 class="settings-h">General</h2>
<h3 class="settings-subhead first">Dictation</h3>
{#if isAndroid}
  <div class="setting-row" data-setting-target="general-hotkey">
    <div><div class="label">Dictation control</div><div class="desc">Open a text field and use the Verenu pill above your keyboard.</div></div>
    <span class="badge key-badge">Keyboard pill</span>
  </div>
  <div class="setting-row" data-setting-target="general-pill-position">
    <div>
      <div class="label">Pill position</div>
      <div class="desc">Where the dictation pill appears while the keyboard is up. Hold the pill and drag it to move it; it snaps to the nearest position here.</div>
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="ui-dropdown pill-dropdown" onkeydown={(e) => { if (e.key === 'Escape' && pillDropdownOpen) { pillDropdownOpen = false; e.stopPropagation(); } }}>
      <button
        class="ui-dropdown-trigger ui-dropdown-trigger--compact mic-btn"
        onclick={() => (pillDropdownOpen = !pillDropdownOpen)}
        aria-haspopup="true"
        aria-expanded={pillDropdownOpen}
        aria-controls={PILL_MENU_ID}
        aria-label="Pill position"
      >
        <span class="mic-btn-label">{pillPositionLabel}</span>
        <svg class:open={pillDropdownOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="m6 9 6 6 6-6"/>
        </svg>
      </button>
      {#if pillDropdownOpen}
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
          id={PILL_MENU_ID}
          class="ui-dropdown-menu ui-dropdown-menu--padded mic-menu scroll-styled scroll-thumb-elev"
          aria-label="Pill position options"
          onclick={(e) => e.stopPropagation()}
          in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }}
          out:fade={{ duration: motionMs(MOTION_MS.fast) }}
        >
          {#each ANDROID_PILL_POSITION_OPTIONS as option}
            <button class="ui-dropdown-option mic-item" class:active={pillPosition === option.id} onclick={() => savePillPosition(option.id)}>{option.label}</button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
  <div class="setting-row" data-setting-target="general-pill-dock-position">
    <div>
      <div class="label">Pill position without a keyboard</div>
      <div class="desc">Where the pill rests when a dictation continues after the keyboard closes. Hold the pill and drag it to move it; it snaps to the nearest position and saves here.</div>
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="ui-dropdown dock-dropdown" onkeydown={(e) => { if (e.key === 'Escape' && dockDropdownOpen) { dockDropdownOpen = false; e.stopPropagation(); } }}>
      <button
        class="ui-dropdown-trigger ui-dropdown-trigger--compact mic-btn"
        onclick={() => (dockDropdownOpen = !dockDropdownOpen)}
        aria-haspopup="true"
        aria-expanded={dockDropdownOpen}
        aria-controls={DOCK_MENU_ID}
        aria-label="Pill position without a keyboard"
      >
        <span class="mic-btn-label">{pillDockPositionLabel}</span>
        <svg class:open={dockDropdownOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="m6 9 6 6 6-6"/>
        </svg>
      </button>
      {#if dockDropdownOpen}
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
          id={DOCK_MENU_ID}
          class="ui-dropdown-menu ui-dropdown-menu--padded mic-menu scroll-styled scroll-thumb-elev"
          aria-label="Pill position without a keyboard options"
          onclick={(e) => e.stopPropagation()}
          in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }}
          out:fade={{ duration: motionMs(MOTION_MS.fast) }}
        >
          {#each ANDROID_PILL_SCREEN_POSITION_OPTIONS as option}
            <button class="ui-dropdown-option mic-item" class:active={pillDockPosition === option.id} onclick={() => savePillDockPosition(option.id)}>{option.label}</button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
  <div class="setting-row" data-setting-target="general-cover-keyboard-mic">
    <div>
      <div class="label">Cover the keyboard's mic button</div>
      <div class="desc">Sit the pill over your keyboard's own voice-typing button so only Verenu's is tapped. Falls back to the position above when the keyboard has no mic button. Hold the pill and drag it to the top to hide it for 15 minutes; it can't be moved while it covers the button.</div>
    </div>
    <Toggle checked={coverKeyboardMic} onchange={handleCoverKeyboardMic} label="Cover the keyboard's mic button" bind:error={coverKeyboardMicError} />
  </div>
  <div class="setting-row" data-setting-target="general-hide-pill-offline">
    <div>
      <div class="label">Hide pill when offline</div>
      <div class="desc">Hide the pill without internet when any selected transcription or enabled cleanup model, including fallbacks, uses a network provider. On-device models keep it available. Turn this off for models hosted on your local network.</div>
    </div>
    <Toggle checked={hidePillOffline} onchange={handleHidePillOffline} label="Hide pill when offline" bind:error={hidePillOfflineError} />
  </div>
{:else}
  <div class="setting-row" data-setting-target="general-hotkey">
    <div><div class="label">Hotkey</div><div class="desc">Hold to record, release to transcribe. Click to change, hold all your keys, then release.</div></div>
    <button
      bind:this={keybindEl}
      class="badge key-badge keybind-btn"
      onclick={startRecordingHotkey}
      disabled={hotkeyState === 'saving'}
      aria-label={recordingHotkey ? 'Recording shortcut' : 'Change dictation hotkey'}
      aria-describedby="hotkey-help"
      class:recording={recordingHotkey}
      class:armed={hotkeyState === 'armed'}
      class:first={hotkeyState === 'first'}
      class:saving={hotkeyState === 'saving'}
      class:success={hotkeyState === 'success'}
      class:error={hotkeyState === 'error'}
    >
      {#key buttonText}
        <span aria-live="polite" in:fade={{ duration: motionMs(MOTION_MS.fast) }}>{buttonText}</span>
      {/key}
    </button>
  </div>
  <p id="hotkey-help" class="hotkey-tip" class:hotkey-error={Boolean(hotkeyError)} role={hotkeyError ? 'alert' : undefined}>
    {hotkeyError || (recordingHotkey ? 'Hold every key in the combination. Release any key to save. Escape before pressing keys or click outside to cancel.' : isMac ? 'Use any modifiers with one key, or a function key on its own.' : isLinux ? 'Use any modifiers with one key, a function key on its own, or a modifier-only combination.' : 'You can use one key or hold several keys together.')}
  </p>
{/if}
{#if isMac && hotkey[0] === 'F5'}
  <p class="hotkey-tip">
    F5 is the 🎤 key on Mac keyboards. If pressing it opens macOS Dictation instead of
    Verenu, turn off Dictation in <strong>System Settings → Keyboard → Dictation</strong>
    (or hold <strong>Fn</strong> with F5). You can also pick any other key above.
  </p>
{/if}
{#if isLinux && !isAndroid}<DesktopShortcutStatus id="dictation" />{/if}
<!-- Keyboard chord, OS autostart, and Caps Lock have no phone equivalent —
     Android hides them rather than showing dead or Windows-worded controls. -->
{#if !isAndroid}
  <div class="setting-row" data-setting-target="general-copy-last">
    <div><div class="label">Copy last dictation</div><div class="desc">Re-copies your last dictation to the clipboard, in case a paste didn't land</div></div>
    <span class="badge key-badge">{isLinux && !isAndroid && desktopShortcut('copy') ? desktopShortcut('copy')?.active ?? 'Unavailable' : isMac ? '⌥⌘C' : 'Ctrl+Alt+C'}</span>
  </div>
{/if}
{#if isLinux && !isAndroid}
  <DesktopShortcutStatus id="copy" />
  {#each [{ id: 'cancel', label: 'Cancel dictation', description: 'Available while recording or processing' }, { id: 'handsfree', label: 'Switch to hands-free', description: 'Available while holding the dictation shortcut' }] as control}
    <div class="setting-row">
      <div><div class="label">{control.label}</div><div class="desc">{control.description}</div></div>
      <span class="badge key-badge">{desktopShortcut(control.id as 'cancel' | 'handsfree')?.active ?? 'Unavailable'}</span>
    </div>
    <DesktopShortcutStatus id={control.id as 'cancel' | 'handsfree'} />
  {/each}
{/if}
<div class="setting-row" data-setting-target="general-language">
  <div class="lang-setting-text"><div class="label">Spoken Language</div><div class="desc">Tells transcription what language to expect{languageScopeNote}</div></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ui-dropdown language-dropdown" onkeydown={(e) => { if (e.key === 'Escape' && languageDropdownOpen) { languageDropdownOpen = false; e.stopPropagation(); } }}>
    <button
      class="ui-dropdown-trigger ui-dropdown-trigger--compact language-btn"
      use:animateWidth={{ text: getTranscriptionLanguageLabel(selectedLanguage) }}
      onclick={() => (languageDropdownOpen = !languageDropdownOpen)}
      aria-haspopup="true"
      aria-expanded={languageDropdownOpen}
      aria-controls={LANGUAGE_MENU_ID}
      aria-label="Spoken language"
    >
      <span>{getTranscriptionLanguageLabel(selectedLanguage)}</span>
      <span class="language-code">{selectedLanguage}</span>
      <svg class:open={languageDropdownOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="m6 9 6 6 6-6"/>
      </svg>
    </button>
    {#if languageDropdownOpen}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div
        id={LANGUAGE_MENU_ID}
        class="ui-dropdown-menu ui-dropdown-menu--padded language-menu scroll-styled scroll-thumb-elev"
        aria-label="Spoken language options"
        onclick={(e) => e.stopPropagation()}
        in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }}
        out:fade={{ duration: motionMs(MOTION_MS.fast) }}
      >
        {#each visibleLanguages as language}
          <button
            class="ui-dropdown-option language-item"
            class:active={selectedLanguage === language.code}
            onclick={() => saveLanguage(language.code)}
          >
            <span>{language.label}</span>
            <span>{language.code}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</div>
<div class="setting-row" data-setting-target="general-microphone">
  <div>
    <div class="label">{microphoneCopy.inputDeviceLabel}</div>
    <div class="desc">{microphoneCopy.inputDeviceDescription}</div>
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ui-dropdown mic-dropdown" onkeydown={(e) => { if (e.key === 'Escape' && micDropdownOpen) { micDropdownOpen = false; e.stopPropagation(); } }}>
    <button
      class="ui-dropdown-trigger ui-dropdown-trigger--compact mic-btn"
      use:animateWidth={{ text: selectedMic || microphoneCopy.defaultDevice, max: 180 }}
      onclick={() => (micDropdownOpen = !micDropdownOpen)}
      aria-haspopup="true"
      aria-expanded={micDropdownOpen}
      aria-controls={MIC_MENU_ID}
      aria-label="Microphone device"
    >
      <span class="mic-btn-label">{selectedMic || microphoneCopy.defaultDevice}</span>
      <svg class:open={micDropdownOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="m6 9 6 6 6-6"/>
      </svg>
    </button>
    {#if micDropdownOpen}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div
        id={MIC_MENU_ID}
        class="ui-dropdown-menu ui-dropdown-menu--padded mic-menu scroll-styled scroll-thumb-elev"
        aria-label="Microphone device options"
        onclick={(e) => e.stopPropagation()}
        in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }}
        out:fade={{ duration: motionMs(MOTION_MS.fast) }}
      >
        <button class="ui-dropdown-option mic-item" class:active={!selectedMic} onclick={() => saveMic('')}>{microphoneCopy.defaultDevice}</button>
        {#each microphones as m}
          <button class="ui-dropdown-option mic-item" class:active={selectedMic === m} onclick={() => saveMic(m)}>{m}</button>
        {/each}
        {#if microphones.length === 0}
          <div class="mic-empty">{microphoneCopy.noDevicesFound}</div>
        {/if}
      </div>
    {/if}
  </div>
</div>
<h3 class="settings-subhead">Appearance & System</h3>
<AppearanceSettings />
<div class="setting-row" data-setting-target="general-accent">
  <div><div class="label">Accent color</div><div class="desc">Used for actions, highlights, focus rings, and status details</div></div>
  <AccentColorPicker value={appStore.accentColor} onchange={handleAccentColor} />
</div>
{#if !isAndroid}
  <div class="setting-row" data-setting-target="general-startup">
    <div><div class="label">Start on Boot</div><div class="desc">{isMac ? 'Launch Verenu when macOS starts' : isLinux ? 'Launch Verenu when you log in' : 'Launch Verenu when Windows starts'}</div></div>
    <Toggle checked={autostart} onchange={handleAutostart} label="Start on boot" bind:error={autostartError} />
  </div>
{/if}
<h3 class="settings-subhead">Text processing</h3>
<div class="setting-row" data-setting-target="general-cleanup">
  <div><div class="label">Cleanup</div><div class="desc">Runs an LLM-powered cleanup pass after transcription for tone and formatting.</div></div>
  <Toggle checked={appStore.cleanupEnabled} onchange={handleCleanup} label="Cleanup" bind:error={cleanupError} />
</div>
<div class="setting-row" data-setting-target="general-spacing">
  <div><div class="label">Smart spacing &amp; capitalization</div><div class="desc">Adjusts capitalization and spacing around inserted text when the cursor context is clear.</div></div>
  <Toggle checked={contextualFormatting} onchange={handleContextualFormatting} label="Smart spacing and capitalization" bind:error={contextualFormattingError} />
</div>
{#if !isAndroid}
  <div class="setting-row" data-setting-target="general-caps-lock">
    <div><div class="label">Automatic caps lock detection</div><div class="desc">When Caps Lock is on, output your dictation in ALL CAPS</div></div>
    <Toggle checked={capsLockUppercase} onchange={handleCapsLockUppercase} label="Automatic caps lock detection" bind:error={capsLockUppercaseError} />
  </div>
{/if}
<h3 class="settings-subhead">Legacy</h3>
<div class="setting-row" data-setting-target="general-legacy">
  <div><div class="label">Legacy pages</div><div class="desc">Bring back the standalone App Mappings settings page and the Dictionary/Snippets pages, superseded by Contexts.</div></div>
  <Toggle checked={appStore.legacyFeaturesEnabled} onchange={handleLegacyFeatures} label="Legacy pages" bind:error={legacyFeaturesToggleError} />
</div>
{#if legacyFeaturesError}
  <p class="settings-error" role="alert">{legacyFeaturesError}</p>
{/if}

{#if confirmCleanupOff}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
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

{#if confirmLegacyOn}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <button class="modal-backdrop" aria-label="Close dialog" onclick={() => (confirmLegacyOn = false)} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
  <div
    class="modal-card"
    use:modalFocusTrap={{
      active: confirmLegacyOn,
      initialFocus: () => legacyCancelButton,
    }}
    role="dialog"
    aria-modal="true"
    aria-labelledby="legacy-on-confirm-title"
    tabindex="-1"
    in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }}
    out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}
  >
    <div class="modal-header">
      <h2 id="legacy-on-confirm-title" class="modal-title">Turn on Legacy pages?</h2>
    </div>
    <div class="modal-body">
      <p class="confirm-copy">
        This brings back the standalone App Mappings settings page and the Dictionary/Snippets
        pages. They're no longer actively maintained now that Contexts covers the same ground, so
        expect rough edges. You can turn this back off anytime.
      </p>
    </div>
    <div class="modal-footer">
      <div class="footer-actions">
        <button bind:this={legacyCancelButton} class="btn-ghost" onclick={() => (confirmLegacyOn = false)}>Cancel</button>
        <button class="btn-primary" onclick={confirmLegacyOnAction}>Turn on</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .hotkey-tip {
    margin: -2px 0 2px;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--ink-mute);
    max-width: 52ch;
  }
  .hotkey-tip strong { color: var(--ink-soft); font-weight: 600; }

  .keybind-btn {
    cursor: pointer;
    border: 1px solid transparent;
    transition:
      width 240ms cubic-bezier(0.22, 1, 0.36, 1),
      background 0.18s cubic-bezier(0.22, 1, 0.36, 1),
      color 0.18s cubic-bezier(0.22, 1, 0.36, 1),
      transform 0.18s cubic-bezier(0.22, 1, 0.36, 1),
      box-shadow 0.18s cubic-bezier(0.22, 1, 0.36, 1),
      border-color 0.18s cubic-bezier(0.22, 1, 0.36, 1),
      opacity 0.18s cubic-bezier(0.22, 1, 0.36, 1);
    user-select: none;
    transform-origin: center;
    white-space: normal;
    max-width: 100%;
    overflow-wrap: anywhere;
  }
  .hotkey-error { color: var(--danger); }
  .keybind-btn:hover { background: var(--control-hover); }
  .keybind-btn.recording { background: var(--accent); color: var(--on-accent); animation: pulse 1.5s infinite; }
  .keybind-btn.armed { transform: scale(1.02); }
  .keybind-btn.first { transform: scale(1.03); box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 40%, transparent); }
  .keybind-btn.saving { opacity: 0.9; }
  .keybind-btn.success { background: color-mix(in srgb, var(--accent) 82%, white 18%); color: var(--on-accent); transform: scale(1.03); }
  .keybind-btn.error { background: var(--danger-bg); color: var(--danger); border-color: var(--danger-line); animation: none; }
  .settings-error {
    margin: -2px 0 12px;
    color: var(--danger);
    font-size: 12px;
    line-height: 1.45;
  }
  @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.7; } }
  .mic-btn {
    max-width: 180px;
  }
  .mic-btn-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    text-align: left;
  }
  .mic-menu {
    width: 220px;
  }
  .mic-empty { padding: 8px 10px; font-size: 12px; color: var(--ink-mute); text-align: center; }
  /* Let the label+desc column take remaining width and wrap within itself,
     so the (sometimes long) language-scope note never runs under the
     fixed-width dropdown button to its right. */
  .lang-setting-text { min-width: 0; flex: 1; padding-right: 14px; }
  .language-btn { max-width: 210px; }
  .language-btn span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 140px; }
  .language-code {
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--ink-faint);
    text-transform: uppercase;
  }
  .language-menu {
    min-width: 220px;
    max-width: 280px;
    max-height: 260px;
  }
  .language-item {
    display: flex;
    gap: 12px;
    justify-content: space-between;
  }
  .language-item span:last-child {
    color: var(--ink-faint);
    font-family: var(--mono);
    font-size: 10.5px;
    text-transform: uppercase;
  }
  /* ── cleanup-off confirm modal ── */
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
  .modal-header {
    padding: 20px 20px 0;
  }
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
  .modal-footer {
    padding: 0 20px 20px;
  }
  .footer-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
