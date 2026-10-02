<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '../tauri';
  import { sessionInfo, sessionShareLink, sessionRequest, loadSessionFixture, submitSessionAudio, normalizeBrowserAudio, type SessionInfo } from '../devSession';
  let session = $state<SessionInfo | null>(sessionInfo());
  let open = $state(false);
  let fixtures = $state<string[]>([]);
  let selected = $state('');
  let contexts = $state<{ id: number; name: string }[]>([]);
  let context = $state('');
  let processName = $state('browser-test');
  let domain = $state('');
  let result = $state('');
  let busy = $state(false);
  let message = $state('Ready to test the production pipeline');
  let failed = $state(false);
  let logs = $state<string[]>([]);
  let recording = $state(false);
  let hostRecording = $state(false);
  let recorder: MediaRecorder | null = null;
  let stream: MediaStream | null = null;
  let recordingTimer: ReturnType<typeof setTimeout> | undefined;

  function fail(error: unknown) { failed = true; message = error instanceof Error ? error.message : 'The test failed'; }
  async function refresh() {
    try {
      const [next, list, rows] = await Promise.all([sessionRequest<SessionInfo>('/session'), sessionRequest<{ fixtures: string[] }>('/fixtures'), invoke<{ id: number; name: string }[]>('get_contexts')]);
      session = next; fixtures = list.fixtures; contexts = rows;
      if (!selected) selected = fixtures[0] || '';
    } catch (error) { fail(error); }
  }
  async function runAudio(bytes: Uint8Array<ArrayBuffer>) {
    busy = true; failed = false; result = ''; message = 'Running real transcription and cleanup…';
    try {
      const response = await submitSessionAudio(bytes, { context: context ? Number(context) : undefined, process: processName, domain });
      result = response.text; message = 'Production pipeline completed. Result saved to this session’s history.';
    } catch (error) { fail(error); }
    finally { busy = false; await refresh(); }
  }
  async function runFixture() {
    busy = true;
    try { await runAudio(await loadSessionFixture(selected)); } catch (error) { fail(error); busy = false; }
  }
  async function upload(event: Event) {
    const file = (event.target as HTMLInputElement).files?.[0];
    if (!file) return;
    busy = true;
    try { await runAudio(await normalizeBrowserAudio(file)); } catch (error) { fail(error); busy = false; }
    (event.target as HTMLInputElement).value = '';
  }
  async function toggleBrowserMic() {
    if (recording) { recorder?.stop(); return; }
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      recorder = new MediaRecorder(stream);
      const chunks: Blob[] = [];
      recorder.ondataavailable = (event) => { if (event.data.size) chunks.push(event.data); };
      recorder.onstop = () => {
        if (recordingTimer) clearTimeout(recordingTimer);
        stream?.getTracks().forEach((track) => track.stop()); recording = false; busy = true;
        void normalizeBrowserAudio(new Blob(chunks, { type: recorder?.mimeType })).then(runAudio).catch((error) => { fail(error); busy = false; });
      };
      recorder.start(); recording = true; failed = false; message = 'Recording this browser’s microphone. Stop to run dictation.';
      recordingTimer = setTimeout(() => recorder?.stop(), 119_000);
    } catch (error) { stream?.getTracks().forEach((track) => track.stop()); fail(error); }
  }
  async function toggleHostMic() {
    busy = true; failed = false;
    try {
      if (hostRecording) {
        await invoke('stop_setup_try_recording'); hostRecording = false;
        message = 'Host dictation is processing. Watch the result and session history.';
      } else {
        await invoke('start_setup_try_recording'); hostRecording = true;
        message = 'Recording the desktop microphone. Stop to run real dictation.';
      }
    } catch (error) { fail(error); }
    finally { busy = false; }
  }
  async function showLogs() {
    try { logs = (await sessionRequest<{ lines: string[] }>('/logs')).lines; } catch (error) { fail(error); }
  }
  async function copyPhoneLink() {
    const link = sessionShareLink();
    if (!link) return;
    try { await navigator.clipboard.writeText(link); message = 'Private phone access link copied.'; }
    catch { fail(new Error('Could not copy the link. Use this session’s private access.json file.')); }
  }
  onMount(() => {
    void refresh();
    const connection = (event: Event) => fail(new Error((event as CustomEvent<string>).detail));
    window.addEventListener('verenu:dev-connection', connection);
    let unlisten: (() => void) | undefined;
    void import('../tauri').then(({ listen }) => listen<string>('verenu:transcribed', (event) => { result = event.payload; message = 'Production dictation completed.'; })).then((cleanup) => { unlisten = cleanup; });
    return () => { window.removeEventListener('verenu:dev-connection', connection); unlisten?.(); if (recordingTimer) clearTimeout(recordingTimer); stream?.getTracks().forEach((track) => track.stop()); };
  });
</script>

<div class="dev-session-launcher">
  <button class="btn-ghost btn-compact" aria-expanded={open} aria-controls="dev-session-panel" onclick={() => { open = !open; if (open) void refresh(); }}>Dev tests{failed ? ' · needs attention' : ''}</button>
