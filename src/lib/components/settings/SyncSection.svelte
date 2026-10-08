<script lang="ts">
  import { formatSyncError as formatIpcError } from '../../errors';
  import { invoke } from '../../tauri';
  import Toggle from '../Toggle.svelte';
  import { SyncMutingSetting } from '../../syncMutingSetting.svelte';
  import {
    syncStore,
    refreshSyncStatus,
    thisDeviceName,
    type DiscoveredDevice,
    type PairedDevice,
  } from '../../syncStore.svelte';
  import { modalFocusTrap } from '../../modalFocus';
  import {
    modalBackdrop,
    modalCard,
    listItemCollapse,
    motionMs,
    motionPx,
    MOTION_MS,
    MOTION_PX,
  } from '../../motion';
  import { fade, fly, scale, slide } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import { cubicOut } from 'svelte/easing';
  import { onMount, tick, untrack } from 'svelte';
  import { portal } from '../../portal';
  import { icons } from '../../icons';

  // Local device name editing.
  let deviceName = $state('');
  let nameSaved = $state('');
  let nameBusy = $state(false);

  const syncMuting = new SyncMutingSetting();

  // Per-row action state.
  let pairingUuid = $state('');
  let syncingUuid = $state('');
  let removingUuid = $state('');
  let statusMsg = $state('');
  let statusKind = $state<'' | 'ok' | 'err'>('');
  let statusTimer: ReturnType<typeof setTimeout> | undefined;

  // Tailscale setup.
  let setupOpen = $state(false);
  let tipsOpen = $state(false);
  let tailscaleIp = $state('');
  let editAddress = $state(false);
  let connectionDetails = $state('');
  let connectionBusy = $state(false);
  let connectionError = $state('');
  let copiedKey = $state<'' | 'setup' | 'nudge'>('');
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;
  let manualCopy = $state(false);
  let detailsCopied = $state(false);

  // Per-device route editor and return-route reminder.
  let editingConnection = $state<PairedDevice | null>(null);
  let peerAddress = $state('');
  let routeBusy = $state(false);
  let routeError = $state('');
  let freshPeers = $state<Record<string, true>>({});
  let knownPeers: Set<string> | null = null;

  let now = $state(Date.now());

  const detectedIp = $derived(syncStore.status?.this_device.tailscale_ips?.[0] ?? '');
  const ownIp = $derived(tailscaleIp.trim() || detectedIp);
  const ownPort = $derived(syncStore.status?.this_device.port);
  const ownDetails = $derived(ownIp && ownPort && syncStore.status
    ? `verenu-sync://${syncStore.status.this_device.uuid}@${ownIp}:${ownPort}` : '');

  let confirmRemove = $state<PairedDevice | null>(null);
  let cancelRemoveButton = $state<HTMLButtonElement | null>(null);

  const discovered = $derived(syncStore.status?.discovered ?? []);
  const peers = $derived(syncStore.status?.peers ?? []);
  const unpairedNearby = $derived(discovered.filter((d) => !d.paired));
  const outgoing = $derived(
    syncStore.status?.pairing?.kind === 'outgoing' && syncStore.status.pairing.phase !== 'failed'
      ? syncStore.status.pairing
      : null,
  );
  const pairingError = $derived(
    syncStore.status?.pairing?.phase === 'failed'
      ? formatIpcError(syncStore.status.pairing.error, 'Could not pair these devices')
      : '',
  );
  const listenerActive = $derived(syncStore.status?.listener_active ?? false);
  const listenerHint = $derived(
    syncStore.status && !listenerActive
      ? formatIpcError(
          syncStore.status.last_error_hint ??
            'Sync is unavailable on this device. Restart Verenu and check that your system keyring is unlocked.',
        )
      : '',
  );
  const pairPhase = $derived(
    outgoing?.phase === 'verifying' ? 2 : outgoing?.phase === 'connecting' ? 0 : 1,
  );
  const codeDigits = $derived((outgoing?.code ?? '').split('').slice(0, 6));

  const durBase = () => motionMs(MOTION_MS.base);
  const durFast = () => motionMs(MOTION_MS.fast);

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    if (outgoing) {
      event.preventDefault();
      cancelOutgoing();
    } else if (confirmRemove) {
      event.preventDefault();
      confirmRemove = null;
    } else if (
      editingConnection &&
      event.target instanceof Element &&
      event.target.closest('.connection-editor')
    ) {
      event.preventDefault();
      closeEditor();
    }
  }

  onMount(() => {
    void refreshSyncStatus().then(() => {
      deviceName = thisDeviceName();
      nameSaved = deviceName;
    });
    void syncMuting.load();
    const clock = setInterval(() => (now = Date.now()), 30_000);
    return () => {
      clearInterval(clock);
      clearTimeout(statusTimer);
      clearTimeout(copiedTimer);
    };
  });

  // A device that appears in the paired list is the result of a finished
  // pairing. Announce it, and remind the user to give the other device a way
  // back when this side reaches it through a saved Tailscale connection.
  $effect(() => {
    if (!syncStore.status) return;
    const current = peers;
    untrack(() => {
      const ids = new Set(current.map((p) => p.uuid));
      if (knownPeers === null) {
        knownPeers = ids;
        return;
      }
      for (const device of current) {
        if (!knownPeers.has(device.uuid)) {
          freshPeers = { ...freshPeers, [device.uuid]: true };
          flash(`Paired with ${device.name}.`, 'ok');
        }
      }
      for (const id of knownPeers) if (!ids.has(id)) knownPeers.delete(id);
      for (const id of ids) knownPeers.add(id);
    });
  });

  function flash(message: string, kind: 'ok' | 'err'): void {
    clearTimeout(statusTimer);
    statusMsg = message;
    statusKind = kind;
    statusTimer = setTimeout(() => {
      statusMsg = '';
      statusKind = '';
    }, 4500);
  }

  function dismissFlash(): void {
    clearTimeout(statusTimer);
    statusMsg = '';
    statusKind = '';
  }

  async function saveName(): Promise<void> {
    const name = deviceName.trim();
    if (!name) {
      deviceName = nameSaved;
      return;
    }
    if (name === nameSaved || nameBusy) return;
    nameBusy = true;
    try {
      await invoke('sync_set_device_name', { name });
      nameSaved = name;
      await refreshSyncStatus();
    } catch (err) {
      deviceName = nameSaved;
      flash(formatIpcError(err, 'Could not save this device name'), 'err');
    } finally {
      nameBusy = false;
    }
  }

  async function startPairing(device: DiscoveredDevice): Promise<void> {
    if (pairingUuid) return;
    pairingUuid = device.uuid;
    try {
      await invoke<string>('sync_start_pairing', { deviceUuid: device.uuid });
      await refreshSyncStatus();
    } catch (err) {
      flash(formatIpcError(err, 'Could not start pairing'), 'err');
    } finally {
      pairingUuid = '';
    }
  }

  function cancelOutgoing(): void {
    void invoke('sync_cancel_pairing').catch(() => {});
    void refreshSyncStatus();
  }

  async function pairConnection(): Promise<void> {
    connectionBusy = true;
    connectionError = '';
    try {
      await invoke('sync_pair_connection', { details: connectionDetails });
      connectionDetails = '';
      await refreshSyncStatus();
    } catch (err) {
      connectionError = formatIpcError(err, 'Could not pair this connection');
    } finally {
      connectionBusy = false;
    }
  }

  async function copyDetails(source: 'setup' | 'nudge'): Promise<void> {
    if (!ownDetails) return;
    try {
      await navigator.clipboard.writeText(ownDetails);
      manualCopy = false;
      copiedKey = source;
      detailsCopied = true;
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => (copiedKey = ''), 2200);
    } catch {
      manualCopy = true;
      connectionError = 'Could not copy automatically. Select the details below and copy them yourself.';
    }
  }

  function openEditor(device: PairedDevice): void {
    if (editingConnection?.uuid === device.uuid) {
      closeEditor();
      return;
    }
    editingConnection = device;
    peerAddress = device.connection_address ?? '';
    routeError = '';
  }

  function closeEditor(): void {
    const uuid = editingConnection?.uuid;
    editingConnection = null;
    if (!uuid) return;
    void tick().then(() => {
      document.querySelector<HTMLElement>(`[data-connection-button="${CSS.escape(uuid)}"]`)?.focus();
    });
  }

  async function saveConnection(): Promise<void> {
    if (!editingConnection) return;
    routeBusy = true;
    routeError = '';
    try {
      await invoke('sync_set_peer_address', { deviceUuid: editingConnection.uuid, address: peerAddress });
      closeEditor();
      await refreshSyncStatus();
      flash('Connection saved. Verenu will try syncing automatically.', 'ok');
    } catch (err) {
      routeError = formatIpcError(err, 'Could not save this connection');
    } finally {
      routeBusy = false;
    }
  }

  function dismissNudge(uuid: string): void {
    const { [uuid]: _removed, ...rest } = freshPeers;
    freshPeers = rest;
  }

  async function syncNow(device: PairedDevice): Promise<void> {
    if (syncingUuid) return;
    syncingUuid = device.uuid;
    try {
      await invoke('sync_now', { deviceUuid: device.uuid });
      flash(`Syncing with ${device.name}…`, 'ok');
    } catch (err) {
      flash(formatIpcError(err, 'Could not sync with this device'), 'err');
    } finally {
      setTimeout(() => {
        syncingUuid = '';
        void refreshSyncStatus();
      }, 800);
    }
  }

  function askRemove(device: PairedDevice): void {
    confirmRemove = device;
  }

  async function removeDevice(): Promise<void> {
    if (!confirmRemove) return;
    removingUuid = confirmRemove.uuid;
    const name = confirmRemove.name;
    try {
      await invoke('sync_remove_device', { deviceUuid: confirmRemove.uuid });
      if (editingConnection?.uuid === confirmRemove.uuid) editingConnection = null;
      confirmRemove = null;
      flash(`${name} removed. It can no longer sync with this device.`, 'ok');
      await refreshSyncStatus();
    } catch (err) {
      flash(formatIpcError(err, 'Could not remove this paired device'), 'err');
    } finally {
      removingUuid = '';
    }
  }

  function stateLabel(state: string): string {
    switch (state) {
      case 'synced':
        return 'Up to date';
      case 'syncing':
        return 'Syncing';
      case 'connecting':
        return 'Connecting';
      case 'error':
        return 'Sync failed';
      default:
        return 'Offline';
    }
  }

  function routeLabel(device: PairedDevice): string {
    if (device.connection_address) return 'Tailscale';
    return device.online ? 'Same network' : '';
  }

  function relativeTime(iso: string | null, reference: number): string {
    if (!iso) return 'never';
    const withT = iso.includes('T') ? iso : iso.replace(' ', 'T');
    const normalized = /(?:Z|[+-]\d{2}:?\d{2})$/.test(withT) ? withT : `${withT}Z`;
    const then = new Date(normalized).getTime();
    if (Number.isNaN(then)) return iso;
    const seconds = Math.max(0, Math.round((reference - then) / 1000));
    if (seconds < 45) return 'just now';
    if (seconds < 90) return 'a minute ago';
    if (seconds < 3600) return `${Math.round(seconds / 60)} min ago`;
    if (seconds < 7200) return 'an hour ago';
    if (seconds < 86400) return `${Math.round(seconds / 3600)} hours ago`;
    const days = Math.round(seconds / 86400);
    return days === 1 ? 'yesterday' : `${days} days ago`;
  }

  function spokenCode(code: string): string {
    return code.split('').join(' ');
  }

  function focusOnMount(node: HTMLInputElement) {
    node.focus();
    node.select();
  }
