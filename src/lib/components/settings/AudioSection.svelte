<script module lang="ts">
  let enableReuseAfterSetup = false;
  let enableRealtimeAfterSetup = false;
</script>

<script lang="ts">
  import { slide } from 'svelte/transition';
  import { onDestroy } from 'svelte';
  import Toggle from '../Toggle.svelte';
  import Callout from '../Callout.svelte';
  import Disclosure from '../Disclosure.svelte';
  import RollingNumber from '../RollingNumber.svelte';
  import LocalDownloadProgress from './LocalDownloadProgress.svelte';
  import { isAndroid, isLinux, isMac, isWindows } from '../../platform';
  import { booleanSettingHandler, loadSettingsSnapshot, saveSetting, saveSettingIfUnchanged } from '../../settings';
  import { MOTION_MS, motionMs } from '../../motion';
  import { defaultMicGain, micGainAdvice, shouldPersistMicGain } from './audioGain';
  import { invoke } from '../../tauri';

  type ReuseStatus = { state: 'unknown' | 'not_installed' | 'unavailable' | 'checking' | 'installing' | 'removing' | 'ready' | 'error'; message: string; has_files?: boolean; progress?: number | null };
  let reuseStatus = $state<ReuseStatus>({ state: 'unknown', message: 'Checking availability…' });
  let reuseEnabled = $state(false);
  let reuseError = $state(false);
  let realtimeEnabled = $state(false);
  let realtimeReady = $state(false);
  let realtimeError = $state(false);
  let reuseTimer: ReturnType<typeof setTimeout> | null = null;
  let reuseDisposed = false;

  let reuseResolved = $state(false);
  let reuseEligible = $state(false);
  let reuseCleanupOnly = $state(false);
  let reuseConfirmRemove = $state(false);
  let fadesOpen = $state(false);
  let reuseRemoveError = $state('');
  let realtimeSetupError = $state(false);

  function reuseErrorMessage(err: unknown, fallback: string): string {
    const detail = typeof err === 'string' ? err : err instanceof Error ? err.message : '';
    return detail.trim() || fallback;
  }

  async function refreshReuseStatus() {
    try { reuseStatus = await invoke<ReuseStatus | null>('nvidia_reuse_status') ?? { state: 'unavailable', message: 'NVIDIA RE-USE is available in the desktop app.' }; }
    catch (err) { reuseStatus = { state: 'error', message: reuseErrorMessage(err, 'Could not check the NVIDIA runtime.') }; }
    try { realtimeReady = await invoke<boolean>('nvidia_reuse_realtime_ready') ?? false; }
    catch { realtimeReady = false; }
    if (enableRealtimeAfterSetup && reuseStatus.state === 'ready' && realtimeReady) {
      enableRealtimeAfterSetup = false;
      realtimeSetupError = false;
      await handleRealtime(true);
    } else if (enableRealtimeAfterSetup && reuseStatus.state === 'ready' && !realtimeReady) {
      enableRealtimeAfterSetup = false;
      realtimeSetupError = true;
    } else if (reuseStatus.state === 'error' || reuseStatus.state === 'unavailable') {
      enableRealtimeAfterSetup = false;
    }
    if (enableReuseAfterSetup && reuseStatus.state === 'ready') {
      enableReuseAfterSetup = false;
      if (!noiseReduction) await handleNoiseReduction(true);
      if (noiseReduction) await handleReuse(true);
    } else if (reuseStatus.state === 'error' || reuseStatus.state === 'unavailable') {
      enableReuseAfterSetup = false;
    }
    if (reuseStatus.state === 'unavailable') {
      reuseEligible = false;
      reuseCleanupOnly = !!reuseStatus.has_files;
    } else if (['not_installed', 'installing', 'ready'].includes(reuseStatus.state)) {
      reuseEligible = true;
      reuseCleanupOnly = false;
    } else if (reuseStatus.state === 'error' && reuseResolved && !reuseCleanupOnly) reuseEligible = true;
    reuseResolved = true;
    if (!reuseDisposed) {
      const delay = ['unknown', 'checking', 'installing', 'removing'].includes(reuseStatus.state) ? 1500 : 10000;
      reuseTimer = setTimeout(() => void refreshReuseStatus(), delay);
    }
  }

  async function installReuse() {
    if (reuseTimer) clearTimeout(reuseTimer);
    try { await invoke('nvidia_reuse_install'); await refreshReuseStatus(); }
    catch (err) {
      enableReuseAfterSetup = false;
      reuseStatus = { state: 'error', message: reuseErrorMessage(err, 'Could not start runtime setup.') };
      if (!reuseDisposed) reuseTimer = setTimeout(() => void refreshReuseStatus(), 10000);
    }
  }

  async function checkReuse() {
    if (reuseTimer) clearTimeout(reuseTimer);
    reuseStatus = { state: 'checking', message: 'Testing the GPU and model…' };
    try { await invoke('nvidia_reuse_check'); await refreshReuseStatus(); }
    catch (err) {
      reuseStatus = { state: 'error', message: reuseErrorMessage(err, 'Could not check the NVIDIA runtime.') };
      if (!reuseDisposed) reuseTimer = setTimeout(() => void refreshReuseStatus(), 10000);
    }
  }

  const reuseBusy = $derived(['checking', 'installing', 'removing'].includes(reuseStatus.state));
  const reuseCanRemove = $derived(!!reuseStatus.has_files && !reuseBusy);
  const reuseDesc = $derived(
    reuseStatus.state === 'ready' ? 'Ready. Enhances audio on your NVIDIA GPU before transcription.'
    : reuseStatus.state === 'installing' ? 'Downloading and setting up the runtime…'
    : reuseStatus.state === 'checking' ? 'Testing the GPU and model…'
    : reuseStatus.state === 'removing' ? 'Deleting runtime files…'
    : reuseStatus.has_files ? 'Setup is incomplete. Turn on to finish, or delete the files.'
    : 'Experimental GPU enhancement. Turning on downloads several GB and needs 20 GiB of free disk space.'
  );
  const reuseProgressLabel = $derived(
    reuseStatus.state === 'removing' ? 'Deleting' : reuseStatus.state === 'checking' ? 'Testing' : 'Setting up'
  );

  const handleReuse = booleanSettingHandler('nvidia_reuse_enabled', value => reuseEnabled = value, () => reuseError = true);
  const handleRealtime = booleanSettingHandler('nvidia_reuse_realtime_enabled', value => realtimeEnabled = value, () => realtimeError = true);

  async function changeRealtime(value: boolean) {
    if (!value) realtimeSetupError = false;
    if (value && !realtimeReady) {
      realtimeSetupError = false;
      enableRealtimeAfterSetup = true;
      if (reuseTimer) clearTimeout(reuseTimer);
      try { await invoke('nvidia_reuse_realtime_install'); await refreshReuseStatus(); }
      catch {
        enableRealtimeAfterSetup = false;
        realtimeSetupError = true;
        if (!reuseDisposed) reuseTimer = setTimeout(() => void refreshReuseStatus(), 10000);
      }
      return;
    }
    await handleRealtime(value);
  }

  async function retryRealtime() {
    await changeRealtime(true);
  }

  async function removeReuse() {
    reuseConfirmRemove = false;
    reuseRemoveError = '';
    enableReuseAfterSetup = false;
    enableRealtimeAfterSetup = false;
    if (reuseTimer) clearTimeout(reuseTimer);
    if (realtimeEnabled) await handleRealtime(false);
    if (realtimeEnabled) { reuseRemoveError = 'Could not turn off real-time RE-USE.'; void refreshReuseStatus(); return; }
    if (reuseEnabled) await handleReuse(false);
    if (reuseEnabled) {
      reuseRemoveError = 'Could not turn off NVIDIA RE-USE, so the files were not deleted.';
      void refreshReuseStatus();
      return;
    }
    reuseStatus = { ...reuseStatus, state: 'removing', message: 'Deleting runtime files…' };
    try { await invoke('nvidia_reuse_remove'); }
    catch (err) { reuseRemoveError = reuseErrorMessage(err, 'Could not delete the NVIDIA runtime files.'); }
    await refreshReuseStatus();
  }

  async function changeReuse(value: boolean) {
    reuseRemoveError = '';
    if (!value && realtimeEnabled) {
      await handleRealtime(false);
      if (realtimeEnabled) return;
    }
    if (value && reuseStatus.state !== 'ready') {
      enableReuseAfterSetup = true;
      await installReuse();
      return;
    }
    if (value && !noiseReduction) await handleNoiseReduction(true);
    if (!value || noiseReduction) await handleReuse(value);
  }

  let noiseReduction = $state(true);
  let echoCancellation = $state(false);
  let duckTarget = $state(100);
  let savedDuckTarget = 100;
  let duckFadeDown = $state(250);
  let duckFadeUp = $state(150);
  let duckUpSmooth = $state(true);
  let muteAudio = $state(false);
  let exclusiveMic = $state(false);
  let pauseMediaDuringDictation = $state(false);
  let micMuteButtonDictation = $state(false);
  let soundEffectsVolume = $state(100);
  let micGain = $state(defaultMicGain(isAndroid));
  let micGainSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let lastSavedMicGain: number | null = null;
  let micGainChangedByUser = false;
  let micGainLoaded = false;
  let micGainSaving: Promise<void> | null = null;
  let micGainError = $state(false);
  const gainAdvice = $derived(micGainAdvice(micGain, noiseReduction));

  async function loadSettings() {
    try {
      const { noise_reduction: nr, nvidia_reuse_enabled: reuse, nvidia_reuse_realtime_enabled: realtime, echo_cancellation: echo, mute_audio: mute, exclusive_mic: exclusive,
        pause_media_during_dictation: pauseMedia, mic_mute_button_dictation: micMute,
        play_start_stop_sounds: legacySounds, sound_effects_volume: savedVolume,
        mic_gain: savedGain, duck_audio: duck, duck_target: target,
        duck_fade_down_ms: fadeDown, duck_fade_up_ms: fadeUp,
        duck_fade_up_curve: upCurve } = await loadSettingsSnapshot();
      noiseReduction = nr ?? true;
      reuseEnabled = reuse ?? false;
      realtimeEnabled = realtime ?? false;
      echoCancellation = echo ?? false;
      duckTarget = isLinux && !isAndroid && mute ? 0 : duck ? target ?? 25 : 100;
      savedDuckTarget = duckTarget;
      duckFadeDown = fadeDown ?? 250;
      duckFadeUp = fadeUp ?? 150;
      duckUpSmooth = upCurve !== 'linear';
      muteAudio = mute ?? false;
      exclusiveMic = exclusive ?? false;
      pauseMediaDuringDictation = pauseMedia ?? false;
      micMuteButtonDictation = micMute ?? false;
      soundEffectsVolume = savedVolume !== null && savedVolume !== undefined
        ? Math.max(0, Math.min(100, savedVolume))
        : legacySounds === false ? 0 : 100;
      if (savedGain !== null && savedGain !== undefined) {
        micGain = Math.max(1, Math.min(8, savedGain));
        lastSavedMicGain = savedGain;
      } else {
        lastSavedMicGain = null;
      }
      micGainLoaded = true;
    } catch (err) {
      console.error('AudioSection load failed:', err);
    }
  }

  let noiseReductionError = $state(false);
  let echoCancellationError = $state(false);
  let duckUpCurveError = $state(false);
  let duckSettingError = $state(false);
  let muteAudioError = $state(false);
  let exclusiveMicError = $state(false);
  let pauseMediaError = $state(false);
  let micMuteButtonDictationError = $state(false);

  const handleNoiseReduction = booleanSettingHandler('noise_reduction', value => noiseReduction = value, () => noiseReductionError = true);
  const handleEchoCancellation = booleanSettingHandler('echo_cancellation', value => echoCancellation = value, () => echoCancellationError = true);
  async function saveDuckSetting(key: 'duck_target' | 'duck_fade_down_ms' | 'duck_fade_up_ms', value: number) {
    try { await saveSetting(key, value); if (key === 'duck_target') { muteAudio = false; savedDuckTarget = value; } duckSettingError = false; }
    catch (err) { if (key === 'duck_target') duckTarget = savedDuckTarget; duckSettingError = true; console.error(`Saving ${key} failed:`, err); }
  }
  async function setDuckCurve(smooth: boolean) {
    try {
      await saveSetting('duck_fade_up_curve', smooth ? 'smooth' : 'linear');
      duckUpSmooth = smooth;
    } catch (err) {
      duckUpCurveError = true;
      console.error('Saving fade curve failed:', err);
    }
  }
  const handleMuteAudio = booleanSettingHandler('mute_audio', value => muteAudio = value, () => muteAudioError = true);
  const handleExclusiveMic = booleanSettingHandler('exclusive_mic', value => exclusiveMic = value, () => exclusiveMicError = true);
  const handlePauseMedia = booleanSettingHandler('pause_media_during_dictation', value => pauseMediaDuringDictation = value, () => pauseMediaError = true);
  const handleMicMuteButtonDictation = booleanSettingHandler('mic_mute_button_dictation', value => micMuteButtonDictation = value, () => micMuteButtonDictationError = true);

  async function saveSoundEffectsVolume() {
    try {
      await saveSetting('sound_effects_volume', soundEffectsVolume);
    } catch (err) {
      console.error('save sound_effects_volume failed:', err);
    }
  }

  async function persistMicGain() {
    while (micGainSaving) await micGainSaving;
    if (!micGainLoaded || micGainError) return;
    const value = micGain;
    if (!shouldPersistMicGain(value, lastSavedMicGain, micGainChangedByUser)) return;
    const expected = lastSavedMicGain;
    micGainSaving = (async () => {
      try {
        await saveSettingIfUnchanged('mic_gain', value, { mic_gain: expected });
        lastSavedMicGain = value;
        if (micGain === value) micGainChangedByUser = false;
      } catch (err) {
        micGainError = true;
        console.error('saveMicGain failed:', err);
      }
    })();
    await micGainSaving;
    micGainSaving = null;
  }

  function scheduleMicGainSave() {
    micGainChangedByUser = true;
    if (micGainSaveTimer) clearTimeout(micGainSaveTimer);
    micGainSaveTimer = setTimeout(() => {
      micGainSaveTimer = null;
      void persistMicGain();
    }, 250);
  }

  function saveMicGainOnRelease() {
    micGainChangedByUser = true;
    if (micGainSaveTimer) {
      clearTimeout(micGainSaveTimer);
      micGainSaveTimer = null;
    }
    void persistMicGain();
  }

  onDestroy(() => {
    reuseDisposed = true;
    if (reuseTimer) clearTimeout(reuseTimer);
    if (micGainSaveTimer) clearTimeout(micGainSaveTimer);
    void persistMicGain();
  });

  loadSettings();
  void refreshReuseStatus();
