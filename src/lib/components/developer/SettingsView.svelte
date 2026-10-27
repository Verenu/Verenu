<script lang="ts">
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { expoOut } from 'svelte/easing';
  import Toggle from '../Toggle.svelte';
  import Dropdown from '../Dropdown.svelte';
  import { appStore } from '../../stores';
  import { checkStatus } from '../../serviceStatus';
  import { ensureNotificationPermission } from '../../notifications';
  import { invoke } from '../../tauri';
  import { saveSetting, type ProviderId } from '../../settings';
  import { MOTION_MS, MOTION_PX, modalBackdrop, modalCard, motionMs, motionPx } from '../../motion';
  import { modalFocusTrap } from '../../modalFocus';

  const PROVIDERS: { value: ProviderId; label: string }[] = [
    { value: 'groq', label: 'Groq' }, { value: 'openai', label: 'OpenAI' },
    { value: 'google', label: 'Gemini' }, { value: 'assemblyai', label: 'AssemblyAI' },
    { value: 'openrouter', label: 'OpenRouter' }, { value: 'xai', label: 'xAI' },
  ];

  let verbose = $state(false);
  let ruin = $state(false);
  let devStartup = $state(false);
  let forceSetup = $state(false);
  let sync = $state(false);
  let storageFull = $state(false);
  let provider = $state<ProviderId>('groq');
  let providerOpen = $state(false);
  let message = $state('');
  let statusRaw = $state('');
  let privacyOpen = $state(false);
  let privacyClose: HTMLButtonElement | null = $state(null);

  async function setVerbose(value: boolean) {
    verbose = value;
    await invoke('set_dev_logging_enabled', { enabled: value }).catch(() => { verbose = !value; });
  }
  async function setRuin(value: boolean) {
    ruin = value; appStore.ruinAccessibility = value;
    if (value) appStore.devModeEnabled = true;
    await saveSetting('ruin_accessibility', value).catch(() => { ruin = !value; appStore.ruinAccessibility = !value; });
  }
  async function setDevStartup(value: boolean) {
    const previous = devStartup;
    devStartup = value; appStore.devModeOnStartup = value;
    await saveSetting('dev_mode_on_startup', value).catch(() => { devStartup = previous; appStore.devModeOnStartup = previous; });
  }
  async function setForceSetup(value: boolean) {
    forceSetup = value;
    await saveSetting('force_setup_on_launch', value).catch(() => { forceSetup = !value; });
  }
  async function setSync(value: boolean) {
    if (value && typeof window !== 'undefined' && !window.confirm('Enable experimental encrypted LAN sync?')) return;
    sync = value;
    await saveSetting('sync_enabled', value).then(() => { appStore.syncEnabled = value; }).catch(() => { sync = !value; });
  }

  function providerDown() {
    appStore.providerStatusSimulation = true;
    appStore.providerStatusAlerts = [{ providerId: provider, providerName: provider === 'google' ? 'Gemini' : provider, status: 'degraded', severity: 'high', message: 'Diagnostic provider degradation preview.', detailsUrl: '' }];
    message = `${provider} degraded preview on.`;
  }
  function offline() { appStore.isOnline = false; message = 'Offline state on.'; }
  function driveFull() { appStore.recoveryStorageWarning = true; message = 'Drive-full notice on.'; }
  function globalMessage() { appStore.globalMessageSimulation = true; appStore.globalMessage = { message: 'Diagnostic global message preview.', showToUsers: true }; message = 'Global message on.'; }
  async function toggleStorage(enabled: boolean) {
    storageFull = enabled;
    await invoke('set_storage_full_simulation', { enabled }).catch(() => { storageFull = !enabled; });
    message = enabled ? 'Storage writes failing.' : 'Storage writes normal.';
  }
  async function clearFaults() {
    appStore.providerStatusAlerts = []; appStore.providerStatusSimulation = false; appStore.globalMessage = null; appStore.globalMessageSimulation = false; appStore.recoveryStorageWarning = false; appStore.isOnline = true;
    await invoke('set_storage_full_simulation', { enabled: false }).catch(() => {});
    storageFull = false; message = 'All simulations cleared.'; void checkStatus().catch(() => {});
  }
  async function notify() {
    if (!(await ensureNotificationPermission())) { message = 'Notification permission not granted.'; return; }
    await invoke('test_notifications', { notificationType: 'service' }).then(() => { message = 'Notification sent.'; }).catch((error) => { message = `Notification failed: ${String(error).slice(0, 100)}`; });
  }
  async function installer() {
    await invoke<string>('reinstall_latest_update').then((version) => { message = `Installer started for v${version}.`; }).catch((error) => { message = `Installer failed: ${String(error).slice(0, 100)}`; });
  }
  async function providerStatus() {
    statusRaw = '';
    await invoke('check_provider_status_raw').then((value) => { statusRaw = JSON.stringify(value, null, 2); }).catch((error) => { statusRaw = `Check failed: ${String(error).slice(0, 160)}`; });
  }

  onMount(() => {
    Promise.all([
      invoke<boolean>('get_dev_logging_enabled'), invoke<boolean | null>('get_setting', { key: 'ruin_accessibility' }),
      invoke<boolean | null>('get_setting', { key: 'dev_mode_on_startup' }),
      invoke<boolean | null>('get_setting', { key: 'force_setup_on_launch' }), invoke<boolean | null>('get_setting', { key: 'sync_enabled' }),
      invoke<boolean>('get_storage_full_simulation'),
    ]).then(([v, r, d, f, s, st]) => { verbose = v; ruin = r ?? false; devStartup = d ?? false; forceSetup = f ?? false; sync = s ?? false; storageFull = st; }).catch(() => {});
  });