</div>
{#if open}
  <aside id="dev-session-panel" class="dev-session-panel" aria-label="Dev session tests">
    <div class="dev-heading">
      <div><h2>Dev session</h2><p>{session?.branch} · {session?.commit.slice(0, 8)}</p></div>
      <button class="btn-ghost btn-compact" onclick={() => open = false}>Close</button>
    </div>
    <p class="dev-note">Real Rust backend · isolated database · live providers</p>
    {#if session?.shareUrl}<button class="btn-ghost" onclick={copyPhoneLink}>Copy phone access link</button>{/if}
    {#if session?.privateHistory}<p class="dev-warning">Private history is present. Keep screenshots and reports local.</p>{/if}
    <p class="dev-note">{session?.runs ?? 0} / {session?.maxRuns ?? 0} live runs used. Native hotkeys and desktop insertion require separate verification.</p>
    <div class="dev-fields">
      <label>Context<select class="ui-dropdown-trigger ui-dropdown-trigger--compact" bind:value={context}><option value="">Resolve from app and website</option>{#each contexts as row}<option value={String(row.id)}>{row.name}</option>{/each}</select></label>
      <label>Target app<input bind:value={processName} placeholder="browser-test" /></label>
      <label>Website domain<input bind:value={domain} placeholder="example.com" /></label>
      <label>Audio fixture<select class="ui-dropdown-trigger ui-dropdown-trigger--compact" bind:value={selected}><option value="">Choose a fixture</option>{#each fixtures as name}<option value={name}>{name}</option>{/each}</select></label>
    </div>
    {#if !fixtures.length}<p class="dev-note">No fixtures yet. Import audio below or launch with --fixtures pointing to your generated WAV clips.</p>{/if}
    <div class="dev-actions">
      <button class="btn-primary" disabled={busy || recording || hostRecording || !selected} onclick={runFixture}>Run fixture</button>
      <button class="btn-ghost" disabled={busy || hostRecording} onclick={toggleBrowserMic}>{recording ? 'Stop browser recording' : 'Record browser mic'}</button>
      {#if session?.capabilities.hostMicrophone}<button class="btn-ghost" disabled={busy || recording} onclick={toggleHostMic}>{hostRecording ? 'Stop desktop recording' : 'Record desktop mic'}</button>{/if}
      <label class="dev-upload">Import audio<input type="file" accept="audio/*" disabled={busy || recording || hostRecording} onchange={upload} /></label>
    </div>
    <p role="status" class:dev-warning={failed}>{message}</p>
    <label class="dev-result">Dictation target<textarea aria-label="Dictation test result" bind:value={result} rows="5" placeholder="The real pipeline result appears here"></textarea></label>
    <details><summary>Redacted backend logs</summary><button class="btn-ghost btn-compact" onclick={showLogs}>Refresh logs</button><pre>{logs.join('\n') || 'Refresh to inspect this session’s logs.'}</pre></details>
  </aside>
{/if}

<style>
  .dev-session-launcher { position: fixed; right: 16px; bottom: calc(16px + env(safe-area-inset-bottom)); z-index: 10000; background: var(--paper); border-radius: var(--radius-md, 8px); box-shadow: 0 2px 12px #0002; }
  .dev-session-panel { position: fixed; right: 16px; bottom: calc(64px + env(safe-area-inset-bottom)); z-index: 10001; width: min(640px, calc(100vw - 32px)); max-height: calc(100dvh - 100px); overflow: auto; padding: 24px; box-sizing: border-box; border: 1px solid var(--line); border-radius: var(--radius-lg, 12px); background: var(--paper); color: var(--ink); box-shadow: 0 8px 32px #0003; display: grid; gap: 16px; }
  .dev-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }
  h2, p { margin: 0; }
  h2 { font-family: var(--font-display); font-size: 24px; }
  .dev-heading p { font-size: 12px; overflow-wrap: anywhere; margin-top: 6px; }
  .dev-note { color: var(--ink-secondary, var(--ink)); font-size: 13px; line-height: 1.5; }
  .dev-warning { color: var(--error, #a13925); font-size: 13px; line-height: 1.5; }
  .dev-fields { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  label { display: grid; gap: 6px; font-size: 13px; min-width: 0; }
  input, select, textarea { box-sizing: border-box; width: 100%; min-height: 44px; background: var(--paper); color: var(--ink); border: 1px solid var(--line); border-radius: var(--radius-sm, 6px); padding: 8px; font: inherit; }
  textarea { resize: vertical; line-height: 1.5; }
  .dev-actions { display: flex; flex-wrap: wrap; gap: 8px; }
  .dev-actions button { min-height: 44px; }
  .dev-upload { width: 100%; }
  summary { cursor: pointer; padding: 8px 0; }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: 11px; max-height: 240px; overflow: auto; }
  @media (max-width: 500px) { .dev-session-panel { padding: 16px; } .dev-fields { grid-template-columns: 1fr; } .dev-actions > button { flex: 1 1 100%; } }
</style>
