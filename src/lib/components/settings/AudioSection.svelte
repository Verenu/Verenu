<script lang="ts">
  import { invoke } from '../../tauri';
  import { slide } from 'svelte/transition';
  import { onDestroy } from 'svelte';
  import Toggle from '../Toggle.svelte';
  import { isMac, isWindows } from '../../platform';
  import { saveSetting } from '../../settings';
  import { MOTION_MS, motionMs } from '../../motion';

  let noiseReduction = $state(true);
  let muteAudio = $state(false);
  let exclusiveMic = $state(false);
  let pauseMediaDuringDictation = $state(false);
  let micMuteButtonDictation = $state(false);
  let soundEffectsVolume = $state(100);
  let micGain = $state(3.5);
  let micGainSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let lastSavedMicGain: number | null = null;

  async function loadSettings() {
    try {
      const [nr, mute, exclusive, pauseMedia, micMute, legacySounds, savedVolume, savedGain] = await Promise.all([
        invoke<boolean | null>('get_setting', { key: 'noise_reduction' }),
        invoke<boolean | null>('get_setting', { key: 'mute_audio' }),
        invoke<boolean | null>('get_setting', { key: 'exclusive_mic' }),
        invoke<boolean | null>('get_setting', { key: 'pause_media_during_dictation' }),
        invoke<boolean | null>('get_setting', { key: 'mic_mute_button_dictation' }),
        invoke<boolean | null>('get_setting', { key: 'play_start_stop_sounds' }),
        invoke<number | null>('get_setting', { key: 'sound_effects_volume' }),
        invoke<number | null>('get_setting', { key: 'mic_gain' }),
      ]);
      noiseReduction = nr ?? true;
      muteAudio = mute ?? false;
      exclusiveMic = exclusive ?? false;
      pauseMediaDuringDictation = pauseMedia ?? false;
      micMuteButtonDictation = micMute ?? false;
      soundEffectsVolume = savedVolume !== null && savedVolume !== undefined
        ? Math.max(0, Math.min(100, savedVolume))
        : legacySounds === false ? 0 : 100;
      if (savedGain !== null && savedGain !== undefined) {
        micGain = Math.max(1, Math.min(8, savedGain));
      }
      lastSavedMicGain = micGain;
    } catch (err) {
      console.error('AudioSection load failed:', err);
    }
  }

  async function handleNoiseReduction(value: boolean) {
    noiseReduction = value;
    try {
      await saveSetting('noise_reduction', value);
    } catch (err) {
      noiseReduction = !value;
      console.error('save noise_reduction failed:', err);
    }
  }

  async function handleMuteAudio(value: boolean) {
    muteAudio = value;
    try {
      await saveSetting('mute_audio', value);
    } catch (err) {
      muteAudio = !value;
      console.error('save mute_audio failed:', err);
    }
  }

  async function handleExclusiveMic(value: boolean) {
    exclusiveMic = value;
    try {
      await saveSetting('exclusive_mic', value);
    } catch (err) {
      exclusiveMic = !value;
      console.error('save exclusive_mic failed:', err);
    }
  }

  async function handlePauseMedia(value: boolean) {
    pauseMediaDuringDictation = value;
    try {
      await saveSetting('pause_media_during_dictation', value);
    } catch (err) {
      pauseMediaDuringDictation = !value;
      console.error('save pause_media_during_dictation failed:', err);
    }
  }

  async function handleMicMuteButtonDictation(value: boolean) {
    micMuteButtonDictation = value;
    try {
      await saveSetting('mic_mute_button_dictation', value);
    } catch (err) {
      micMuteButtonDictation = !value;
      console.error('save mic_mute_button_dictation failed:', err);
    }
  }

  async function saveSoundEffectsVolume() {
    try {
      await saveSetting('sound_effects_volume', soundEffectsVolume);
    } catch (err) {
      console.error('save sound_effects_volume failed:', err);
    }
  }

  async function persistMicGain() {
    const value = micGain;
    if (lastSavedMicGain === value) return;
    lastSavedMicGain = value;
    try {
      await saveSetting('mic_gain', value);
    } catch (err) {
      lastSavedMicGain = null;
      console.error('saveMicGain failed:', err);
    }
  }

  function scheduleMicGainSave() {
    if (micGainSaveTimer) clearTimeout(micGainSaveTimer);
    micGainSaveTimer = setTimeout(() => {
      micGainSaveTimer = null;
      void persistMicGain();
    }, 250);
  }

  function saveMicGainOnRelease() {
    if (micGainSaveTimer) {
      clearTimeout(micGainSaveTimer);
      micGainSaveTimer = null;
    }
    void persistMicGain();
  }

  onDestroy(() => {
    if (micGainSaveTimer) clearTimeout(micGainSaveTimer);
    void persistMicGain();
  });

  loadSettings();