</script>

{#snippet chevron()}
  <svg class="chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="m6 9 6 6 6-6" />
  </svg>
{/snippet}

{#snippet copyGlyph(done: boolean)}
  {#key done}
    <svg
      class="copy-glyph"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
      in:scale={{ start: 0.5, duration: durFast(), easing: cubicOut }}
    >
      {@html done ? icons.check : icons.copy}
    </svg>
  {/key}
{/snippet}

<svelte:window onkeydown={handleKeydown} />

<h2 class="settings-h">Sync</h2>

<!-- This device -->
<div class="id-card" data-setting-target="sync-this-device">
  <div class="tile" aria-hidden="true">
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      {@html icons.devices}
    </svg>
  </div>
  <div class="id-main">
    <div class="id-name-wrap">
      <input
        class="id-name"
        bind:value={deviceName}
        maxlength={60}
        spellcheck="false"
        aria-label="This device's sync name"
        disabled={nameBusy}
        onkeydown={(e) => {
          if (e.key === 'Enter') void saveName();
        }}
        onblur={() => void saveName()}
      />
      <svg class="id-pencil" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        {@html icons.pencil}
      </svg>
    </div>
    <div class="id-sub">
      <span class="live-dot" class:off={syncStore.loaded && !listenerActive} aria-hidden="true"></span>
      {#if !syncStore.loaded}
        Checking…
      {:else if listenerActive}
        Ready to sync
        {#if ownIp}<span class="id-ip">{ownIp}</span>{/if}
      {:else}
        Sync unavailable right now
      {/if}
    </div>
  </div>
</div>

<div class="notices" aria-live="polite">
  {#if statusMsg}
    <div
      class="notice"
      class:ok={statusKind === 'ok'}
      class:err={statusKind === 'err'}
      role={statusKind === 'err' ? 'alert' : 'status'}
      transition:slide={{ duration: durBase(), easing: cubicOut }}
    >
      <span class="notice-inner">
        <svg class="notice-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          {#if statusKind === 'ok'}{@html icons.check}{:else}<circle cx="12" cy="12" r="9" /><path d="M12 8v5M12 16.5v.01" />{/if}
        </svg>
        <span class="notice-text">{statusMsg}</span>
        <button class="notice-x" onclick={dismissFlash} aria-label="Dismiss message">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg>
        </button>
      </span>
    </div>
  {/if}

  {#if pairingError}
    <div class="notice err" role="alert" transition:slide={{ duration: durBase(), easing: cubicOut }}>
      <span class="notice-inner">
        <svg class="notice-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="12" cy="12" r="9" /><path d="M12 8v5M12 16.5v.01" />
        </svg>
        <span class="notice-text">{pairingError}</span>
        <button class="btn-ghost btn-compact" onclick={cancelOutgoing}>Dismiss</button>
      </span>
    </div>
  {/if}

  {#if listenerHint}
    <div class="notice err" role="alert" transition:slide={{ duration: durBase(), easing: cubicOut }}>
      <span class="notice-inner">
        <svg class="notice-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <circle cx="12" cy="12" r="9" /><path d="M12 8v5M12 16.5v.01" />
        </svg>
        <span class="notice-text">{listenerHint}</span>
      </span>
    </div>
  {/if}
</div>

<h3 class="settings-subhead">Muting</h3>
<div class="setting-row" data-setting-target="sync-muting">
  <div>
    <div class="label">Synchronous muting</div>
    <div class="desc">
      When a paired device records, mute this device. When you record, mute paired devices with this on. Works over LAN or saved Tailscale connections. Your own dictation mute setting stays separate.
    </div>
    {#if syncMuting.loadFailed}
      <div class="field-error sync-muting-error" role="alert">
        <span>Could not load this setting.</span>
        <button type="button" class="btn-ghost btn-compact" onclick={() => syncMuting.load()}>Retry</button>
      </div>
    {:else if syncMuting.saveFailed}
      <div class="field-error" role="alert">Could not save this setting. Try again.</div>
    {/if}
  </div>
  <Toggle
    checked={syncMuting.enabled}
    onchange={(value) => syncMuting.setEnabled(value)}
    disabled={syncMuting.disabled}
    label="Synchronous muting"
    bind:error={syncMuting.flashError}
  />
</div>

<!-- Paired devices -->
<h3 class="settings-subhead" data-setting-target="sync-paired">Paired devices</h3>
{#if peers.length === 0}
  <div class="empty-note" in:fade={{ duration: durFast() }}>
    <div class="discover-title">Nothing paired yet</div>
    <div class="discover-hint">Pick a nearby device below, or connect through Tailscale. Paired devices stay connected until either side removes them.</div>
  </div>
{:else}
  <div class="device-list">
    {#each peers as device (device.uuid)}
      <div
        class="device"
        class:is-error={device.state === 'error'}
        class:is-open={editingConnection?.uuid === device.uuid}
        animate:flip={{ duration: durBase() }}
        in:fly={{ y: motionPx(MOTION_PX.nudge), duration: durBase(), easing: cubicOut }}
        out:listItemCollapse
      >
        <div class="device-row">
          <div class="tile tile-dim" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              {@html icons.devices}
            </svg>
          </div>
          <div class="device-main">
            <div class="device-top">
              <span class="device-name">{device.name}</span>
              {#key device.state}
                <span class="pill {device.state}" in:fade={{ duration: durFast() }}>
                  <span class="pill-dot" aria-hidden="true"></span>{stateLabel(device.state)}
                </span>
              {/key}
            </div>
            <div class="desc device-meta">
              Last synced {relativeTime(device.last_sync_at, now)}
              {#if routeLabel(device)}
                <span class="id-sep" aria-hidden="true">·</span>{routeLabel(device)}
              {/if}
            </div>
            {#if device.error && device.state === 'error'}
              <div class="desc device-error" transition:slide={{ duration: durBase(), easing: cubicOut }}>
                {formatIpcError(device.error, 'Sync did not finish')}
              </div>
            {/if}
          </div>
          <div class="device-actions">
            <button
              class="btn-ghost btn-compact sync-btn"
              onclick={() => void syncNow(device)}
              disabled={syncingUuid !== '' || device.state === 'syncing'}
            >
              <svg
                class="sync-glyph"
                class:spin={syncingUuid === device.uuid || device.state === 'syncing'}
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                {@html icons.refresh}
              </svg>
              {syncingUuid === device.uuid || device.state === 'syncing' ? 'Syncing…' : 'Sync now'}
            </button>
            <button
              class="btn-ghost btn-compact"
              data-connection-button={device.uuid}
              aria-expanded={editingConnection?.uuid === device.uuid}
              aria-controls="sync-route-{device.uuid}"
              onclick={() => openEditor(device)}
            >Connection</button>
            <button
              class="btn-ghost btn-compact danger-ghost"
              onclick={() => askRemove(device)}
              disabled={removingUuid !== ''}
            >
              Remove
            </button>
          </div>
        </div>

        {#if editingConnection?.uuid === device.uuid}
          <div id="sync-route-{device.uuid}" class="drawer" transition:slide={{ duration: durBase(), easing: cubicOut }}>
            <form
              class="connection-editor"
              onsubmit={(event) => { event.preventDefault(); void saveConnection(); }}
            >
              <label for="sync-peer-address">How to reach {device.name}</label>
              <div class="field">
                <input
                  id="sync-peer-address"
                  class="ui-input field-input"
                  bind:value={peerAddress}
                  placeholder="Paste connection details or 100.x.x.x:port"
                  spellcheck="false"
                  autocomplete="off"
                  disabled={routeBusy}
                  use:focusOnMount
                />
                <div class="field-actions">
                  <button class="btn-primary btn-compact" type="submit" disabled={routeBusy}>{routeBusy ? 'Saving…' : 'Save'}</button>
                  <button class="btn-ghost btn-compact" type="button" onclick={closeEditor} disabled={routeBusy}>Cancel</button>
                </div>
              </div>
              <div class="desc">On {device.name}, open Sync and copy its connection details. Leave this empty to use nearby discovery only.</div>
              {#if routeError}
                <div class="field-error" role="alert" transition:slide={{ duration: durFast(), easing: cubicOut }}>{routeError}</div>
              {/if}
            </form>
          </div>
        {/if}

        {#if freshPeers[device.uuid] && device.connection_address && ownDetails && editingConnection?.uuid !== device.uuid}
          <div class="drawer" transition:slide={{ duration: durBase(), easing: cubicOut }}>
            <div class="nudge">
              <div class="nudge-text">
                <div class="nudge-title">One more step so {device.name} can reach you</div>
                <div class="desc">On {device.name}, open Sync, press Connection on {nameSaved || 'this device'}, and paste this device's details.</div>
              </div>
              <div class="nudge-actions">
                <button class="btn-ghost btn-compact copy-action" class:copied={copiedKey === 'nudge'} onclick={() => void copyDetails('nudge')}>
                  {@render copyGlyph(copiedKey === 'nudge')}
                  {copiedKey === 'nudge' ? 'Copied' : 'Copy details'}
                </button>
                <button class="btn-ghost btn-compact" onclick={() => dismissNudge(device.uuid)}>Done</button>
              </div>
            </div>
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<!-- Nearby devices -->
<h3 class="settings-subhead" data-setting-target="sync-nearby">Nearby devices</h3>
{#if !syncStore.loaded}
  <div class="discover-card" role="status" in:fade={{ duration: durFast() }}>
    <span class="search-dots" aria-hidden="true"><i></i><i></i><i></i></span>
    Searching your network for other devices…
  </div>
{:else if unpairedNearby.length === 0}
  <div class="discover-card discover-empty" in:fade={{ duration: durBase() }}>
    <span class="radar" aria-hidden="true">
      <i></i><i></i>
      <svg class="discover-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
        {@html icons.devices}
      </svg>
    </span>
    <div class="discover-title">No devices found yet</div>
    <div class="discover-hint">
      Open Verenu on your other device and join the same Wi-Fi or wired network. It shows up here on
      its own. On different networks, use Tailscale below.
    </div>
  </div>
{:else}
  <div class="device-list">
    {#each unpairedNearby as device (device.uuid)}
      <div
        class="device"
        animate:flip={{ duration: durBase() }}
        in:fly={{ y: motionPx(MOTION_PX.nudge), duration: durBase(), easing: cubicOut }}
        out:listItemCollapse
      >
        <div class="device-row">
          <div class="tile tile-dim" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              {@html icons.devices}
            </svg>
          </div>
          <div class="device-main">
            <div class="device-top">
              <span class="device-name">{device.name}</span>
            </div>
            <div class="desc">Ready to pair. You'll confirm with a short code.</div>
          </div>
          <div class="device-actions">
            <button
              class="btn-primary btn-compact"
              onclick={() => void startPairing(device)}
              disabled={pairingUuid !== '' || !!outgoing}
            >
              {pairingUuid === device.uuid ? 'Starting…' : 'Pair'}
            </button>
          </div>
        </div>
      </div>
    {/each}
  </div>
{/if}

<!-- Tailscale setup -->
<section class="setup" class:is-open={setupOpen} data-setting-target="sync-tailscale">
  <button
    class="setup-toggle ui-focus-ring"
    aria-expanded={setupOpen}
    aria-controls="sync-setup-panel"
    onclick={() => (setupOpen = !setupOpen)}
  >
    <span class="tile tile-dim" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
        <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
      </svg>
    </span>
    <span class="setup-text">
      <span class="setup-title">Connect through Tailscale</span>
      <span class="setup-sub">For devices on different networks.</span>
    </span>
    {@render chevron()}
  </button>

  {#if setupOpen}
    <div id="sync-setup-panel" class="drawer" transition:slide={{ duration: motionMs(MOTION_MS.panel), easing: cubicOut }}>
      <p class="setup-intro">Both devices need Tailscale on the same tailnet, with Verenu open.</p>
      <ol class="steps">
        <li class="step" class:done={detailsCopied}>
          <span class="marker" aria-hidden="true">
            <span class="num">1</span>
            <svg class="tick" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">{@html icons.check}</svg>
          </span>
          <div class="step-body">
            <div class="step-title">Copy this device's details</div>
            <div class="desc">Do the same on the other device.</div>
            {#if detectedIp && !editAddress}
              <div class="field">
                <div class="addr" aria-label="This device's Tailscale address">
                  <span class="addr-ip">{ownIp}</span>{#if ownPort}<span class="addr-port">:{ownPort}</span>{/if}
                </div>
                <div class="field-actions">
                  <button
                    class="btn-primary btn-compact copy-action"
                    class:copied={copiedKey === 'setup'}
                    onclick={() => void copyDetails('setup')}
                    disabled={!listenerActive || !ownDetails}
                  >
                    {@render copyGlyph(copiedKey === 'setup')}
                    {copiedKey === 'setup' ? 'Copied' : 'Copy details'}
                  </button>
                  <button class="btn-ghost btn-compact" onclick={() => (editAddress = true)}>Change</button>
                </div>
              </div>
            {:else}
              <label class="field-label" for="sync-own-ip">This device's Tailscale IPv4 address</label>
              <div class="field">
                <input
                  id="sync-own-ip"
                  class="ui-input field-input"
                  bind:value={tailscaleIp}
                  placeholder={detectedIp || '100.x.x.x'}
                  spellcheck="false"
                  autocomplete="off"
                />
                <div class="field-actions">
                  <button
                    class="btn-primary btn-compact copy-action"
                    class:copied={copiedKey === 'setup'}
                    onclick={() => void copyDetails('setup')}
                    disabled={!listenerActive || !ownDetails}
                  >
                    {@render copyGlyph(copiedKey === 'setup')}
                    {copiedKey === 'setup' ? 'Copied' : 'Copy details'}
                  </button>
                  {#if detectedIp}
                    <button class="btn-ghost btn-compact" onclick={() => { editAddress = false; tailscaleIp = ''; }}>Use detected</button>
                  {/if}
                </div>
              </div>
              {#if !detectedIp}
                <div class="desc" transition:slide={{ duration: durFast(), easing: cubicOut }}>Verenu could not detect the address. Copy it from the Tailscale app.</div>
              {/if}
            {/if}
            {#if manualCopy && ownDetails}
              <input class="ui-input field-input manual" value={ownDetails} readonly spellcheck="false" aria-label="This device's connection details" onfocus={(e) => e.currentTarget.select()} transition:slide={{ duration: durFast(), easing: cubicOut }} />
            {/if}
          </div>
        </li>

        <li class="step" class:done={connectionDetails.trim() !== ''}>
          <span class="marker" aria-hidden="true">
            <span class="num">2</span>
            <svg class="tick" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">{@html icons.check}</svg>
          </span>
          <div class="step-body">
            <label class="step-title" for="sync-other-details">Paste the other device's details</label>
            <div class="field">
              <input
                id="sync-other-details"
                class="ui-input field-input"
                bind:value={connectionDetails}
                oninput={() => (connectionError = '')}
                onkeydown={(e) => {
                  if (e.key === 'Enter' && connectionDetails.trim() && !connectionBusy && !outgoing && listenerActive) void pairConnection();
                }}
                placeholder="Paste from the other device"
                spellcheck="false"
                autocomplete="off"
                disabled={connectionBusy}
              />
              <div class="field-actions">
                <button
                  class="btn-primary btn-compact"
                  onclick={() => void pairConnection()}
                  disabled={connectionBusy || !!outgoing || !connectionDetails.trim() || !listenerActive}
                >{connectionBusy ? 'Connecting…' : 'Pair'}</button>
              </div>
            </div>
            {#if connectionError}
              <div class="field-error" role="alert" transition:slide={{ duration: durFast(), easing: cubicOut }}>{connectionError}</div>
            {/if}
          </div>
        </li>

        <li class="step" class:done={!!outgoing}>
          <span class="marker" aria-hidden="true">
            <span class="num">3</span>
            <svg class="tick" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">{@html icons.check}</svg>
          </span>
          <div class="step-body">
            <div class="step-title">Enter the code on the other device</div>
            <div class="desc">A short code appears here. Type it there to finish pairing.</div>
          </div>
        </li>
      </ol>

      <button
        class="tips-toggle ui-focus-ring"
        aria-expanded={tipsOpen}
        aria-controls="sync-tips"
        onclick={() => (tipsOpen = !tipsOpen)}
      >
        Tips and troubleshooting
        {@render chevron()}
      </button>
      {#if tipsOpen}
        <ul id="sync-tips" class="tips" transition:slide={{ duration: durBase(), easing: cubicOut }}>
          <li>With three or more devices, pair each new one with a device you already have. Changes relay between them.</li>
          <li>Keep Verenu and Tailscale running. Android can pause Verenu in the background, so keep it open while syncing.</li>
          <li>If it fails to connect, allow TCP port {ownPort ?? 'shown above'} through your firewall and Tailscale access rules.</li>
        </ul>
      {/if}
    </div>
  {/if}
</section>

{#if outgoing}
  <div class="dialog-layer" use:portal>
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <button
    class="modal-backdrop sync-backdrop"
    aria-label="Cancel pairing"
    onclick={cancelOutgoing}
    in:modalBackdrop={{ duration: 180 }}
    out:modalBackdrop={{ duration: 160 }}
  ></button>
  <div
    class="modal-card sync-dialog outgoing-card"
    role="dialog"
    aria-modal="true"
    aria-label="Pairing with {outgoing.peer_name}"
    use:modalFocusTrap={{ active: !!outgoing, initialFocus: () => null }}
    in:modalCard={{ duration: motionMs(MOTION_MS.panel) }}
    out:modalCard={{ duration: motionMs(MOTION_MS.fast) }}
  >
    <div class="pair-head">
      <div class="tile" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          {@html icons.devices}
        </svg>
      </div>
      <div>
        <div class="pair-title">Pair with {outgoing.peer_name}</div>
        <div class="pair-sub">Type this code on {outgoing.peer_name} to confirm.</div>
      </div>
    </div>

    <div class="outgoing-code" role="img" aria-label={codeDigits.length ? `Pairing code ${spokenCode(codeDigits.join(''))}` : 'Pairing code loading'}>
      {#if codeDigits.length}
        {#key outgoing.code}
          {#each codeDigits as digit, index (index)}
            <span
              class="digit"
              in:fly={{ y: motionPx(MOTION_PX.lift), delay: index * 35, duration: durBase(), easing: cubicOut }}
            >{digit}</span>
          {/each}
        {/key}
      {:else}
        {#each [0, 1, 2, 3, 4, 5] as index (index)}
          <span class="digit placeholder" style="animation-delay: {index * 90}ms"></span>
        {/each}
      {/if}
    </div>

    <div class="phase-track" aria-hidden="true">
      {#each [0, 1, 2] as index (index)}
        <span class="phase-seg" class:on={pairPhase >= index}></span>
      {/each}
    </div>
    {#key pairPhase}
      <div class="pair-wait" role="status" in:fade={{ duration: durFast() }}>
        {#if pairPhase === 2}
          <span class="search-dots" aria-hidden="true"><i></i><i></i><i></i></span>
          Checking the code…
        {:else if pairPhase === 0}
          <span class="search-dots" aria-hidden="true"><i></i><i></i><i></i></span>
          Contacting {outgoing.peer_name}…
        {:else}
          <span class="search-dots" aria-hidden="true"><i></i><i></i><i></i></span>
          Waiting for {outgoing.peer_name}. The code expires in a few minutes.
        {/if}
      </div>
    {/key}
    <div class="pair-actions">
      <button class="btn-ghost btn-compact" onclick={cancelOutgoing}>Cancel</button>
    </div>
  </div>
  </div>
{/if}

{#if confirmRemove}
  <div class="dialog-layer" use:portal>
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <button
    class="modal-backdrop sync-backdrop"
    aria-label="Close dialog"
    onclick={() => (confirmRemove = null)}
    in:modalBackdrop={{ duration: 180 }}
    out:modalBackdrop={{ duration: 160 }}
  ></button>
  <div
    class="modal-card sync-dialog remove-card"
    role="dialog"
    aria-modal="true"
    aria-label="Remove {confirmRemove.name}"
    use:modalFocusTrap={{
      active: !!confirmRemove,
      initialFocus: () => cancelRemoveButton,
    }}
    in:modalCard={{ duration: motionMs(MOTION_MS.panel) }}
    out:modalCard={{ duration: motionMs(MOTION_MS.fast) }}
  >
    <div class="pair-title">Remove {confirmRemove.name}?</div>
    <div class="pair-sub">
      It will immediately lose access to sync with this device. To sync again you'd have to pair
      both devices again.
    </div>
    <div class="pair-actions">
      <button
        bind:this={cancelRemoveButton}
        class="btn-ghost btn-compact"
        onclick={() => (confirmRemove = null)}
      >
        Cancel
      </button>
      <button
        class="btn-danger btn-compact"
        onclick={() => void removeDevice()}
        disabled={removingUuid !== ''}
      >
        {removingUuid ? 'Removing…' : 'Remove device'}
      </button>
    </div>
  </div>
  </div>
{/if}

<div class="panel-note sync-note" in:fade={{ duration: motionMs(MOTION_MS.fast) }}>
  <svg
    class="note-icon"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    {@html icons.lock}
  </svg>
  Sync runs between paired devices over your local network or a saved Tailscale connection,
  encrypted end to end. Provider and model choices, themes, API keys,
  microphone settings, and hotkeys stay on each device.
</div>

<style>
  /* Shared device glyph tile */
  .tile {
    width: 38px;
    height: 38px;
    border-radius: var(--r-md);
    background: var(--accent-soft);
    color: var(--accent-ink);
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .tile svg {
    width: 20px;
    height: 20px;
  }
  .tile-dim {
    background: var(--control-hover);
    color: var(--ink-mute);
  }

  /* This device identity card */
  .id-card {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    background: var(--bg-elev);
  }
  .id-main {
    min-width: 0;
    flex: 1;
  }
  .id-name-wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
    max-width: 100%;
  }
  .id-name {
    font-family: var(--sans);
    font-size: 16px;
    font-weight: 600;
    color: var(--ink-strong);
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--r-sm);
    padding: 2px 26px 2px 8px;
    margin-left: -8px;
    min-width: 120px;
    max-width: 100%;
    transition:
      border-color var(--ui-duration-fast) var(--ui-ease-out),
      background-color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .id-name:hover {
    border-color: var(--line-strong);
  }
  .id-name:focus-visible {
    outline: none;
    border-color: var(--accent);
    background: var(--control-active);
  }
  .id-pencil {
    position: absolute;
    right: 8px;
    width: 13px;
    height: 13px;
    color: var(--ink-faint);
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out);
  }
  .id-name-wrap:hover .id-pencil,
  .id-name:focus-visible ~ .id-pencil {
    opacity: 1;
  }
  .id-sub {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px 8px;
    font-size: 12px;
    color: var(--ink-mute);
    margin-top: 3px;
  }
  .id-sep {
    opacity: 0.6;
  }
  .id-ip {
    font-family: var(--mono);
    font-size: 11.5px;
  }
  .live-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--success);
    flex-shrink: 0;
    transition: background-color var(--ui-duration-base) var(--ui-ease-out);
  }
  .live-dot.off {
    background: var(--danger);
  }

  /* Transient notices (saved, copied, failed) */
  .notices {
    display: grid;
  }
  .notice {
    margin-top: 10px;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    background: var(--bg-elev);
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--ink-soft);
  }
  .notice.ok {
    background: var(--success-bg);
    border-color: var(--success-line);
    color: var(--success);
  }
  .notice.err {
    background: var(--danger-bg);
    border-color: var(--danger-line);
    color: var(--danger);
  }
  .notice-inner {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 9px 10px 9px 12px;
  }
  .notice-icon {
    width: 15px;
    height: 15px;
    margin-top: 1px;
    flex-shrink: 0;
  }
  .notice-text {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .notice-x {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    margin: -2px 0;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: inherit;
    opacity: 0.7;
    cursor: pointer;
    flex-shrink: 0;
    transition: opacity var(--ui-duration-fast) var(--ui-ease-out), background-color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .notice-x:hover {
    opacity: 1;
    background: color-mix(in srgb, currentColor 12%, transparent);
  }
  .notice-x:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .notice-x svg {
    width: 12px;
    height: 12px;
  }

  /* Device list. Each device is one container: its row, route editor and
     reminder share a border so the editor reads as part of the device. */
  .device-list {
    margin-top: 8px;
  }
  .device {
    container-type: inline-size;
    margin-top: 10px;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    background: var(--bg-elev);
    transition:
      border-color var(--ui-duration-base) var(--ui-ease-out),
      box-shadow var(--ui-duration-base) var(--ui-ease-out);
  }
  .device:first-child {
    margin-top: 0;
  }
  .device.is-open {
    border-color: var(--line-strong);
  }
  .device.is-error {
    border-color: var(--danger-line);
  }
  .device-row {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 14px;
    padding: 12px 14px;
  }
  .device-row > .tile {
    align-self: start;
  }
  .device-main {
    min-width: 0;
  }
  .device-top {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .device-name {
    overflow-wrap: anywhere;
    font-size: 13.5px;
    font-weight: 600;
    color: var(--ink-strong);
  }
  .device-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0 6px;
  }
  .device-error {
    overflow-wrap: anywhere;
    color: var(--danger);
    margin-top: 4px;
  }
  .device-actions {
    display: flex;
    gap: 8px;
  }
  .danger-ghost:hover {
    color: var(--danger);
    border-color: var(--danger);
  }
  .sync-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .sync-glyph {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
  }
  .sync-glyph.spin {
    animation: sync-spin 900ms linear infinite;
  }
  @keyframes sync-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @container (max-width: 540px) {
    .device-row {
      grid-template-columns: auto minmax(0, 1fr);
    }
    .device-actions {
      grid-column: 1 / -1;
      display: grid;
      grid-auto-flow: column;
      grid-auto-columns: minmax(0, 1fr);
    }
    .device-actions :global(button) {
      justify-content: center;
    }
  }
  /* Three labels no longer fit on one line: Sync now takes its own row. */
  @container (max-width: 400px) {
    .device-actions {
      grid-auto-flow: row;
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .device-actions .sync-btn,
    .device-actions > :global(:only-child) {
      grid-column: 1 / -1;
    }
  }

  /* Expanding regions inside a device or the setup card */
  .drawer {
    border-top: 1px solid var(--line);
  }
  .connection-editor {
    display: grid;
    gap: 8px;
    padding: 12px 14px 14px;
  }
  .connection-editor label,
  .field-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--ink-strong);
  }

  /* One input with its action beside it; stacked on narrow containers. */
  .field {
    display: flex;
    align-items: stretch;
    gap: 8px;
    margin-top: 8px;
  }
  .field-label {
    display: block;
    margin-top: 10px;
  }
  .field-label + .field {
    margin-top: 6px;
  }
  .field-input {
    flex: 1;
    min-width: 0;
    box-sizing: border-box;
    font-family: var(--mono);
    font-size: 12px;
  }
  .field-input.manual {
    width: 100%;
    margin-top: 8px;
  }
  .field-actions {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }
  .field-error {
    font-size: 12px;
    line-height: 1.45;
    color: var(--danger);
    overflow-wrap: anywhere;
    margin-top: 6px;
  }
  .connection-editor .field-error {
    margin-top: 0;
  }
  .sync-muting-error {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }

  .copy-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-width: 112px;
  }
  .copy-glyph {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
  }

  .nudge {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 14px;
    padding: 11px 14px;
    background: var(--control-hover);
    border-radius: 0 0 calc(var(--r-md) - 1px) calc(var(--r-md) - 1px);
  }
  .nudge-text {
    flex: 1 1 220px;
    min-width: 0;
  }
  .nudge-title {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--ink-strong);
  }
  .nudge-actions {
    display: flex;
    gap: 8px;
  }

  @container (max-width: 540px) {
    .field {
      flex-direction: column;
    }
    .field-actions {
      display: grid;
      grid-auto-flow: column;
      grid-auto-columns: minmax(0, 1fr);
    }
    .field-actions :global(button) {
      justify-content: center;
    }
    .field-actions .copy-action,
    .nudge-actions .copy-action {
      min-width: 0;
    }
    .nudge-actions {
      width: 100%;
      display: grid;
      grid-auto-flow: column;
      grid-auto-columns: minmax(0, 1fr);
    }
  }

  /* Status pill */
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.01em;
    padding: 2px 9px;
    border-radius: 999px;
    border: 1px solid var(--line);
    color: var(--ink-mute);
    white-space: nowrap;
  }
  .pill-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ink-faint);
    flex-shrink: 0;
  }
  .pill.synced {
    background: var(--success-bg);
    border-color: var(--success-line);
    color: var(--success);
  }
  .pill.synced .pill-dot {
    background: var(--success);
  }
  .pill.syncing,
  .pill.connecting {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent-ink);
  }
  .pill.syncing .pill-dot,
  .pill.connecting .pill-dot {
    background: var(--accent);
    animation: sync-pulse 1.2s ease-in-out infinite;
  }
  .pill.error {
    background: var(--danger-bg);
    border-color: var(--danger-line);
    color: var(--danger);
  }
  .pill.error .pill-dot {
    background: var(--danger);
  }
  @keyframes sync-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.35;
    }
  }

  /* Searching / empty nearby states */
  .discover-card {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 8px;
    padding: 18px 16px;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    color: var(--ink-mute);
    font-size: 12.5px;
  }
  .discover-empty {
    flex-direction: column;
    text-align: center;
    gap: 6px;
    padding: 26px 24px;
    border-style: dashed;
    border-color: var(--line-strong);
  }
  .radar {
    position: relative;
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    margin-bottom: 2px;
  }
  .radar i {
    position: absolute;
    inset: 0;
    border-radius: 50%;
    border: 1px solid var(--ink-faint);
    opacity: 0;
    animation: radar-ping 2.8s var(--ui-ease-out) infinite;
  }
  .radar i:nth-child(2) {
    animation-delay: 1.4s;
  }
  @keyframes radar-ping {
    0% {
      transform: scale(0.5);
      opacity: 0.55;
    }
    100% {
      transform: scale(1.15);
      opacity: 0;
    }
  }
  .discover-icon {
    position: relative;
    width: 22px;
    height: 22px;
    color: var(--ink-faint);
  }
  .discover-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--ink-soft);
  }
  .discover-hint {
    max-width: 400px;
    line-height: 1.5;
    font-size: 12px;
    color: var(--ink-mute);
  }
  .search-dots {
    display: inline-flex;
    gap: 3px;
    align-items: center;
    flex-shrink: 0;
  }
  .search-dots i {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--ink-faint);
    animation: dot-pulse 1.2s ease-in-out infinite;
  }
  .search-dots i:nth-child(2) {
    animation-delay: 150ms;
  }
  .search-dots i:nth-child(3) {
    animation-delay: 300ms;
  }
  @keyframes dot-pulse {
    0%,
    100% {
      opacity: 0.25;
    }
    50% {
      opacity: 1;
    }
  }

  .empty-note {
    display: grid;
    gap: 3px;
    padding: 2px 0 4px;
  }
  .empty-note .discover-hint {
    max-width: 56ch;
    text-align: left;
  }

  /* Tailscale setup */
  .setup {
    margin-top: 14px;
    container-type: inline-size;
    border: 1px solid var(--line);
    border-radius: var(--r-md);
    background: var(--bg-elev);
    transition: border-color var(--ui-duration-base) var(--ui-ease-out);
  }
  .setup.is-open {
    border-color: var(--line-strong);
  }
  .setup-toggle {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 12px 14px;
    border: 0;
    border-radius: var(--r-md);
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background-color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .setup-toggle:hover {
    background: var(--control-hover);
  }
  .setup-text {
    display: grid;
    gap: 1px;
    flex: 1;
    min-width: 0;
  }
  .setup-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--ink-strong);
  }
  .setup-sub {
    font-size: 12px;
    color: var(--ink-mute);
  }
  .chev {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    color: var(--ink-mute);
    transition: transform var(--ui-duration-base) var(--ui-ease-out);
  }
  [aria-expanded='true'] > .chev {
    transform: rotate(180deg);
  }
  .setup-intro {
    margin: 0;
    padding: 12px 14px 4px;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--ink-mute);
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 10px 14px 14px;
  }
  .step {
    position: relative;
    display: grid;
    grid-template-columns: 26px minmax(0, 1fr);
    gap: 12px;
    padding-bottom: 20px;
  }
  .step:last-child {
    padding-bottom: 2px;
  }
  /* Connector between markers; it fills once its step is done. */
  .step::before,
  .step::after {
    content: '';
    position: absolute;
    left: 12.5px;
    top: 30px;
    bottom: 4px;
    width: 1px;
    background: var(--line-strong);
  }
  .step::after {
    background: var(--accent);
    transform: scaleY(0);
    transform-origin: top;
    transition: transform var(--ui-duration-base) var(--ui-ease-out);
  }
  .step.done::after {
    transform: scaleY(1);
  }
  .step:last-child::before,
  .step:last-child::after {
    display: none;
  }
  .marker {
    position: relative;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    border: 1px solid var(--line-strong);
    background: var(--bg-elev);
    color: var(--ink-mute);
    font-size: 12px;
    font-weight: 600;
    transition:
      background-color var(--ui-duration-base) var(--ui-ease-out),
      border-color var(--ui-duration-base) var(--ui-ease-out),
      color var(--ui-duration-base) var(--ui-ease-out);
  }
  .marker .num,
  .marker .tick {
    grid-area: 1 / 1;
    transition:
      transform var(--ui-duration-base) var(--ui-ease-out),
      opacity var(--ui-duration-base) var(--ui-ease-out);
  }
  .marker .tick {
    width: 13px;
    height: 13px;
    opacity: 0;
    transform: scale(0.4);
  }
  .step.done .marker {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .step.done .marker .num {
    opacity: 0;
    transform: scale(0.4);
  }
  .step.done .marker .tick {
    opacity: 1;
    transform: scale(1);
  }
  .step-body {
    min-width: 0;
  }
  .step-title {
    display: block;
    font-size: 13px;
    font-weight: 600;
    color: var(--ink-strong);
    line-height: 26px;
    margin-top: -3px;
  }
  .step-body > .desc {
    margin-top: 0;
  }
  .addr {
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
    padding: 8px 11px;
    border: 1px solid var(--line);
    border-radius: var(--r-sm);
    background: var(--control-hover);
    font-family: var(--mono);
    font-size: 12.5px;
    color: var(--ink-strong);
    overflow-wrap: anywhere;
  }
  .addr-port {
    color: var(--ink-mute);
  }

  .tips-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 10px 14px;
    border: 0;
    border-top: 1px solid var(--line);
    border-radius: 0 0 var(--r-md) var(--r-md);
    background: transparent;
    color: var(--ink-mute);
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition:
      background-color var(--ui-duration-fast) var(--ui-ease-out),
      color var(--ui-duration-fast) var(--ui-ease-out);
  }
  .tips-toggle:hover {
    background: var(--control-hover);
    color: var(--ink-strong);
  }
  .tips {
    margin: 0;
    padding: 2px 14px 14px 32px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--ink-mute);
  }
  .tips li + li {
    margin-top: 6px;
  }

  /* Pairing and removal dialogs. They are moved to the body so the settings
     page transform cannot offset them, and sit above the settings layer like
     the incoming pairing prompt. */
  .sync-backdrop {
    z-index: 70;
  }
  .sync-dialog {
    z-index: 71;
  }
  .outgoing-card,
  .remove-card {
    width: min(400px, calc(100vw - 48px));
    padding: 20px;
    display: grid;
    gap: 14px;
  }
  .pair-head {
    display: flex;
    gap: 12px;
    align-items: center;
  }
  .pair-title {
    font-size: 14.5px;
    font-weight: 600;
    color: var(--ink-strong);
  }
  .pair-sub {
    font-size: 12.5px;
    color: var(--ink-mute);
    margin-top: 2px;
    line-height: 1.45;
  }
  .outgoing-code {
    display: flex;
    justify-content: center;
    gap: clamp(4px, 1.6vw, 7px);
    user-select: all;
  }
  .digit {
    display: grid;
    place-items: center;
    width: clamp(32px, 10vw, 46px);
    height: clamp(44px, 13vw, 58px);
    border-radius: var(--r-sm);
    border: 1px solid var(--line);
    background: var(--control-hover);
    color: var(--ink-strong);
    font-family: var(--mono);
    font-size: clamp(20px, 6.5vw, 28px);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
  .digit:nth-child(3) {
    margin-right: clamp(4px, 1.6vw, 8px);
  }
  .digit.placeholder {
    animation: dot-pulse 1.2s ease-in-out infinite;
  }
  .phase-track {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 4px;
  }
  .phase-seg {
    height: 3px;
    border-radius: 2px;
    background: var(--line);
    position: relative;
    overflow: hidden;
  }
  .phase-seg::after {
    content: '';
    position: absolute;
    inset: 0;
    background: var(--accent);
    transform: scaleX(0);
    transform-origin: left;
    transition: transform var(--ui-duration-base) var(--ui-ease-out);
  }
  .phase-seg.on::after {
    transform: scaleX(1);
  }
  .pair-wait {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 18px;
    font-size: 12px;
    color: var(--ink-mute);
  }
  .pair-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .sync-note {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin-top: 20px;
  }
  .note-icon {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    margin-top: 1px;
    opacity: 0.7;
  }

  /* Looping indicators stop for users who prefer reduced motion; labels and
     colors still carry each state. */
  @media (prefers-reduced-motion: reduce) {
    .sync-glyph.spin,
    .pill.syncing .pill-dot,
    .pill.connecting .pill-dot,
    .search-dots i,
    .digit.placeholder {
      animation: none;
    }
    .radar i {
      animation: none;
      opacity: 0;
    }
  }
</style>