</script>

<svelte:window onkeydown={(event) => { if (event.key === 'Escape') privacyOpen = false; }} />

<div class="dev-settings" data-setting-target="developer-settings">
  <section>
    <div class="setting-row" data-setting-target="developer-logs"><div><div class="label">Verbose logging</div><div class="desc">Can include dictated text and prompts. <button class="link" onclick={() => (privacyOpen = true)}>Why</button></div></div><Toggle checked={verbose} onchange={setVerbose} label="Verbose logging" /></div>
    <div class="setting-row" data-setting-target="developer-ruin-accessibility"><div><div class="label">Ruin accessibility</div><div class="desc">Expose a privacy-filtered state dump for T3 Code.</div></div><Toggle checked={ruin} onchange={setRuin} label="Ruin accessibility" /></div>
    <div class="setting-row" data-setting-target="developer-dev-mode-startup"><div><div class="label">Dev mode on startup</div><div class="desc">Keep Developer visible after every launch.</div></div><Toggle checked={devStartup} onchange={setDevStartup} label="Enable dev mode on startup" /></div>
    <div class="setting-row" data-setting-target="developer-setup"><div><div class="label">Force setup on launch</div><div class="desc">Show onboarding without erasing settings.</div></div><Toggle checked={forceSetup} onchange={setForceSetup} label="Force setup" /></div>
    <div class="setting-row" data-setting-target="developer-sync"><div><div class="label">LAN device sync</div><div class="desc">Experimental. Off by default.</div></div><Toggle checked={sync} onchange={setSync} label="Enable LAN sync" /></div>
  </section>

  <section data-setting-target="developer-simulations">
    <header class="dx-label"><span>Simulate</span><b>reversible</b></header>
    <div class="row">
      <Dropdown bind:open={providerOpen} closeSelector=".fault-provider-dropdown">
        <div class="ui-dropdown fault-provider-dropdown">
          <button class="ui-dropdown-trigger ui-dropdown-trigger--compact" aria-haspopup="true" aria-expanded={providerOpen} aria-label="Simulation provider" onclick={() => (providerOpen = !providerOpen)}>
            <span>{PROVIDERS.find((p) => p.value === provider)?.label}</span>
            <svg class:open={providerOpen} width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6" /></svg>
          </button>
          {#if providerOpen}
            <div class="ui-dropdown-menu ui-dropdown-menu--padded" role="listbox" aria-label="Simulation provider options" in:fly={{ y: -motionPx(MOTION_PX.nudge), duration: motionMs(MOTION_MS.panel), easing: expoOut }} out:fade={{ duration: motionMs(MOTION_MS.fast) }}>
              {#each PROVIDERS as opt (opt.value)}<button class="ui-dropdown-option" class:active={provider === opt.value} onclick={() => { provider = opt.value; providerOpen = false; }}>{opt.label}</button>{/each}
            </div>
          {/if}
        </div>
      </Dropdown>
      <span class="storage" data-setting-target="developer-storage-simulation"><span>Storage writes fail</span><Toggle checked={storageFull} onchange={toggleStorage} label="Storage writes fail" /></span>
      <span class="spacer"></span>
      <button class="btn-danger btn-compact" onclick={() => void clearFaults()}>Clear all</button>
    </div>
    <div class="grid">
      <button class="btn-ghost btn-compact" onclick={providerDown}>Provider down</button>
      <button class="btn-ghost btn-compact" onclick={offline}>Offline</button>
      <button class="btn-ghost btn-compact" onclick={globalMessage}>Global message</button>
      <button class="btn-ghost btn-compact" onclick={driveFull}>Drive full</button>
      <button class="btn-ghost btn-compact" data-setting-target="developer-notifications" onclick={() => void notify()}>Notification</button>
      <button class="btn-ghost btn-compact" data-setting-target="developer-installer" onclick={() => void installer()}>Reinstall stable</button>
      <button class="btn-ghost btn-compact" data-setting-target="developer-status" onclick={() => void providerStatus()}>Check providers</button>
    </div>
    {#if message}<p class="msg" transition:fade={{ duration: motionMs(MOTION_MS.fast) }}>{message}</p>{/if}
    {#if statusRaw}<pre class="out">{statusRaw}</pre>{/if}
  </section>
</div>

{#if privacyOpen}
  <button class="modal-backdrop" aria-label="Close dialog" onclick={() => (privacyOpen = false)} in:modalBackdrop={{ duration: 180 }} out:modalBackdrop={{ duration: 160 }}></button>
  <div class="modal-card" use:modalFocusTrap={{ active: privacyOpen, initialFocus: () => privacyClose }} role="dialog" aria-modal="true" aria-labelledby="privacy-warning-title" tabindex="-1" in:modalCard={{ duration: 220, distance: motionPx(MOTION_PX.panel), scaleFrom: 0.97 }} out:modalCard={{ duration: 160, distance: motionPx(MOTION_PX.nudge), scaleFrom: 0.985 }}>
    <div class="modal-header"><h2 id="privacy-warning-title" class="modal-title">Verbose logging privacy</h2></div>
    <div class="modal-body"><p class="confirm-copy">Verbose logs can contain dictated text and provider prompts. Keep it off unless you are investigating a local issue.</p></div>
    <div class="modal-footer"><div class="footer-actions"><button bind:this={privacyClose} class="btn-primary" onclick={() => (privacyOpen = false)}>Got it</button></div></div>
  </div>
{/if}

<style>
  .dev-settings { display:flex; flex-direction:column; gap:32px; }
  .link { background:none; border:0; padding:0; font:inherit; color:var(--ink-soft); text-decoration:underline; cursor:pointer; }
  .link:hover { color:var(--ink); }
  .grid { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:10px; }
  .row { display:flex; align-items:center; gap:16px; margin-bottom:14px; }
  .spacer { flex:1; }
  .storage { display:inline-flex; align-items:center; gap:10px; font-size:12px; color:var(--ink-soft); }
  .msg { margin:14px 0 0; font-size:12px; color:var(--ink-mute); }
  .out { max-height:160px; overflow:auto; margin:14px 0 0; padding:10px; color:var(--ink-mute); font:10.5px var(--mono); white-space:pre-wrap; background:var(--bg-elev); border:1px solid var(--line); border-radius:var(--r-sm); }

  .modal-backdrop { position:fixed; inset:0; border:0; padding:0; appearance:none; background:var(--overlay); z-index:50; outline:none; }
  .modal-card { position:fixed; top:50%; left:50%; translate:-50% -50%; z-index:51; isolation:isolate; background:var(--bg-elev); border:1px solid var(--line); border-radius:var(--r-lg); width:min(420px, calc(100vw - 40px)); box-shadow:var(--shadow-elev); overflow:hidden; }
  .modal-header { padding:20px 20px 0; }
  .modal-title { font-family:var(--sans); font-size:17px; font-weight:600; letter-spacing:-0.01em; color:var(--ink); margin:0; }
  .modal-body { padding:10px 20px 18px; }
  .confirm-copy { margin:0; font-size:13px; line-height:1.5; color:var(--ink-soft); }
  .modal-footer { padding:0 20px 20px; }
  .footer-actions { display:flex; justify-content:flex-end; gap:8px; }

  @media (max-width: 760px) {
    .grid { grid-template-columns:repeat(2,minmax(0,1fr)); }
    .fault-provider-dropdown :global(.ui-dropdown-menu) { left:0; right:auto; }
  }
</style>