</script>

<h2 class="settings-h">
  Audio
  {#if import.meta.env.DEV}
    <span class="legacy-label" aria-hidden="true">Microphone</span>
  {/if}
</h2>

<h3 class="settings-subhead first">Gain</h3>
<div class="setting-row gain-row" data-setting-target="audio-gain">
  <div class="gain-header">
    <div>
      <div class="label">Microphone gain</div>
      <div class="desc">Boost signal strength before sending audio to the voice model</div>
    </div>
    <span class="gain-value">{micGain.toFixed(1)}×</span>
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
  {#if micGain >= 5}
    <div class="gain-tip" transition:slide={{ duration: motionMs(MOTION_MS.base) }}>
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
      At high gain, enable <strong>noise reduction</strong> to avoid amplifying background noise.
    </div>
  {/if}
</div>

<h3 class="settings-subhead">Input</h3>
<div class="setting-row" data-setting-target="audio-system-mute">
  <div><div class="label">{isMac ? 'Mute System Audio' : 'Mute PC Audio'}</div><div class="desc">{isMac ? 'Mutes system volume while dictating to prevent audio interference' : 'Mutes Windows volume while dictating to prevent audio interference'}</div></div>
  <Toggle checked={muteAudio} onchange={handleMuteAudio} label={isMac ? 'Mute system audio' : 'Mute PC audio'} />
</div>
{#if isMac}
  <div class="setting-row" data-setting-target="audio-exclusive">
    <div><div class="label">Exclusive microphone access</div><div class="desc">Reserves the mic for Verenu while dictating, muting it for all other apps</div></div>
    <Toggle checked={exclusiveMic} onchange={handleExclusiveMic} label="Exclusive microphone access" />
  </div>
{/if}
{#if isWindows}
  <div class="setting-row" data-setting-target="audio-pause-media">
    <div><div class="label">Pause media while dictating</div><div class="desc">Pauses active Windows media sessions and resumes them after transcription finishes. Works with apps that expose Windows media controls.</div></div>
    <Toggle checked={pauseMediaDuringDictation} onchange={handlePauseMedia} label="Pause media while dictating" />
  </div>
  <div class="setting-row" data-setting-target="audio-mic-mute-button">
    <div>
      <div class="label">Use microphone mute button for dictation</div>
      <div class="desc">Mute then unmute the selected mic (within ~3s) to toggle hands-free dictation. Works with mute buttons Windows can see — mixer mute, USB/headset hardware mute, or a mute that silences the capture stream. Keyboard hotkey is unchanged.</div>
    </div>
    <Toggle
      checked={micMuteButtonDictation}
      onchange={handleMicMuteButtonDictation}
      label="Use microphone mute button for dictation"
    />
  </div>
{/if}
<div class="setting-row" data-setting-target="audio-noise">
  <div><div class="label">Noise reduction</div><div class="desc">Suppress background noise before transcription (RNNoise)</div></div>
  <Toggle checked={noiseReduction} onchange={handleNoiseReduction} label="Noise reduction" />
</div>

<h3 class="settings-subhead">Sound effects</h3>
<div class="setting-row sound-volume-row" data-setting-target="audio-sounds">
  <div class="sound-volume-header">
    <div>
      <div class="label">Sound effects volume</div>
      <div class="desc">Set to 0% to silence dictation chimes</div>
    </div>
    <span class="sound-volume-value">{Math.round(soundEffectsVolume)}%</span>
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

  .gain-row { flex-direction: column; align-items: stretch; gap: 0; }
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
  .gain-tip strong { font-weight: 600; }

  .sound-volume-row { flex-direction: column; align-items: stretch; gap: 0; }
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