</script>

<h2 class="settings-h">
  Audio
  {#if import.meta.env.DEV}
    <span class="legacy-label" aria-hidden="true">Microphone</span>
  {/if}
</h2>

<h3 class="settings-subhead first">Microphone</h3>
<div class="setting-row gain-row" data-setting-target="audio-gain">
  <div class="gain-header">
    <div>
      <div class="label">Microphone gain</div>
      <div class="desc">Speech level adjusts automatically. Leave this at the default unless speech is too quiet or not detected; higher values target a louder level.</div>
    </div>
    <span class="gain-value"><RollingNumber value={micGain} decimals={1} suffix="×" /></span>
  </div>
  <div class="gain-slider-wrap">
    <input
      type="range"
      class="gain-slider"
      min="1" max="8" step="0.1"
      bind:value={micGain}
      oninput={scheduleMicGainSave}
      onchange={saveMicGainOnRelease}
      style="--pct: {((micGain - 1) / 7 * 100).toFixed(1)}%"
      aria-label="Microphone gain"
    />
    <div class="gain-ticks">
      <span>1×</span>
      <span>4×</span>
      <span>8×</span>
    </div>
  </div>
  {#if micGainError}<div class="desc" role="alert">Could not save microphone gain. Reload Audio settings before trying again.</div>{/if}
  {#if gainAdvice}
    <div class="gain-tip" role="note" transition:slide={{ duration: motionMs(MOTION_MS.base) }}>
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
      {gainAdvice}
    </div>
  {/if}
</div>

{#if isLinux && !isAndroid}
  <div class="setting-row" data-setting-target="audio-echo-cancellation">
    <div><div class="label">Echo cancellation</div><div class="desc">Reduce speaker audio picked up by the default microphone. Requires PipeWire echo cancellation and the Default Device microphone. Turn this off if your microphone already cancels echo.</div></div>
    <Toggle checked={echoCancellation} onchange={handleEchoCancellation} label="Echo cancellation" bind:error={echoCancellationError} />
  </div>
{/if}
<div class="setting-row" data-setting-target="audio-noise">
  <div><div class="label">Noise reduction</div><div class="desc">{reuseEnabled && reuseStatus.state === 'ready' && (!realtimeEnabled || realtimeReady) ? 'Enhance audio before transcription with NVIDIA RE-USE' : 'Suppress background noise before transcription (RNNoise)'}</div></div>
  <Toggle checked={noiseReduction} onchange={handleNoiseReduction} label="Noise reduction" bind:error={noiseReductionError} />
</div>

{#if reuseResolved && ((reuseEligible && reuseStatus.state !== 'unavailable') || reuseCleanupOnly)}
  <div class="reuse" data-setting-target="audio-nvidia-reuse" transition:slide={{ duration: motionMs(MOTION_MS.base) }}>
    <div class="setting-row">
      {#if reuseCleanupOnly}
        <div><div class="label">Downloaded NVIDIA RE-USE files</div><div class="desc">A compatible NVIDIA GPU is unavailable. You can delete the downloaded files.</div></div>
      {:else}
        <div>
          <div class="label">NVIDIA RE-USE <span class="reuse-tag">Experimental</span></div>
          <div class="desc">{reuseDesc}</div>
        </div>
        <Toggle checked={reuseEnabled} onchange={changeReuse} label="Use NVIDIA RE-USE" disabled={reuseBusy} bind:error={reuseError} />
      {/if}
      {#if reuseBusy}
        <div class="reuse-progress" transition:slide={{ duration: motionMs(MOTION_MS.base) }} role="status" aria-live="polite">
          <LocalDownloadProgress stage="downloading" label={reuseProgressLabel} percent={reuseStatus.progress ?? 0} indeterminate={reuseStatus.progress == null} />
          {#if reuseStatus.message}<div class="desc reuse-note">{reuseStatus.message}</div>{/if}
        </div>
      {/if}
    </div>
    {#if reuseEnabled && reuseStatus.state === 'ready'}
      <div class="setting-row">
        <div>
          <div class="label">Real-time RE-USE <span class="reuse-tag">Research</span></div>
          <div class="desc">Process audio during recording. NVIDIA permits this model for noncommercial research and development only. Uses the local model memory policy.</div>
        </div>
        <Toggle checked={realtimeEnabled} onchange={changeRealtime} label="Use real-time RE-USE for research" disabled={reuseBusy} bind:error={realtimeError} />
      </div>
      {#if (realtimeEnabled && !realtimeReady) || realtimeSetupError}
        <div class="reuse-extra"><Callout severity="warning" compact title="Real-time model unavailable" action={{ label: 'Retry setup', onclick: retryRealtime }}>Noise reduction uses RNNoise until setup succeeds. Turn off real-time RE-USE to use the existing NVIDIA model.</Callout></div>
      {/if}
    {/if}
    {#if reuseStatus.state === 'error' && !reuseRemoveError && !reuseCleanupOnly}
      <div class="reuse-extra" transition:slide={{ duration: motionMs(MOTION_MS.base) }}>
        <Callout severity="error" compact title="NVIDIA RE-USE needs attention" action={{ label: 'Retry setup', onclick: () => changeReuse(true) }} secondaryAction={{ label: 'Check again', onclick: checkReuse }}>{reuseStatus.message}</Callout>
      </div>
    {/if}
    {#if reuseRemoveError}
      <div class="reuse-extra" transition:slide={{ duration: motionMs(MOTION_MS.base) }}>
        <Callout severity="error" compact title="Could not delete runtime" onDismiss={() => (reuseRemoveError = '')}>{reuseRemoveError}</Callout>
      </div>
    {/if}
    {#if reuseCanRemove}
      <div class="reuse-actions" transition:slide={{ duration: motionMs(MOTION_MS.base) }}>
        {#if reuseConfirmRemove}
          <span class="desc">Delete the downloaded runtime{reuseEnabled ? ' and turn off NVIDIA RE-USE' : ''}?</span>
          <button type="button" class="btn-danger btn-compact" onclick={removeReuse}>Delete files</button>
          <button type="button" class="btn-ghost btn-compact" onclick={() => (reuseConfirmRemove = false)}>Cancel</button>
        {:else}
          <button type="button" class="btn-ghost btn-compact" onclick={() => (reuseConfirmRemove = true)}>Delete downloaded files</button>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<h3 class="settings-subhead">While you dictate</h3>
{#if !isLinux || isAndroid}
<div class="setting-row" data-setting-target="audio-system-mute">
  <div><div class="label">{isMac ? 'Mute System Audio' : isAndroid ? 'Mute device audio' : 'Mute PC Audio'}</div><div class="desc">{isMac ? 'Mutes system volume while dictating to prevent audio interference' : isAndroid ? 'Mutes device audio while dictating to prevent audio interference' : isWindows ? 'Mutes Windows volume while dictating to prevent audio interference' : 'Mutes the default output while dictating to prevent audio interference'}</div></div>
  <Toggle checked={muteAudio} onchange={handleMuteAudio} label={isMac ? 'Mute system audio' : isAndroid ? 'Mute device audio' : 'Mute PC audio'} bind:error={muteAudioError} />
</div>
{/if}
{#if isLinux && !isAndroid}
  <div data-setting-target="audio-system-mute">
    {#if duckSettingError}<div class="setting-row"><div class="desc" role="alert">Could not save the volume setting. Try changing it again.</div></div>{/if}
    <div class="setting-row sound-volume-row" transition:slide={{ duration: motionMs(MOTION_MS.base) }}>
      <div class="sound-volume-header"><div><div class="label">Volume of audio while dictating</div><div class="desc">Maximum system volume while dictating. Audio already below this level stays unchanged. 0% mutes immediately; 100% leaves it unchanged. Other levels fade down smoothly. Playback continues unless media pause is enabled. Partial reduction requires PipeWire's PulseAudio service and pactl.</div></div><span class="sound-volume-value"><RollingNumber value={duckTarget} suffix="%" /></span></div>
      <div class="sound-volume-slider-wrap">
        <input type="range" class="sound-volume-slider" min="0" max="100" step="1" bind:value={duckTarget} onchange={() => saveDuckSetting('duck_target', duckTarget)} style="--pct: {duckTarget}%" aria-label="Volume of audio while dictating" />
        <div class="sound-volume-ticks"><span>0%</span><span>50%</span><span>100%</span></div>
      </div>
    </div>
    {#if duckTarget > 0 && duckTarget < 100}
    <div class="volume-fades" transition:slide={{ duration: motionMs(MOTION_MS.base) }}>
    <Disclosure label="Advanced volume fades" id="volume-fades-details" bind:open={fadesOpen}>
    <div class="setting-row sound-volume-row">
      <div class="sound-volume-header"><div><div class="label">Fade down duration</div><div class="desc">Time to reach the target when dictation starts.</div></div><span class="sound-volume-value"><RollingNumber value={duckFadeDown} suffix=" ms" /></span></div>
      <div class="sound-volume-slider-wrap"><input type="range" class="sound-volume-slider" min="0" max="5000" step="50" bind:value={duckFadeDown} onchange={() => saveDuckSetting('duck_fade_down_ms', duckFadeDown)} style="--pct: {duckFadeDown / 50}%" aria-label="Fade down duration" /></div>
    </div>
    <div class="setting-row sound-volume-row">
      <div class="sound-volume-header"><div><div class="label">Fade up duration</div><div class="desc">Time to restore the volume when dictation ends.</div></div><span class="sound-volume-value"><RollingNumber value={duckFadeUp} suffix=" ms" /></span></div>
      <div class="sound-volume-slider-wrap"><input type="range" class="sound-volume-slider" min="0" max="5000" step="50" bind:value={duckFadeUp} onchange={() => saveDuckSetting('duck_fade_up_ms', duckFadeUp)} style="--pct: {duckFadeUp / 50}%" aria-label="Fade up duration" /></div>
    </div>
    <div class="setting-row"><div><div class="label">Smooth fade up</div><div class="desc">Ease the start and end of the fade; turn off for a linear curve.</div></div><Toggle checked={duckUpSmooth} onchange={setDuckCurve} label="Smooth fade up" bind:error={duckUpCurveError} /></div>
    </Disclosure>
    </div>
    {/if}
  </div>
{/if}
{#if isMac}
  <div class="setting-row" data-setting-target="audio-exclusive">
    <div><div class="label">Exclusive microphone access</div><div class="desc">Reserves the mic for Verenu while dictating, muting it for all other apps</div></div>
    <Toggle checked={exclusiveMic} onchange={handleExclusiveMic} label="Exclusive microphone access" bind:error={exclusiveMicError} />
  </div>
{/if}
{#if isWindows || (isLinux && !isAndroid)}
  <div class="setting-row" data-setting-target="audio-pause-media">
    <div><div class="label">Pause media while dictating</div><div class="desc">{isWindows ? 'Pauses active Windows media sessions and resumes them after transcription finishes. Works with apps that expose Windows media controls.' : 'Pauses playing music and videos and resumes them after transcription finishes. Works with apps and browsers that expose Linux media controls (MPRIS).'}</div></div>
    <Toggle checked={pauseMediaDuringDictation} onchange={handlePauseMedia} label="Pause media while dictating" bind:error={pauseMediaError} />
  </div>
{/if}
{#if isWindows || (isLinux && !isAndroid)}
  <h3 class="settings-subhead">Hands-free</h3>
  <div class="setting-row" data-setting-target="audio-mic-mute-button">
    <div>
      <div class="label">Use microphone mute button for dictation</div>
      <div class="desc">Mute then unmute the selected mic (within ~3s) to toggle hands-free dictation. Works with mute buttons the system can see — mixer mute, USB/headset hardware mute, or a mute that silences the capture stream (PipeWire on Linux). Keyboard hotkey is unchanged.</div>
    </div>
    <Toggle
      checked={micMuteButtonDictation}
      onchange={handleMicMuteButtonDictation}
      label="Use microphone mute button for dictation"
      bind:error={micMuteButtonDictationError}
    />
  </div>
{/if}

<h3 class="settings-subhead">Sounds</h3>
<div class="setting-row sound-volume-row" data-setting-target="audio-sounds">
  <div class="sound-volume-header">
    <div>
      <div class="label">Sound effects volume</div>
      <div class="desc">Set to 0% to silence dictation chimes</div>
    </div>
    <span class="sound-volume-value"><RollingNumber value={Math.round(soundEffectsVolume)} suffix="%" /></span>
  </div>
  <div class="sound-volume-slider-wrap">
    <input
      type="range"
      class="sound-volume-slider"
      min="0" max="100" step="1"
      bind:value={soundEffectsVolume}
      onchange={saveSoundEffectsVolume}
      style="--pct: {soundEffectsVolume.toFixed(1)}%"
      aria-label="Sound effects volume"
    />
    <div class="sound-volume-ticks">
      <span>0%</span>
      <span>50%</span>
      <span>100%</span>
    </div>
  </div>
</div>

<style>
  .volume-fades { margin-top: 8px; }
  .volume-fades :global(.disclosure-body) { padding-top: 4px; }

  .setting-row.gain-row { display: flex; flex-direction: column; align-items: stretch; gap: 0; }
  .gain-header { display: flex; align-items: center; justify-content: space-between; width: 100%; }
  .gain-value {
    font-family: var(--mono);
    font-size: 13px;
    font-weight: 500;
    color: var(--accent);
    min-width: 36px;
    text-align: right;
    flex-shrink: 0;
  }
  .gain-slider-wrap { margin-top: 10px; width: 100%; }
  .gain-slider {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(
      to right,
      var(--accent) 0%, var(--accent) var(--pct),
      var(--line-strong) var(--pct), var(--line-strong) 100%
    );
    outline: none;
    cursor: pointer;
    border: none;
    display: block;
  }
  .gain-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--bg-elev);
    border: 2px solid var(--accent);
    box-shadow: 0 1px 4px color-mix(in srgb, var(--accent) 35%, transparent);
    cursor: pointer;
    transition: box-shadow 0.15s ease, transform 0.15s ease;
  }
  .gain-slider::-webkit-slider-thumb:hover { box-shadow: 0 2px 8px color-mix(in srgb, var(--accent) 45%, transparent); transform: scale(1.1); }
  .gain-slider::-webkit-slider-thumb:active { box-shadow: 0 2px 10px color-mix(in srgb, var(--accent) 55%, transparent); transform: scale(1.15); }
  .gain-slider::-moz-range-thumb {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--bg-elev);
    border: 2px solid var(--accent);
    box-shadow: 0 1px 4px color-mix(in srgb, var(--accent) 35%, transparent);
    cursor: pointer;
  }
  .gain-ticks { display: flex; justify-content: space-between; margin-top: 5px; font-size: 10px; color: var(--ink-mute); font-family: var(--mono); user-select: none; }
  .gain-tip {
    grid-column: 1 / -1;
    width: 100%;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 10px;
    padding: 7px 10px;
    background: var(--warning-bg);
    border: 1px solid var(--warning-line);
    border-radius: 7px;
    font-size: 11.5px;
    color: var(--warning);
    line-height: 1.4;
  }
  .gain-tip svg { flex-shrink: 0; color: var(--warning); }

  .setting-row.sound-volume-row { display: flex; flex-direction: column; align-items: stretch; gap: 0; }
  .sound-volume-header { display: flex; align-items: center; justify-content: space-between; width: 100%; }
  .sound-volume-value {
    font-family: var(--mono);
    font-size: 13px;
    font-weight: 500;
    color: var(--accent);
    min-width: 44px;
    text-align: right;
    flex-shrink: 0;
  }
  .sound-volume-slider-wrap { margin-top: 10px; width: 100%; }
  .sound-volume-slider {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(
      to right,
      var(--accent) 0%, var(--accent) var(--pct),
      var(--line-strong) var(--pct), var(--line-strong) 100%
    );
    outline: none;
    cursor: pointer;
    border: none;
    display: block;
    margin: 0;
  }
  .sound-volume-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--bg-elev);
    border: 2px solid var(--accent);
    box-shadow: 0 1px 4px color-mix(in srgb, var(--accent) 35%, transparent);
    cursor: pointer;
    transition: box-shadow 0.15s ease, transform 0.15s ease;
  }
  .sound-volume-slider::-webkit-slider-thumb:hover { box-shadow: 0 2px 8px color-mix(in srgb, var(--accent) 45%, transparent); transform: scale(1.1); }
  .sound-volume-slider::-webkit-slider-thumb:active { box-shadow: 0 2px 10px color-mix(in srgb, var(--accent) 55%, transparent); transform: scale(1.15); }
  .sound-volume-slider::-moz-range-thumb {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--bg-elev);
    border: 2px solid var(--accent);
    box-shadow: 0 1px 4px color-mix(in srgb, var(--accent) 35%, transparent);
    cursor: pointer;
  }
  .sound-volume-ticks {
    position: relative;
    height: 16px;
    margin-top: 5px;
    font-size: 10px;
    color: var(--ink-mute);
    font-family: var(--mono);
    user-select: none;
  }
  .sound-volume-ticks span { position: absolute; top: 0; white-space: nowrap; }
  .sound-volume-ticks span:first-child { left: 0; }
  .sound-volume-ticks span:nth-child(2) { left: 50%; transform: translateX(-50%); }
  .sound-volume-ticks span:last-child { right: 0; }

  .reuse-tag {
    margin-left: 6px;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-ink);
    font-size: 10.5px;
    font-weight: 500;
    vertical-align: 1px;
  }
  .reuse-progress { grid-column: 1 / -1; margin-top: -12px; width: 100%; }
  @container settings-panel (max-width: 440px) {
    .reuse > .setting-row:first-child { grid-template-columns: minmax(0, 1fr) auto; align-items: center; column-gap: 16px; }
    .reuse > .setting-row:first-child > :global(.toggle) { justify-self: end; }
    .reuse-progress { margin-top: 0; }
  }
  .reuse-note { margin-top: 6px; }
  .reuse-extra { margin-top: 8px; }
  .reuse-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; margin-top: 8px; }

  .legacy-label {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

</style>
